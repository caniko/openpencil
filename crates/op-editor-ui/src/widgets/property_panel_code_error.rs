//! Error-state presentation for the Code panel.
//!
//! Pipeline failures are diagnostic strings. Keep those details bounded and
//! subordinate to a localized heading, while preserving access to an older
//! successful result when a regeneration fails.

use super::code_i18n::CodePanelStrings;
use super::{action_hovered, origin};
use crate::theme::Theme;
use crate::widgets::icons::{draw_icon, Icon};
use crate::widgets::property_panel_action::CodegenAction;
use crate::widgets::property_panel_inputs::{INPUT_HEIGHT, PAD_X};
use crate::widgets::PaintCx;
use crate::{Point2D, Rect, TextLayout};
use op_editor_core::codegen::{CodegenHover, CodegenState};

const CARD_RADIUS: f32 = 10.0;
const CARD_H: f32 = 82.0;
const DIAGNOSTIC_EXTRA_H: f32 = 18.0;
const PREVIOUS_EXTRA_H: f32 = 22.0;
const CARD_PAD: f32 = 12.0;
const ICON_SIZE: f32 = 16.0;
const ICON_GAP: f32 = 10.0;
const DETAIL_FONT_SIZE: f32 = 11.0;
const DETAIL_LINE_H: f32 = 15.0;
const ACTION_GAP: f32 = 8.0;
const ACTION_TOP_GAP: f32 = 12.0;

pub(super) fn display_error_detail(strings: CodePanelStrings, raw: &str) -> String {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let summary = split_diagnostic(&normalized).0.trim();
    let lower = summary.to_ascii_lowercase();
    if lower.contains("all chunks failed") || lower.contains("no code to assemble") {
        strings.no_usable_code().to_owned()
    } else if summary.is_empty() {
        strings.generation_failed().to_owned()
    } else {
        summary.to_owned()
    }
}

fn card_height(state: &CodegenState) -> f32 {
    let mut height = CARD_H;
    if diagnostic_detail(state.error.as_deref().unwrap_or_default()).is_some() {
        height += DIAGNOSTIC_EXTRA_H;
    }
    if !state.code.is_empty() {
        height += PREVIOUS_EXTRA_H;
    }
    height
}

fn action_y(state: &CodegenState, y: f32) -> f32 {
    y + card_height(state) + ACTION_TOP_GAP
}

pub(super) fn error_action_rects(
    state: &CodegenState,
    x: f32,
    y: f32,
    w: f32,
) -> Vec<(CodegenAction, Rect)> {
    let row = Rect {
        origin: Point2D::new(x + PAD_X, action_y(state, y)),
        size: Point2D::new((w - PAD_X * 2.0).max(0.0), INPUT_HEIGHT),
    };
    if state.code.is_empty() {
        return vec![(CodegenAction::Regenerate, row)];
    }

    let third = ((row.size.x - ACTION_GAP * 2.0) / 3.0).max(0.0);
    let copy = Rect {
        origin: row.origin,
        size: Point2D::new(third, row.size.y),
    };
    let download = Rect {
        origin: Point2D::new(row.origin.x + third + ACTION_GAP, row.origin.y),
        size: Point2D::new(third, row.size.y),
    };
    let regenerate = Rect {
        origin: Point2D::new(row.origin.x + (third + ACTION_GAP) * 2.0, row.origin.y),
        size: Point2D::new(third, row.size.y),
    };
    vec![
        (CodegenAction::Copy, copy),
        (CodegenAction::Download, download),
        (CodegenAction::Regenerate, regenerate),
    ]
}

