//! Signed-in account dropdown — anchored under the TopBar avatar button.
//!
//! Header (display name + `@handle`), divider, "Current workspace" row
//! (chevron affordance, no submenu yet — click is a no-op), divider,
//! "Settings" (opens the settings modal on the Account tab), "Sign Out".

use crate::theme::Theme;
use crate::widgets::editor_state_ext::theme_for;
use crate::widgets::icons::{draw_icon, Icon};
use crate::widgets::{LayoutBox, LayoutCx, PaintCx, Widget, WidgetId};
use crate::{Point2D, Rect, TextLayout};
use op_editor_core::editor_ui_state::{EditorUiState, Locale};
use op_editor_core::AccountMenuRow;

pub const MENU_WIDTH: f32 = 220.0;
const PAD_X: f32 = 12.0;
const ROW_HEIGHT: f32 = 34.0;
const HEADER_HEIGHT: f32 = 54.0;
const DIVIDER_GAP: f32 = 4.0;
const ICON_SIZE: f32 = 15.0;

fn t(locale: Locale, key: &'static str) -> &'static str {
    op_i18n::translate(locale, key)
}

pub struct AccountMenu<'a> {
    pub id: WidgetId,
    pub theme: Theme,
    ui: &'a EditorUiState,
    display_name: String,
    handle: String,
    hover: Option<AccountMenuRow>,
}

impl<'a> AccountMenu<'a> {
    /// `None` when the account is not signed in — the host should not
    /// paint/hit-test this widget in that state.
    pub fn for_editor_ui(ui: &'a EditorUiState) -> Option<Self> {
        let (display_name, handle) = match &ui.account {
            op_editor_core::AccountState::SignedIn {
                display_name,
                handle,
            } => (display_name.clone(), handle.clone()),
            op_editor_core::AccountState::Anonymous => return None,
        };
        Some(Self {
            id: WidgetId::new(5600),
            theme: theme_for(ui),
            ui,
            display_name,
            handle,
            hover: ui.account_menu_hover,
        })
    }

    /// Anchored under the avatar button, right-aligned to its right edge
    /// (a dropdown grows down-and-left from a top-right button).
    pub fn rect_at(&self, anchor: Rect) -> Rect {
        let right = anchor.origin.x + anchor.size.x;
        Rect {
            origin: Point2D::new(right - MENU_WIDTH, anchor.origin.y + anchor.size.y + 6.0),
            size: Point2D::new(MENU_WIDTH, self.height()),
        }
    }

    fn height(&self) -> f32 {
        HEADER_HEIGHT
            + (DIVIDER_GAP * 2.0 + 1.0)
            + ROW_HEIGHT
            + (DIVIDER_GAP * 2.0 + 1.0)
            + ROW_HEIGHT * 2.0
            + DIVIDER_GAP
    }

    pub fn row_at(&self, panel: Rect, point: Point2D) -> Option<AccountMenuRow> {
        if !(panel).contains(point) {
            return None;
        }
        let mut y = panel.origin.y + HEADER_HEIGHT + DIVIDER_GAP * 2.0 + 1.0;
        if row_hit(panel.origin.x, y, point) {
            return Some(AccountMenuRow::Workspace);
        }
        y += ROW_HEIGHT + DIVIDER_GAP * 2.0 + 1.0;
        if row_hit(panel.origin.x, y, point) {
            return Some(AccountMenuRow::Settings);
        }
        y += ROW_HEIGHT;
        if row_hit(panel.origin.x, y, point) {
            return Some(AccountMenuRow::SignOut);
        }
        None
    }

    pub fn hit_test(&self, panel: Rect, point: Point2D) -> Option<AccountMenuRow> {
        self.row_at(panel, point)
    }
}

fn row_hit(x: f32, y: f32, point: Point2D) -> bool {
    (Rect {
        origin: Point2D::new(x, y),
        size: Point2D::new(MENU_WIDTH, ROW_HEIGHT),
    })
    .contains(point)
}

fn paint_row_tint(cx: &mut PaintCx<'_>, theme: &Theme, x: f32, y: f32) {
    let inset = 4.0;
    let mut tint = theme.muted_foreground;
    tint.a = 0.14;
    cx.backend.fill_round_rect(
        Rect {
            origin: Point2D::new(x + inset, y + 2.0),
            size: Point2D::new(MENU_WIDTH - inset * 2.0, ROW_HEIGHT - 4.0),
        },
        6.0,
        tint,
    );
}

fn paint_divider(cx: &mut PaintCx<'_>, theme: &Theme, rect: Rect, y: f32) -> f32 {
    let line_y = y + DIVIDER_GAP;
    jian_widgets::components::separator::Separator {
        orientation: jian_widgets::components::separator::Orientation::Horizontal,
        thickness: 1.0,
    }
    .paint(
        cx.backend,
        Rect {
            origin: Point2D::new(rect.origin.x + PAD_X, line_y),
            size: Point2D::new(MENU_WIDTH - PAD_X * 2.0, 1.0),
        },
        theme.border,
    );
    y + DIVIDER_GAP * 2.0 + 1.0
}

