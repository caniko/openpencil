use jian_ops_schema::node::base::{NumberOrExpression, PenNodeBase};
use jian_ops_schema::node::container::{
    AlignItems, ContainerProps, CornerRadius, JustifyContent, LayoutMode, Padding,
};
use jian_ops_schema::node::{FrameNode, ImageSrc, PenNode};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
use jian_ops_schema::style::{
    GradientStop, ImageFillBody, LinearGradientBody, PenEffect, PenFill, PenStroke,
    RadialGradientBody, ShadowBody, SolidFillBody, StrokeThickness,
};

use crate::color::parse_css_color;
use crate::css::cascade::{compute_style, ComputedStyle, StyleRule};
use crate::dom::{DomElement, DomNode};
use crate::length::{parse_length, CssLength, LengthCtx};
use crate::HtmlImportOptions;

#[path = "layout_heuristics.rs"]
mod layout_heuristics;
pub use layout_heuristics::infer_gap_from_margins;

pub struct MapCtx<'a> {
    pub opts: &'a HtmlImportOptions,
    pub rules: &'a [StyleRule],
    pub warnings: Vec<String>,
    pub next_id: u32,
    pub node_count: usize,
}

impl MapCtx<'_> {
    pub fn generate_id(&mut self) -> String {
        let id = format!("html_{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    pub fn warn_once(&mut self, message: &str) {
        if !self.warnings.iter().any(|warning| warning == message) {
            self.warnings.push(message.to_string());
        }
    }
}

pub fn map_element(
    context: &mut MapCtx<'_>,
    path: &[&DomElement],
    parent_style: Option<&ComputedStyle>,
) -> Option<PenNode> {
    let element = *path.last()?;
    let style = compute_style(
        path,
        context.rules,
        parent_style,
        context.opts.base_font_size,
    );
    if style.get("display") == Some("none") {
        return None;
    }
    layout_heuristics::warn_for_degradations(&style, context);
    if let Some(mapped) = crate::special::map_radio_group(context, path, &style)
        .or_else(|| crate::special::map_special(context, element, &style))
    {
        return mapped;
    }
    let children = crate::text::map_children(context, path, &style, &element.children);
    let mut container = container_props_from(&style, context);
    if container.gap.is_none() {
        let child_styles: Vec<_> = element
            .children
            .iter()
            .filter_map(|child| match child {
                DomNode::Element(child_element) => {
                    let mut child_path = path.to_vec();
                    child_path.push(child_element);
                    Some(compute_style(
                        &child_path,
                        context.rules,
                        Some(&style),
                        context.opts.base_font_size,
                    ))
                }
                DomNode::Text(_) => None,
            })
            .collect();
        let refs: Vec<_> = child_styles.iter().collect();
        let (gap, deviated) = infer_gap_from_margins(&refs, style.font_size);
        container.gap = gap.map(NumberOrExpression::Number);
        if deviated {
            context.warn_once("mixed adjacent margins approximated using the most common gap");
        }
    }
    layout_heuristics::apply_sizing_defaults(&mut container, &style, parent_style);
    let mut base = PenNodeBase {
        id: context.generate_id(),
        name: Some(element.tag.clone()),
        role: ((element.tag == "button") || element.attr("role") == Some("button"))
            .then_some("button".to_string()),
        ..Default::default()
    };
    apply_base_style(&mut base, &style, context);
    context.node_count += 1;
    Some(frame(base, container, children))
}

pub fn container_props_from(style: &ComputedStyle, context: &mut MapCtx<'_>) -> ContainerProps {
    let layout = match (style.get("display"), style.get("flex-direction")) {
        (Some("flex" | "inline-flex"), Some("column" | "column-reverse")) => LayoutMode::Vertical,
        (Some("flex" | "inline-flex"), _) => LayoutMode::Horizontal,
        _ => LayoutMode::Vertical,
    };
    let gap = style
        .get("gap")
        .and_then(|value| length_px(value, style.font_size, context.opts))
        .map(NumberOrExpression::Number);
    let padding = map_padding(style, context.opts);
    let justify_content = style.get("justify-content").and_then(map_justify);
    let align_items = style.get("align-items").and_then(AlignItems::from_css);
    let width = style
        .get("width")
        .and_then(|value| map_sizing(value, style.font_size, context.opts));
    let height = style
        .get("height")
        .and_then(|value| map_sizing(value, style.font_size, context.opts));
    let fill = map_fill(style);
    let stroke = map_stroke(style, context);
    let corner_radius = style
        .get("border-radius")
        .and_then(|value| map_corner_radius(value, style.font_size, context.opts));
    let effects = style.get("box-shadow").and_then(map_shadows);
    ContainerProps {
        width,
        height,
        layout: Some(layout),
        gap,
        padding,
        justify_content,
        align_items,
        clip_content: (style.get("overflow") == Some("hidden")).then_some(true),
        corner_radius,
        fill,
        stroke,
        effects,
        // No responsive-schema (jian formatVersion 1.2) source in HTML
        // import — this pipeline only ever emits non-responsive documents.
        limits: Default::default(),
    }
}

