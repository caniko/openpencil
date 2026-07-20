use std::cmp::Ordering;

use jian_ops_schema::constraints::{Constraints, HConstraint, VConstraint};
use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::container::{AlignItems, ContainerProps, LayoutMode};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};

use crate::css::cascade::{compute_style_for_viewport, ComputedStyle};
use crate::dom::{DomElement, DomNode};
use crate::length::{parse_length, LengthCtx};

use super::MapCtx;

pub(crate) fn infer_child_alignment(
    context: &MapCtx<'_>,
    path: &[&DomElement],
    parent_style: &ComputedStyle,
    children: &[DomNode],
) -> Option<AlignItems> {
    let styles: Vec<_> = children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(element) => {
                let mut child_path = path.to_vec();
                child_path.push(element);
                let style = compute_style_for_viewport(
                    &child_path,
                    context.rules,
                    Some(parent_style),
                    context.opts.base_font_size,
                    context.opts.viewport_width,
                    context.opts.viewport_width * 0.625,
                );
                (style.get("display") != Some("none")
                    && !matches!(style.get("position"), Some("absolute" | "fixed")))
                .then_some(style)
            }
            DomNode::Text(text) if text.trim().is_empty() => None,
            DomNode::Text(_) => None,
        })
        .collect();
    (!styles.is_empty()
        && styles.iter().all(|style| {
            style.get("margin-left") == Some("auto") && style.get("margin-right") == Some("auto")
        }))
    .then_some(AlignItems::Center)
}

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
            .map(|length| length.resolve(0.0))
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
    parent_width_is_definite: bool,
) {
    let parent_layout = parent.map(layout_for).unwrap_or(LayoutMode::Vertical);
    if container.width.is_none() && parent.is_some() {
        let shrink_to_fit = parent_layout == LayoutMode::Horizontal
            || is_inline_level(style)
            || !parent_width_is_definite;
        container.width = Some(SizingBehavior::Keyword(if shrink_to_fit {
            SizingKeyword::FitContent
        } else {
            SizingKeyword::FillContainer
        }));
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

pub(super) fn width_is_definite(
    width: Option<&SizingBehavior>,
    parent_width_is_definite: bool,
) -> bool {
    match width {
        Some(SizingBehavior::Number(value)) => value.is_finite(),
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)) => parent_width_is_definite,
        None => parent_width_is_definite,
        _ => false,
    }
}

pub(super) fn resolved_axis(
    sizing: Option<&SizingBehavior>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    fallback: f64,
) -> f64 {
    let mut value = match sizing {
        Some(SizingBehavior::Number(value)) if value.is_finite() => *value,
        _ => fallback,
    };
    if let Some(maximum) = maximum {
        value = value.min(maximum);
    }
    if let Some(minimum) = minimum {
        value = value.max(minimum);
    }
    value.max(0.0)
}

pub(super) fn has_non_auto(style: &ComputedStyle, property: &str) -> bool {
    style
        .get(property)
        .is_some_and(|value| !value.trim().eq_ignore_ascii_case("auto"))
}

pub(super) fn establishes_positioning_context(style: &ComputedStyle) -> bool {
    !matches!(style.get("position"), None | Some("static"))
        || style.get("transform").is_some_and(|value| value != "none")
}

/// Non-responsive Jian documents do not consume `SizeLimits` during layout.
/// Preserve the limits for serialization, but also bake any constraint that
/// changes the imported size at the selected viewport into the legacy axis.
pub(super) fn apply_legacy_size_limits(
    container: &mut ContainerProps,
    available_width: f64,
    available_height: f64,
) {
    constrain_legacy_axis(
        &mut container.width,
        container.limits.min_width,
        container.limits.max_width,
        available_width,
    );
    constrain_legacy_axis(
        &mut container.height,
        container.limits.min_height,
        container.limits.max_height,
        available_height,
    );
}

fn constrain_legacy_axis(
    sizing: &mut Option<SizingBehavior>,
    minimum: Option<f64>,
    maximum: Option<f64>,
    available: f64,
) {
    let clamp = |mut value: f64| {
        if let Some(maximum) = maximum {
            value = value.min(maximum);
        }
        if let Some(minimum) = minimum {
            value = value.max(minimum);
        }
        value.max(0.0)
    };
    match sizing {
        Some(SizingBehavior::Number(value)) => *value = clamp(*value),
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)) => {
            let constrained = clamp(available);
            if (constrained - available).abs() > f64::EPSILON {
                *sizing = Some(SizingBehavior::Number(constrained));
            }
        }
        Some(SizingBehavior::Keyword(SizingKeyword::FitContent)) | None => {
            if let Some(minimum) = minimum {
                *sizing = Some(SizingBehavior::Number(clamp(minimum)));
            }
        }
        Some(SizingBehavior::Expression(_)) => {
            if minimum.is_some() || maximum.is_some() {
                *sizing = Some(SizingBehavior::Number(clamp(available)));
            }
        }
    }
}

