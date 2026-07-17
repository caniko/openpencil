//! Sign-in modal — opened from the TopBar avatar button while signed
//! out, or from the settings modal's Account tab. Product logo/name +
//! a single primary "Sign in with browser" action.
//!
//! The real auth flow (v0.8.2 M3: OIDC Auth Code + PKCE via system
//! browser) is not wired yet, so the primary button is an honest stub:
//! it stays clickable, but production builds only reveal a "coming
//! soon" note instead of pretending to sign in. `AccountState::
//! dev_fake_signed_in` is the dev/demo fast path (gated host-side by
//! `OPENPENCIL_DEV_FAKE_LOGIN=1`, see `op-host-native`), never reachable
//! from this widget on its own.

use crate::theme::Theme;
use crate::widgets::editor_state_ext::theme_for;
use crate::widgets::icons::Icon;
use crate::widgets::{LayoutBox, LayoutCx, PaintCx, Widget, WidgetId};
use crate::{Point2D, Rect, TextLayout};
use op_editor_core::editor_ui_state::Locale;
use op_editor_core::{EditorState, LoginModalButton};

pub const MODAL_WIDTH: f32 = 360.0;
pub const MODAL_HEIGHT: f32 = 220.0;
const SIGN_IN_BTN_W: f32 = 260.0;
const SIGN_IN_BTN_H: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginModalHit {
    Close,
    SignIn,
    Outside,
    Inside,
}

pub struct LoginModal {
    pub id: WidgetId,
    pub theme: Theme,
    locale: Locale,
    /// Set after the primary button is clicked in a production build —
    /// reveals the honest "coming soon" note.
    stub_hint_shown: bool,
    hover: Option<LoginModalButton>,
    pressed: Option<LoginModalButton>,
}

impl LoginModal {
    pub fn for_editor(state: &EditorState) -> Self {
        Self {
            id: WidgetId::new(5500),
            theme: theme_for(&state.editor_ui),
            locale: state.editor_ui.locale,
            stub_hint_shown: state.editor_ui.login_modal_stub_hint_shown,
            hover: state.editor_ui.login_modal_hover,
            pressed: match state.editor_ui.pressed_button {
                Some(op_editor_core::ButtonPressTarget::LoginModal(button)) => Some(button),
                _ => None,
            },
        }
    }

    pub fn rect(&self, viewport_w: f32, viewport_h: f32) -> Rect {
        let x = ((viewport_w - MODAL_WIDTH) / 2.0).max(16.0);
        let y = ((viewport_h - MODAL_HEIGHT) / 2.0).max(crate::widgets::TOP_BAR_HEIGHT + 16.0);
        Rect {
            origin: Point2D::new(x, y),
            size: Point2D::new(MODAL_WIDTH, MODAL_HEIGHT),
        }
    }

    pub fn hit_test(&self, panel: Rect, point: Point2D) -> LoginModalHit {
        if !(panel).contains(point) {
            return LoginModalHit::Outside;
        }
        if (close_rect(panel)).contains(point) {
            return LoginModalHit::Close;
        }
        if (sign_in_rect(panel)).contains(point) {
            return LoginModalHit::SignIn;
        }
        LoginModalHit::Inside
    }
}

fn close_rect(panel: Rect) -> Rect {
    let s = 14.0;
    Rect {
        origin: Point2D::new(
            panel.origin.x + panel.size.x - 14.0 - s,
            panel.origin.y + 14.0,
        ),
        size: Point2D::new(s, s),
    }
}

fn sign_in_rect(panel: Rect) -> Rect {
    Rect {
        origin: Point2D::new(
            panel.origin.x + (panel.size.x - SIGN_IN_BTN_W) / 2.0,
            panel.origin.y + panel.size.y - 76.0,
        ),
        size: Point2D::new(SIGN_IN_BTN_W, SIGN_IN_BTN_H),
    }
}

