//! Shared geometry + paint for the Position section's per-corner editor.

use crate::theme::Theme;
use crate::widgets::icons::{draw_icon, Icon};
use crate::widgets::property_panel_inputs::{INPUT_HEIGHT, PAD_X};
use crate::widgets::property_panel_sections::EditContext;
use crate::widgets::PaintCx;
use crate::{Point2D, Rect, TextLayout};
use op_editor_core::PropertyFocus;

pub const CORNER_INPUT_HEIGHT: f32 = 28.0;
pub const CORNER_GRID_EXTRA_HEIGHT: f32 = 68.0;

#[derive(Debug, Clone, Copy)]
pub struct CornerGridRects {
    pub tl: Rect,
    pub tr: Rect,
    pub bl: Rect,
    pub br: Rect,
    pub glyph: Rect,
}

pub fn uniform_and_toggle_rects(x: f32, y: f32, width: f32) -> (Rect, Rect) {
    let usable_w = width - PAD_X * 2.0;
    let half_w = (usable_w - 8.0) / 2.0;
    let right_x = x + PAD_X + half_w + 8.0;
    let toggle_w = 24.0;
    (
        Rect {
            origin: Point2D::new(right_x, y),
            size: Point2D::new(half_w - toggle_w - 4.0, INPUT_HEIGHT),
        },
        Rect {
            origin: Point2D::new(right_x + half_w - toggle_w, y + 3.0),
            size: Point2D::new(toggle_w, 24.0),
        },
    )
}

pub fn grid_rects(x: f32, y: f32, width: f32) -> CornerGridRects {
    let left = x + PAD_X;
    let usable_w = width - PAD_X * 2.0;
    let input_w = (usable_w - 38.0) / 2.0;
    let right = left + usable_w - input_w;
    let top = y + 6.0;
    let bottom = top + CORNER_INPUT_HEIGHT + 6.0;
    CornerGridRects {
        tl: Rect::xywh(left, top, input_w, CORNER_INPUT_HEIGHT),
        tr: Rect::xywh(right, top, input_w, CORNER_INPUT_HEIGHT),
        bl: Rect::xywh(left, bottom, input_w, CORNER_INPUT_HEIGHT),
        br: Rect::xywh(right, bottom, input_w, CORNER_INPUT_HEIGHT),
        glyph: Rect::xywh(left + input_w + 7.0, top + 18.0, 24.0, 24.0),
    }
}

pub fn input_rects(x: f32, y: f32, width: f32) -> [(PropertyFocus, Rect); 4] {
    let grid = grid_rects(x, y, width);
    [
        (PropertyFocus::CornerTL, grid.tl),
        (PropertyFocus::CornerTR, grid.tr),
        (PropertyFocus::CornerBL, grid.bl),
        (PropertyFocus::CornerBR, grid.br),
    ]
}

pub fn radii_are_uniform(radii: [f32; 4]) -> bool {
    radii
        .iter()
        .all(|value| (*value - radii[0]).abs() < f32::EPSILON)
}

pub fn value_for_focus(radii: [f32; 4], focus: PropertyFocus) -> Option<f32> {
    match focus {
        PropertyFocus::CornerTL => Some(radii[0]),
        PropertyFocus::CornerTR => Some(radii[1]),
        PropertyFocus::CornerBR => Some(radii[2]),
        PropertyFocus::CornerBL => Some(radii[3]),
        _ => None,
    }
}

pub fn paint_toggle(cx: &mut PaintCx<'_>, theme: &Theme, rect: Rect, expanded: bool) {
    if expanded {
        cx.backend.fill_round_rect(rect, 6.0, theme.accent);
    }
    draw_icon(
        cx.backend,
        Icon::SquareRoundCorner,
        Point2D::new(rect.origin.x + 4.0, rect.origin.y + 4.0),
        16.0,
        if expanded {
            theme.primary
        } else {
            theme.muted_foreground
        },
        1.4,
    );
}

pub fn paint_tooltip(cx: &mut PaintCx<'_>, theme: &Theme, anchor: Rect, label: &str) {
    let width = cx.backend.measure_text(label, 10.0) + 14.0;
    let rect = Rect::xywh(
        anchor.origin.x + anchor.size.x - width,
        anchor.origin.y - 25.0,
        width,
        22.0,
    );
    cx.backend.fill_round_rect(rect, 5.0, theme.popover);
    cx.backend.stroke_round_rect(rect, 5.0, theme.border, 1.0);
    let text = TextLayout::single_run(
        label,
        "system-ui",
        10.0,
        theme.popover_foreground.to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &text,
        Point2D::new(rect.origin.x + 7.0, rect.origin.y + 15.0),
    );
}

pub fn paint_grid(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    edit: &EditContext<'_>,
    radii: [f32; 4],
    x: f32,
    y: f32,
    width: f32,
) {
    let grid = grid_rects(x, y, width);
    for (focus, rect) in input_rects(x, y, width) {
        let value = format!(
            "{}",
            value_for_focus(radii, focus).unwrap_or(0.0).round() as i32
        );
        let prefix = match focus {
            PropertyFocus::CornerTL => "TL",
            PropertyFocus::CornerTR => "TR",
            PropertyFocus::CornerBL => "BL",
            PropertyFocus::CornerBR => "BR",
            _ => unreachable!(),
        };
        crate::widgets::property_panel_inputs::paint_input_with_prefix_focused_state(
            cx,
            theme,
            rect,
            prefix,
            edit.value_for(focus, &value),
            edit.focus == Some(focus),
            edit.caret_at(focus),
            edit.select_all_at(focus),
            edit.input_at(focus),
            edit.now_ms,
        );
    }
    draw_icon(
        cx.backend,
        Icon::SquareRoundCorner,
        grid.glyph.origin,
        grid.glyph.size.x,
        theme.muted_foreground,
        1.2,
    );
}
