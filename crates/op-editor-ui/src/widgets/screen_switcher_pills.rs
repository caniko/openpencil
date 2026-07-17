//! APP MODE screen-switcher pill row shared by paint and hit-testing.
//!
//! Track C-2 of the interactive-preview plan: one pill per routed screen,
//! the currently-mounted screen highlighted, click dispatches
//! `router.replace(path)` (the host wires the click, see
//! `op-host-native/src/widget_host/preview_frame.rs`). This is also the
//! escape hatch for a screen whose nav tab Track A never managed to bind to
//! `on_tap` — the row always lets every screen be reached directly.
//!
//! Sibling of `preview_device_switcher.rs` — same paint/hit-test split,
//! positioned directly BELOW its pill (never the same row) so the two
//! floating controls can never collide.

use crate::widgets::file_menu::truncate_to_width;
use crate::widgets::preview_device_switcher::PreviewDeviceSwitcher;
use crate::widgets::PaintCx;
use crate::{Point2D, Rect, TextLayout, Theme};

const PILL_H: f32 = 24.0;
const PILL_W: f32 = 88.0;
const PILL_GAP: f32 = 6.0;
const RADIUS: f32 = 10.0;
/// Gap below the device switcher's pill so the two rows never touch.
const ROW_GAP: f32 = 8.0;
const LABEL_FONT: f32 = 11.0;
const LABEL_PAD_X: f32 = 8.0;

/// One pill per screen. Only meaningful in APP MODE (multi-screen); the
/// host paints/hit-tests this row only when `PreviewSession::is_app_mode`.
pub struct ScreenSwitcherPills<'a> {
    /// Display labels, in the SAME order as the session's screen list
    /// (`PreviewSession::screen_switcher_entries`) — index `i` here is the
    /// path at index `i` there.
    pub labels: &'a [String],
    /// Index of the currently-mounted screen, if resolvable.
    pub active: Option<usize>,
    /// Index of the pill under the cursor (hover wash).
    pub hover: Option<usize>,
}

impl ScreenSwitcherPills<'_> {
    fn row_width(count: usize) -> f32 {
        if count == 0 {
            return 0.0;
        }
        count as f32 * PILL_W + count.saturating_sub(1) as f32 * PILL_GAP
    }

    /// Row bounds: horizontally centered like the device switcher's pill,
    /// its top edge `ROW_GAP` below that pill's bottom edge.
    pub fn row_rect(canvas: Rect, count: usize) -> Rect {
        let width = Self::row_width(count);
        let device_pill = PreviewDeviceSwitcher::pill_rect(canvas);
        Rect {
            origin: Point2D::new(
                canvas.origin.x + (canvas.size.x - width) / 2.0,
                device_pill.origin.y + device_pill.size.y + ROW_GAP,
            ),
            size: Point2D::new(width, PILL_H),
        }
    }

    /// Fixed-width pill rects (no text measurement needed) so hit-testing
    /// never depends on a `RenderBackend` — only paint truncates labels.
    pub fn pill_rects(canvas: Rect, count: usize) -> Vec<Rect> {
        let row = Self::row_rect(canvas, count);
        (0..count)
            .map(|i| Rect {
                origin: Point2D::new(row.origin.x + i as f32 * (PILL_W + PILL_GAP), row.origin.y),
                size: Point2D::new(PILL_W, PILL_H),
            })
            .collect()
    }

    pub fn hit_test(canvas: Rect, count: usize, point: Point2D) -> Option<usize> {
        Self::pill_rects(canvas, count)
            .into_iter()
            .enumerate()
            .find_map(|(index, rect)| {
                (point.x >= rect.origin.x
                    && point.x <= rect.origin.x + rect.size.x
                    && point.y >= rect.origin.y
                    && point.y <= rect.origin.y + rect.size.y)
                    .then_some(index)
            })
    }

    pub fn paint(&self, cx: &mut PaintCx<'_>, canvas: Rect, theme: &Theme) {
        let rects = Self::pill_rects(canvas, self.labels.len());
        for (index, rect) in rects.into_iter().enumerate() {
            let active = self.active == Some(index);
            let fill = if active {
                theme.accent
            } else if self.hover == Some(index) {
                theme.muted
            } else {
                theme.card
            };
            cx.backend.fill_round_rect(rect, RADIUS, fill);
            cx.backend
                .stroke_round_rect(rect, RADIUS, theme.border, 1.0);

            let max_label_w = (rect.size.x - LABEL_PAD_X * 2.0).max(0.0);
            let label = truncate_to_width(cx, &self.labels[index], LABEL_FONT, max_label_w);
            let text_width = cx.backend.measure_text(&label, LABEL_FONT);
            let text_color = if active {
                theme.accent_foreground
            } else {
                theme.foreground
            };
            let layout = TextLayout::single_run(
                &label,
                "system-ui",
                LABEL_FONT,
                text_color.to_jian(),
                Point2D::new(0.0, 0.0),
            );
            let text_x = rect.origin.x + (rect.size.x - text_width) / 2.0;
            let baseline_y = rect.origin.y + rect.size.y / 2.0 + 4.0;
            cx.backend
                .draw_text(&layout, Point2D::new(text_x, baseline_y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Point2D, Rect};

    fn canvas() -> Rect {
        Rect {
            origin: Point2D::new(100.0, 40.0),
            size: Point2D::new(1000.0, 700.0),
        }
    }

    #[test]
    fn row_is_centered_below_device_switcher_pill() {
        let row = ScreenSwitcherPills::row_rect(canvas(), 3);
        let device_pill = PreviewDeviceSwitcher::pill_rect(canvas());
        assert_eq!(row.size.y, PILL_H);
        assert!((row.origin.y - (device_pill.origin.y + device_pill.size.y + ROW_GAP)).abs() < 0.5);
        let expected_w = 3.0 * PILL_W + 2.0 * PILL_GAP;
        assert!((row.size.x - expected_w).abs() < 0.5);
        assert!(
            (row.origin.x - (canvas().origin.x + (canvas().size.x - expected_w) / 2.0)).abs() < 0.5
        );
    }

    #[test]
    fn zero_screens_yields_empty_row() {
        let row = ScreenSwitcherPills::row_rect(canvas(), 0);
        assert_eq!(row.size.x, 0.0);
        assert!(ScreenSwitcherPills::pill_rects(canvas(), 0).is_empty());
    }

    #[test]
    fn pills_are_evenly_spaced_and_hit_test_by_index() {
        let rects = ScreenSwitcherPills::pill_rects(canvas(), 3);
        assert_eq!(rects.len(), 3);
        for rect in &rects {
            assert_eq!(rect.size.x, PILL_W);
            assert_eq!(rect.size.y, PILL_H);
        }
        assert!((rects[1].origin.x - (rects[0].origin.x + PILL_W + PILL_GAP)).abs() < 0.5);

        let midpoint = |rect: Rect| {
            Point2D::new(
                rect.origin.x + rect.size.x / 2.0,
                rect.origin.y + rect.size.y / 2.0,
            )
        };
        assert_eq!(
            ScreenSwitcherPills::hit_test(canvas(), 3, midpoint(rects[2])),
            Some(2)
        );
        assert_eq!(
            ScreenSwitcherPills::hit_test(canvas(), 3, Point2D::new(0.0, 0.0)),
            None
        );
    }
}