impl<'a> Widget for AccountMenu<'a> {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn layout(&self, _cx: &LayoutCx) -> LayoutBox {
        LayoutBox {
            rect: Rect {
                origin: Point2D::new(0.0, 0.0),
                size: Point2D::new(MENU_WIDTH, self.height()),
            },
        }
    }

    fn paint(&self, cx: &mut PaintCx<'_>, rect: Rect) {
        cx.backend.fill_round_rect(rect, 10.0, self.theme.card);
        cx.backend
            .stroke_round_rect(rect, 10.0, self.theme.border, 1.0);

        // Header: display name + @handle.
        let name_layout = TextLayout::single_run(
            &self.display_name,
            "system-ui",
            13.0,
            (self.theme.foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &name_layout,
            Point2D::new(rect.origin.x + PAD_X, rect.origin.y + 22.0),
        );
        let handle_display = format!("@{}", self.handle);
        let handle_layout = TextLayout::single_run(
            &handle_display,
            "system-ui",
            11.0,
            (self.theme.muted_foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &handle_layout,
            Point2D::new(rect.origin.x + PAD_X, rect.origin.y + 40.0),
        );

        let mut y = rect.origin.y + HEADER_HEIGHT;
        y = paint_divider(cx, &self.theme, rect, y);

        // "Current workspace" — stacked label + value, chevron affordance.
        let hovered_workspace = self.hover == Some(AccountMenuRow::Workspace);
        if hovered_workspace {
            paint_row_tint(cx, &self.theme, rect.origin.x, y);
        }
        let workspace_header = TextLayout::single_run(
            t(self.ui.locale, "account.currentWorkspace"),
            "system-ui",
            10.0,
            (self.theme.muted_foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &workspace_header,
            Point2D::new(rect.origin.x + PAD_X, y + 13.0),
        );
        let workspace_value = TextLayout::single_run(
            t(self.ui.locale, "account.personalWorkspace"),
            "system-ui",
            12.0,
            (self.theme.foreground).to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &workspace_value,
            Point2D::new(rect.origin.x + PAD_X, y + 27.0),
        );
        draw_icon(
            cx.backend,
            Icon::ChevronRight,
            Point2D::new(
                rect.origin.x + MENU_WIDTH - PAD_X - ICON_SIZE,
                y + (ROW_HEIGHT - ICON_SIZE) / 2.0,
            ),
            ICON_SIZE,
            self.theme.muted_foreground,
            1.4,
        );
        y += ROW_HEIGHT;
        y = paint_divider(cx, &self.theme, rect, y);

        paint_action_row(
            cx,
            &self.theme,
            rect.origin.x,
            y,
            Icon::Settings,
            t(self.ui.locale, "account.settings"),
            self.hover == Some(AccountMenuRow::Settings),
        );
        y += ROW_HEIGHT;
        paint_action_row(
            cx,
            &self.theme,
            rect.origin.x,
            y,
            Icon::LogOut,
            t(self.ui.locale, "account.signOut"),
            self.hover == Some(AccountMenuRow::SignOut),
        );
    }

    fn access_node(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(accesskit::Role::Menu);
        node.set_label("Account menu");
        node
    }
}

fn paint_action_row(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    x: f32,
    y: f32,
    icon: Icon,
    label: &str,
    hovered: bool,
) {
    if hovered {
        paint_row_tint(cx, theme, x, y);
    }
    draw_icon(
        cx.backend,
        icon,
        Point2D::new(x + PAD_X, y + (ROW_HEIGHT - ICON_SIZE) / 2.0),
        ICON_SIZE,
        theme.muted_foreground,
        1.4,
    );
    let label_layout = TextLayout::single_run(
        label,
        "system-ui",
        13.0,
        (theme.foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &label_layout,
        Point2D::new(x + PAD_X + ICON_SIZE + 10.0, y + ROW_HEIGHT / 2.0 + 5.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_in_ui() -> EditorUiState {
        EditorUiState {
            account: op_editor_core::AccountState::SignedIn {
                display_name: "Fini".to_string(),
                handle: "fini".to_string(),
            },
            ..EditorUiState::default()
        }
    }

    #[test]
    fn anonymous_state_has_no_menu() {
        let ui = EditorUiState::default();
        assert!(AccountMenu::for_editor_ui(&ui).is_none());
    }

    #[test]
    fn settings_row_hit_maps_to_settings_variant() {
        let ui = signed_in_ui();
        let menu = AccountMenu::for_editor_ui(&ui).expect("signed in");
        let panel = Rect {
            origin: Point2D::new(100.0, 50.0),
            size: Point2D::new(MENU_WIDTH, menu.height()),
        };
        let y = panel.origin.y
            + HEADER_HEIGHT
            + DIVIDER_GAP * 2.0
            + 1.0
            + ROW_HEIGHT
            + DIVIDER_GAP * 2.0
            + 1.0
            + ROW_HEIGHT / 2.0;
        let hit = menu.hit_test(panel, Point2D::new(panel.origin.x + 20.0, y));
        assert_eq!(hit, Some(AccountMenuRow::Settings));
    }
}
