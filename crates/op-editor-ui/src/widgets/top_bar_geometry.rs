//! `TopBar` right-cluster + file-menu/git-button geometry helpers — split
//! out of `top_bar.rs` to keep that file under the repo's 800-line cap.
//! Paint (`top_bar_paint.rs`) and hit-test (`top_bar.rs::hit_test`) both
//! route through these so button rects can never drift between the two.

use crate::widgets::top_bar::*;
use crate::{Point2D, Rect};

impl TopBar {
    /// Returns the on-screen rect of the Globe-plus-chevron locale
    /// button. Used by the host to anchor the LocalePicker dropdown
    /// directly underneath when `Document.ui.locale_picker.open ==
    /// true`. The button itself is wider than a normal icon button
    /// so the chevron-down has room to render.
    /// Anchor rect for the file-menu dropdown overlay (folder +
    /// chevron compound). Host anchors the dropdown directly under
    /// this rect when `Document.ui.file_menu_open == true`.
    pub fn file_menu_rect(top_bar_rect: Rect, fullscreen: bool) -> Rect {
        // Mirror the paint layout: panel button │ divider │ file-menu.
        // The divider span (gap + width + gap) pushes the file-menu
        // right of the sidebar toggle — keep this anchor in sync so
        // the dropdown opens under the folder button, not left of it.
        let divider_span = DIVIDER_GAP + DIVIDER_W + DIVIDER_GAP;
        let file_menu_x = top_bar_rect.origin.x
            + PAD
            + Self::left_inset_for(fullscreen)
            + ICON_BUTTON
            + divider_span;
        Rect {
            origin: Point2D::new(file_menu_x, top_bar_rect.origin.y + 8.0),
            size: Point2D::new(FILE_MENU_BUTTON_WIDTH, ICON_BUTTON),
        }
    }

    pub fn file_menu_rect_for(&self, top_bar_rect: Rect) -> Rect {
        let divider_span = DIVIDER_GAP + DIVIDER_W + DIVIDER_GAP;
        let file_menu_x =
            top_bar_rect.origin.x + PAD + self.left_inset() + ICON_BUTTON + divider_span;
        Rect {
            origin: Point2D::new(file_menu_x, top_bar_rect.origin.y + 8.0),
            size: Point2D::new(FILE_MENU_BUTTON_WIDTH, ICON_BUTTON),
        }
    }

    /// Import button, right of the file menu. Canonical anchor shared by
    /// hit-test, paint, and the import dropdown so they cannot drift.
    pub fn import_button_rect(&self, top_bar_rect: Rect) -> Rect {
        let divider_span = DIVIDER_GAP + DIVIDER_W + DIVIDER_GAP;
        let file_menu = self.file_menu_rect_for(top_bar_rect);
        Rect {
            origin: Point2D::new(
                file_menu.origin.x + FILE_MENU_BUTTON_WIDTH + divider_span,
                file_menu.origin.y,
            ),
            size: Point2D::new(FILE_MENU_BUTTON_WIDTH, ICON_BUTTON),
        }
    }

    /// Whether the Preview (Play) button paints / hit-tests. Gated only by
    /// the host capability (`PREVIEW_BUTTON_AVAILABLE`, desktop-only) —
    /// preview interaction graduated out of the experimental-features gate
    /// (widget-config and other experimental items stay gated separately;
    /// see `EditorUiState::agent_settings.experimental_features_enabled`).
    /// The right-cluster layout collapses when this is false, so paint,
    /// hit-test, and the globe-anchored locale picker all key off this one
    /// predicate.
    pub fn preview_button_visible(&self) -> bool {
        PREVIEW_BUTTON_AVAILABLE
    }

    /// File-scoped chrome (open menu, Figma import, centered file name):
    /// hidden inside a VS Code embed — the workbench owns file identity.
    pub(super) fn file_controls_visible(&self) -> bool {
        self.embed != op_editor_core::EmbedHost::VsCode
    }

    /// The Maximize toggle is meaningless inside an embed iframe.
    pub(super) fn fullscreen_button_visible(&self) -> bool {
        self.embed != op_editor_core::EmbedHost::VsCode
    }

    pub fn globe_rect(&self, top_bar_rect: Rect) -> Rect {
        let right = top_bar_rect.origin.x + top_bar_rect.size.x;
        // Right-cluster layout (right → left): Maximize (hidden in a
        // VS Code embed) | Play (native only) | Sun | Globe. Icon buttons
        // are normal ICON_BUTTON wide; Globe is the wider
        // GLOBE_BUTTON_WIDTH so the chevron fits.
        let icon_count =
            1.0 + if self.fullscreen_button_visible() {
                1.0
            } else {
                0.0
            } + if self.preview_button_visible() {
                1.0
            } else {
                0.0
            };
        let globe_x = right - PAD - ICON_BUTTON * icon_count - GLOBE_BUTTON_WIDTH;
        Rect {
            origin: Point2D::new(globe_x, top_bar_rect.origin.y + 8.0),
            size: Point2D::new(GLOBE_BUTTON_WIDTH, ICON_BUTTON),
        }
    }

