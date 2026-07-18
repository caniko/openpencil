use std::cmp::Ordering;

use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::container::{ContainerProps, LayoutMode};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};

use crate::css::cascade::ComputedStyle;
use crate::length::{parse_length, CssLength, LengthCtx};

use super::MapCtx;

pub fn infer_gap_from_margins(
    child_styles: &[&ComputedStyle],
    context_font: f64,
) -> (Option<f64>, bool) {
    if child_styles.len() < 2 {
        return (None, false);
    }
    let context = LengthCtx {
        font_size: context_font,
        root_font_size: context_font,
        viewport_w: 0.0,
        viewport_h: 0.0,
    };
    let margin = |style: &ComputedStyle, name: &str| {
        style
            .get(name)
            .and_then(|value| parse_length(value, &context))
            .and_then(|length| match length {
                CssLength::Px(value) => Some(value),
                CssLength::Percent(_) => None,
            })
            .unwrap_or(0.0)
    };
    let mut gaps: Vec<_> = child_styles
        .windows(2)
        .map(|pair| margin(pair[0], "margin-bottom") + margin(pair[1], "margin-top"))
        .collect();
    gaps.sort_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));
    let mut modes: Vec<(f64, usize)> = Vec::new();
    for gap in &gaps {
        if let Some((_, count)) = modes.iter_mut().find(|(value, _)| *value == *gap) {
            *count += 1;
        } else {
            modes.push((*gap, 1));
        }
    }
    let mode = modes
        .into_iter()
        .max_by(|(left_value, left_count), (right_value, right_count)| {
            left_count.cmp(right_count).then_with(|| {
                right_value
                    .partial_cmp(left_value)
                    .unwrap_or(Ordering::Equal)
            })
        })
        .map(|(value, _)| value);
    let deviated = mode.is_some_and(|mode| gaps.iter().any(|gap| *gap != mode));
    (mode, deviated)
}

pub(super) fn apply_sizing_defaults(
    container: &mut ContainerProps,
    style: &ComputedStyle,
    parent: Option<&ComputedStyle>,
) {
    let parent_layout = parent.map(layout_for).unwrap_or(LayoutMode::Vertical);
    if container.width.is_none() && parent.is_some() && parent_layout == LayoutMode::Vertical {
        container.width = Some(SizingBehavior::Keyword(SizingKeyword::FillContainer));
    }
    if container.height.is_none() {
        container.height = Some(SizingBehavior::Keyword(SizingKeyword::FitContent));
    }
    if style
        .get("flex-grow")
        .and_then(|value| value.parse::<f64>().ok())
        .is_some_and(|value| value > 0.0)
    {
        let fill = Some(SizingBehavior::Keyword(SizingKeyword::FillContainer));
        if parent_layout == LayoutMode::Horizontal {
            container.width = fill;
        } else {
            container.height = fill;
        }
    }
}

fn layout_for(style: &ComputedStyle) -> LayoutMode {
    match (style.get("display"), style.get("flex-direction")) {
        (Some("flex" | "inline-flex"), Some("column" | "column-reverse")) => LayoutMode::Vertical,
        (Some("flex" | "inline-flex"), _) => LayoutMode::Horizontal,
        _ => LayoutMode::Vertical,
    }
}

pub(super) fn apply_position(
    base: &mut PenNodeBase,
    style: &ComputedStyle,
    context: &mut MapCtx<'_>,
) {
    if !matches!(style.get("position"), Some("absolute" | "fixed")) {
        return;
    }
    let (x, x_percent) = position_value(
        style.get("left"),
        style.font_size,
        context.opts.viewport_width,
        context.opts.base_font_size,
    );
    let (y, y_percent) = position_value(
        style.get("top"),
        style.font_size,
        context.opts.viewport_width * 0.625,
        context.opts.base_font_size,
    );
    base.x = x;
    base.y = y;
    if x_percent || y_percent {
        context.warn_once("percentage absolute offsets approximated against the import viewport");
    }
}

fn position_value(
    value: Option<&str>,
    font_size: f64,
    viewport: f64,
    root_font_size: f64,
) -> (Option<f64>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    let length = parse_length(
        value,
        &LengthCtx {
            font_size,
            root_font_size,
            viewport_w: viewport,
            viewport_h: viewport,
        },
    );
    match length {
        Some(CssLength::Px(value)) => (Some(value), false),
        Some(CssLength::Percent(percent)) => (Some(viewport * percent / 100.0), true),
        None => (None, false),
    }
}

pub(super) fn warn_for_degradations(style: &ComputedStyle, context: &mut MapCtx<'_>) {
    if style.get("display") == Some("grid") {
        context.warn_once("CSS grid approximated as a vertical auto-layout frame");
    }
    if matches!(style.get("flex-wrap"), Some("wrap" | "wrap-reverse")) {
        context.warn_once("flex-wrap ignored; imported auto-layout does not wrap");
    }
    if style
        .get("float")
        .is_some_and(|value| value != "none" && value != "initial")
    {
        context.warn_once("CSS float ignored during structured HTML import");
    }
}