fn is_inline_level(style: &ComputedStyle) -> bool {
    matches!(
        style.get("display").map(str::trim),
        Some(
            "inline"
                | "inline flow"
                | "inline-block"
                | "inline flow-root"
                | "inline-flex"
                | "inline flex"
                | "inline-grid"
                | "inline grid"
        )
    )
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
    let (left, left_percent) = position_value(
        style.get("left"),
        style.font_size,
        context.containing_width,
        context,
    );
    let (top, top_percent) = position_value(
        style.get("top"),
        style.font_size,
        context.containing_height,
        context,
    );
    // Jian treats a child as absolute as soon as either coordinate is
    // authored. CSS `position:absolute` without offsets therefore maps to
    // the static origin instead of accidentally entering auto layout.
    let (right, right_percent) = position_value(
        style.get("right"),
        style.font_size,
        context.containing_width,
        context,
    );
    let (bottom, bottom_percent) = position_value(
        style.get("bottom"),
        style.font_size,
        context.containing_height,
        context,
    );
    let own_width = own_size(
        style.get("width"),
        style.font_size,
        context.containing_width,
        context,
    );
    let own_height = own_size(
        style.get("height"),
        style.font_size,
        context.containing_height,
        context,
    );
    let x = left.or_else(|| right.map(|right| context.containing_width - own_width - right));
    let y = top.or_else(|| bottom.map(|bottom| context.containing_height - own_height - bottom));
    base.x = Some(x.unwrap_or(0.0));
    base.y = Some(y.unwrap_or(0.0));
    let has_left = has_offset(style, "left");
    let has_right = has_offset(style, "right");
    let has_top = has_offset(style, "top");
    let has_bottom = has_offset(style, "bottom");
    base.constraints = Some(Constraints {
        h: match (has_left, has_right) {
            (true, true) => HConstraint::LeftRight,
            (false, true) => HConstraint::Right,
            _ => HConstraint::Left,
        },
        v: match (has_top, has_bottom) {
            (true, true) => VConstraint::TopBottom,
            (false, true) => VConstraint::Bottom,
            _ => VConstraint::Top,
        },
    });
    if left_percent || top_percent || right_percent || bottom_percent {
        context.warn_once(
            "percentage absolute offsets resolved against the inferred containing block",
        );
    }
}

fn own_size(value: Option<&str>, font_size: f64, reference: f64, context: &MapCtx<'_>) -> f64 {
    value
        .and_then(|value| {
            parse_length(
                value,
                &LengthCtx {
                    font_size,
                    root_font_size: context.opts.base_font_size,
                    viewport_w: context.opts.viewport_width,
                    viewport_h: context.opts.viewport_width * 0.625,
                },
            )
        })
        .map(|length| length.resolve(reference))
        .filter(|value| value.is_finite())
        .unwrap_or(0.0)
}

fn has_offset(style: &ComputedStyle, name: &str) -> bool {
    style
        .get(name)
        .is_some_and(|value| !value.trim().eq_ignore_ascii_case("auto"))
}

fn position_value(
    value: Option<&str>,
    font_size: f64,
    reference: f64,
    context: &MapCtx<'_>,
) -> (Option<f64>, bool) {
    let Some(value) = value else {
        return (None, false);
    };
    let length = parse_length(
        value,
        &LengthCtx {
            font_size,
            root_font_size: context.opts.base_font_size,
            viewport_w: context.opts.viewport_width,
            viewport_h: context.opts.viewport_width * 0.625,
        },
    );
    length.map_or((None, false), |length| {
        let depends_on_reference = length.depends_on_reference();
        (Some(length.resolve(reference)), depends_on_reference)
    })
}

pub(super) fn resolve_absolute_fill(sizing: &mut Option<SizingBehavior>, reference: f64) {
    if matches!(
        sizing,
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
    ) {
        *sizing = Some(SizingBehavior::Number(reference.max(0.0)));
    }
}

pub(super) fn stretched_absolute_axis(
    style: &ComputedStyle,
    context: &MapCtx<'_>,
    start: &str,
    end: &str,
    reference: f64,
) -> f64 {
    let inset = |name: &str| {
        position_value(style.get(name), style.font_size, reference, context)
            .0
            .unwrap_or(0.0)
    };
    (reference - inset(start) - inset(end)).max(0.0)
}

pub(super) fn warn_for_degradations(
    style: &ComputedStyle,
    supports_image_blend: bool,
    context: &mut MapCtx<'_>,
) {
    if style.get("position") == Some("relative")
        && ["top", "right", "bottom", "left"]
            .iter()
            .any(|property| has_offset(style, property))
    {
        context.warn_once(
            "CSS relative-position offsets were ignored to preserve auto-layout flow order",
        );
    }
    if style.get("position") == Some("sticky") {
        context.warn_once("CSS position:sticky has no static canvas equivalent and was ignored");
    }
    if matches!(style.get("display"), Some("grid" | "inline-grid")) {
        if let Some(columns) = style.get("grid-template-columns") {
            if super::grid::grid_column_count(style).is_none()
                && !columns.contains("auto-fit")
                && !columns.contains("auto-fill")
            {
                context.warn_once("unsupported CSS grid tracks approximated as a vertical frame");
            }
        }
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
    if !supports_image_blend
        && style
            .get("mix-blend-mode")
            .is_some_and(|value| value != "normal")
    {
        context.warn_once(
            "CSS mix-blend-mode has no node-level equivalent; fill blending is used where possible",
        );
    }
}