    /// User-avatar button — anchored directly left of the Globe button,
    /// between the locale/theme cluster and the agent-status chip's
    /// divider (TS parity spot: "between the agents chip and the
    /// globe/theme icons"). Derived from [`Self::globe_rect`] so the two
    /// can never drift apart.
    pub fn account_button_rect(&self, top_bar_rect: Rect) -> Rect {
        let globe = self.globe_rect(top_bar_rect);
        Rect {
            origin: Point2D::new(globe.origin.x - ICON_BUTTON, globe.origin.y),
            size: Point2D::new(ICON_BUTTON, ICON_BUTTON),
        }
    }

    /// Left edge the agent chip's divider hangs off — the avatar
    /// button's left edge when it's available (desktop), else directly
    /// the Globe button's (web, where `ACCOUNT_BUTTON_AVAILABLE` is
    /// false and the button doesn't paint). Shared by paint + hit-test
    /// so the chip anchor can't drift from whichever layout is active.
    pub(super) fn chip_right_anchor_x(&self, top_bar_rect: Rect) -> f32 {
        if ACCOUNT_BUTTON_AVAILABLE {
            self.account_button_rect(top_bar_rect).origin.x
        } else {
            self.globe_rect(top_bar_rect).origin.x
        }
    }

    /// Play / Stop toggle button — second from the right (just left of
    /// Maximize), or rightmost when Maximize is hidden in a VS Code embed.
    /// Shared by paint + hit-test so they can't drift.
    pub(super) fn preview_button_rect(&self, top_bar_rect: Rect) -> Rect {
        let right = top_bar_rect.origin.x + top_bar_rect.size.x;
        let icon_y = top_bar_rect.origin.y + 8.0;
        let fullscreen_slot = if self.fullscreen_button_visible() {
            1.0
        } else {
            0.0
        };
        Rect {
            origin: Point2D::new(right - PAD - ICON_BUTTON * (fullscreen_slot + 1.0), icon_y),
            size: Point2D::new(ICON_BUTTON, ICON_BUTTON),
        }
    }

    /// Theme toggle button. Its x-position shifts right in the web/wasm build
    /// where the Preview button is hidden, and in a VS Code embed where the
    /// Maximize button is hidden.
    pub(super) fn theme_button_rect(&self, top_bar_rect: Rect) -> Rect {
        let right = top_bar_rect.origin.x + top_bar_rect.size.x;
        let right_icons = if self.fullscreen_button_visible() {
            1.0
        } else {
            0.0
        } + if self.preview_button_visible() {
            1.0
        } else {
            0.0
        };
        Rect {
            origin: Point2D::new(
                right - PAD - ICON_BUTTON * (right_icons + 1.0),
                top_bar_rect.origin.y + 8.0,
            ),
            size: Point2D::new(ICON_BUTTON, ICON_BUTTON),
        }
    }

    /// Git-panel toggle button — sits just right of the centred file
    /// name. Width holds the branch glyph plus an optional branch
    /// label. Shared by paint + hit-test so they can't drift.
    pub(super) fn git_button_rect(&self, top_bar_rect: Rect) -> Rect {
        let center_y = top_bar_rect.origin.y + top_bar_rect.size.y / 2.0;
        // The name is *centred* using the 9 px/char heuristic, but a
        // CJK glyph renders ~14 px wide, so the real right edge is
        // further out — use a CJK-aware estimate so the button clears
        // the (often CJK) file name instead of overlapping it.
        let center_approx = self.title_approx_width();
        let render_w: f32 = self
            .file_name
            .chars()
            .map(|c| if is_wide_glyph(c) { 14.0 } else { 7.5 })
            .sum();
        let filename_left = top_bar_rect.origin.x + (top_bar_rect.size.x - center_approx) / 2.0;
        let filename_right = filename_left + render_w + self.edited_approx_width();
        let branch_w = self
            .git_branch
            .as_deref()
            .map(|b| 6.0 + b.chars().count() as f32 * 7.0)
            .unwrap_or(0.0);
        Rect {
            origin: Point2D::new(filename_right + 10.0, center_y - ICON_BUTTON / 2.0),
            size: Point2D::new(GIT_BUTTON_PAD_X * 2.0 + ICON_SIZE + branch_w, ICON_BUTTON),
        }
    }

    pub(super) fn title_approx_width(&self) -> f32 {
        self.file_name.chars().count() as f32 * 9.0 + self.edited_approx_width()
    }

    pub(super) fn edited_approx_width(&self) -> f32 {
        if self.edited {
            8.0 + self.label_edited.chars().count() as f32 * 7.0
        } else {
            0.0
        }
    }

    pub(super) fn git_icon_left(git_button: Rect) -> f32 {
        git_button.origin.x + GIT_BUTTON_PAD_X
    }

    /// Center-x of the Git-panel toggle button when it is shown
    /// (desktop only — see `GIT_BUTTON_AVAILABLE`). The floating Git
    /// panel anchors its caret here so it reads as a popover hanging
    /// off the button (TS parity); `None` when the button is hidden.
    pub fn git_button_center_x(&self, top_bar_rect: Rect) -> Option<f32> {
        if !GIT_BUTTON_AVAILABLE {
            return None;
        }
        let r = self.git_button_rect(top_bar_rect);
        Some(r.origin.x + r.size.x / 2.0)
    }
}