fn frame(base: PenNodeBase, container: ContainerProps, children: Vec<PenNode>) -> PenNode {
    PenNode::Frame(FrameNode {
        base,
        container,
        children: Some(children),
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
        // HTML import never authors a responsive breakpoint variant.
        breakpoint: None,
    })
}

fn apply_base_style(base: &mut PenNodeBase, style: &ComputedStyle, context: &mut MapCtx<'_>) {
    base.opacity = style
        .get("opacity")
        .and_then(|value| value.parse::<f64>().ok())
        .map(NumberOrExpression::Number);
    if let Some(transform) = style.get("transform") {
        if let Some(degrees) = parse_rotate(transform) {
            base.rotation = Some(degrees);
        } else if transform != "none" {
            context.warn_once("unsupported CSS transform ignored (only rotate is imported)");
        }
    }
    layout_heuristics::apply_position(base, style, context);
}

fn parse_rotate(value: &str) -> Option<f64> {
    let inner = value.trim().strip_prefix("rotate(")?.strip_suffix(')')?;
    inner.strip_suffix("deg")?.trim().parse().ok()
}

fn length_context(font_size: f64, options: &HtmlImportOptions) -> LengthCtx {
    LengthCtx {
        font_size,
        root_font_size: options.base_font_size,
        viewport_w: options.viewport_width,
        viewport_h: options.viewport_width * 0.625,
    }
}

fn length_px(value: &str, font_size: f64, options: &HtmlImportOptions) -> Option<f64> {
    match parse_length(value, &length_context(font_size, options))? {
        CssLength::Px(value) => Some(value),
        CssLength::Percent(_) => None,
    }
}

fn map_sizing(value: &str, font_size: f64, options: &HtmlImportOptions) -> Option<SizingBehavior> {
    match parse_length(value, &length_context(font_size, options))? {
        CssLength::Px(value) => Some(SizingBehavior::Number(value)),
        CssLength::Percent(100.0) => Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)),
        CssLength::Percent(_) => None,
    }
}

fn map_padding(style: &ComputedStyle, options: &HtmlImportOptions) -> Option<Padding> {
    let names = [
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
    ];
    if !names.iter().any(|name| style.get(name).is_some()) {
        return None;
    }
    let values = names.map(|name| {
        style
            .get(name)
            .and_then(|value| length_px(value, style.font_size, options))
            .unwrap_or(0.0)
    });
    if values.iter().all(|value| *value == values[0]) {
        Some(Padding::Uniform(values[0]))
    } else {
        Some(Padding::LtrB(values))
    }
}

fn map_justify(value: &str) -> Option<JustifyContent> {
    match value.trim().to_ascii_lowercase().as_str() {
        "flex-start" | "start" | "left" => Some(JustifyContent::Start),
        "center" => Some(JustifyContent::Center),
        "flex-end" | "end" | "right" => Some(JustifyContent::End),
        "space-between" => Some(JustifyContent::SpaceBetween),
        "space-around" | "space-evenly" => Some(JustifyContent::SpaceAround),
        _ => None,
    }
}

fn solid_fill(color: String) -> PenFill {
    PenFill::Solid(SolidFillBody {
        color,
        explain: None,
        opacity: None,
        blend_mode: None,
    })
}

fn map_fill(style: &ComputedStyle) -> Option<Vec<PenFill>> {
    let mut fills = Vec::new();
    if let Some(color) = style.get("background-color").and_then(parse_css_color) {
        fills.push(solid_fill(color));
    }
    if let Some(image) = style.get("background-image") {
        if let Some(gradient) = map_gradient(image) {
            fills.push(gradient);
        } else if let Some(url) = extract_url(image) {
            fills.push(PenFill::Image(ImageFillBody {
                url: ImageSrc::from(url),
                mode: None,
                original_size: None,
                transform: None,
                explain: None,
                opacity: None,
                exposure: None,
                contrast: None,
                saturation: None,
                temperature: None,
                tint: None,
                highlights: None,
                shadows: None,
            }));
        }
    }
    (!fills.is_empty()).then_some(fills)
}

