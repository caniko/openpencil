use jian_ops_schema::node::container::Padding;
use jian_ops_schema::sizing::{SizeLimits, SizingBehavior};

use crate::css::cascade::ComputedStyle;
use crate::length::{parse_length, LengthCtx};

use crate::mapper::MapCtx;

pub(super) fn map_padding(
    style: &ComputedStyle,
    context: &mut MapCtx<'_>,
    border: [f64; 4],
    has_visual_box: bool,
) -> Option<Padding> {
    let padding_names = [
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ];
    let margin_names = ["margin-top", "margin-right", "margin-bottom", "margin-left"];
    let margins = margin_names.map(|name| resolve(style, name, context));
    let padding_values = padding_names.map(|name| resolve(style, name, context));
    if margins.iter().flatten().any(|value| *value < 0.0) {
        context.warn_once("negative CSS margins are not representable and were ignored");
    }
    let positive_margin = margins.iter().flatten().any(|value| *value > 0.0);
    if has_visual_box && positive_margin {
        context.warn_once(
            "CSS margins on visual boxes cannot be represented without changing the box and were ignored",
        );
    }
    let values = std::array::from_fn(|index| {
        padding_values[index].unwrap_or(0.0)
            + border[index]
            + if has_visual_box {
                0.0
            } else {
                margins[index].unwrap_or(0.0).max(0.0)
            }
    });
    if values.iter().all(|value| value.abs() <= f64::EPSILON) {
        return None;
    }
    if values.iter().all(|value| *value == values[0]) {
        Some(Padding::Uniform(values[0]))
    } else {
        Some(Padding::LtrB(values))
    }
}

pub(super) fn apply_box_sizing(
    style: &ComputedStyle,
    context: &mut MapCtx<'_>,
    border: [f64; 4],
    width: &mut Option<SizingBehavior>,
    height: &mut Option<SizingBehavior>,
    limits: &mut SizeLimits,
) {
    if style
        .get("box-sizing")
        .is_some_and(|value| value.eq_ignore_ascii_case("border-box"))
    {
        return;
    }
    let padding = [
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ]
    .map(|name| resolve(style, name, context).unwrap_or(0.0).max(0.0));
    let horizontal = padding[1] + padding[3] + border[1] + border[3];
    let vertical = padding[0] + padding[2] + border[0] + border[2];
    expand_number(width, horizontal, context);
    expand_number(height, vertical, context);
    limits.min_width = limits.min_width.map(|value| value + horizontal);
    limits.max_width = limits.max_width.map(|value| value + horizontal);
    limits.min_height = limits.min_height.map(|value| value + vertical);
    limits.max_height = limits.max_height.map(|value| value + vertical);
}

fn expand_number(sizing: &mut Option<SizingBehavior>, extra: f64, context: &mut MapCtx<'_>) {
    if extra <= 0.0 {
        return;
    }
    match sizing {
        Some(SizingBehavior::Number(value)) => *value += extra,
        Some(SizingBehavior::Keyword(_)) | Some(SizingBehavior::Expression(_)) => context
            .warn_once(
                "content-box percentage sizing cannot include padding exactly and was approximated",
            ),
        None => {}
    }
}

pub(super) fn border_widths(style: &ComputedStyle, context: &MapCtx<'_>) -> [f64; 4] {
    let sides = ["top", "right", "bottom", "left"];
    sides.map(|side| {
        let border_style = style
            .get(&format!("border-{side}-style"))
            .or_else(|| style.get("border-style"))
            .unwrap_or("none");
        if matches!(
            border_style.trim().to_ascii_lowercase().as_str(),
            "none" | "hidden"
        ) {
            return 0.0;
        }
        style
            .get(&format!("border-{side}-width"))
            .or_else(|| style.get("border-width"))
            .and_then(|value| border_width(value, style, context))
            .unwrap_or(3.0)
            .max(0.0)
    })
}

fn border_width(value: &str, style: &ComputedStyle, context: &MapCtx<'_>) -> Option<f64> {
    match value.trim().to_ascii_lowercase().as_str() {
        "thin" => Some(1.0),
        "medium" => Some(3.0),
        "thick" => Some(5.0),
        _ => resolve_value(value, style, context),
    }
}

fn resolve(style: &ComputedStyle, name: &str, context: &MapCtx<'_>) -> Option<f64> {
    let value = style.get(name)?;
    resolve_value(value, style, context)
}

