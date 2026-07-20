//! Framework selector geometry, paint, and hit-test helpers for the Code
//! panel. Kept separate because the selector is a self-contained horizontal
//! scrolling control shared by native and web hosts.

use super::{action_hovered, code_neutral_hover_color};
use crate::theme::Theme;
use crate::widgets::icons::{draw_icon, Icon};
use crate::widgets::property_panel_inputs::{
    PAD_X, SECTION_GAP, SECTION_HEADER_HEIGHT, TAB_HEIGHT,
};
use crate::widgets::PaintCx;
use crate::{Point2D, Rect};
use op_editor_core::codegen::{CodegenHover, CodegenPhase, CodegenState, Framework};

const CHIP_HEIGHT: f32 = 22.0;
const CHIP_PAD_X: f32 = 8.0;
const CHIP_FONT_SIZE: f32 = 11.0;
const CHIP_GAP: f32 = 2.0;
pub(super) const CHEVRON_ZONE_W: f32 = 18.0;
const CHIP_DIVIDER_GAP: f32 = 8.0;

fn framework_tab_label(fw: Framework) -> &'static str {
    match fw {
        Framework::React => "React",
        Framework::Vue => "Vue",
        Framework::Svelte => "Svelte",
        Framework::Html => "HTML",
        Framework::Flutter => "Flutter",
        Framework::SwiftUi => "SwiftUI",
        Framework::Compose => "Compose",
        Framework::ReactNative => "RN",
    }
}

fn chip_label_width(label: &str) -> f32 {
    label.chars().fold(0.0, |width, ch| {
        width
            + if ch.is_ascii() {
                CHIP_FONT_SIZE * 0.55
            } else {
                CHIP_FONT_SIZE
            }
    })
}

fn framework_row_width() -> f32 {
    Framework::ALL
        .iter()
        .enumerate()
        .map(|(index, framework)| {
            let gap = if index == 0 { 0.0 } else { CHIP_GAP };
            gap + chip_label_width(framework_tab_label(*framework)) + CHIP_PAD_X * 2.0
        })
        .sum()
}

fn framework_overflows(width: f32) -> bool {
    framework_row_width() > (width - PAD_X * 2.0).max(0.0)
}

pub fn framework_row_overflow(width: f32) -> f32 {
    let usable = (width - PAD_X * 2.0).max(0.0);
    if framework_row_width() <= usable {
        return 0.0;
    }
    (framework_row_width() - (usable - 2.0 * CHEVRON_ZONE_W)).max(0.0)
}

pub fn framework_row_band(panel_top: f32) -> (f32, f32) {
    let top = panel_top + TAB_HEIGHT + SECTION_HEADER_HEIGHT;
    (top, top + CHIP_HEIGHT)
}

pub(super) fn framework_chip_rects(
    x: f32,
    y: f32,
    width: f32,
    scroll: f32,
) -> Vec<(Framework, Rect)> {
    let inset = if framework_overflows(width) {
        CHEVRON_ZONE_W
    } else {
        0.0
    };
    let widths: Vec<f32> = Framework::ALL
        .iter()
        .map(|framework| chip_label_width(framework_tab_label(*framework)) + CHIP_PAD_X * 2.0)
        .collect();
    let advances: Vec<f32> = widths.iter().map(|width| width + CHIP_GAP).collect();
    let rects = jian_widgets::components::tabs::Tabs::content_rects(
        Point2D::new(x + PAD_X + inset, y),
        &widths,
        &advances,
        CHIP_HEIGHT,
        scroll,
    );
    Framework::ALL.iter().copied().zip(rects).collect()
}

pub(super) fn chips_body_top(y: f32) -> f32 {
    y + CHIP_HEIGHT + CHIP_DIVIDER_GAP + SECTION_GAP
}

pub(super) fn framework_chevron_zones(x: f32, y: f32, width: f32) -> Option<(Rect, Rect)> {
    if !framework_overflows(width) {
        return None;
    }
    let band_left = x + PAD_X;
    let band_right = x + width - PAD_X;
    Some((
        Rect {
            origin: Point2D::new(band_left, y),
            size: Point2D::new(CHEVRON_ZONE_W, CHIP_HEIGHT),
        },
        Rect {
            origin: Point2D::new(band_right - CHEVRON_ZONE_W, y),
            size: Point2D::new(CHEVRON_ZONE_W, CHIP_HEIGHT),
        },
    ))
}

