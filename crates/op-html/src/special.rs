use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use jian_ops_schema::node::base::{BoolOrExpression, NumberOrExpression, PenNodeBase};
use jian_ops_schema::node::container::CornerRadius;
use jian_ops_schema::node::{
    CheckboxNode, FrameNode, ImageFitMode, ImageNode, ImageSrc, PenNode, ProgressNode,
    RadioGroupNode, RectangleNode, SelectNode, SelectOption, SliderNode, TextAreaNode,
    TextInputNode,
};
use jian_ops_schema::sizing::{SizeLimits, SizingBehavior, SizingKeyword};
use jian_ops_schema::style::{BlendMode, PenEffect, PenFill, PenStroke, SolidFillBody};

use crate::css::cascade::ComputedStyle;
use crate::dom::{DomElement, DomNode};
use crate::mapper::{container_props_from, MapCtx};

pub fn map_special(
    context: &mut MapCtx<'_>,
    element: &DomElement,
    style: &ComputedStyle,
) -> Option<Option<PenNode>> {
    let node = match element.tag.as_str() {
        "img" => map_image(context, element, style),
        "svg" => map_svg(context, element, style),
        "input" => map_input(context, element, style),
        "textarea" => map_text_area(context, element, style),
        "select" => map_select(context, element, style),
        "progress" => map_progress(context, element, style),
        "hr" => map_horizontal_rule(context, style),
        "iframe" | "video" | "canvas" => map_placeholder(context, element, style),
        _ => return None,
    };
    Some(Some(node))
}

