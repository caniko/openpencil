//! Live-canvas sync glue — bidirectional document + selection sync between
//! the browser shell and the web-canvas daemon. The Rust counterpart of the
//! TS `apps/web/src/hooks/use-mcp-sync.ts`:
//!
//! * **Pull** (external MCP/CLI writes → browser canvas): a 400 ms tick
//!   probes `GET /api/mcp/version` and fetches + applies the full document
//!   only when the daemon's monotonic version advanced. TS receives
//!   `document:update` pushes over SSE instead — a documented transport
//!   divergence (the daemon's SSE stream carries version bumps only, and the
//!   verified XHR machinery is reused); worst-case latency is one tick. Every
//!   pull decision — the tick gate AND the apply-time re-check — is keyed on
//!   the CURRENT `(document_generation, document_revision)` pair via
//!   [`op_editor_core::sync_gate::SyncGate`], not on any cheap dirty hint.
//! * **Push** (browser edits → daemon, so external MCP/CLI clients see them):
//!   a 2000 ms tick — the TS `PUSH_DEBOUNCE_MS` cadence — serializes the live
//!   document when `SyncGate::needs_push` says the current pair moved past
//!   the last-synced baseline (the sole authoritative gate; a stored conflict
//!   holds it closed until explicit resolution), skips the actual POST when
//!   the content hash matches the daemon baseline (re-baselining directly so
//!   the gate can't deadlock), and POSTs `{document, baseVersion}` to
//!   `/api/mcp/document`. Pushes over 2 MiB are skipped with a one-shot
//!   console warning (TS `SYNC_MAX_BODY_BYTES` parity), baseline untouched. A
//!   `version-conflict` response suspends the gate until the host resolves it
//!   (no silent retry); other failures are dropped best-effort (TS catch{}
//!   parity) — the next local edit retriggers.
//! * **Selection push**: the 400 ms tick also samples the selection key and
//!   POSTs `{selectedIds, activePageId}` to `/api/mcp/selection` when it
//!   changed (TS debounces 300 ms; a 400 ms trailing sample is the same
//!   order of latency). One-way browser → daemon, exactly like TS.
//!
//! Architectural divergence (documented): TS's BROWSER is the document
//! authority (it pushes its document on `client:id` and the Nitro server only
//! caches), while the Rust daemon is the authority after mount — so this glue
//! never pushes before the first daemon document has been applied (subsumed
//! by `SyncGate::needs_push`'s None-baseline rule). The static host page calls
//! `/api/mcp/sync-reset` before mounting so a browser refresh starts from the
//! starter document instead of replaying the previous transient web `.op`
//! state; bootstrap pushes remain disabled so stale page state cannot
//! overwrite a deliberately opened daemon document. Live screenshot requests
//! are served by the `--serve-web` daemon's MCP route from this same synced
//! document authority, rather than by browser-side capture.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use op_editor_core::sync_gate::SyncGate;
use op_editor_core::web_sync::{self, WebSyncClient};

use crate::live_sync;
use crate::repaint_ctx::RepaintContext;

/// Pull cadence (version probe). TS gets SSE pushes; one tick of latency.
const POLL_INTERVAL_MS: i32 = 400;
/// Push cadence — TS `PUSH_DEBOUNCE_MS` (use-mcp-sync.ts:6).
const PUSH_INTERVAL_MS: i32 = 2000;
/// TS `SYNC_MAX_BODY_BYTES` (use-mcp-sync.ts:10): documents larger than this
/// are not pushed (warned once), mirroring the renderer's oversize guard.
const SYNC_MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Shared sync state: the gate deciding pull/push eligibility, the wire-level
/// client (version/hash bookkeeping), and the push single-flight latch. Built
/// once in `mount_ck` and handed to both this module's ticks and the Task 7
/// postMessage bridge, so both sides observe and mutate the exact same
/// `SyncGate` instance (a v1 defect — the bridge couldn't reach a local
/// `WebSyncClient` — is fixed by sharing this one struct).
pub(crate) struct SyncController {
    pub gate: SyncGate,
    pub client: WebSyncClient,
    pub push_busy: bool,
}

impl SyncController {
    pub(crate) fn new() -> Self {
        Self {
            gate: SyncGate::default(),
            client: WebSyncClient::new(),
            push_busy: false,
        }
    }
}

pub(crate) type SharedSync = Rc<RefCell<SyncController>>;