fn t(locale: Locale, key: &'static str) -> &'static str {
    op_i18n::translate(locale, key)
}

impl Widget for LoginModal {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn layout(&self, _cx: &LayoutCx) -> LayoutBox {
        LayoutBox {
            rect: Rect {
                origin: Point2D::new(0.0, 0.0),
                size: Point2D::new(MODAL_WIDTH, MODAL_HEIGHT),
            },
        }
    }

    fn paint(&self, cx: &mut PaintCx<'_>, rect: Rect) {
        cx.backend.fill_round_rect(rect, 14.0, self.theme.card);
        cx.backend
            .stroke_round_rect(rect, 14.0, self.theme.border, 1.0);

        // Close X.
        let close = close_rect(rect);
        let close_hovered = self.hover == Some(LoginModalButton::Close);
        let close_pressed = self.pressed == Some(LoginModalButton::Close);
        let pad = 5.0;
        let close_bg = Rect {
            origin: Point2D::new(close.origin.x - pad, close.origin.y - pad),
            size: Point2D::new(close.size.x + pad * 2.0, close.size.y + pad * 2.0),
        };
        jian_widgets::components::icon_button::IconButton {
            icon_paths: Icon::Close.paths(),
            hovered: close_hovered,
            pressed: close_pressed,
            active: false,
            enabled: true,
            icon_size: close.size.x,
            stroke_width: 1.6,
        }
        .paint(
            cx.backend,
            close_bg,
            &crate::widgets::button::tokens_from_theme(&self.theme),
        );

        // Product name, centred — stands in for a logo glyph.
        let title = t(self.locale, "account.signInTitle");
        let title_w = cx.backend.measure_text(title, 15.0);
        let title_layout = TextLayout::single_run(
            title,
            "system-ui",
            15.0,
            (self.theme.foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &title_layout,
            Point2D::new(
                rect.origin.x + (rect.size.x - title_w) / 2.0,
                rect.origin.y + 56.0,
            ),
        );

        // Primary "Sign in with browser" action.
        let sign_in = sign_in_rect(rect);
        let sign_in_hovered = self.hover == Some(LoginModalButton::SignIn);
        let sign_in_pressed = self.pressed == Some(LoginModalButton::SignIn);
        let btn_bg = if sign_in_pressed {
            self.theme.primary.with_alpha(self.theme.primary.a * 0.85)
        } else if sign_in_hovered {
            self.theme.primary.with_alpha(self.theme.primary.a * 0.92)
        } else {
            self.theme.primary
        };
        cx.backend.fill_round_rect(sign_in, 8.0, btn_bg);
        let label = t(self.locale, "account.signInWithBrowser");
        let label_w = cx.backend.measure_text(label, 13.0);
        let label_layout = TextLayout::single_run(
            label,
            "system-ui",
            13.0,
            (self.theme.primary_foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &label_layout,
            Point2D::new(
                sign_in.origin.x + (sign_in.size.x - label_w) / 2.0,
                sign_in.origin.y + sign_in.size.y / 2.0 + 4.0,
            ),
        );

        // Honest stub note — only appears once the button has been
        // clicked, so an idle modal doesn't read as "broken by default".
        if self.stub_hint_shown {
            let hint = t(self.locale, "account.signInComingSoon");
            let hint_w = cx.backend.measure_text(hint, 11.0);
            let hint_layout = TextLayout::single_run(
                hint,
                "system-ui",
                11.0,
                (self.theme.muted_foreground).to_jian(),
                Point2D::new(0.0, 0.0),
            );
            cx.backend.draw_text(
                &hint_layout,
                Point2D::new(
                    rect.origin.x + (rect.size.x - hint_w) / 2.0,
                    sign_in.origin.y + sign_in.size.y + 20.0,
                ),
            );
        }
    }

    fn access_node(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(accesskit::Role::Dialog);
        node.set_label(t(self.locale, "account.signInTitle"));
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_sign_in_button_is_recognised() {
        let modal = LoginModal::for_editor(&EditorState::new());
        let panel = modal.rect(800.0, 600.0);
        let btn = sign_in_rect(panel);
        let point = Point2D::new(
            btn.origin.x + btn.size.x / 2.0,
            btn.origin.y + btn.size.y / 2.0,
        );
        assert_eq!(modal.hit_test(panel, point), LoginModalHit::SignIn);
    }

    #[test]
    fn clicking_outside_the_panel_is_outside() {
        let modal = LoginModal::for_editor(&EditorState::new());
        let panel = modal.rect(800.0, 600.0);
        assert_eq!(
            modal.hit_test(panel, Point2D::new(panel.origin.x - 5.0, panel.origin.y)),
            LoginModalHit::Outside
        );
    }
}
