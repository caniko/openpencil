//! Account / sign-in state for the v0.8.2 user system (platform +
//! zseven-sso). The real backend is not live yet — this module carries
//! the state model plus a dev-only fake-login seam so the topbar avatar
//! button, its dropdown, the sign-in modal, and the settings modal's
//! Account tab can all be built and exercised end-to-end before the
//! actual OIDC flow lands.
//!
//! Same wasm32-clean discipline as the other `*_state` mirrors — plain
//! data only, no session/token material.

/// Signed-in / signed-out state for the current user. `SignedIn` carries
/// only display fields; the real OIDC client (not this crate) will own
/// any token/session material.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum AccountState {
    #[default]
    Anonymous,
    SignedIn {
        display_name: String,
        handle: String,
    },
}

impl AccountState {
    pub fn is_signed_in(&self) -> bool {
        matches!(self, AccountState::SignedIn { .. })
    }

    /// First uppercase character of the display name — the avatar-circle
    /// glyph when signed in. `Anonymous` (or an empty display name) has
    /// no letter to show; callers gate on [`Self::is_signed_in`] first.
    pub fn initial(&self) -> char {
        match self {
            AccountState::SignedIn { display_name, .. } => display_name
                .chars()
                .next()
                .map(|c| c.to_ascii_uppercase())
                .unwrap_or('?'),
            AccountState::Anonymous => '?',
        }
    }

    /// Dev/test fake sign-in — the fast path gated by
    /// `OPENPENCIL_DEV_FAKE_LOGIN=1` (checked host-side; this crate reads
    /// no env vars so it stays wasm32-clean). Never reachable from the
    /// production sign-in button.
    pub fn dev_fake_signed_in() -> Self {
        AccountState::SignedIn {
            display_name: "Fini".to_string(),
            handle: "fini".to_string(),
        }
    }
}

/// One row in the signed-in account dropdown (anchored under the topbar
/// avatar button).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountMenuRow {
    /// "Current workspace / Personal workspace" — chevron affordance,
    /// no submenu yet (click is a no-op).
    Workspace,
    /// Opens the settings modal on the Account tab.
    Settings,
    /// Clears `AccountState` back to `Anonymous`.
    SignOut,
}

impl AccountMenuRow {
    pub const ALL: [AccountMenuRow; 3] = [
        AccountMenuRow::Workspace,
        AccountMenuRow::Settings,
        AccountMenuRow::SignOut,
    ];
}

/// Which control in the sign-in modal the cursor is over / has pressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginModalButton {
    Close,
    /// The primary "Sign in with browser" action. Production builds show
    /// an honest "coming soon" note on click (see
    /// `AccountState::dev_fake_signed_in` for the dev-only fast path).
    SignIn,
}