fn extract_url(value: &str) -> Option<String> {
    let start = value.find("url(")? + 4;
    let end = value[start..].find(')')? + start;
    Some(
        value[start..end]
            .trim()
            .trim_matches(['\'', '"'])
            .to_string(),
    )
}

fn map_gradient(value: &str) -> Option<PenFill> {
    let (radial, body) = if let Some(body) = value.strip_prefix("linear-gradient(") {
        (false, body.strip_suffix(')')?)
    } else if let Some(body) = value.strip_prefix("radial-gradient(") {
        (true, body.strip_suffix(')')?)
    } else {
        return None;
    };
    let mut parts = split_top_level(body, ',');
    let mut angle = None;
    if !radial {
        angle = parts.first().and_then(|part| parse_gradient_angle(part));
        if angle.is_some() {
            parts.remove(0);
        }
    } else if parts
        .first()
        .is_some_and(|part| parse_color_stop(part).is_none())
    {
        parts.remove(0);
    }
    let stop_count = parts.len();
    if stop_count < 2 {
        return None;
    }
    let stops: Option<Vec<_>> = parts
        .iter()
        .enumerate()
        .map(|(index, part)| {
            let (color, explicit) = parse_color_stop(part)?;
            let offset = explicit.unwrap_or_else(|| index as f32 / (stop_count - 1) as f32);
            Some(GradientStop { offset, color })
        })
        .collect();
    let stops = stops?;
    if radial {
        Some(PenFill::RadialGradient(RadialGradientBody {
            cx: None,
            cy: None,
            radius: None,
            stops,
            explain: None,
            opacity: None,
            blend_mode: None,
        }))
    } else {
        Some(PenFill::LinearGradient(LinearGradientBody {
            angle: Some(angle.unwrap_or(180.0)),
            stops,
            explain: None,
            opacity: None,
            blend_mode: None,
        }))
    }
}

fn parse_gradient_angle(value: &str) -> Option<f32> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(degrees) = value.strip_suffix("deg") {
        return degrees.trim().parse().ok();
    }
    match value.as_str() {
        "to top" => Some(0.0),
        "to right" => Some(90.0),
        "to bottom" => Some(180.0),
        "to left" => Some(270.0),
        _ => None,
    }
}

fn parse_color_stop(value: &str) -> Option<(String, Option<f32>)> {
    let value = value.trim();
    if let Some(color) = parse_css_color(value) {
        return Some((color, None));
    }
    let (color_source, remainder) = if value.starts_with("rgb") || value.starts_with("hsl") {
        let end = value.find(')')? + 1;
        (&value[..end], value[end..].trim())
    } else {
        value
            .split_once(char::is_whitespace)
            .map_or((value, ""), |(color, rest)| (color, rest.trim()))
    };
    let color = parse_css_color(color_source)?;
    let offset = remainder
        .strip_suffix('%')
        .and_then(|number| number.trim().parse::<f32>().ok())
        .map(|percent| percent / 100.0);
    Some((color, offset))
}

fn map_stroke(style: &ComputedStyle, context: &mut MapCtx<'_>) -> Option<PenStroke> {
    let widths = ["top", "right", "bottom", "left"].map(|side| {
        style
            .get(&format!("border-{side}-width"))
            .or_else(|| style.get("border-width"))
            .and_then(|value| length_px(value, style.font_size, context.opts))
            .unwrap_or(0.0) as f32
    });
    let max_width = widths.iter().copied().fold(0.0_f32, f32::max);
    if max_width <= 0.0 {
        return None;
    }
    if widths.iter().any(|width| *width != widths[0]) {
        context.warn_once("per-side border widths approximated using the widest side");
    }
    let color = style
        .get("border-color")
        .or_else(|| {
            ["top", "right", "bottom", "left"]
                .iter()
                .find_map(|side| style.get(&format!("border-{side}-color")))
        })
        .or_else(|| style.get("color"))
        .and_then(parse_css_color)
        .unwrap_or_else(|| "#000000".to_string());
    Some(PenStroke {
        thickness: StrokeThickness::Uniform(max_width),
        align: None,
        join: None,
        cap: None,
        dash_pattern: None,
        dash_offset: None,
        fill: Some(vec![solid_fill(color)]),
    })
}