/// The document-identity pair every gating decision is keyed on. Read fresh
/// from the live editor state at each decision point — never cached — so an
/// edit that lands between a tick firing and its async response landing is
/// always observed.
fn current_pair<C: RepaintContext>(b: &C) -> (u64, u64) {
    let s = b.host().editor_state();
    (s.document_generation(), s.document_revision())
}

/// Wire the bidirectional sync loops onto the mounted shell. Called once from
/// `mount_ck`; both intervals run for the page lifetime. `sync` is shared with
/// the Task 7 bridge — this module only ever borrows it for the duration of a
/// single decision, never across an await/callback boundary.
pub(crate) fn start<C: RepaintContext + 'static>(inner: &Rc<RefCell<C>>, sync: SharedSync) {
    let base = crate::daemon_base::daemon_base();
    // One document fetch / one push at a time; ticks observing an in-flight
    // request skip (the TS hook queues at most one — same effective shape).
    // `fetch_busy` stays a plain Cell (pull-side, local to this module);
    // `push_busy` moved onto the shared controller so the bridge can also
    // observe it.
    let fetch_busy = Rc::new(Cell::new(false));
    let oversize_warned = Rc::new(Cell::new(false));
    // `None` forces a selection re-push on the next tick (used after a doc
    // apply, which resets daemon-side selection). Seeded with the current key
    // so mount does not push an initial no-op selection (TS pushes selection
    // only on change).
    let last_selection_key = Rc::new(RefCell::new(Some(web_sync::selection_sync_key(
        inner.borrow().host().editor_state(),
    ))));

    // ---- pull + selection tick ----
    {
        let inner = inner.clone();
        let sync = sync.clone();
        let base = base.clone();
        let fetch_busy = fetch_busy.clone();
        let last_selection_key = last_selection_key.clone();
        let tick: Rc<dyn Fn()> = Rc::new(move || {
            poll_version(&inner, &base, &sync, &fetch_busy, &last_selection_key);
            push_selection_if_changed(&inner, &base, &last_selection_key);
        });
        let _ = live_sync::start_interval(POLL_INTERVAL_MS, tick);
    }

    // ---- document push tick ----
    {
        let inner = inner.clone();
        let sync = sync.clone();
        let tick: Rc<dyn Fn()> = Rc::new(move || {
            push_document_if_changed(&inner, &base, &sync, &oversize_warned);
        });
        let _ = live_sync::start_interval(PUSH_INTERVAL_MS, tick);
    }
}

/// Probe the daemon version; on a newer version fetch + apply the document.
/// Gated on the sync-gate FIRST (before any network round-trip): an accept
/// window broken by an intervening local edit re-enters the conflict flow,
/// and otherwise `pull_allowed` must hold for the current pair.
fn poll_version<C: RepaintContext + 'static>(
    inner: &Rc<RefCell<C>>,
    base: &str,
    sync: &SharedSync,
    fetch_busy: &Rc<Cell<bool>>,
    last_selection_key: &Rc<RefCell<Option<String>>>,
) {
    if fetch_busy.get() {
        return;
    }
    let pair = current_pair(&*inner.borrow());
    // Borrow discipline: Rust 2021 extends an `if let` scrutinee's temporary
    // borrow through the whole arm body, so a `borrow_mut()` inside the body
    // below would panic at runtime. Binding the `Option` first lets the
    // `borrow()` temporary drop at the end of THIS statement instead.
    let broken = sync.borrow().gate.accept_window_broken(pair); // borrow ends HERE
    if let Some(v) = broken {
        sync.borrow_mut().gate.note_conflict(v); // observer latch reports; user decides again
        return;
    }
    if !sync.borrow().gate.pull_allowed(pair) {
        return;
    }

    let inner = inner.clone();
    let sync = sync.clone();
    let base_owned = base.to_string();
    let fetch_busy = fetch_busy.clone();
    let last_selection_key = last_selection_key.clone();
    let on_version: Rc<dyn Fn(String)> = Rc::new(move |body: String| {
        let Some(version) = WebSyncClient::parse_version_probe(&body) else {
            return; // daemon down / non-JSON error body — retry next tick
        };
        let wants_version = sync
            .try_borrow()
            .map(|s| s.client.wants_version(version))
            .unwrap_or(false);
        if !wants_version {
            return;
        }
        // Fetch the full document; the latch is released when the response
        // lands (or never taken if the request can't start).
        fetch_busy.set(true);
        let inner = inner.clone();
        let sync = sync.clone();
        let fetch_busy_done = fetch_busy.clone();
        let last_selection_key = last_selection_key.clone();
        let on_doc: Rc<dyn Fn(String)> = Rc::new(move |doc_body: String| {
            apply_document_response(&inner, &doc_body, &sync, &last_selection_key);
            fetch_busy_done.set(false);
        });
        if !live_sync::get(&format!("{base_owned}/api/mcp/document"), on_doc) {
            fetch_busy.set(false);
        }
    });
    let _ = live_sync::get(&format!("{base}/api/mcp/version"), on_version);
}

