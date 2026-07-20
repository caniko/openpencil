use jian_ops_schema::node::base::{NumberOrExpression, PenNodeBase};
use jian_ops_schema::node::container::{AlignItems, ContainerProps, JustifyContent, LayoutMode};
use jian_ops_schema::node::{FrameNode, PenNode};
use jian_ops_schema::sizing::{SizeLimits, SizingBehavior, SizingKeyword};

use crate::css::cascade::{compute_style_for_viewport, ComputedStyle, StyleRule};
use crate::dom::{DomElement, DomNode};
use crate::length::{parse_length, CssLength, LengthCtx};
use crate::HtmlImportOptions;

#[path = "layout_heuristics.rs"]
mod layout_heuristics;
pub(crate) use layout_heuristics::infer_child_alignment;
pub use layout_heuristics::infer_gap_from_margins;

#[path = "mapper_visual.rs"]
mod visual;
pub(crate) use visual::map_blend_mode;

#[path = "mapper_grid.rs"]
mod grid;

#[path = "mapper_pseudo.rs"]
pub(crate) mod pseudo;

#[path = "mapper_stack.rs"]
mod stack;

#[path = "mapper_box.rs"]
mod box_model;

#[cfg(test)]
#[path = "mapper_position_tests.rs"]
mod position_tests;

pub struct MapCtx<'a> {
    pub opts: &'a HtmlImportOptions,
    pub rules: &'a [StyleRule],
    pub warnings: Vec<String>,
    pub next_id: u32,
    pub node_count: usize,
    /// Current CSS containing block used to resolve percentages.
    pub containing_width: f64,
    pub containing_height: f64,
    pub containing_width_is_definite: bool,
    /// Nearest non-static ancestor used by absolutely positioned descendants.
    pub positioned_width: f64,
    pub positioned_height: f64,
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
    if context.node_count >= crate::MAX_OUTPUT_NODES {
        context.warn_once("node limit reached while mapping HTML");
        return None;
    }
    let element = *path.last()?;
    let style = compute_style_for_viewport(
        path,
        context.rules,
        parent_style,
        context.opts.base_font_size,
        context.opts.viewport_width,
        context.opts.viewport_width * 0.625,
    );
    if style.get("display") == Some("none") {
        return None;
    }
    let previous_containing = (context.containing_width, context.containing_height);
    let previous_width_is_definite = context.containing_width_is_definite;
    let previous_positioned = (context.positioned_width, context.positioned_height);
    match style.get("position") {
        Some("fixed") => {
            context.containing_width = context.opts.viewport_width;
            context.containing_height = context.opts.viewport_width * 0.625;
            context.containing_width_is_definite = true;
        }
        Some("absolute") => {
            context.containing_width = context.positioned_width;
            context.containing_height = context.positioned_height;
            context.containing_width_is_definite = true;
        }
        _ => {}
    }
    layout_heuristics::warn_for_degradations(
        &style,
        matches!(element.tag.as_str(), "img" | "svg"),
        context,
    );
    if let Some(mapped) = crate::special::map_radio_group(context, path, &style)
        .or_else(|| crate::special::map_special(context, element, &style))
    {
        context.containing_width = previous_containing.0;
        context.containing_height = previous_containing.1;
        context.containing_width_is_definite = previous_width_is_definite;
        return mapped;
    }
    // Reserve this frame before generated content and descendants consume the
    // remaining budget, so no successfully mapped child becomes orphaned.
    context.node_count += 1;
    let mut container = container_props_from(&style, context);
    if container.align_items.is_none() {
        container.align_items = infer_child_alignment(context, path, &style, &element.children);
    }
    // Native buttons center their anonymous content box even when the author
    // does not opt into flex/grid. A plain vertical Jian frame otherwise puts
    // labels such as the 31×31 product-card `+` at the top-left. Preserve any
    // explicit author alignment and only supply the browser control defaults.
    if element.tag == "button" {
        if container.justify_content.is_none() {
            container.justify_content = Some(JustifyContent::Center);
        }
        if container.align_items.is_none() {
            container.align_items = Some(AlignItems::Center);
        }
    }
    layout_heuristics::apply_sizing_defaults(
        &mut container,
        &style,
        parent_style,
        context.containing_width_is_definite,
    );
    layout_heuristics::apply_legacy_size_limits(
        &mut container,
        context.containing_width,
        context.containing_height,
    );
    let parent_reference = (context.containing_width, context.containing_height);
    let mut base = PenNodeBase {
        id: context.generate_id(),
        name: Some(element.tag.clone()),
        role: ((element.tag == "button") || element.attr("role") == Some("button"))
            .then_some("button".to_string()),
        ..Default::default()
    };
    apply_base_style(&mut base, &style, context);
    context.containing_width_is_definite = layout_heuristics::width_is_definite(
        container.width.as_ref(),
        context.containing_width_is_definite,
    );
    context.containing_width = layout_heuristics::resolved_axis(
        container.width.as_ref(),
        container.limits.min_width,
        container.limits.max_width,
        parent_reference.0,
    );
    context.containing_height = layout_heuristics::resolved_axis(
        container.height.as_ref(),
        container.limits.min_height,
        container.limits.max_height,
        parent_reference.1,
    );
    if layout_heuristics::establishes_positioning_context(&style) {
        context.positioned_width = context.containing_width;
        context.positioned_height = context.containing_height;
    }
    let children = map_container_children(context, path, &style, &element.children);
    context.containing_width = previous_containing.0;
    context.containing_height = previous_containing.1;
    context.containing_width_is_definite = previous_width_is_definite;
    context.positioned_width = previous_positioned.0;
    context.positioned_height = previous_positioned.1;
    Some(frame(base, container, children))
}