fn map_corner_radius(
    value: &str,
    font_size: f64,
    options: &HtmlImportOptions,
) -> Option<CornerRadius> {
    let parts: Vec<_> = value.split_whitespace().collect();
    let values = match parts.as_slice() {
        [all] => [*all, *all, *all, *all],
        [vertical, horizontal] => [*vertical, *horizontal, *vertical, *horizontal],
        [top, horizontal, bottom] => [*top, *horizontal, *bottom, *horizontal],
        [top, right, bottom, left] => [*top, *right, *bottom, *left],
        _ => return None,
    };
    let radii = values
        .map(|part| length_px(part, font_size, options))
        .into_iter()
        .collect::<Option<Vec<_>>>()?;
    if radii.iter().all(|radius| *radius == radii[0]) {
        Some(CornerRadius::Uniform(radii[0]))
    } else {
        Some(CornerRadius::PerCorner([
            radii[0], radii[1], radii[2], radii[3],
        ]))
    }
}

fn map_shadows(value: &str) -> Option<Vec<PenEffect>> {
    let effects: Vec<_> = split_top_level(value, ',')
        .into_iter()
        .filter_map(map_shadow)
        .map(PenEffect::Shadow)
        .collect();
    (!effects.is_empty()).then_some(effects)
}

fn map_shadow(value: &str) -> Option<ShadowBody> {
    let mut inner = false;
    let mut color = "#000000".to_string();
    let mut lengths = Vec::new();
    for token in split_whitespace_top_level(value) {
        if token == "inset" {
            inner = true;
        } else if let Some(parsed) = parse_css_color(token) {
            color = parsed;
        } else if let Some(length) = parse_shadow_length(token) {
            lengths.push(length);
        }
    }
    if lengths.len() < 2 {
        return None;
    }
    Some(ShadowBody {
        inner: inner.then_some(true),
        visible: None,
        offset_x: lengths[0],
        offset_y: lengths[1],
        blur: lengths.get(2).copied().unwrap_or(0.0),
        spread: lengths.get(3).copied().unwrap_or(0.0),
        color,
    })
}

fn parse_shadow_length(value: &str) -> Option<f32> {
    let number = value.strip_suffix("px").unwrap_or(value);
    number.parse().ok()
}