pub fn framework_at(x: f32, y: f32, width: f32, point: Point2D, scroll: f32) -> Option<Framework> {
    let usable = (width - PAD_X * 2.0).max(0.0);
    let inset = if framework_row_overflow(width) > 0.0 {
        CHEVRON_ZONE_W
    } else {
        0.0
    };
    let band_left = x + PAD_X + inset;
    let band_right = x + PAD_X + usable - inset;
    framework_chip_rects(x, y, width, scroll)
        .into_iter()
        .filter(|(_, rect)| rect.origin.x + rect.size.x > band_left && rect.origin.x < band_right)
        .find(|(_, rect)| {
            point.x >= rect.origin.x.max(band_left)
                && point.x <= (rect.origin.x + rect.size.x).min(band_right)
                && point.y >= rect.origin.y
                && point.y <= rect.origin.y + rect.size.y
        })
        .map(|(framework, _)| framework)
}

fn paint_chevron(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    icon: Icon,
    zone: Rect,
    enabled: bool,
    hovered: bool,
) {
    cx.backend.fill_round_rect(zone, 6.0, theme.muted);
    if hovered {
        cx.backend
            .fill_round_rect(zone, 6.0, code_neutral_hover_color(theme));
    }
    let glyph = 14.0;
    let color = if enabled {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    draw_icon(
        cx.backend,
        icon,
        Point2D::new(
            zone.origin.x + (zone.size.x - glyph) / 2.0,
            zone.origin.y + (zone.size.y - glyph) / 2.0,
        ),
        glyph,
        color,
        1.6,
    );
}

pub(super) fn paint_framework_chips(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    state: &CodegenState,
    x: f32,
    y: f32,
    width: f32,
) -> f32 {
    let usable = (width - PAD_X * 2.0).max(0.0);
    let zones = framework_chevron_zones(x, y, width);
    let inset = if zones.is_some() { CHEVRON_ZONE_W } else { 0.0 };
    let band = Rect {
        origin: Point2D::new(x + PAD_X + inset, y),
        size: Point2D::new((usable - inset * 2.0).max(0.0), CHIP_HEIGHT),
    };
    cx.backend.save();
    cx.backend.clip_rect(band);
    let labels: Vec<&str> = Framework::ALL
        .iter()
        .map(|framework| framework_tab_label(*framework))
        .collect();
    let rects: Vec<Rect> = framework_chip_rects(x, y, width, state.framework_scroll.offset)
        .into_iter()
        .map(|(_, chip)| chip)
        .collect();
    let active = Framework::ALL
        .iter()
        .position(|framework| *framework == state.framework)
        .unwrap_or(0);
    let interactive = !matches!(state.phase, CodegenPhase::Generating);
    let hover = interactive
        .then_some(state.framework_hover)
        .flatten()
        .and_then(|hovered| {
            Framework::ALL
                .iter()
                .position(|framework| *framework == hovered)
        });
    jian_widgets::components::tabs::Tabs {
        labels: &labels,
        active,
        hover,
    }
    .paint_content(
        cx.backend,
        &rects,
        jian_widgets::components::tabs::ActiveStyle::PrimaryPill,
        false,
        CHIP_PAD_X,
        CHIP_FONT_SIZE,
        &crate::widgets::button::tokens_from_theme(theme),
    );
    cx.backend.restore();

    if let Some((left, right)) = zones {
        let max = framework_row_overflow(width);
        paint_chevron(
            cx,
            theme,
            Icon::ChevronLeft,
            left,
            interactive && state.framework_scroll.offset > 0.0,
            interactive && action_hovered(state, CodegenHover::ScrollFrameworksLeft),
        );
        paint_chevron(
            cx,
            theme,
            Icon::ChevronRight,
            right,
            interactive && state.framework_scroll.offset < max,
            interactive && action_hovered(state, CodegenHover::ScrollFrameworksRight),
        );
    }
    let divider_y = y + CHIP_HEIGHT + CHIP_DIVIDER_GAP / 2.0;
    cx.backend.fill_rect(
        Rect {
            origin: Point2D::new(x + PAD_X, divider_y),
            size: Point2D::new(usable, 1.0),
        },
        theme.border,
    );
    chips_body_top(y)
}
