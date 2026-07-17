//! Account tab of the settings modal.
//!
//! Signed out: a login-guidance card reusing the same primary "Sign in
//! with browser" affordance as [`crate::widgets::login_modal`]. Signed
//! in: display name / handle / avatar-initial + a Sign Out row. Mirrors
//! the System tab's card-based layout (`agent_settings_system.rs`).

use crate::theme::Theme;
use crate::widgets::agent_settings_i18n::t as t_settings;
use crate::widgets::button::tokens_from_theme;
use crate::widgets::PaintCx;
use crate::{Point2D, Rect, TextLayout};
use jian_widgets::components::card::Card;
use op_editor_core::editor_ui_state::EditorUiState;
use op_editor_core::AccountState;

const TITLE_H: f32 = 36.0;
const CARD_H: f32 = 96.0;
const AVATAR: f32 = 40.0;
// 56x28 mirrors the connect-button metrics used elsewhere in this modal
// (`agent_settings_panel::CONNECT_BTN_W/H`) — kept local since this tab
// has no dependency on the Agents-tab geometry module otherwise.
const ACTION_BTN_W: f32 = 96.0;
const ACTION_BTN_H: f32 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountTabHit {
    /// Signed out: opens the sign-in modal.
    SignIn,
    /// Signed in: clears the account back to `Anonymous`.
    SignOut,
    None,
}

pub(super) fn content_height() -> f32 {
    12.0 + TITLE_H + CARD_H + 24.0
}

fn card_rect(content: Rect) -> Rect {
    Rect {
        origin: Point2D::new(content.origin.x, content.origin.y + 12.0 + TITLE_H),
        size: Point2D::new(content.size.x, CARD_H),
    }
}

fn action_btn_rect(card: Rect) -> Rect {
    Rect {
        origin: Point2D::new(
            card.origin.x + card.size.x - 16.0 - ACTION_BTN_W,
            card.origin.y + (CARD_H - ACTION_BTN_H) / 2.0,
        ),
        size: Point2D::new(ACTION_BTN_W, ACTION_BTN_H),
    }
}

pub fn hit_test(content: Rect, ui: &EditorUiState, scrolled: Point2D) -> AccountTabHit {
    if !(action_btn_rect(card_rect(content))).contains(scrolled) {
        return AccountTabHit::None;
    }
    if ui.account.is_signed_in() {
        AccountTabHit::SignOut
    } else {
        AccountTabHit::SignIn
    }
}

pub(super) fn paint_account_tab(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    ui: &EditorUiState,
    content: Rect,
) {
    let title = TextLayout::single_run(
        t_settings(ui, "settings.account.title"),
        "system-ui",
        15.0,
        (theme.foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &title,
        Point2D::new(content.origin.x, content.origin.y + 20.0),
    );

    let card = card_rect(content);
    Card {
        fill: Some(theme.muted),
        border: Some(theme.border),
        radius: 10.0,
    }
    .paint(cx.backend, card, &tokens_from_theme(theme));

    match &ui.account {
        AccountState::Anonymous => paint_signed_out(cx, theme, ui, card),
        AccountState::SignedIn {
            display_name,
            handle,
        } => paint_signed_in(cx, theme, ui, card, display_name, handle),
    }
}

fn paint_signed_out(cx: &mut PaintCx<'_>, theme: &Theme, ui: &EditorUiState, card: Rect) {
    let label = TextLayout::single_run(
        t_settings(ui, "settings.account.notSignedIn"),
        "system-ui",
        13.0,
        (theme.foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &label,
        Point2D::new(
            card.origin.x + 16.0,
            card.origin.y + card.size.y / 2.0 + 4.0,
        ),
    );
    paint_action_button(
        cx,
        theme,
        action_btn_rect(card),
        t_settings(ui, "settings.account.signIn"),
    );
}

fn paint_signed_in(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    ui: &EditorUiState,
    card: Rect,
    display_name: &str,
    handle: &str,
) {
    let avatar_rect = Rect {
        origin: Point2D::new(
            card.origin.x + 16.0,
            card.origin.y + (CARD_H - AVATAR) / 2.0,
        ),
        size: Point2D::new(AVATAR, AVATAR),
    };
    cx.backend.fill_oval(avatar_rect, theme.primary);
    let initial = ui.account.initial().to_string();
    let initial_w = cx.backend.measure_text(&initial, 15.0);
    let initial_label = TextLayout::single_run(
        &initial,
        "system-ui",
        15.0,
        (theme.primary_foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &initial_label,
        Point2D::new(
            avatar_rect.origin.x + (AVATAR - initial_w) / 2.0,
            avatar_rect.origin.y + AVATAR / 2.0 + 5.0,
        ),
    );

    let text_x = avatar_rect.origin.x + AVATAR + 12.0;
    let name_label = TextLayout::single_run(
        display_name,
        "system-ui",
        13.0,
        (theme.foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &name_label,
        Point2D::new(text_x, card.origin.y + CARD_H / 2.0 - 2.0),
    );
    let handle_display = format!("@{}", handle);
    let handle_label = TextLayout::single_run(
        &handle_display,
        "system-ui",
        11.0,
        (theme.muted_foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &handle_label,
        Point2D::new(text_x, card.origin.y + CARD_H / 2.0 + 16.0),
    );

    paint_action_button(
        cx,
        theme,
        action_btn_rect(card),
        t_settings(ui, "settings.account.signOut"),
    );
}

fn paint_action_button(cx: &mut PaintCx<'_>, theme: &Theme, rect: Rect, label: &str) {
    cx.backend.fill_round_rect(rect, 8.0, theme.muted);
    cx.backend.stroke_round_rect(rect, 8.0, theme.border, 1.0);
    let w = cx.backend.measure_text(label, 12.0);
    let layout = TextLayout::single_run(
        label,
        "system-ui",
        12.0,
        (theme.foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &layout,
        Point2D::new(
            rect.origin.x + (rect.size.x - w) / 2.0,
            rect.origin.y + rect.size.y / 2.0 + 4.0,
        ),
    );
}