fn split_top_level(input: &str, delimiter: char) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0u32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if ch == delimiter && depth == 0 => {
                result.push(input[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(input[start..].trim());
    result
}

fn split_whitespace_top_level(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = None;
    let mut depth = 0u32;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => {
                depth += 1;
                start.get_or_insert(index);
            }
            ')' => depth = depth.saturating_sub(1),
            _ if ch.is_whitespace() && depth == 0 => {
                if let Some(token_start) = start.take() {
                    result.push(&input[token_start..index]);
                }
            }
            _ => {
                start.get_or_insert(index);
            }
        }
    }
    if let Some(token_start) = start {
        result.push(&input[token_start..]);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::cascade::parse_stylesheet;
    use jian_ops_schema::node::container::{
        AlignItems, CornerRadius, JustifyContent, LayoutMode, Padding,
    };
    use jian_ops_schema::node::PenNode;
    use jian_ops_schema::style::{PenEffect, PenFill, StrokeThickness};

    fn map_one(html_element: crate::dom::DomElement, css: &str) -> Option<PenNode> {
        let (rules, _) = parse_stylesheet(css, 1000);
        let options = crate::HtmlImportOptions::default();
        let mut context = MapCtx {
            opts: &options,
            rules: &rules,
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
        };
        map_element(&mut context, &[&html_element], None)
    }

    #[test]
    fn flex_row_maps_to_horizontal_frame() {
        let element = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![(
                "style".into(),
                "display:flex;gap:12px;justify-content:space-between;align-items:center;padding:16px"
                    .into(),
            )],
            children: Vec::new(),
        };
        let Some(PenNode::Frame(frame)) = map_one(element, "") else {
            panic!("expected frame")
        };
        assert_eq!(frame.container.layout, Some(LayoutMode::Horizontal));
        assert!(matches!(
            frame.container.gap,
            Some(jian_ops_schema::node::base::NumberOrExpression::Number(value)) if value == 12.0
        ));
        assert_eq!(
            frame.container.justify_content,
            Some(JustifyContent::SpaceBetween)
        );
        assert_eq!(frame.container.align_items, Some(AlignItems::Center));
        assert!(matches!(
            frame.container.padding,
            Some(Padding::Uniform(value)) if value == 16.0
        ));
    }

    #[test]
    fn visual_styles_map_to_fill_stroke_effects() {
        let element = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![(
                "style".into(),
                "background-color:#102030;border:2px solid #ff0000;border-radius:8px;\
                 box-shadow:0 4px 8px rgba(0,0,0,0.25);overflow:hidden"
                    .into(),
            )],
            children: Vec::new(),
        };
        let Some(PenNode::Frame(frame)) = map_one(element, "") else {
            panic!()
        };
        let fills = frame.container.fill.as_ref().unwrap();
        assert!(matches!(&fills[0], PenFill::Solid(solid) if solid.color == "#102030"));
        let stroke = frame.container.stroke.as_ref().unwrap();
        assert!(matches!(
            stroke.thickness,
            StrokeThickness::Uniform(width) if width == 2.0
        ));
        assert!(matches!(
            frame.container.corner_radius,
            Some(CornerRadius::Uniform(radius)) if radius == 8.0
        ));
        let effects = frame.container.effects.as_ref().unwrap();
        assert!(matches!(&effects[0], PenEffect::Shadow(shadow)
            if shadow.offset_y == 4.0 && shadow.blur == 8.0 && shadow.color == "#00000040"));
        assert_eq!(frame.container.clip_content, Some(true));
    }

    #[test]
    fn linear_gradient_fill() {
        let element = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![(
                "style".into(),
                "background:linear-gradient(90deg,#000000,#ffffff)".into(),
            )],
            children: Vec::new(),
        };
        let Some(PenNode::Frame(frame)) = map_one(element, "") else {
            panic!()
        };
        let fills = frame.container.fill.as_ref().unwrap();
        let PenFill::LinearGradient(gradient) = &fills[0] else {
            panic!("expected gradient")
        };
        assert_eq!(gradient.angle, Some(90.0));
        assert_eq!(gradient.stops.len(), 2);
        assert_eq!(gradient.stops[1].color, "#ffffff");
        assert_eq!(gradient.stops[1].offset, 1.0);
    }

    #[test]
    fn display_none_is_skipped() {
        let element = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![("style".into(), "display:none".into())],
            children: Vec::new(),
        };
        assert!(map_one(element, "").is_none());
    }

    #[test]
    fn gap_mode_from_margins() {
        let make_style = |margin_top: &str, margin_bottom: &str| {
            let mut style = ComputedStyle {
                props: Default::default(),
                font_size: 16.0,
            };
            style.props.insert("margin-top".into(), margin_top.into());
            style
                .props
                .insert("margin-bottom".into(), margin_bottom.into());
            style
        };
        let first = make_style("0", "8px");
        let second = make_style("8px", "8px");
        let third = make_style("8px", "16px");
        let fourth = make_style("8px", "0");
        let styles = vec![&first, &second, &third, &fourth];
        let (gap, deviated) = infer_gap_from_margins(&styles, 16.0);
        assert_eq!(gap, Some(16.0));
        assert!(deviated);
    }

    #[test]
    fn block_child_defaults_to_fill_width_and_frames_hug_height() {
        let parent = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![],
            children: vec![crate::dom::DomNode::Element(crate::dom::DomElement {
                tag: "div".into(),
                attrs: vec![],
                children: Vec::new(),
            })],
        };
        let Some(PenNode::Frame(frame)) = map_one(parent, "") else {
            panic!()
        };
        let PenNode::Frame(child) = &frame.children.as_ref().unwrap()[0] else {
            panic!()
        };
        use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
        assert_eq!(
            child.container.width,
            Some(SizingBehavior::Keyword(SizingKeyword::FillContainer))
        );
        assert_eq!(
            child.container.height,
            Some(SizingBehavior::Keyword(SizingKeyword::FitContent))
        );
    }

    #[test]
    fn absolute_positioning_lands_on_base_xy() {
        let element = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![(
                "style".into(),
                "position:absolute;left:24px;top:48px".into(),
            )],
            children: Vec::new(),
        };
        let Some(PenNode::Frame(frame)) = map_one(element, "") else {
            panic!()
        };
        assert_eq!(frame.base.x, Some(24.0));
        assert_eq!(frame.base.y, Some(48.0));
    }

    #[test]
    fn grid_degrades_with_single_warning() {
        let (rules, _) = crate::css::cascade::parse_stylesheet("", 1000);
        let options = crate::HtmlImportOptions::default();
        let mut context = MapCtx {
            opts: &options,
            rules: &rules,
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
        };
        for _ in 0..2 {
            let element = crate::dom::DomElement {
                tag: "div".into(),
                attrs: vec![("style".into(), "display:grid".into())],
                children: Vec::new(),
            };
            map_element(&mut context, &[&element], None);
        }
        assert_eq!(
            context
                .warnings
                .iter()
                .filter(|warning| warning.contains("grid"))
                .count(),
            1
        );
    }
}