pub(crate) fn map_container_children(
    context: &mut MapCtx<'_>,
    path: &[&DomElement],
    style: &ComputedStyle,
    dom_children: &[DomNode],
) -> Vec<PenNode> {
    let mut children = crate::text::map_children(context, path, style, dom_children);
    if matches!(
        style.get("flex-direction"),
        Some("row-reverse" | "column-reverse")
    ) {
        children.reverse();
    }
    children = stack::layer_absolute_children(children);
    grid::wrap_grid_rows(context, style, children)
}

pub(crate) fn layer_positioned_children(children: Vec<PenNode>) -> Vec<PenNode> {
    stack::layer_absolute_children(children)
}

pub fn container_props_from(style: &ComputedStyle, context: &mut MapCtx<'_>) -> ContainerProps {
    let layout = match (style.get("display"), style.get("flex-direction")) {
        (Some("flex" | "inline-flex"), Some("column" | "column-reverse")) => LayoutMode::Vertical,
        (Some("flex" | "inline-flex"), _) => LayoutMode::Horizontal,
        _ => LayoutMode::Vertical,
    };
    let gap_property = if layout == LayoutMode::Horizontal {
        "column-gap"
    } else {
        "row-gap"
    };
    let gap = if matches!(style.get("display"), Some("grid" | "inline-grid")) {
        grid::grid_row_gap(style, context)
    } else {
        style
            .get(gap_property)
            .or_else(|| style.get("gap"))
            .and_then(|value| {
                length_px(
                    value,
                    style.font_size,
                    context.opts,
                    context.containing_width,
                )
            })
    }
    .map(NumberOrExpression::Number);
    let fill = visual::map_fill(style, context);
    let border_widths = box_model::border_widths(style, context);
    let stroke = visual::map_stroke(style, context);
    let effects = visual::map_effects(style, context);
    let padding = box_model::map_padding(
        style,
        context,
        border_widths,
        fill.is_some() || stroke.is_some() || effects.is_some() || border_widths != [0.0; 4],
    );
    let justify_content = style
        .get("justify-content")
        .or_else(|| style.get("justify-items"))
        .and_then(map_justify);
    let align_items = style.get("align-items").and_then(AlignItems::from_css);
    let mut width = style.get("width").and_then(|value| {
        map_sizing(
            value,
            style.font_size,
            context.opts,
            context.containing_width,
        )
    });
    let mut height = style.get("height").and_then(|value| {
        map_sizing(
            value,
            style.font_size,
            context.opts,
            context.containing_height,
        )
    });
    let absolute = matches!(style.get("position"), Some("absolute" | "fixed"));
    let infer_stretched_width = absolute
        && width.is_none()
        && layout_heuristics::has_non_auto(style, "left")
        && layout_heuristics::has_non_auto(style, "right");
    let infer_stretched_height = absolute
        && height.is_none()
        && layout_heuristics::has_non_auto(style, "top")
        && layout_heuristics::has_non_auto(style, "bottom");
    // A FillContainer leaf is represented as a percentage in Jian/Taffy. An
    // absolutely positioned percentage has no flex track to fill, so the
    // legacy loader can resolve it to zero. Bake the selected viewport's
    // containing block into an explicit size instead. This is also the exact
    // static-canvas meaning of CSS `width/height:100%` at import time.
    if absolute {
        layout_heuristics::resolve_absolute_fill(&mut width, context.containing_width);
        layout_heuristics::resolve_absolute_fill(&mut height, context.containing_height);
    }
    let mut limits = map_size_limits(style, context);
    box_model::apply_box_sizing(
        style,
        context,
        border_widths,
        &mut width,
        &mut height,
        &mut limits,
    );
    // Auto size plus opposing insets uses the remaining containing-block
    // space. Apply this after content-box expansion: unlike an authored
    // width, the inferred value already denotes the outer box.
    if infer_stretched_width {
        width = Some(SizingBehavior::Number(
            layout_heuristics::stretched_absolute_axis(
                style,
                context,
                "left",
                "right",
                context.containing_width,
            ),
        ));
    }
    if infer_stretched_height {
        height = Some(SizingBehavior::Number(
            layout_heuristics::stretched_absolute_axis(
                style,
                context,
                "top",
                "bottom",
                context.containing_height,
            ),
        ));
    }
    let own_width =
        layout_heuristics::resolved_axis(width.as_ref(), None, None, context.containing_width);
    let own_height =
        layout_heuristics::resolved_axis(height.as_ref(), None, None, context.containing_height);
    let corner_radius = visual::map_corner_radius(style, context, own_width, own_height);
    let mut container = ContainerProps {
        width,
        height,
        layout: Some(layout),
        gap,
        padding,
        justify_content,
        align_items,
        clip_content: matches!(
            style
                .get("overflow")
                .or_else(|| style.get("overflow-x"))
                .or_else(|| style.get("overflow-y")),
            Some("hidden" | "clip")
        )
        .then_some(true),
        corner_radius,
        fill,
        stroke,
        effects,
        // No responsive-schema (jian formatVersion 1.2) source in HTML
        // import — this pipeline only ever emits non-responsive documents.
        limits,
    };
    layout_heuristics::apply_legacy_size_limits(
        &mut container,
        context.containing_width,
        context.containing_height,
    );
    container
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

pub(crate) fn apply_base_style(
    base: &mut PenNodeBase,
    style: &ComputedStyle,
    context: &mut MapCtx<'_>,
) {
    base.opacity = style
        .get("opacity")
        .and_then(|value| value.parse::<f64>().ok())
        .map(|value| NumberOrExpression::Number(value.clamp(0.0, 1.0)));
    if matches!(style.get("visibility"), Some("hidden" | "collapse")) {
        base.visible = Some(false);
    }
    let position = style.get("position");
    let explicit_z_index = style
        .get("z-index")
        .filter(|value| !value.eq_ignore_ascii_case("auto"))
        .and_then(|value| value.parse::<i32>().ok());
    // Relative/sticky elements with z-index:auto remain ordinary flow items.
    // Marking them as a layer would reorder auto-layout children and change
    // their geometry even though CSS preserves their source-order position.
    if matches!(position, Some("absolute" | "fixed")) || explicit_z_index.is_some() {
        let z_index = explicit_z_index.unwrap_or(0);
        base.explain = Some(format!("{}{z_index}", stack::Z_INDEX_HINT));
    }
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
    let inner = inner.trim();
    if let Some(value) = inner.strip_suffix("deg") {
        value.trim().parse().ok()
    } else if let Some(value) = inner.strip_suffix("turn") {
        value.trim().parse::<f64>().ok().map(|turns| turns * 360.0)
    } else if let Some(value) = inner.strip_suffix("grad") {
        value.trim().parse::<f64>().ok().map(|grads| grads * 0.9)
    } else if let Some(value) = inner.strip_suffix("rad") {
        value.trim().parse::<f64>().ok().map(f64::to_degrees)
    } else {
        None
    }
}

fn length_context(font_size: f64, options: &HtmlImportOptions) -> LengthCtx {
    LengthCtx {
        font_size,
        root_font_size: options.base_font_size,
        viewport_w: options.viewport_width,
        viewport_h: options.viewport_width * 0.625,
    }
}

fn length_px(
    value: &str,
    font_size: f64,
    options: &HtmlImportOptions,
    reference: f64,
) -> Option<f64> {
    Some(parse_length(value, &length_context(font_size, options))?.resolve(reference))
}

fn map_sizing(
    value: &str,
    font_size: f64,
    options: &HtmlImportOptions,
    reference: f64,
) -> Option<SizingBehavior> {
    let length = parse_length(value, &length_context(font_size, options))?;
    match &length {
        CssLength::Px(value) => Some(SizingBehavior::Number(*value)),
        CssLength::Percent(100.0) => Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)),
        _ => Some(SizingBehavior::Number(length.resolve(reference))),
    }
}

