use super::WidgetHostNative;
use op_editor_core::{AccountMenuRow, AccountState, ButtonPressTarget, LoginModalButton};
use op_editor_ui::widgets::account_menu::AccountMenu;
use op_editor_ui::widgets::login_modal::LoginModal;
use op_editor_ui::widgets::top_bar::TopBar;
use op_editor_ui::widgets::TOP_BAR_HEIGHT;
use op_editor_ui::{Point2D, Rect};
use std::sync::{Mutex, MutexGuard, OnceLock};

const VW: f32 = 1200.0;
const VH: f32 = 800.0;

fn top_bar_rect() -> Rect {
    Rect {
        origin: Point2D::new(0.0, 0.0),
        size: Point2D::new(VW, TOP_BAR_HEIGHT),
    }
}

/// `cargo test` runs a crate's tests on multiple threads by default, but
/// `std::env` is process-global — two tests racing to set/unset
/// `OPENPENCIL_DEV_FAKE_LOGIN` concurrently can observe each other's
/// writes mid-test. Every test that touches this env var takes this
/// lock first (mirrors `font_registry_test_support::lock` in
/// `op-host-native/src/lib.rs`, same rationale for a different global).
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Restores an env var to its pre-test value on drop, so
/// `OPENPENCIL_DEV_FAKE_LOGIN` can't leak into other tests running in
/// the same process (mirrors `op-host-services`' `EnvVarGuard`).
struct EnvVarGuard {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }

    fn unset(key: &'static str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn clicking_avatar_while_anonymous_opens_login_modal() {
    let mut host = WidgetHostNative::new();
    assert_eq!(
        host.editor_state().editor_ui.account,
        AccountState::Anonymous
    );
    let top_bar = TopBar::for_editor_ui(&host.editor_state().editor_ui);
    let account_rect = top_bar.account_button_rect(top_bar_rect());
    let center = Point2D::new(
        account_rect.origin.x + account_rect.size.x / 2.0,
        account_rect.origin.y + account_rect.size.y / 2.0,
    );

    assert!(host.apply_press(center.x, center.y, VW, VH));

    assert!(host.editor_state().editor_ui.login_modal_open);
    assert!(!host.editor_state().editor_ui.account_menu_open);
}

#[test]
fn clicking_avatar_while_signed_in_opens_account_menu() {
    let mut host = WidgetHostNative::new();
    host.editor_state_mut().editor_ui.account = AccountState::SignedIn {
        display_name: "Fini".into(),
        handle: "fini".into(),
    };
    let top_bar = TopBar::for_editor_ui(&host.editor_state().editor_ui);
    let account_rect = top_bar.account_button_rect(top_bar_rect());
    let center = Point2D::new(
        account_rect.origin.x + account_rect.size.x / 2.0,
        account_rect.origin.y + account_rect.size.y / 2.0,
    );

    assert!(host.apply_press(center.x, center.y, VW, VH));

    assert!(host.editor_state().editor_ui.account_menu_open);
    assert!(!host.editor_state().editor_ui.login_modal_open);
}

#[test]
fn login_modal_sign_in_without_dev_flag_shows_honest_stub_hint() {
    let _lock = env_lock();
    let _guard = EnvVarGuard::unset("OPENPENCIL_DEV_FAKE_LOGIN");
    let mut host = WidgetHostNative::new();
    host.editor_state_mut().editor_ui.login_modal_open = true;

    let modal = LoginModal::for_editor(host.editor_state());
    let panel = modal.rect(VW, VH);
    // `sign_in_rect` is private to the widget module; recompute the
    // same geometry via `hit_test` scanning is overkill here — the
    // widget's own test (`login_modal.rs`) already locks the rect, so
    // this test drives the dispatcher directly through the panel.
    let hit_point = Point2D::new(
        panel.origin.x + panel.size.x / 2.0,
        panel.origin.y + panel.size.y - 56.0,
    );
    assert_eq!(
        modal.hit_test(panel, hit_point),
        op_editor_ui::widgets::login_modal::LoginModalHit::SignIn
    );

    host.dispatch_login_modal_press(hit_point.x, hit_point.y, VW, VH);

    assert_eq!(
        host.editor_state().editor_ui.account,
        AccountState::Anonymous
    );
    assert!(host.editor_state().editor_ui.login_modal_open);
    assert!(host.editor_state().editor_ui.login_modal_stub_hint_shown);
    assert_eq!(
        host.editor_state().editor_ui.pressed_button,
        Some(ButtonPressTarget::LoginModal(LoginModalButton::SignIn))
    );
}

