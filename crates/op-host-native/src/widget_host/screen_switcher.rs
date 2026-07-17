//! Track C-2 host glue: the APP MODE screen-switcher pill row.
//!
//! Sibling of `preview_frame.rs` (device-frame + device-switcher glue),
//! split into its own file to keep `preview_frame.rs` under the repo's
//! 800-line-per-file cap. Mirrors `preview_frame.rs`'s
//! `paint_preview_switcher` / `preview_switcher_press` / `_release` /
//! `_hover` quartet exactly, but for the [`op_editor_ui::widgets::
//! ScreenSwitcherPills`] row instead of the 3-way device switcher — see
//! that widget's module doc for why the two floating rows never collide.
//!
//! The row is painted/hit-tested for every APP MODE session regardless of
//! device kind (Phone / Desktop / Canvas) — same as the device switcher
//! itself, which paints in every Preview mode, not just the framed ones.

use op_editor_ui::widgets::{PaintCx, ScreenSwitcherPills};
use op_editor_ui::{Point2D, Rect, RenderBackend};

impl super::WidgetHostNative {
    /// Paint the screen-switcher pill row. No-op outside APP MODE (a
    /// single-screen document keeps today's chrome unchanged).
    pub(crate) fn paint_screen_switcher(
        &self,
        frame_backend: &mut dyn RenderBackend,
        canvas_rect: Rect,
    ) {
        let Some(session) = self.preview.as_ref() else {
            return;
        };
        if !session.is_app_mode() {
            return;
        }
        let labels: Vec<String> = session
            .screen_switcher_entries()
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        let switcher = ScreenSwitcherPills {
            labels: &labels,
            active: session.current_screen_index(),
            hover: self.editor_state.editor_ui.screen_switcher_hover,
        };
        let mut cx = PaintCx {
            backend: frame_backend,
        };
        switcher.paint(&mut cx, canvas_rect, &self.theme);
    }

    /// Press on a screen pill — arms it for release, mirroring
    /// `preview_switcher_press`. Returns whether a pill was hit (the
    /// caller swallows the press either way while previewing, but the
    /// return value matches the sibling's contract for symmetry).
    pub(crate) fn screen_switcher_press(
        &mut self,
        x: f32,
        y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) -> bool {
        let Some(session) = self.preview.as_ref() else {
            return false;
        };
        if !session.is_app_mode() {
            return false;
        }
        let count = session.screen_switcher_entries().len();
        let canvas = self.preview_canvas_rect(viewport_w, viewport_h);
        let hit = ScreenSwitcherPills::hit_test(canvas, count, Point2D::new(x, y));
        self.editor_state.editor_ui.screen_switcher_pressed = hit;
        hit.is_some()
    }

    /// Release — activates ONLY when the maintained hover still matches
    /// the pressed index (mirrors `preview_switcher_release`), then
    /// dispatches `router.replace(path)` through the session.
    pub(crate) fn screen_switcher_release(&mut self) -> bool {
        let pressed = self.editor_state.editor_ui.screen_switcher_pressed.take();
        let Some(pressed) = pressed else {
            return false;
        };
        if self.editor_state.editor_ui.screen_switcher_hover == Some(pressed) {
            if let Some(session) = self.preview.as_ref() {
                if let Some((path, _)) = session.screen_switcher_entries().get(pressed) {
                    session.navigate_to_screen(path);
                }
            }
        }
        self.mark_dirty();
        true
    }

    /// Hover tracking, mirroring `preview_switcher_hover`. Clears (rather
    /// than no-ops) when APP MODE just ended so a stale hover index can't
    /// linger into a later single-screen preview.
    pub(crate) fn screen_switcher_hover(
        &mut self,
        x: f32,
        y: f32,
        viewport_w: f32,
        viewport_h: f32,
    ) {
        let Some(session) = self.preview.as_ref() else {
            return;
        };
        if !session.is_app_mode() {
            if self.editor_state.editor_ui.screen_switcher_hover.is_some() {
                self.editor_state.editor_ui.screen_switcher_hover = None;
                self.mark_dirty();
            }
            return;
        }
        let count = session.screen_switcher_entries().len();
        let canvas = self.preview_canvas_rect(viewport_w, viewport_h);
        let hit = ScreenSwitcherPills::hit_test(canvas, count, Point2D::new(x, y));
        if self.editor_state.editor_ui.screen_switcher_hover != hit {
            self.editor_state.editor_ui.screen_switcher_hover = hit;
            self.mark_dirty();
        }
    }
}