fn resolve_value(value: &str, style: &ComputedStyle, context: &MapCtx<'_>) -> Option<f64> {
    let length = parse_length(
        value,
        &LengthCtx {
            font_size: style.font_size,
            root_font_size: context.opts.base_font_size,
            viewport_w: context.opts.viewport_width,
            viewport_h: context.opts.viewport_width * 0.625,
        },
    )?;
    let value = length.resolve(context.containing_width);
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn mapped(props: &[(&str, &str)], visual: bool) -> (Option<Padding>, Vec<String>) {
        let style = ComputedStyle {
            props: props
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect::<BTreeMap<_, _>>(),
            font_size: 16.0,
        };
        let options = crate::HtmlImportOptions::default();
        let mut context = MapCtx {
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
        let border = border_widths(&style, &context);
        let padding = map_padding(&style, &mut context, border, visual || border != [0.0; 4]);
        (padding, context.warnings)
    }

    #[test]
    fn visual_box_margin_does_not_expand_its_background_padding() {
        let (padding, warnings) = mapped(&[("padding-top", "4px"), ("margin-top", "12px")], true);
        assert!(matches!(padding, Some(Padding::LtrB(values)) if values[0] == 4.0));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("visual boxes")));
    }

    #[test]
    fn transparent_box_keeps_positive_margin_spacing_approximation() {
        let (padding, warnings) = mapped(&[("margin-top", "12px")], false);
        assert!(matches!(padding, Some(Padding::LtrB(values)) if values[0] == 12.0));
        assert!(warnings.is_empty());
    }

    #[test]
    fn content_box_expands_authored_outer_size_but_border_box_does_not() {
        let result = crate::import_html(
            "<div style='width:100px;height:50px;padding:10px;border:2px solid #000'></div>\
             <div style='box-sizing:border-box;width:100px;height:50px;padding:10px;\
                         border:2px solid #000'></div>",
            &crate::HtmlImportOptions::default(),
        );
        let jian_ops_schema::node::PenNode::Frame(root) = &result.nodes[0] else {
            panic!()
        };
        let children = root.children.as_ref().unwrap();
        let jian_ops_schema::node::PenNode::Frame(content_box) = &children[0] else {
            panic!()
        };
        let jian_ops_schema::node::PenNode::Frame(border_box) = &children[1] else {
            panic!()
        };
        assert_eq!(
            content_box.container.width,
            Some(SizingBehavior::Number(124.0))
        );
        assert_eq!(
            content_box.container.height,
            Some(SizingBehavior::Number(74.0))
        );
        assert_eq!(
            border_box.container.width,
            Some(SizingBehavior::Number(100.0))
        );
        assert_eq!(
            border_box.container.height,
            Some(SizingBehavior::Number(50.0))
        );
        assert_eq!(content_box.container.padding, Some(Padding::Uniform(12.0)));
        assert_eq!(border_box.container.padding, Some(Padding::Uniform(12.0)));
    }

    #[test]
    fn border_box_padding_reserves_the_css_content_area() {
        let result = crate::import_html(
            "<div style='box-sizing:border-box;width:100px;height:50px;padding:10px;\
                         border:2px solid #000'>\
               <span style='display:block;width:100%;height:1px'></span>\
             </div>",
            &crate::HtmlImportOptions::default(),
        );
        let jian_ops_schema::node::PenNode::Frame(root) = &result.nodes[0] else {
            panic!()
        };
        let jian_ops_schema::node::PenNode::Frame(box_node) = &root.children.as_ref().unwrap()[0]
        else {
            panic!()
        };
        assert_eq!(
            box_node.container.width,
            Some(SizingBehavior::Number(100.0))
        );
        assert_eq!(box_node.container.padding, Some(Padding::Uniform(12.0)));
        let jian_ops_schema::node::PenNode::Frame(child) = &box_node.children.as_ref().unwrap()[0]
        else {
            panic!()
        };
        assert_eq!(
            child.container.width,
            Some(SizingBehavior::Keyword(
                jian_ops_schema::sizing::SizingKeyword::FillContainer
            ))
        );
    }

    #[test]
    fn transparent_border_still_participates_in_the_box_model() {
        let result = crate::import_html(
            "<div style='width:100px;padding:4px;border:3px solid transparent'></div>",
            &crate::HtmlImportOptions::default(),
        );
        let jian_ops_schema::node::PenNode::Frame(root) = &result.nodes[0] else {
            panic!()
        };
        let jian_ops_schema::node::PenNode::Frame(box_node) = &root.children.as_ref().unwrap()[0]
        else {
            panic!()
        };
        assert_eq!(
            box_node.container.width,
            Some(SizingBehavior::Number(114.0))
        );
        assert_eq!(box_node.container.padding, Some(Padding::Uniform(7.0)));
        assert!(box_node.container.stroke.is_none());
    }

    #[test]
    fn nova_category_auto_height_includes_its_border() {
        let result = crate::import_html(
            "<div style='display:flex;padding:18px;border:1px solid #ebedf3'>\
               <span style='display:block;width:48px;height:48px'></span>\
             </div>",
            &crate::HtmlImportOptions::default(),
        );
        let jian_ops_schema::node::PenNode::Frame(root) = &result.nodes[0] else {
            panic!()
        };
        let jian_ops_schema::node::PenNode::Frame(category) = &root.children.as_ref().unwrap()[0]
        else {
            panic!()
        };
        assert_eq!(category.container.padding, Some(Padding::Uniform(19.0)));
        assert_eq!(
            category.container.height,
            Some(SizingBehavior::Keyword(
                jian_ops_schema::sizing::SizingKeyword::FitContent
            ))
        );
        // A 48 px child plus 19 px on each side resolves to the CSS 86 px.
    }
}