fn map_size_limits(style: &ComputedStyle, context: &MapCtx<'_>) -> SizeLimits {
    let resolve = |name: &str, reference: f64| {
        style
            .get(name)
            .and_then(|value| parse_length(value, &length_context(style.font_size, context.opts)))
            .map(|length| length.resolve(reference))
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    SizeLimits {
        min_width: resolve("min-width", context.containing_width),
        max_width: resolve("max-width", context.containing_width),
        min_height: resolve("min-height", context.containing_height),
        max_height: resolve("max-height", context.containing_height),
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
            containing_width: options.viewport_width,
            containing_height: options.viewport_width * 0.625,
            containing_width_is_definite: true,
            positioned_width: options.viewport_width,
            positioned_height: options.viewport_width * 0.625,
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
    fn definite_column_child_fills_but_row_flex_item_shrink_wraps() {
        let parent = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![(
                "style".into(),
                "display:flex;flex-direction:column;width:600px".into(),
            )],
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

        let row = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![("style".into(), "display:flex;width:600px".into())],
            children: vec![crate::dom::DomNode::Element(crate::dom::DomElement {
                tag: "div".into(),
                attrs: vec![],
                children: Vec::new(),
            })],
        };
        let Some(PenNode::Frame(row)) = map_one(row, "") else {
            panic!()
        };
        let PenNode::Frame(item) = &row.children.as_ref().unwrap()[0] else {
            panic!()
        };
        assert_eq!(
            item.container.width,
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
    fn right_offset_and_inline_flex_use_parent_geometry() {
        let parent = crate::dom::DomElement {
            tag: "div".into(),
            attrs: vec![(
                "style".into(),
                "position:relative;width:38px;height:38px".into(),
            )],
            children: vec![DomNode::Element(crate::dom::DomElement {
                tag: "small".into(),
                attrs: vec![(
                    "style".into(),
                    "position:absolute;right:-4px;top:-5px;width:17px;height:17px".into(),
                )],
                children: vec![DomNode::Text("2".into())],
            })],
        };
        let Some(PenNode::Frame(frame)) = map_one(parent, "") else {
            panic!()
        };
        let PenNode::Frame(badge) = &frame.children.as_ref().unwrap()[0] else {
            panic!()
        };
        assert_eq!((badge.base.x, badge.base.y), (Some(25.0), Some(-5.0)));

        let child = crate::dom::DomElement {
            tag: "button".into(),
            attrs: vec![("style".into(), "display:inline-flex".into())],
            children: vec![DomNode::Text("Go".into())],
        };
        let parent = crate::dom::DomElement {
            tag: "div".into(),
            attrs: Vec::new(),
            children: vec![DomNode::Element(child)],
        };
        let Some(PenNode::Frame(frame)) = map_one(parent, "") else {
            panic!()
        };
        let PenNode::Frame(inline_row) = &frame.children.as_ref().unwrap()[0] else {
            panic!()
        };
        let PenNode::Frame(button) = &inline_row.children.as_ref().unwrap()[0] else {
            panic!()
        };
        assert_eq!(
            button.container.width,
            Some(SizingBehavior::Keyword(SizingKeyword::FitContent))
        );
    }

    #[test]
    fn implicit_single_column_grid_does_not_warn() {
        let (rules, _) = crate::css::cascade::parse_stylesheet("", 1000);
        let options = crate::HtmlImportOptions::default();
        let mut context = MapCtx {
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
        for _ in 0..2 {
            let element = crate::dom::DomElement {
                tag: "div".into(),
                attrs: vec![("style".into(), "display:grid".into())],
                children: Vec::new(),
            };
            map_element(&mut context, &[&element], None);
        }
        assert!(context
            .warnings
            .iter()
            .all(|warning| !warning.contains("grid")));
    }
}
