use std::cell::RefCell;
use std::rc::Rc;

use serde::Deserialize;

const MAX_RETRY_ATTEMPTS: u8 = 5;
const INITIAL_RETRY_DELAY_MS: i32 = 1_000;

#[derive(Debug, Default)]
struct CredentialSyncState {
    server_persistence: Option<bool>,
    pending: Option<String>,
    in_flight: Option<String>,
    policy_in_flight: bool,
    retry_attempts: u8,
    retry_generation: u64,
    /// Latest sync failure worth showing the user; cleared on success.
    last_error: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum SyncAction {
    None,
    FetchPolicy,
    Post(String),
    ScheduleRetry { delay_ms: i32, generation: u64 },
}

impl CredentialSyncState {
    // Combined reset + policy-check. The mount now drives these two phases
    // separately (`reset` early, `start` after the bridge gate), so this stays
    // as the state-machine's tested "full start" shape.
    #[cfg(test)]
    fn start(&mut self) -> SyncAction {
        *self = Self::default();
        self.begin_policy_check()
    }

    fn changed(&mut self, json: String) -> SyncAction {
        self.pending = Some(json);
        self.retry_attempts = 0;
        // A fresh user edit supersedes the failed one — drop the stale banner
        // now instead of leaving it up until the corrective post resolves.
        self.last_error = None;
        self.invalidate_retry();
        if self.policy_in_flight || self.in_flight.is_some() {
            return SyncAction::None;
        }
        self.begin_policy_check()
    }

    fn resolve_policy(&mut self, server_persistence: Option<bool>) -> SyncAction {
        self.policy_in_flight = false;
        self.server_persistence = server_persistence;

        match server_persistence {
            Some(false) => {
                self.pending = None;
                self.retry_attempts = 0;
                // Persistence is off — a prior save failure is moot.
                self.last_error = None;
                self.invalidate_retry();
                SyncAction::None
            }
            None => self.schedule_retry(),
            Some(true) if self.in_flight.is_some() => SyncAction::None,
            Some(true) => {
                self.invalidate_retry();
                let Some(json) = self.pending.take() else {
                    return SyncAction::None;
                };
                self.in_flight = Some(json.clone());
                SyncAction::Post(json)
            }
        }
    }

    fn complete(&mut self, status: Option<u16>) -> SyncAction {
        let Some(completed) = self.in_flight.take() else {
            return SyncAction::None;
        };

        if status == Some(403) {
            // Persistence disabled by deployment policy — expected on the
            // public demo, not an error worth surfacing.
            self.server_persistence = Some(false);
            self.pending = None;
            self.retry_attempts = 0;
            self.last_error = None;
            self.invalidate_retry();
            return SyncAction::None;
        }

        if status.is_some_and(|status| (200..300).contains(&status)) {
            self.retry_attempts = 0;
            self.last_error = None;
            self.invalidate_retry();
            if self.pending.is_some() {
                return self.begin_policy_check();
            }
            return SyncAction::None;
        }

        if status.is_some_and(is_deterministic_rejection) {
            // The daemon validated and refused this exact payload — replaying
            // it can never succeed. Drop it, record the failure for the UI,
            // and still flush any newer (possibly corrected) pending change.
            self.last_error = Some(format!(
                "server rejected the credential snapshot ({})",
                status.expect("status checked above")
            ));
            self.retry_attempts = 0;
            self.invalidate_retry();
            if self.pending.is_some() {
                return self.begin_policy_check();
            }
            return SyncAction::None;
        }

        self.last_error = Some(status.map_or_else(
            || "could not reach the server to save credentials".to_string(),
            |status| format!("saving credentials to the server failed ({status})"),
        ));
        if self.pending.is_none() {
            self.pending = Some(completed);
        }
        self.schedule_retry()
    }

    #[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
    fn retry_due(&mut self, generation: u64) -> SyncAction {
        if generation != self.retry_generation
            || self.pending.is_none()
            || self.policy_in_flight
            || self.in_flight.is_some()
        {
            return SyncAction::None;
        }
        self.begin_policy_check()
    }

    fn begin_policy_check(&mut self) -> SyncAction {
        self.policy_in_flight = true;
        SyncAction::FetchPolicy
    }

    fn schedule_retry(&mut self) -> SyncAction {
        if self.pending.is_none() || self.retry_attempts >= MAX_RETRY_ATTEMPTS {
            return SyncAction::None;
        }
        self.retry_attempts += 1;
        self.retry_generation = self.retry_generation.wrapping_add(1);
        let shift = u32::from(self.retry_attempts.saturating_sub(1));
        let delay_ms = INITIAL_RETRY_DELAY_MS.saturating_mul(1_i32 << shift);
        SyncAction::ScheduleRetry {
            delay_ms,
            generation: self.retry_generation,
        }
    }