pub(super) fn paint_error_body(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    state: &CodegenState,
    strings: CodePanelStrings,
    x: f32,
    y: f32,
    w: f32,
) -> f32 {
    let card = Rect {
        origin: Point2D::new(x + PAD_X, y),
        size: Point2D::new((w - PAD_X * 2.0).max(0.0), card_height(state)),
    };
    cx.backend.fill_round_rect(card, CARD_RADIUS, theme.muted);
    cx.backend
        .stroke_round_rect(card, CARD_RADIUS, theme.border, 1.0);

    draw_icon(
        cx.backend,
        Icon::AlertTriangle,
        Point2D::new(card.origin.x + CARD_PAD, card.origin.y + CARD_PAD),
        ICON_SIZE,
        theme.destructive,
        1.6,
    );
    let text_x = card.origin.x + CARD_PAD + ICON_SIZE + ICON_GAP;
    let text_w = (card.origin.x + card.size.x - CARD_PAD - text_x).max(1.0);
    draw_text(
        cx,
        strings.generation_failed(),
        text_x,
        card.origin.y + 23.0,
        13.0,
        theme.foreground,
    );

    let detail = display_error_detail(
        strings,
        state
            .error
            .as_deref()
            .unwrap_or(strings.generation_failed()),
    );
    let lines = detail_lines(&detail, text_w, |text| {
        cx.backend.measure_text(text, DETAIL_FONT_SIZE)
    });
    for (index, line) in lines.iter().enumerate() {
        draw_text(
            cx,
            line,
            text_x,
            card.origin.y + 43.0 + index as f32 * DETAIL_LINE_H,
            DETAIL_FONT_SIZE,
            theme.muted_foreground,
        );
    }
    let has_diagnostic = if let Some(diagnostic) = state
        .error
        .as_deref()
        .and_then(diagnostic_detail)
        .map(|detail| format!("Details: {detail}"))
    {
        let diagnostic = crate::util::ellipsize_to_width(&diagnostic, text_w, |text| {
            cx.backend.measure_text(text, DETAIL_FONT_SIZE)
        });
        draw_text(
            cx,
            &diagnostic,
            text_x,
            card.origin.y + 77.0,
            DETAIL_FONT_SIZE,
            theme.muted_foreground,
        );
        true
    } else {
        false
    };
    if !state.code.is_empty() {
        draw_text(
            cx,
            strings.previous_result_available(),
            text_x,
            card.origin.y + if has_diagnostic { 99.0 } else { 81.0 },
            DETAIL_FONT_SIZE,
            theme.muted_foreground,
        );
    }

    let actions = error_action_rects(state, x, y, w);
    for (action, rect) in &actions {
        let (label, primary, hover) = match action {
            CodegenAction::Copy => (strings.copy(), false, CodegenHover::Copy),
            CodegenAction::Download => (strings.save(), false, CodegenHover::Download),
            CodegenAction::Regenerate => (strings.regenerate(), true, CodegenHover::Regenerate),
            _ => continue,
        };
        paint_action_button(
            cx,
            theme,
            label,
            *rect,
            primary,
            action_hovered(state, hover),
        );
    }

    actions
        .last()
        .map(|(_, rect)| rect.origin.y + rect.size.y + 12.0)
        .unwrap_or(card.origin.y + card.size.y)
}

fn diagnostic_detail(raw: &str) -> Option<String> {
    let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let detail = split_diagnostic(&normalized).1?.trim();
    (!detail.is_empty()).then(|| detail.to_owned())
}

fn split_diagnostic(normalized: &str) -> (&str, Option<&str>) {
    let lower = normalized.to_ascii_lowercase();
    let Some(start) = lower.find("details:") else {
        return (normalized, None);
    };
    let detail_start = start + "details:".len();
    (&normalized[..start], Some(&normalized[detail_start..]))
}

fn paint_action_button(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    label: &str,
    rect: Rect,
    primary: bool,
    hovered: bool,
) {
    let variant = if primary {
        jian_widgets::components::button::ButtonVariant::Primary
    } else {
        jian_widgets::components::button::ButtonVariant::Outline
    };
    jian_widgets::components::button::Button {
        label,
        icon_paths: None,
        variant,
        enabled: true,
        hovered,
        pressed: false,
        font_size: 13.0,
    }
    .paint(
        cx.backend,
        rect,
        &crate::widgets::button::tokens_from_theme(theme),
    );
}

fn draw_text(
    cx: &mut PaintCx<'_>,
    text: &str,
    x: f32,
    baseline_y: f32,
    font_size: f32,
    color: crate::Color,
) {
    let layout = TextLayout::single_run(text, "system-ui", font_size, color.to_jian(), origin());
    cx.backend.draw_text(&layout, Point2D::new(x, baseline_y));
}

/// At most two measured lines. Prefer a word boundary for the first line;
/// the second line is ellipsized, so arbitrary provider errors cannot escape
/// the card or make the retry action jump unpredictably.
pub(super) fn detail_lines(
    text: &str,
    max_width: f32,
    mut measure: impl FnMut(&str) -> f32,
) -> Vec<String> {
    if measure(text) <= max_width {
        return vec![text.to_owned()];
    }

    let mut fitting_end = 0usize;
    for (start, ch) in text.char_indices() {
        let end = start + ch.len_utf8();
        if measure(&text[..end]) > max_width {
            break;
        }
        fitting_end = end;
    }
    if fitting_end == 0 {
        return vec!["…".to_owned()];
    }
    let fitting = &text[..fitting_end];
    let first_end = fitting
        .char_indices()
        .filter(|(_, ch)| ch.is_whitespace())
        .map(|(start, ch)| start + ch.len_utf8())
        .next_back()
        .filter(|end| measure(text[..*end].trim_end()) >= max_width * 0.45)
        .unwrap_or(fitting_end);
    let first = text[..first_end].trim_end().to_owned();
    let remaining = text[first_end..].trim_start();
    let second = crate::util::ellipsize_to_width(remaining, max_width, &mut measure);
    vec![first, second]
}