/// Apply a `GET /api/mcp/document` response to the live shell.
/// `WebSyncClient::sync` runs the apply closure only for a newer document and
/// commits that exact version only when the closure returns `true` (swap +
/// repaint both succeeded) — so the committed version is never stale and a
/// failed repaint is retried on the next poll. On success the local
/// serialization becomes the push baseline (echo suppression), the selection
/// key is invalidated (the doc swap reset daemon + local selection, so the
/// daemon must be told the browser's current one again), and the sync-gate
/// baseline is committed to the PAIR AFTER the apply (a `replace_document`
/// bumps generation, so the pre-apply pair would be stale).
fn apply_document_response<C: RepaintContext + 'static>(
    inner: &Rc<RefCell<C>>,
    body: &str,
    sync: &SharedSync,
    last_selection_key: &Rc<RefCell<Option<String>>>,
) {
    let Ok(mut inner_mut) = inner.try_borrow_mut() else {
        return;
    };
    let inner_ref = &mut *inner_mut;
    // Apply-time re-check: a local edit may have landed while the fetch was
    // in flight; re-read the CURRENT pair and re-run `pull_allowed` before
    // taking any mutable borrow of the controller. Borrow ends at the end of
    // this statement (plain `if`, not `if let` — the condition's temporary
    // does not extend into the body).
    if !sync.borrow().gate.pull_allowed(current_pair(inner_ref)) {
        return; // a local edit landed while the fetch was in flight — abort apply
    }
    // Captured BEFORE sync() flips it on the first apply: the
    // mount-time pull (starter doc → daemon doc) must NOT become an
    // undo step; every later external apply (AI turn, MCP client) must.
    let undoable = sync
        .try_borrow()
        .map(|s| s.client.initialized())
        .unwrap_or(false);
    // The existing `WebSyncClient::sync` closure runs while `sync` is held
    // mutably borrowed (via `s` below) — it must never itself touch `sync`;
    // it only touches `inner_ref`, which is a separate RefCell.
    let applied = sync
        .try_borrow_mut()
        .ok()
        .and_then(|mut s| {
            s.client
                .sync(body, |doc, _version| {
                    inner_ref
                        .host_mut()
                        .replace_document_from_sync(doc, undoable);
                    inner_ref.repaint().is_ok()
                })
                .ok()
        })
        .unwrap_or(false);
    if applied {
        // Baseline = OUR serialization of the just-applied document, so the
        // push tick compares apples to apples (serde normalization differs
        // from the daemon's wire bytes).
        if let Ok(json) = serde_json::to_string(&inner_ref.host().editor_state().doc) {
            if let Ok(mut s) = sync.try_borrow_mut() {
                s.client.note_applied_snapshot(&json);
            }
        }
        if let Ok(mut last_selection_key) = last_selection_key.try_borrow_mut() {
            *last_selection_key = None;
        }
        // Commit the sync-gate baseline AFTER the apply closure has returned
        // (its mutable borrow above is released) and using the POST-apply
        // pair (`replace_document` bumped generation).
        let post_apply = current_pair(inner_ref);
        sync.borrow_mut()
            .gate
            .note_synced(post_apply.0, post_apply.1);
    }
}