    fn invalidate_retry(&mut self) {
        self.retry_generation = self.retry_generation.wrapping_add(1);
    }
}

/// 4xx statuses that re-sending the identical payload cannot fix. 408
/// (timeout) and 429 (rate limit) are transient and keep the retry path.
fn is_deterministic_rejection(status: u16) -> bool {
    (400..500).contains(&status) && status != 408 && status != 429
}

#[derive(Deserialize)]
struct CredentialPolicyResponse {
    #[serde(rename = "serverPersistence")]
    server_persistence: bool,
}

thread_local! {
    static SYNC_STATE: RefCell<CredentialSyncState> = RefCell::new(CredentialSyncState::default());
}

/// Clear the transient credential-sync queue. Split out of `start` so the
/// mount can reset state BEFORE repaint callbacks are wired: `repaint` calls
/// `credential_changed` when a credential edit lands, so an early repaint must
/// not queue a change that a later reset silently wipes. This issues no daemon
/// request, so it is safe to run ahead of the postMessage bridge init gate.
pub(crate) fn reset() {
    SYNC_STATE.with(|state| *state.borrow_mut() = CredentialSyncState::default());
}

/// Begin credential-policy discovery against the daemon. This issues a daemon
/// request, so the mount calls it only AFTER the bridge init gate (managed mode
/// needs the auth token on the wire). State must already be cleared via `reset`.
pub(crate) fn start() {
    let action = SYNC_STATE.with(|state| state.borrow_mut().begin_policy_check());
    dispatch(action);
}

pub(crate) fn credential_changed(json: String) {
    let action = SYNC_STATE.with(|state| state.borrow_mut().changed(json));
    dispatch(action);
}

/// Latest credential-sync failure worth showing in the settings modal;
/// `None` after a successful sync. The repaint loop mirrors this into
/// `AgentSettings::web_credential_sync_error`.
pub(crate) fn current_sync_error() -> Option<String> {
    SYNC_STATE.with(|state| state.borrow().last_error.clone())
}

fn parse_policy_response(status: u16, body: &str) -> Option<bool> {
    if !(200..300).contains(&status) {
        return None;
    }
    serde_json::from_str::<CredentialPolicyResponse>(body)
        .ok()
        .map(|response| response.server_persistence)
}

fn dispatch(action: SyncAction) {
    match action {
        SyncAction::None => {}
        SyncAction::FetchPolicy => fetch_policy(),
        SyncAction::Post(body) => post_credentials(&body),
        SyncAction::ScheduleRetry {
            delay_ms,
            generation,
        } => schedule_retry(delay_ms, generation),
    }
}

/// Repaint after an async sync step mutates the UI-visible error state. The
/// settings modal mirrors `last_error` only during paint, and these callbacks
/// fire from XHR completions that schedule no frame of their own — without
/// this the banner would appear (or clear) only on the next unrelated repaint.
/// No-op outside the browser (test path) and coalesced to one paint per frame.
fn request_repaint() {
    crate::repaint_coalescer::request();
}

fn fetch_policy() {
    let base = crate::daemon_base::daemon_base();
    let on_response: Rc<dyn Fn(u16, String)> = Rc::new(|status, body| {
        let policy = parse_policy_response(status, &body);
        if policy.is_none() {
            report_sync_failure(Some(status));
        }
        let action = SYNC_STATE.with(|state| state.borrow_mut().resolve_policy(policy));
        request_repaint();
        dispatch(action);
    });
    if !crate::live_sync::get_with_status(
        &format!("{base}/api/settings/credential-policy"),
        on_response,
    ) {
        report_sync_failure(None);
        let action = SYNC_STATE.with(|state| state.borrow_mut().resolve_policy(None));
        request_repaint();
        dispatch(action);
    }
}

fn post_credentials(body: &str) {
    let base = crate::daemon_base::daemon_base();
    let on_response: Rc<dyn Fn(u16, String)> = Rc::new(|status, _| {
        if !(200..300).contains(&status) {
            report_sync_failure(Some(status));
        }
        complete_in_flight(Some(status));
    });
    if !crate::live_sync::post_json_with_status(
        &format!("{base}/api/settings/credentials"),
        body,
        on_response,
    ) {
        report_sync_failure(None);
        complete_in_flight(None);
    }
}

fn complete_in_flight(status: Option<u16>) {
    let action = SYNC_STATE.with(|state| state.borrow_mut().complete(status));
    request_repaint();
    dispatch(action);
}

fn schedule_retry(delay_ms: i32, generation: u64) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;

