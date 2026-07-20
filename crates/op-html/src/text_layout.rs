use jian_ops_schema::node::text::TextGrowth;
use jian_ops_schema::sizing::{SizeLimits, SizingBehavior, SizingKeyword};

use crate::css::cascade::ComputedStyle;
use crate::dom::DomElement;
use crate::length::{parse_length, CssLength, LengthCtx};
use crate::mapper::MapCtx;

pub(super) struct TextBox {
    pub width: Option<SizingBehavior>,
    pub height: Option<SizingBehavior>,
    pub limits: SizeLimits,
    pub growth: Option<TextGrowth>,
}

pub(super) fn map_text_box(
    style: &ComputedStyle,
    path: &[&DomElement],
    context: &MapCtx<'_>,
) -> TextBox {
    let available_width = context.containing_width.max(0.0);
    let min_width = length(style, "min-width", available_width, context);
    let max_width = length(style, "max-width", available_width, context);
    let limits = SizeLimits {
        min_width,
        max_width,
        min_height: None,
        max_height: None,
    };

    let authored_width = style
        .get("width")
        .filter(|value| !value.eq_ignore_ascii_case("auto"))
        .and_then(|value| parse(style, value, context));
    let mut width = match authored_width {
        Some(CssLength::Percent(percent))
            if (percent - 100.0).abs() <= f64::EPSILON
                && min_width.is_none()
                && max_width.is_none() =>
        {
            Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
        }
        Some(value) => Some(SizingBehavior::Number(clamp(
            value.resolve(available_width),
            min_width,
            max_width,
        ))),
        None if max_width.is_some() => Some(SizingBehavior::Number(clamp(
            available_width,
            min_width,
            max_width,
        ))),
        None if min_width.is_some_and(|minimum| minimum > available_width) => {
            Some(SizingBehavior::Number(min_width.unwrap_or(available_width)))
        }
        // Let Taffy's text measure callback provide max-content/min-content
        // sizes for an auto-width flex item. Giving the leaf `fit_content`
        // together with FixedWidth makes it settle on min-content even when
        // ample space exists (for example, "全部分类 →" wrapped only the arrow).
        None if !context.containing_width_is_definite => None,
        None if is_block_text_container(style, path) => {
            Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
        }
        None => None,
    };
    if matches!(width.as_ref(), Some(SizingBehavior::Number(value)) if !value.is_finite()) {
        width = None;
    }

    // This is an anonymous text box inside the mapped element frame. CSS
    // block height constraints belong to that host frame; copying them onto
    // its text leaf defeats flex/grid centering and stretches a single line
    // to the full control or artwork height.
    let height = None;
    let can_wrap = !matches!(style.get("white-space"), Some("nowrap" | "pre"));
    let growth = Some(if can_wrap && width.is_some() {
        TextGrowth::FixedWidth
    } else if width.is_some() {
        TextGrowth::FixedWidthHeight
    } else {
        TextGrowth::Auto
    });
    TextBox {
        width,
        height,
        limits,
        growth,
    }
}

fn parse(style: &ComputedStyle, value: &str, context: &MapCtx<'_>) -> Option<CssLength> {
    parse_length(
        value,
        &LengthCtx {
            font_size: style.font_size,
            root_font_size: context.opts.base_font_size,
            viewport_w: context.opts.viewport_width,
            viewport_h: context.opts.viewport_width * 0.625,
        },
    )
}

fn length(style: &ComputedStyle, name: &str, reference: f64, context: &MapCtx<'_>) -> Option<f64> {
    let value = style.get(name)?;
    if matches!(value, "none" | "auto") {
        return None;
    }
    let value = parse(style, value, context)?.resolve(reference);
    (value.is_finite() && value >= 0.0).then_some(value)
}

fn clamp(mut value: f64, minimum: Option<f64>, maximum: Option<f64>) -> f64 {
    if let Some(maximum) = maximum {
        value = value.min(maximum);
    }
    if let Some(minimum) = minimum {
        value = value.max(minimum);
    }
    value.max(0.0)
}

fn is_block_text_container(style: &ComputedStyle, path: &[&DomElement]) -> bool {
    match style.get("display").map(str::trim) {
        Some(
            "inline" | "inline-block" | "inline flow" | "inline flow-root" | "inline-flex"
            | "inline flex" | "inline-grid" | "inline grid",
        ) => false,
        Some("flex" | "block flex") => matches!(
            style.get("flex-direction"),
            Some("column" | "column-reverse")
        ),
        Some("block" | "flow-root" | "list-item" | "table" | "grid" | "block grid") => true,
        Some(_) => false,
        None => path
            .last()
            .is_some_and(|element| is_semantic_block(&element.tag)),
    }
}

fn is_semantic_block(tag: &str) -> bool {
    matches!(
        tag,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "body"
            | "dd"
            | "div"
            | "dl"
            | "dt"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "header"
            | "li"
            | "main"
            | "nav"
            | "ol"
            | "p"
            | "pre"
            | "section"
            | "table"
            | "td"
            | "th"
            | "ul"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn style(props: &[(&str, &str)]) -> ComputedStyle {
        ComputedStyle {
            props: props
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect::<BTreeMap<_, _>>(),
            font_size: 16.0,
        }
    }

    fn element(tag: &str) -> DomElement {
        DomElement {
            tag: tag.to_string(),
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }

    fn with_box(tag: &str, props: &[(&str, &str)], check: impl FnOnce(TextBox)) {
        let options = crate::HtmlImportOptions::default();
        let context = MapCtx {
            opts: &options,
            rules: &[],
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
            containing_width: 600.0,
            containing_height: 400.0,
            containing_width_is_definite: true,
            positioned_width: 600.0,
            positioned_height: 400.0,
        };
        let element = element(tag);
        check(map_text_box(&style(props), &[&element], &context));
    }

    #[test]
    fn block_copy_fills_and_wraps_but_inline_copy_stays_natural() {
        with_box("p", &[], |text| {
            assert_eq!(
                text.width,
                Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
            );
            assert_eq!(text.growth, Some(TextGrowth::FixedWidth));
        });
        with_box("span", &[], |text| {
            assert_eq!(text.width, None);
            assert_eq!(text.growth, Some(TextGrowth::Auto));
        });
    }

    #[test]
    fn max_width_is_baked_into_wrap_width_and_nowrap_stays_single_line() {
        with_box("h1", &[("max-width", "470px")], |text| {
            assert_eq!(text.width, Some(SizingBehavior::Number(470.0)));
            assert_eq!(text.growth, Some(TextGrowth::FixedWidth));
        });
        with_box("p", &[("white-space", "nowrap")], |text| {
            assert_eq!(text.growth, Some(TextGrowth::FixedWidthHeight));
        });
    }

    #[test]
    fn anonymous_text_does_not_copy_host_height_constraints() {
        with_box(
            "div",
            &[
                ("height", "195px"),
                ("min-height", "120px"),
                ("max-height", "240px"),
            ],
            |text| {
                assert_eq!(text.height, None);
                assert_eq!(text.limits.min_height, None);
                assert_eq!(text.limits.max_height, None);
            },
        );
    }
}