/// Serialize + conditionally push the local document. `SyncGate::needs_push`
/// is the SOLE authoritative gate (false while a conflict is pending, or
/// while the current pair matches the last-synced baseline — including
/// before the first daemon apply, daemon-authority per the module docs);
/// `take_doc_sync_dirty` is consumed purely as a cheap "maybe changed" hint
/// and never substitutes for the gate.
fn push_document_if_changed<C: RepaintContext + 'static>(
    inner: &Rc<RefCell<C>>,
    base: &str,
    sync: &SharedSync,
    oversize_warned: &Rc<Cell<bool>>,
) {
    let busy = sync.borrow().push_busy;
    if busy {
        return;
    }
    let (pair, doc_json) = {
        let Ok(mut b) = inner.try_borrow_mut() else {
            return;
        };
        // Consume the hint — NEVER a gate. A false positive here just costs
        // one wasted `needs_push` check below; a false negative would be
        // caught by the gate anyway since it reads the live pair directly.
        let _ = b.host_mut().take_doc_sync_dirty();
        let pair = current_pair(&*b);
        let due = sync.borrow().gate.needs_push(pair);
        if !due {
            return;
        }
        let Ok(json) = serde_json::to_string(&b.host().editor_state().doc) else {
            return;
        };
        (pair, json)
    };

    let should_push = sync.borrow().client.should_push(&doc_json);
    if !should_push {
        // Bytes already match the daemon baseline (e.g. a generation-only
        // replace — same content, new generation): no push to send, but the
        // baseline MUST advance directly or `needs_push` stays true forever,
        // the hash check keeps skipping the push, and the pull gate never
        // reopens (deadlock).
        sync.borrow_mut().gate.note_synced(pair.0, pair.1);
        return;
    }
    if !SyncGate::periodic_push_allowed(doc_json.len()) {
        // TS warns once per oversize streak and skips the push; baseline
        // stays put (the uncapped snapshot channel is the fallback path).
        let warned = oversize_warned.get();
        if !warned {
            oversize_warned.set(true);
            web_sys::console::warn_1(
                &format!(
                    "[mcp-sync] Skip oversized document push: {:.2}MiB > {:.2}MiB",
                    doc_json.len() as f64 / (1024.0 * 1024.0),
                    SYNC_MAX_BODY_BYTES as f64 / (1024.0 * 1024.0)
                )
                .into(),
            );
        }
        return;
    }
    oversize_warned.set(false);
    let base_version = sync.borrow().client.last_version();
    let body = WebSyncClient::wrap_push_body_with_base(&doc_json, base_version);
    sync.borrow_mut().push_busy = true;
    let sync_done = sync.clone();
    // The pair captured AT SERIALIZATION TIME (above), not whatever the
    // current pair is when the response lands — an edit landing while this
    // push is in flight must keep reporting `needs_push`.
    let pushed_pair = pair;
    let on_response: Rc<dyn Fn(String)> = Rc::new(move |resp: String| {
        sync_done.borrow_mut().push_busy = false;
        // Conflict check first: a rejected push must never fall through to
        // the success branch's `note_synced` (which would wrongly reopen
        // the pull gate on rejected bytes).
        let conflict_version = WebSyncClient::parse_push_conflict(&resp);
        if let Some(server_v) = conflict_version {
            sync_done.borrow_mut().gate.note_conflict(server_v);
            return;
        }
        let accepted_version = WebSyncClient::parse_push_response(&resp);
        if let Some(version) = accepted_version {
            let mut s = sync_done.borrow_mut();
            s.client.mark_pushed(&doc_json, version);
            s.gate.note_synced(pushed_pair.0, pushed_pair.1);
        }
        // Neither shape recognized (network/parse failure): dropped
        // best-effort (TS catch{} parity) — the next local edit re-flags
        // needs_push and retries.
    });
    if !live_sync::post_json(
        &format!("{base}/api/mcp/document"),
        &body,
        Some(on_response),
    ) {
        sync.borrow_mut().push_busy = false;
    }
}

/// POST the selection to the daemon when it changed since the last sample
/// (TS pushes `{selectedIds, activePageId}` debounced 300 ms; this samples on
/// the 400 ms tick). Fire-and-forget like the TS fetch().catch(() => {}).
/// Unaffected by the sync gate (one-way browser → daemon presentation state,
/// not document content).
fn push_selection_if_changed<C: RepaintContext + 'static>(
    inner: &Rc<RefCell<C>>,
    base: &str,
    last_selection_key: &Rc<RefCell<Option<String>>>,
) {
    let (key, body) = {
        let Ok(b) = inner.try_borrow() else {
            return;
        };
        let state = b.host().editor_state();
        (
            web_sync::selection_sync_key(state),
            web_sync::selection_push_body(state),
        )
    };
    if last_selection_key
        .try_borrow()
        .map(|last| last.as_deref() == Some(key.as_str()))
        .unwrap_or(true)
    {
        return;
    }
    let Ok(mut last_selection_key) = last_selection_key.try_borrow_mut() else {
        return;
    };
    *last_selection_key = Some(key);
    let _ = live_sync::post_json(&format!("{base}/api/mcp/selection"), &body, None);
}