#[test]
fn login_modal_sign_in_with_dev_flag_signs_in_and_closes() {
    let _lock = env_lock();
    let _guard = EnvVarGuard::set("OPENPENCIL_DEV_FAKE_LOGIN", "1");
    let mut host = WidgetHostNative::new();
    host.editor_state_mut().editor_ui.login_modal_open = true;

    let modal = LoginModal::for_editor(host.editor_state());
    let panel = modal.rect(VW, VH);
    let hit_point = Point2D::new(
        panel.origin.x + panel.size.x / 2.0,
        panel.origin.y + panel.size.y - 56.0,
    );
    assert_eq!(
        modal.hit_test(panel, hit_point),
        op_editor_ui::widgets::login_modal::LoginModalHit::SignIn
    );

    host.dispatch_login_modal_press(hit_point.x, hit_point.y, VW, VH);

    assert_eq!(
        host.editor_state().editor_ui.account,
        AccountState::SignedIn {
            display_name: "Fini".into(),
            handle: "fini".into(),
        }
    );
    assert!(!host.editor_state().editor_ui.login_modal_open);
}

#[test]
fn account_menu_settings_row_opens_settings_on_account_tab() {
    let mut host = WidgetHostNative::new();
    host.editor_state_mut().editor_ui.account = AccountState::SignedIn {
        display_name: "Fini".into(),
        handle: "fini".into(),
    };
    host.editor_state_mut().editor_ui.account_menu_open = true;

    let top_bar = TopBar::for_editor_ui(&host.editor_state().editor_ui);
    let anchor = top_bar.account_button_rect(top_bar_rect());
    let menu = AccountMenu::for_editor_ui(&host.editor_state().editor_ui).expect("signed in");
    let menu_rect = menu.rect_at(anchor);
    let settings_point = Point2D::new(
        menu_rect.origin.x + 20.0,
        menu_rect.origin.y + menu_rect.size.y - 40.0,
    );
    assert_eq!(
        menu.hit_test(menu_rect, settings_point),
        Some(AccountMenuRow::Settings)
    );

    host.dispatch_account_menu_press(settings_point.x, settings_point.y, VW, VH);

    assert!(!host.editor_state().editor_ui.account_menu_open);
    assert!(host.editor_state().editor_ui.agent_settings_open);
    assert_eq!(
        host.editor_state().editor_ui.agent_settings.tab,
        op_editor_core::agent_settings::AgentSettingsTab::Account
    );
}

#[test]
fn account_menu_sign_out_row_clears_account() {
    let mut host = WidgetHostNative::new();
    host.editor_state_mut().editor_ui.account = AccountState::SignedIn {
        display_name: "Fini".into(),
        handle: "fini".into(),
    };
    host.editor_state_mut().editor_ui.account_menu_open = true;

    let top_bar = TopBar::for_editor_ui(&host.editor_state().editor_ui);
    let anchor = top_bar.account_button_rect(top_bar_rect());
    let menu = AccountMenu::for_editor_ui(&host.editor_state().editor_ui).expect("signed in");
    let menu_rect = menu.rect_at(anchor);
    let sign_out_point = Point2D::new(
        menu_rect.origin.x + 20.0,
        menu_rect.origin.y + menu_rect.size.y - 8.0,
    );
    assert_eq!(
        menu.hit_test(menu_rect, sign_out_point),
        Some(AccountMenuRow::SignOut)
    );

    host.dispatch_account_menu_press(sign_out_point.x, sign_out_point.y, VW, VH);

    assert!(!host.editor_state().editor_ui.account_menu_open);
    assert_eq!(
        host.editor_state().editor_ui.account,
        AccountState::Anonymous
    );
}
