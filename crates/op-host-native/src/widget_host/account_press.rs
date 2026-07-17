//! Sign-in modal + signed-in account-dropdown press dispatchers.
//!
//! Mirrors the other modal/overlay dispatchers in
//! `property_input_dispatch.rs` (figma-import, export-dialog): each
//! consumes every press while its overlay is open, closing silently on
//! an outside click (still counted as a blank press so chrome text
//! inputs blur).

use super::WidgetHostNative;
use op_editor_core::AccountMenuRow;
use op_editor_ui::widgets::account_menu::AccountMenu;
use op_editor_ui::widgets::login_modal::{LoginModal, LoginModalHit};
use op_editor_ui::widgets::top_bar::TopBar;
use op_editor_ui::widgets::TOP_BAR_HEIGHT;
use op_editor_ui::{Point2D, Rect};

impl WidgetHostNative {
    /// Sign-in-modal press dispatcher.
    pub(in crate::widget_host) fn dispatch_login_modal_press(
        &mut self,
        x: f32,
        y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) {
        let modal = LoginModal::for_editor(&self.editor_state);
        let panel_rect = modal.rect(viewport_w, viewport_h);
        let hit = modal.hit_test(panel_rect, Point2D::new(x, y));
        self.editor_state.editor_ui.pressed_button =
            op_editor_ui::widgets::editor_state_ext::login_modal_button(hit)
                .map(op_editor_core::ButtonPressTarget::LoginModal);
        match hit {
            LoginModalHit::Close => {
                self.editor_state.editor_ui.login_modal_open = false;
                self.editor_state.editor_ui.login_modal_hover = None;
                self.editor_state.editor_ui.login_modal_stub_hint_shown = false;
            }
            LoginModalHit::Outside => {
                self.blur_text_inputs_on_blank_press();
                self.editor_state.editor_ui.login_modal_open = false;
                self.editor_state.editor_ui.login_modal_hover = None;
                self.editor_state.editor_ui.login_modal_stub_hint_shown = false;
            }
            LoginModalHit::SignIn => {
                // Dev/demo fast path — never reachable in a production
                // build; the real flow is `v0.8.2 M3: OIDC Auth Code +
                // PKCE via system browser`.
                if dev_fake_login_enabled() {
                    self.editor_state.editor_ui.account =
                        op_editor_core::AccountState::dev_fake_signed_in();
                    self.editor_state.editor_ui.login_modal_open = false;
                    self.editor_state.editor_ui.login_modal_hover = None;
                    self.editor_state.editor_ui.login_modal_stub_hint_shown = false;
                } else {
                    // Honest stub: no session is created — just reveal
                    // the "coming soon" note instead of pretending the
                    // OIDC flow ran.
                    self.editor_state.editor_ui.login_modal_stub_hint_shown = true;
                }
            }
            LoginModalHit::Inside => {
                self.blur_text_inputs_on_blank_press();
            }
        }
        self.mark_dirty();
    }

    /// Signed-in account-dropdown press dispatcher.
    pub(in crate::widget_host) fn dispatch_account_menu_press(
        &mut self,
        x: f32,
        y: f32,
        viewport_w: f32,
        _viewport_h: f32,
    ) {
        let top_bar_rect = Rect {
            origin: Point2D::new(0.0, 0.0),
            size: Point2D::new(viewport_w, TOP_BAR_HEIGHT),
        };
        let top_bar = TopBar::for_editor_ui(&self.editor_state.editor_ui);
        let anchor = top_bar.account_button_rect(top_bar_rect);
        let Some(menu) = AccountMenu::for_editor_ui(&self.editor_state.editor_ui) else {
            // Account state flipped back to Anonymous out from under an
            // open menu (shouldn't normally happen) — close defensively.
            self.editor_state.editor_ui.account_menu_open = false;
            return;
        };
        let menu_rect = menu.rect_at(anchor);
        let point = Point2D::new(x, y);
        match menu.hit_test(menu_rect, point) {
            Some(AccountMenuRow::Workspace) => {
                // No submenu exists yet — a literal no-op. The menu
                // stays open (unlike Settings / Sign Out, which both
                // have a real action to complete).
            }
            Some(AccountMenuRow::Settings) => {
                self.close_account_menu();
                self.editor_state.editor_ui.agent_settings_open = true;
                self.editor_state.editor_ui.agent_settings.tab =
                    op_editor_core::agent_settings::AgentSettingsTab::Account;
                self.editor_state.chat.blur_input(self.now_ms);
            }
            Some(AccountMenuRow::SignOut) => {
                self.close_account_menu();
                self.editor_state.editor_ui.account = op_editor_core::AccountState::Anonymous;
            }
            None => {
                if !(menu_rect).contains(point) {
                    // Outside click — blank press: dismiss + blur inputs.
                    self.blur_text_inputs_on_blank_press();
                    self.close_account_menu();
                }
            }
        }
        self.mark_dirty();
    }

    fn close_account_menu(&mut self) {
        self.editor_state.editor_ui.account_menu_open = false;
        self.editor_state.editor_ui.account_menu_hover = None;
    }
}

/// Dev/demo fast path: `OPENPENCIL_DEV_FAKE_LOGIN=1` makes the sign-in
/// modal's primary button set `AccountState::SignedIn` immediately
/// instead of showing the honest "coming soon" stub. Lets the topbar
/// avatar, its dropdown, and the settings Account tab be exercised
/// end-to-end before the real OIDC backend lands. Never read outside
/// this native host, so it can't leak into the wasm web build.
fn dev_fake_login_enabled() -> bool {
    std::env::var("OPENPENCIL_DEV_FAKE_LOGIN").as_deref() == Ok("1")
}