        let callback = wasm_bindgen::closure::Closure::<dyn FnMut()>::once_into_js(move || {
            let action = SYNC_STATE.with(|state| state.borrow_mut().retry_due(generation));
            dispatch(action);
        });
        let scheduled = web_sys::window().is_some_and(|window| {
            window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.unchecked_ref(),
                    delay_ms,
                )
                .is_ok()
        });
        if !scheduled {
            report_sync_failure(None);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = (delay_ms, generation);
}

fn report_sync_failure(status: Option<u16>) {
    #[cfg(target_arch = "wasm32")]
    {
        let message = status.map_or_else(
            || "OpenPencil could not synchronize server credentials".to_string(),
            |status| format!("OpenPencil server credential synchronization failed ({status})"),
        );
        web_sys::console::warn_1(&wasm_bindgen::JsValue::from_str(&message));
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = status;
}

#[cfg(test)]
mod tests {
    use super::{parse_policy_response, CredentialSyncState, SyncAction, MAX_RETRY_ATTEMPTS};

    #[test]
    fn pending_change_waits_for_policy_and_is_dropped_when_disabled() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.start(), SyncAction::FetchPolicy);
        assert_eq!(sync.changed("secret-json".into()), SyncAction::None);
        assert_eq!(sync.resolve_policy(Some(false)), SyncAction::None);
        assert!(sync.pending.is_none());
    }

    #[test]
    fn pending_change_posts_after_fresh_server_policy_is_enabled() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.start(), SyncAction::FetchPolicy);
        assert_eq!(sync.changed("secret-json".into()), SyncAction::None);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("secret-json".into())
        );
    }

    #[test]
    fn credential_change_rechecks_cached_policy_before_posting() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.start(), SyncAction::FetchPolicy);
        assert_eq!(sync.resolve_policy(Some(true)), SyncAction::None);

        assert_eq!(sync.changed("secret-json".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("secret-json".into())
        );
    }

    #[test]
    fn later_change_rechecks_a_cached_disabled_policy() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.start(), SyncAction::FetchPolicy);
        assert_eq!(sync.resolve_policy(Some(false)), SyncAction::None);

        assert_eq!(sync.changed("new-secret".into()), SyncAction::FetchPolicy);
        assert_eq!(sync.pending.as_deref(), Some("new-secret"));
    }

    #[test]
    fn enabled_policy_posts_one_at_a_time_and_rechecks_for_latest_pending_change() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );
        assert_eq!(sync.changed("credential-b".into()), SyncAction::None);
        assert_eq!(sync.changed("credential-c".into()), SyncAction::None);
        assert_eq!(sync.pending.as_deref(), Some("credential-c"));

        assert_eq!(sync.complete(Some(204)), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-c".into())
        );
        assert_eq!(sync.complete(Some(204)), SyncAction::None);
    }

    #[test]
    fn failed_final_post_retries_after_a_bounded_delay() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );

        let action = sync.complete(Some(500));
        let SyncAction::ScheduleRetry { generation, .. } = action else {
            panic!("a failed post should schedule a retry");
        };
        assert_eq!(sync.retry_due(generation), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );
    }

    #[test]
    fn repeated_post_failures_stop_after_the_retry_limit() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );

        for _ in 0..MAX_RETRY_ATTEMPTS {
            let SyncAction::ScheduleRetry { generation, .. } = sync.complete(Some(500)) else {
                panic!("retry budget should not be exhausted yet");
            };
            assert_eq!(sync.retry_due(generation), SyncAction::FetchPolicy);
            assert_eq!(
                sync.resolve_policy(Some(true)),
                SyncAction::Post("credential-a".into())
            );
        }

        assert_eq!(sync.complete(Some(500)), SyncAction::None);
        assert_eq!(sync.retry_attempts, MAX_RETRY_ATTEMPTS);
    }

    #[test]
    fn deterministic_client_error_does_not_retry_the_same_payload() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("bad-snapshot".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("bad-snapshot".into())
        );

        // A 400 is the daemon telling us this exact payload can never be
        // accepted — replaying it is pointless. No retry, nothing requeued.
        assert_eq!(sync.complete(Some(400)), SyncAction::None);
        assert!(sync.pending.is_none());
        assert!(sync.last_error.is_some());
    }

    #[test]
    fn deterministic_client_error_still_flushes_a_newer_pending_change() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("bad-snapshot".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("bad-snapshot".into())
        );
        assert_eq!(sync.changed("fixed-snapshot".into()), SyncAction::None);

        // The rejected payload is dropped, but the user's newer edit (which
        // may fix the rejection) still syncs.
        assert_eq!(sync.complete(Some(400)), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("fixed-snapshot".into())
        );
    }

    #[test]
    fn transient_client_statuses_still_retry() {
        for status in [408_u16, 429] {
            let mut sync = CredentialSyncState::default();
            assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
            assert_eq!(
                sync.resolve_policy(Some(true)),
                SyncAction::Post("credential-a".into())
            );
            assert!(
                matches!(
                    sync.complete(Some(status)),
                    SyncAction::ScheduleRetry { .. }
                ),
                "status {status} is transient and must retry"
            );
        }
    }

    #[test]
    fn starting_a_corrective_change_clears_the_stale_error() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("bad-snapshot".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("bad-snapshot".into())
        );
        assert_eq!(sync.complete(Some(400)), SyncAction::None);
        assert!(sync.last_error.is_some());

        // The user edits again to fix it — the stale banner must clear
        // immediately, not linger until the corrective post resolves.
        sync.changed("fixed-snapshot".into());
        assert!(sync.last_error.is_none());
    }

    #[test]
    fn disabled_persistence_clears_any_recorded_error() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("bad-snapshot".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("bad-snapshot".into())
        );
        assert_eq!(sync.complete(Some(400)), SyncAction::None);
        assert!(sync.last_error.is_some());

        // Deployment turned server persistence off — the error is moot; a
        // browser-only session must not keep showing a save failure.
        sync.changed("later".into());
        assert_eq!(sync.resolve_policy(Some(false)), SyncAction::None);
        assert!(sync.last_error.is_none());
    }

    #[test]
    fn forbidden_post_clears_a_recorded_error() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );
        // Transport failure records an error and schedules a retry.
        let SyncAction::ScheduleRetry { generation, .. } = sync.complete(None) else {
            panic!("transport failure should schedule a retry");
        };
        assert!(sync.last_error.is_some());
        assert_eq!(sync.retry_due(generation), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );

        // The retried post comes back 403 (persistence now disabled): the
        // stale transport error must clear, not linger.
        assert_eq!(sync.complete(Some(403)), SyncAction::None);
        assert!(sync.last_error.is_none());
    }

    #[test]
    fn success_clears_a_recorded_sync_error() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("bad-snapshot".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("bad-snapshot".into())
        );
        assert_eq!(sync.complete(Some(400)), SyncAction::None);
        assert!(sync.last_error.is_some());

        assert_eq!(
            sync.changed("fixed-snapshot".into()),
            SyncAction::FetchPolicy
        );
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("fixed-snapshot".into())
        );
        assert_eq!(sync.complete(Some(204)), SyncAction::None);
        assert!(sync.last_error.is_none());
    }

    #[test]
    fn forbidden_post_demotes_policy_and_drops_pending_secrets() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );
        assert_eq!(sync.changed("credential-b".into()), SyncAction::None);

        assert_eq!(sync.complete(Some(403)), SyncAction::None);
        assert_eq!(sync.server_persistence, Some(false));
        assert!(sync.pending.is_none());
    }

    #[test]
    fn stale_retry_is_ignored_after_a_new_change() {
        let mut sync = CredentialSyncState::default();
        assert_eq!(sync.changed("credential-a".into()), SyncAction::FetchPolicy);
        assert_eq!(
            sync.resolve_policy(Some(true)),
            SyncAction::Post("credential-a".into())
        );
        let SyncAction::ScheduleRetry { generation, .. } = sync.complete(None) else {
            panic!("transport failure should schedule a retry");
        };

        assert_eq!(sync.changed("credential-b".into()), SyncAction::FetchPolicy);
        assert_eq!(sync.retry_due(generation), SyncAction::None);
    }

    #[test]
    fn policy_response_requires_success_status_and_valid_boolean_json() {
        assert_eq!(
            parse_policy_response(500, r#"{"serverPersistence":true}"#),
            None
        );
        assert_eq!(parse_policy_response(200, "{}"), None);
        assert_eq!(
            parse_policy_response(200, r#"{"serverPersistence":"true"}"#),
            None
        );
        assert_eq!(
            parse_policy_response(200, r#"{"serverPersistence":true}"#),
            Some(true)
        );
        assert_eq!(
            parse_policy_response(200, r#"{"serverPersistence":false}"#),
            Some(false)
        );
    }
}