pub(crate) fn map_radio_group(
    context: &mut MapCtx<'_>,
    path: &[&DomElement],
    style: &ComputedStyle,
) -> Option<Option<PenNode>> {
    let current = *path.last()?;
    if current.tag != "input" || current.attr("type") != Some("radio") {
        return None;
    }
    let name = current.attr("name");
    let radios: Vec<&DomElement> = path
        .get(path.len().saturating_sub(2))
        .map(|parent| {
            parent
                .children
                .iter()
                .filter_map(|child| match child {
                    DomNode::Element(element)
                        if element.tag == "input"
                            && element.attr("type") == Some("radio")
                            && element.attr("name") == name =>
                    {
                        Some(element)
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_else(|| vec![current]);
    if radios
        .first()
        .is_some_and(|first| !std::ptr::eq(*first, current))
    {
        return Some(None);
    }
    let options = radios
        .iter()
        .map(|radio| {
            let value = radio.attr("value").unwrap_or("on").to_string();
            SelectOption {
                label: radio.attr("aria-label").unwrap_or(&value).to_string(),
                value,
            }
        })
        .collect();
    let value = radios
        .iter()
        .find(|radio| has_attr(radio, "checked"))
        .and_then(|radio| radio.attr("value"))
        .map(str::to_string);
    let visual = visual_props(context, style);
    let node = PenNode::RadioGroup(RadioGroupNode {
        base: base(context, "radio_group", style),
        width: visual.width,
        height: visual.height,
        limits: visual.limits,
        value,
        options: Some(options),
        fill: visual.fill,
        stroke: visual.stroke,
        effects: visual.effects,
        corner_radius: visual.corner_radius,
        ..Default::default()
    });
    Some(Some(finish(context, node)))
}

struct VisualProps {
    width: Option<SizingBehavior>,
    height: Option<SizingBehavior>,
    limits: SizeLimits,
    fill: Option<Vec<PenFill>>,
    stroke: Option<PenStroke>,
    effects: Option<Vec<PenEffect>>,
    corner_radius: Option<CornerRadius>,
}

fn visual_props(context: &mut MapCtx<'_>, style: &ComputedStyle) -> VisualProps {
    let container = container_props_from(style, context);
    VisualProps {
        width: container.width,
        height: container.height,
        limits: container.limits,
        fill: container.fill,
        stroke: container.stroke,
        effects: container.effects,
        corner_radius: container.corner_radius,
    }
}

fn base(context: &mut MapCtx<'_>, name: &str, style: &ComputedStyle) -> PenNodeBase {
    let mut base = PenNodeBase {
        id: context.generate_id(),
        name: Some(name.to_string()),
        ..Default::default()
    };
    crate::mapper::apply_base_style(&mut base, style, context);
    base
}

fn finish(context: &mut MapCtx<'_>, node: PenNode) -> PenNode {
    context.node_count += 1;
    node
}

fn map_image(context: &mut MapCtx<'_>, element: &DomElement, style: &ComputedStyle) -> PenNode {
    let visual = visual_props(context, style);
    let object_fit = match style.get("object-fit") {
        Some("cover") => Some(ImageFitMode::Crop),
        Some("contain") => Some(ImageFitMode::Fit),
        Some("fill") => Some(ImageFitMode::Fill),
        Some("scale-down") => {
            context.warn_once("CSS object-fit:scale-down approximated as contain");
            Some(ImageFitMode::Fit)
        }
        Some("none") => {
            context.warn_once("CSS object-fit:none has no image-node equivalent and was ignored");
            None
        }
        _ => None,
    };
    if style
        .get("object-position")
        .is_some_and(|value| !is_default_object_position(value))
    {
        context.warn_once(
            "CSS object-position has no image-node equivalent; the image remains centered",
        );
    }
    let node = PenNode::Image(ImageNode {
        base: base(context, "img", style),
        src: ImageSrc::from(element.attr("src").unwrap_or("")),
        object_fit,
        blend_mode: image_blend_mode(context, style),
        width: visual.width.or_else(|| numeric_attr(element, "width")),
        height: visual.height.or_else(|| numeric_attr(element, "height")),
        corner_radius: visual.corner_radius,
        effects: visual.effects,
        exposure: None,
        contrast: None,
        saturation: None,
        temperature: None,
        tint: None,
        highlights: None,
        shadows: None,
        image_prompt: None,
        image_search_query: None,
        state: None,
        bindings: None,
        events: None,
        lifecycle: None,
        semantics: None,
        gestures: None,
        route: None,
        limits: visual.limits,
    });
    finish(context, node)
}

fn is_default_object_position(value: &str) -> bool {
    let parts: Vec<_> = value
        .split_ascii_whitespace()
        .map(str::to_ascii_lowercase)
        .collect();
    match parts.as_slice() {
        [value] => matches!(value.as_str(), "center" | "50%"),
        [first, second] => {
            matches!(first.as_str(), "center" | "50%")
                && matches!(second.as_str(), "center" | "50%")
        }
        _ => false,
    }
}

fn image_blend_mode(context: &mut MapCtx<'_>, style: &ComputedStyle) -> Option<BlendMode> {
    let value = style.get("mix-blend-mode")?;
    match crate::mapper::map_blend_mode(value) {
        Some(BlendMode::Normal) => None,
        Some(mode) => Some(mode),
        None => {
            context.warn_once("unsupported CSS mix-blend-mode was ignored on an image");
            None
        }
    }
}

fn map_svg(context: &mut MapCtx<'_>, element: &DomElement, style: &ComputedStyle) -> PenNode {
    context.warn_once("inline <svg> imported as image placeholder");
    let source = serialize_element(element);
    let src = format!("data:image/svg+xml;base64,{}", STANDARD.encode(source));
    let visual = visual_props(context, style);
    let node = PenNode::Image(ImageNode {
        base: base(context, "svg", style),
        src: ImageSrc::from(src),
        object_fit: None,
        blend_mode: image_blend_mode(context, style),
        width: visual.width.or_else(|| numeric_attr(element, "width")),
        height: visual.height.or_else(|| numeric_attr(element, "height")),
        corner_radius: visual.corner_radius,
        effects: visual.effects,
        exposure: None,
        contrast: None,
        saturation: None,
        temperature: None,
        tint: None,
        highlights: None,
        shadows: None,
        image_prompt: None,
        image_search_query: None,
        state: None,
        bindings: None,
        events: None,
        lifecycle: None,
        semantics: None,
        gestures: None,
        route: None,
        limits: visual.limits,
    });
    finish(context, node)
}

fn map_input(context: &mut MapCtx<'_>, element: &DomElement, style: &ComputedStyle) -> PenNode {
    let input_type = element.attr("type").unwrap_or("text").to_ascii_lowercase();
    let visual = visual_props(context, style);
    let node = match input_type.as_str() {
        "text" | "email" | "password" | "search" | "url" | "tel" => {
            PenNode::TextInput(TextInputNode {
                base: base(context, "input", style),
                width: visual.width,
                height: visual.height,
                limits: visual.limits,
                placeholder: element.attr("placeholder").map(str::to_string),
                value: element.attr("value").map(str::to_string),
                fill: visual.fill,
                stroke: visual.stroke,
                effects: visual.effects,
                corner_radius: visual.corner_radius,
                ..Default::default()
            })
        }
        "checkbox" => PenNode::Checkbox(CheckboxNode {
            base: base(context, "checkbox", style),
            width: visual.width,
            height: visual.height,
            limits: visual.limits,
            checked: Some(BoolOrExpression::Bool(has_attr(element, "checked"))),
            label: None,
            fill: visual.fill,
            stroke: visual.stroke,
            effects: visual.effects,
            corner_radius: visual.corner_radius,
            ..Default::default()
        }),
        "range" => PenNode::Slider(SliderNode {
            base: base(context, "slider", style),
            width: visual.width,
            height: visual.height,
            limits: visual.limits,
            min: float_attr(element, "min"),
            max: float_attr(element, "max"),
            step: float_attr(element, "step"),
            value: float_attr(element, "value").map(NumberOrExpression::Number),
            fill: visual.fill,
            stroke: visual.stroke,
            effects: visual.effects,
            corner_radius: visual.corner_radius,
            ..Default::default()
        }),
        "radio" => PenNode::RadioGroup(RadioGroupNode {
            base: base(context, "radio_group", style),
            width: visual.width,
            height: visual.height,
            limits: visual.limits,
            value: has_attr(element, "checked")
                .then(|| element.attr("value").unwrap_or("on").to_string()),
            options: Some(vec![SelectOption {
                value: element.attr("value").unwrap_or("on").to_string(),
                label: element.attr("aria-label").unwrap_or("on").to_string(),
            }]),
            fill: visual.fill,
            stroke: visual.stroke,
            effects: visual.effects,
            corner_radius: visual.corner_radius,
            ..Default::default()
        }),
        _ => {
            context.warn_once("unsupported input type imported as a text input");
            PenNode::TextInput(TextInputNode {
                base: base(context, "input", style),
                width: visual.width,
                height: visual.height,
                limits: visual.limits,
                placeholder: element.attr("placeholder").map(str::to_string),
                value: element.attr("value").map(str::to_string),
                fill: visual.fill,
                stroke: visual.stroke,
                effects: visual.effects,
                corner_radius: visual.corner_radius,
                ..Default::default()
            })
        }
    };
    finish(context, node)
}

fn map_text_area(context: &mut MapCtx<'_>, element: &DomElement, style: &ComputedStyle) -> PenNode {
    let visual = visual_props(context, style);
    let node = PenNode::TextArea(TextAreaNode {
        base: base(context, "textarea", style),
        width: visual.width,
        height: visual.height,
        limits: visual.limits,
        placeholder: element.attr("placeholder").map(str::to_string),
        value: Some(element_text(element)),
        fill: visual.fill,
        stroke: visual.stroke,
        effects: visual.effects,
        corner_radius: visual.corner_radius,
        ..Default::default()
    });
    finish(context, node)
}

fn map_select(context: &mut MapCtx<'_>, element: &DomElement, style: &ComputedStyle) -> PenNode {
    let visual = visual_props(context, style);
    let options: Vec<_> = element
        .children
        .iter()
        .filter_map(|child| match child {
            DomNode::Element(option) if option.tag == "option" => Some(option),
            _ => None,
        })
        .collect();
    let value = options
        .iter()
        .find(|option| has_attr(option, "selected"))
        .map(|option| {
            option
                .attr("value")
                .unwrap_or(&element_text(option))
                .to_string()
        });
    let options = options
        .into_iter()
        .map(|option| {
            let label = element_text(option);
            SelectOption {
                value: option.attr("value").unwrap_or(&label).to_string(),
                label,
            }
        })
        .collect();
    let node = PenNode::Select(SelectNode {
        base: base(context, "select", style),
        width: visual.width,
        height: visual.height,
        limits: visual.limits,
        placeholder: element.attr("placeholder").map(str::to_string),
        value,
        options: Some(options),
        fill: visual.fill,
        stroke: visual.stroke,
        effects: visual.effects,
        corner_radius: visual.corner_radius,
        ..Default::default()
    });
    finish(context, node)
}

fn map_progress(context: &mut MapCtx<'_>, element: &DomElement, style: &ComputedStyle) -> PenNode {
    let visual = visual_props(context, style);
    let node = PenNode::Progress(ProgressNode {
        base: base(context, "progress", style),
        width: visual.width,
        height: visual.height,
        limits: visual.limits,
        value: float_attr(element, "value").map(NumberOrExpression::Number),
        max: float_attr(element, "max"),
        fill: visual.fill,
        stroke: visual.stroke,
        effects: visual.effects,
        corner_radius: visual.corner_radius,
        ..Default::default()
    });
    finish(context, node)
}

fn map_horizontal_rule(context: &mut MapCtx<'_>, style: &ComputedStyle) -> PenNode {
    let mut container = container_props_from(style, context);
    container.width = container
        .width
        .or(Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)));
    container.height = container.height.or(Some(SizingBehavior::Number(1.0)));
    container.fill = container.fill.or_else(|| Some(vec![solid_fill("#e0e0e0")]));
    let node = PenNode::Rectangle(RectangleNode {
        base: base(context, "hr", style),
        container,
        children: None,
        state: None,
        bindings: None,
        events: None,
        lifecycle: None,
        semantics: None,
        gestures: None,
        route: None,
    });
    finish(context, node)
}

fn map_placeholder(
    context: &mut MapCtx<'_>,
    element: &DomElement,
    style: &ComputedStyle,
) -> PenNode {
    context.warn_once(&format!("<{}> imported as a placeholder", element.tag));
    let mut container = container_props_from(style, context);
    container.width = container.width.or_else(|| numeric_attr(element, "width"));
    container.height = container.height.or_else(|| numeric_attr(element, "height"));
    if container.fill.is_none() {
        container.fill = Some(vec![solid_fill("#f0f0f0")]);
    }
    let node = PenNode::Frame(FrameNode {
        base: base(context, &element.tag, style),
        container,
        children: Some(Vec::new()),
        image_search_query: None,
        reusable: None,
        slot: None,
        state: None,
        bindings: None,
        events: None,
        lifecycle: None,
        semantics: None,
        gestures: None,
        route: None,
        screen: None,
        breakpoint: None,
    });
    finish(context, node)
}

fn numeric_attr(element: &DomElement, name: &str) -> Option<SizingBehavior> {
    float_attr(element, name).map(SizingBehavior::Number)
}

fn float_attr(element: &DomElement, name: &str) -> Option<f64> {
    element.attr(name)?.parse().ok()
}

fn has_attr(element: &DomElement, name: &str) -> bool {
    element.attrs.iter().any(|(attr, _)| attr == name)
}

fn element_text(element: &DomElement) -> String {
    element
        .children
        .iter()
        .map(|child| match child {
            DomNode::Text(text) => text.clone(),
            DomNode::Element(child) => element_text(child),
        })
        .collect()
}

fn serialize_element(element: &DomElement) -> String {
    let mut source = format!("<{}", element.tag);
    for (name, value) in &element.attrs {
        source.push(' ');
        source.push_str(name);
        source.push_str("=\"");
        source.push_str(&escape_xml(value));
        source.push('"');
    }
    source.push('>');
    for child in &element.children {
        match child {
            DomNode::Text(text) => source.push_str(&escape_xml(text)),
            DomNode::Element(child) => source.push_str(&serialize_element(child)),
        }
    }
    source.push_str("</");
    source.push_str(&element.tag);
    source.push('>');
    source
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn solid_fill(color: &str) -> PenFill {
    PenFill::Solid(SolidFillBody {
        color: color.to_string(),
        explain: None,
        opacity: None,
        blend_mode: None,
    })
}

#[cfg(test)]
mod tests {
    use jian_ops_schema::node::base::NumberOrExpression;
    use jian_ops_schema::node::container::CornerRadius;
    use jian_ops_schema::node::PenNode;
    use jian_ops_schema::sizing::SizingBehavior;
    use jian_ops_schema::style::{BlendMode, PenFill};

    fn element_with(tag: &str, attrs: Vec<(&str, &str)>) -> crate::dom::DomElement {
        crate::dom::DomElement {
            tag: tag.into(),
            attrs: attrs
                .into_iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            children: Vec::new(),
        }
    }

    fn map_it(element: crate::dom::DomElement) -> Option<PenNode> {
        let (rules, _) = crate::css::cascade::parse_stylesheet("", 0);
        let options = crate::HtmlImportOptions::default();
        let mut context = crate::mapper::MapCtx {
            opts: &options,
            rules: &rules,
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
            containing_width: options.viewport_width,
            containing_height: options.viewport_width * 0.625,
            containing_width_is_definite: true,
            positioned_width: options.viewport_width,
            positioned_height: options.viewport_width * 0.625,
        };
        crate::mapper::map_element(&mut context, &[&element], None)
    }

    #[test]
    fn img_maps_to_image_node() {
        let node = map_it(element_with(
            "img",
            vec![
                ("src", "https://x.dev/a.png"),
                ("width", "120"),
                ("height", "80"),
            ],
        ));
        let Some(PenNode::Image(image)) = node else {
            panic!("expected image")
        };
        assert_eq!(image.src.as_str(), "https://x.dev/a.png");
        assert!(matches!(
            image.width,
            Some(SizingBehavior::Number(value)) if value == 120.0
        ));
    }

    #[test]
    fn special_nodes_receive_computed_base_and_visual_styles() {
        let node = map_it(element_with(
            "img",
            vec![
                ("src", "hero.png"),
                (
                    "style",
                    "position:absolute;left:12px;top:8px;opacity:.4;\
                     transform:rotate(15deg);width:50px;height:40px;\
                     border-radius:6px;filter:blur(2px)",
                ),
            ],
        ));
        let Some(PenNode::Image(image)) = node else {
            panic!("expected image")
        };
        assert_eq!(image.base.x, Some(12.0));
        assert_eq!(image.base.y, Some(8.0));
        assert_eq!(image.base.rotation, Some(15.0));
        assert!(matches!(
            image.base.opacity,
            Some(NumberOrExpression::Number(value)) if value == 0.4
        ));
        assert!(matches!(
            image.width,
            Some(SizingBehavior::Number(value)) if value == 50.0
        ));
        assert!(matches!(
            image.height,
            Some(SizingBehavior::Number(value)) if value == 40.0
        ));
        assert!(matches!(
            image.corner_radius,
            Some(CornerRadius::Uniform(value)) if value == 6.0
        ));
        assert!(image
            .effects
            .as_ref()
            .is_some_and(|effects| !effects.is_empty()));
    }

    #[test]
    fn horizontal_rule_and_radio_keep_css_visuals() {
        let Some(PenNode::Rectangle(rule)) = map_it(element_with(
            "hr",
            vec![(
                "style",
                "width:80px;height:3px;background:#123456;border-radius:2px;opacity:.5",
            )],
        )) else {
            panic!("expected rectangle")
        };
        assert!(matches!(
            rule.container.width,
            Some(SizingBehavior::Number(value)) if value == 80.0
        ));
        assert!(matches!(
            rule.container.height,
            Some(SizingBehavior::Number(value)) if value == 3.0
        ));
        assert!(matches!(
            rule.container.fill.as_deref(),
            Some([PenFill::Solid(fill)]) if fill.color == "#123456"
        ));
        assert!(matches!(
            rule.base.opacity,
            Some(NumberOrExpression::Number(value)) if value == 0.5
        ));

        let Some(PenNode::RadioGroup(radio)) = map_it(element_with(
            "input",
            vec![
                ("type", "radio"),
                (
                    "style",
                    "width:24px;height:24px;background:#abcdef;border-radius:12px",
                ),
            ],
        )) else {
            panic!("expected radio group")
        };
        assert!(matches!(
            radio.fill.as_deref(),
            Some([PenFill::Solid(fill)]) if fill.color == "#abcdef"
        ));
        assert!(matches!(
            radio.corner_radius,
            Some(CornerRadius::Uniform(value)) if value == 12.0
        ));
    }

    #[test]
    fn inline_svg_becomes_data_url_image() {
        let mut svg = element_with("svg", vec![("viewBox", "0 0 10 10")]);
        svg.children.push(crate::dom::DomNode::Element(element_with(
            "rect",
            vec![("width", "10")],
        )));
        let Some(PenNode::Image(image)) = map_it(svg) else {
            panic!()
        };
        assert!(image.src.as_str().starts_with("data:image/svg+xml;base64,"));
    }

    #[test]
    fn form_controls_map_to_widget_nodes() {
        assert!(matches!(
            map_it(element_with(
                "input",
                vec![("type", "text"), ("placeholder", "Name")]
            )),
            Some(PenNode::TextInput(input)) if input.placeholder.as_deref() == Some("Name")
        ));
        assert!(matches!(
            map_it(element_with(
                "input",
                vec![("type", "checkbox"), ("checked", "")]
            )),
            Some(PenNode::Checkbox(_))
        ));
        assert!(matches!(
            map_it(element_with(
                "input",
                vec![("type", "range"), ("min", "0"), ("max", "10")]
            )),
            Some(PenNode::Slider(slider)) if slider.max == Some(10.0)
        ));
        assert!(matches!(
            map_it(element_with(
                "progress",
                vec![("value", "3"), ("max", "10")]
            )),
            Some(PenNode::Progress(_))
        ));
    }

    #[test]
    fn select_collects_options() {
        let mut select = element_with("select", vec![]);
        let mut option = element_with("option", vec![("value", "a"), ("selected", "")]);
        option
            .children
            .push(crate::dom::DomNode::Text("Alpha".into()));
        select.children.push(crate::dom::DomNode::Element(option));
        let Some(PenNode::Select(select)) = map_it(select) else {
            panic!()
        };
        assert_eq!(select.value.as_deref(), Some("a"));
        let options = select.options.as_ref().unwrap();
        assert_eq!(options[0].label, "Alpha");
    }

    #[test]
    fn button_is_frame_with_role() {
        let mut button = element_with("button", vec![]);
        button.children.push(crate::dom::DomNode::Text("Go".into()));
        let Some(PenNode::Frame(frame)) = map_it(button) else {
            panic!()
        };
        assert_eq!(frame.base.role.as_deref(), Some("button"));
    }

    #[test]
    fn image_blending_is_preserved_while_unrepresentable_position_is_reported() {
        let result = crate::import_html(
            "<img src='hero.png' style='object-fit:cover;object-position:65% bottom;\
                                        mix-blend-mode:multiply'>",
            &crate::HtmlImportOptions::default(),
        );
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.contains("object-position")));
        assert!(result
            .warnings
            .iter()
            .all(|warning| !warning.contains("mix-blend-mode")));
        let node = map_it(element_with(
            "img",
            vec![("src", "hero.png"), ("style", "mix-blend-mode:multiply")],
        ));
        let Some(PenNode::Image(image)) = node else {
            panic!("expected image node")
        };
        assert_eq!(image.blend_mode, Some(BlendMode::Multiply));
    }
}
