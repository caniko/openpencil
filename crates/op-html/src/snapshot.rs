use std::collections::BTreeMap;

use jian_ops_schema::constraints::{Constraints, HConstraint, VConstraint};
use jian_ops_schema::node::base::{NumberOrExpression, PenNodeBase};
use jian_ops_schema::node::container::{ContainerProps, LayoutMode};
use jian_ops_schema::node::image::{ImageFitMode, ImageNode};
use jian_ops_schema::node::text::{FontStyleKind, FontWeight, TextAlign, TextContent, TextNode};
use jian_ops_schema::node::{FrameNode, ImageSrc, PenNode};
use jian_ops_schema::sizing::SizingBehavior;
use jian_ops_schema::style::{BlendMode, PenFill, SolidFillBody};
use serde_json::{Map, Value};

use crate::color::parse_css_color;
use crate::css::cascade::ComputedStyle;
use crate::mapper::{container_props_from, MapCtx};
use crate::{
    wrap_imported_document, HtmlDocumentResult, HtmlImportOptions, HtmlImportResult,
    MAX_OUTPUT_NODES,
};

const MAX_SNAPSHOT_BYTES: usize = 32 * 1024 * 1024;

pub const SNAPSHOT_EXTRACTOR_JS: &str = include_str!("../assets/snapshot-extractor.js");

pub fn import_snapshot(json: &str, opts: &HtmlImportOptions) -> HtmlImportResult {
    match import_snapshot_inner(json, opts) {
        Ok(result) => result,
        Err(error) => HtmlImportResult {
            nodes: Vec::new(),
            warnings: vec![error],
        },
    }
}

pub fn import_snapshot_document(json: &str, opts: &HtmlImportOptions) -> HtmlDocumentResult {
    wrap_imported_document(import_snapshot(json, opts))
}

fn import_snapshot_inner(json: &str, opts: &HtmlImportOptions) -> Result<HtmlImportResult, String> {
    if json.len() > MAX_SNAPSHOT_BYTES {
        return Err("snapshot JSON exceeds the 32 MiB input limit".into());
    }
    let value: Value =
        serde_json::from_str(json).map_err(|error| format!("invalid snapshot JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "snapshot JSON must be an object".to_string())?;
    match object.get("version").and_then(Value::as_u64) {
        Some(1) => {}
        Some(version) => {
            return Err(format!(
                "unsupported snapshot version {version}; expected 1"
            ))
        }
        None => return Err("snapshot version is required and must equal 1".into()),
    }
    let root = object
        .get("root")
        .and_then(Value::as_object)
        .ok_or_else(|| "snapshot root is required".to_string())?;
    let root_rect = Rect::from_node(root)
        .ok_or_else(|| "snapshot root rect is missing or invalid".to_string())?;
    let title = object
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or("Web Snapshot");
    let mut context = SnapshotCtx::new(opts);
    let root_node = context
        .map_element(root, root_rect, None, Some(title))
        .ok_or_else(|| "snapshot root could not be converted".to_string())?;
    if object
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        context.warn_once("browser snapshot was truncated during extraction");
    }
    if context.output_truncated {
        context.warn_once("node limit reached (20000), remaining snapshot content dropped");
    }
    if context.tainted_images > 0 {
        context.warnings.push(format!(
            "{} images kept as remote URLs (CORS-tainted)",
            context.tainted_images
        ));
    }
    Ok(HtmlImportResult {
        nodes: vec![root_node],
        warnings: context.warnings,
    })
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn from_node(node: &Map<String, Value>) -> Option<Self> {
        let rect = node.get("rect")?.as_object()?;
        let value = Self {
            x: rect.get("x")?.as_f64()?,
            y: rect.get("y")?.as_f64()?,
            w: rect.get("w")?.as_f64()?,
            h: rect.get("h")?.as_f64()?,
        };
        (value.x.is_finite()
            && value.y.is_finite()
            && value.w.is_finite()
            && value.h.is_finite()
            && value.w >= 0.0
            && value.h >= 0.0)
            .then_some(value)
    }
}

struct SnapshotCtx<'a> {
    opts: &'a HtmlImportOptions,
    warnings: Vec<String>,
    next_id: usize,
    node_count: usize,
    output_truncated: bool,
    tainted_images: usize,
}

impl<'a> SnapshotCtx<'a> {
    fn new(opts: &'a HtmlImportOptions) -> Self {
        Self {
            opts,
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
            output_truncated: false,
            tainted_images: 0,
        }
    }

    fn allocate_id(&mut self) -> Option<String> {
        if self.node_count >= MAX_OUTPUT_NODES {
            self.output_truncated = true;
            return None;
        }
        let id = format!("snapshot_{}", self.next_id);
        self.next_id += 1;
        self.node_count += 1;
        Some(id)
    }

    fn warn_once(&mut self, warning: &str) {
        if !self.warnings.iter().any(|existing| existing == warning) {
            self.warnings.push(warning.to_string());
        }
    }

    fn map_child(&mut self, value: &Value, parent_rect: Rect) -> Option<PenNode> {
        let object = value.as_object()?;
        let Some(rect) = Rect::from_node(object) else {
            self.warn_once("snapshot node with missing or invalid rect was skipped");
            return None;
        };
        match object.get("kind").and_then(Value::as_str) {
            Some("element") => self.map_element(object, rect, Some(parent_rect), None),
            Some("text") => self.map_text(object, rect, parent_rect),
            Some("image") => self.map_image(object, rect, parent_rect),
            _ => {
                self.warn_once("snapshot node with unknown kind was skipped");
                None
            }
        }
    }

    fn map_element(
        &mut self,
        object: &Map<String, Value>,
        rect: Rect,
        parent_rect: Option<Rect>,
        root_name: Option<&str>,
    ) -> Option<PenNode> {
        let id = self.allocate_id()?;
        let styles = style_map(object);
        let mut container = self.container_from_styles(&styles);
        container.layout = Some(LayoutMode::None);
        container.width = Some(SizingBehavior::Number(rect.w));
        container.height = Some(SizingBehavior::Number(rect.h));
        if parent_rect.is_none() && container.fill.is_none() {
            container.fill = Some(vec![solid_fill("#ffffff".into())]);
        }
        let name = root_name.map(str::to_string).or_else(|| {
            object
                .get("tag")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        let mut base = self.base(id, name, rect, parent_rect);
        self.apply_base_styles(&mut base, &styles);
        let children = object
            .get("children")
            .and_then(Value::as_array)
            .map(|children| {
                children
                    .iter()
                    .filter_map(|child| self.map_child(child, rect))
                    .collect()
            })
            .unwrap_or_default();
        Some(PenNode::Frame(FrameNode {
            base,
            container,
            breakpoint: None,
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
        }))
    }

    fn map_text(
        &mut self,
        object: &Map<String, Value>,
        rect: Rect,
        parent_rect: Rect,
    ) -> Option<PenNode> {
        let text = object.get("text").and_then(Value::as_str)?.to_string();
        if text.trim().is_empty() {
            return None;
        }
        let id = self.allocate_id()?;
        let styles = style_map(object);
        let font_size = styles
            .get("font-size")
            .and_then(|value| parse_px(value))
            .unwrap_or(self.opts.base_font_size);
        let font_weight = styles.get("font-weight").map(|value| {
            value
                .parse::<u32>()
                .map(FontWeight::Number)
                .unwrap_or_else(|_| FontWeight::Keyword(value.clone()))
        });
        let font_style = match styles.get("font-style").map(String::as_str) {
            Some("italic" | "oblique") => Some(FontStyleKind::Italic),
            Some("normal") => Some(FontStyleKind::Normal),
            _ => None,
        };
        let line_height = styles
            .get("line-height")
            .and_then(|value| parse_px(value))
            .filter(|_| font_size > 0.0)
            .map(|height| height / font_size);
        let letter_spacing = styles
            .get("letter-spacing")
            .filter(|value| value.as_str() != "normal")
            .and_then(|value| parse_px(value));
        let text_align = styles
            .get("text-align")
            .and_then(|value| parse_text_align(value));
        let fill = styles
            .get("color")
            .and_then(|value| parse_css_color(value))
            .map(|color| vec![solid_fill(color)]);
        Some(PenNode::Text(TextNode {
            base: self.base(id, Some("Text".into()), rect, Some(parent_rect)),
            limits: Default::default(),
            width: Some(SizingBehavior::Number(rect.w)),
            height: Some(SizingBehavior::Number(rect.h)),
            content: TextContent::Plain(text),
            font_family: styles.get("font-family").cloned(),
            font_size: Some(font_size),
            font_weight,
            font_style,
            letter_spacing,
            line_height,
            text_align,
            text_align_vertical: None,
            text_growth: None,
            underline: None,
            strikethrough: None,
            fill,
            effects: None,
            state: None,
            bindings: None,
            events: None,
            lifecycle: None,
            semantics: None,
            gestures: None,
            route: None,
        }))
    }

    fn map_image(
        &mut self,
        object: &Map<String, Value>,
        rect: Rect,
        parent_rect: Rect,
    ) -> Option<PenNode> {
        let id = self.allocate_id()?;
        let styles = style_map(object);
        let visual = self.container_from_styles(&styles);
        let object_fit = match styles.get("object-fit").map(String::as_str) {
            Some("cover") => Some(ImageFitMode::Crop),
            Some("contain") => Some(ImageFitMode::Fit),
            Some("fill") => Some(ImageFitMode::Fill),
            _ => None,
        };
        let blend_mode = match styles
            .get("mix-blend-mode")
            .and_then(|value| crate::mapper::map_blend_mode(value))
        {
            Some(BlendMode::Normal) | None => None,
            Some(mode) => Some(mode),
        };
        if styles
            .get("mix-blend-mode")
            .is_some_and(|value| crate::mapper::map_blend_mode(value).is_none())
        {
            self.warn_once("unsupported CSS mix-blend-mode was ignored on an image");
        }
        if object
            .get("tainted")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            self.tainted_images += 1;
        }
        Some(PenNode::Image(ImageNode {
            limits: Default::default(),
            base: self.base(
                id,
                object
                    .get("tag")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| Some("img".into())),
                rect,
                Some(parent_rect),
            ),
            src: ImageSrc::from(
                object
                    .get("src")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            object_fit,
            blend_mode,
            width: Some(SizingBehavior::Number(rect.w)),
            height: Some(SizingBehavior::Number(rect.h)),
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
        }))
    }

    fn base(
        &self,
        id: String,
        name: Option<String>,
        rect: Rect,
        parent_rect: Option<Rect>,
    ) -> PenNodeBase {
        let (x, y, constraints) = parent_rect
            .map(|parent| {
                (
                    Some(rect.x - parent.x),
                    Some(rect.y - parent.y),
                    Some(Constraints {
                        h: HConstraint::Left,
                        v: VConstraint::Top,
                    }),
                )
            })
            .unwrap_or((None, None, None));
        PenNodeBase {
            id,
            name,
            x,
            y,
            // Computed snapshots carry browser-resolved rectangles. Mark
            // every non-root node as explicitly positioned so the downstream
            // fit-content repair cannot pull it back into flex flow or expand
            // its fixed parent around visible overflow.
            constraints,
            ..PenNodeBase::default()
        }
    }

    fn container_from_styles(&mut self, styles: &BTreeMap<String, String>) -> ContainerProps {
        let computed = computed_style(styles, self.opts.base_font_size);
        let rules = [];
        let mut map_context = MapCtx {
            opts: self.opts,
            rules: &rules,
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
            containing_width: self.opts.viewport_width,
            containing_height: self.opts.viewport_width * 0.625,
            containing_width_is_definite: true,
            positioned_width: self.opts.viewport_width,
            positioned_height: self.opts.viewport_width * 0.625,
        };
        let container = container_props_from(&computed, &mut map_context);
        for warning in map_context.warnings {
            self.warn_once(&warning);
        }
        container
    }

    fn apply_base_styles(&mut self, base: &mut PenNodeBase, styles: &BTreeMap<String, String>) {
        base.opacity = styles
            .get("opacity")
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .map(NumberOrExpression::Number);
        let Some(transform) = styles.get("transform") else {
            return;
        };
        if transform == "none" {
            return;
        }
        if let Some(rotation) = matrix_rotation(transform) {
            base.rotation = Some(rotation);
        } else {
            self.warn_once(
                "unsupported snapshot transform ignored (only matrix rotation imported)",
            );
        }
    }
}

fn style_map(object: &Map<String, Value>) -> BTreeMap<String, String> {
    object
        .get("styles")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|styles| styles.iter())
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), value.to_string()))
        })
        .collect()
}

fn computed_style(styles: &BTreeMap<String, String>, default_font_size: f64) -> ComputedStyle {
    let mut props = styles.clone();
    if let Some(border) = styles.get("border") {
        if let Some(width) = border
            .split_whitespace()
            .find(|part| parse_px(part).is_some())
        {
            props.insert("border-width".into(), width.to_string());
        }
        if let Some(style) = border.split_whitespace().find(|part| {
            matches!(
                part.to_ascii_lowercase().as_str(),
                "none"
                    | "hidden"
                    | "dotted"
                    | "dashed"
                    | "solid"
                    | "double"
                    | "groove"
                    | "ridge"
                    | "inset"
                    | "outset"
            )
        }) {
            props.insert("border-style".into(), style.to_string());
        }
        if let Some(color) = color_from_border(border) {
            props.insert("border-color".into(), color);
        }
    }
    let font_size = props
        .get("font-size")
        .and_then(|value| parse_px(value))
        .unwrap_or(default_font_size);
    ComputedStyle { props, font_size }
}

fn color_from_border(border: &str) -> Option<String> {
    for prefix in ["rgba(", "rgb(", "hsla(", "hsl("] {
        if let Some(start) = border.find(prefix) {
            let end = border[start..].find(')')? + start + 1;
            return parse_css_color(&border[start..end]);
        }
    }
    border.split_whitespace().find_map(parse_css_color)
}

fn parse_px(value: &str) -> Option<f64> {
    value
        .trim()
        .strip_suffix("px")
        .unwrap_or(value.trim())
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn parse_text_align(value: &str) -> Option<TextAlign> {
    match value.trim().to_ascii_lowercase().as_str() {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        "justify" => Some(TextAlign::Justify),
        _ => None,
    }
}

fn matrix_rotation(value: &str) -> Option<f64> {
    let body = value.trim().strip_prefix("matrix(")?.strip_suffix(')')?;
    let values: Vec<f64> = body
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() != 6 || !values.iter().all(|value| value.is_finite()) {
        return None;
    }
    Some(values[1].atan2(values[0]).to_degrees())
}

fn solid_fill(color: String) -> PenFill {
    PenFill::Solid(SolidFillBody {
        color,
        explain: None,
        opacity: None,
        blend_mode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HtmlImportOptions;
    use jian_ops_schema::node::container::LayoutMode;
    use jian_ops_schema::node::PenNode;

    const SAMPLE: &str = include_str!("../tests/fixtures/snapshot_v1_sample.json");

    #[test]
    fn snapshot_extractor_contract_markers_are_present() {
        for marker in [
            "getComputedStyle",
            "getBoundingClientRect",
            "createRange",
            "toDataURL",
            "clipboard.writeText",
            "snapshot.json",
            "version: 1",
            "truncated",
        ] {
            assert!(
                SNAPSHOT_EXTRACTOR_JS.contains(marker),
                "extractor is missing {marker}"
            );
        }
    }

    #[test]
    fn sample_snapshot_converts_to_absolute_tree() {
        let result = import_snapshot(SAMPLE, &HtmlImportOptions::default());
        assert!(result.nodes.len() == 1, "warnings: {:?}", result.warnings);
        let PenNode::Frame(root) = &result.nodes[0] else {
            panic!()
        };
        assert!(matches!(
            root.container.layout,
            None | Some(LayoutMode::None)
        ));
        let children = root.children.as_ref().unwrap();
        let PenNode::Frame(card) = &children[0] else {
            panic!("card frame")
        };
        assert_eq!(card.base.x, Some(24.0));
        assert_eq!(card.base.y, Some(24.0));
        use jian_ops_schema::sizing::SizingBehavior;
        use jian_ops_schema::style::StrokeThickness;
        assert!(matches!(card.container.width, Some(SizingBehavior::Number(w)) if w == 300.0));
        assert!(matches!(
            card.container.stroke.as_ref().map(|stroke| &stroke.thickness),
            Some(StrokeThickness::Uniform(width)) if *width == 1.0
        ));
        let PenNode::Text(text) = &card.children.as_ref().unwrap()[0] else {
            panic!("text run")
        };
        assert_eq!(text.base.x, Some(16.0));
        assert_eq!(text.font_size, Some(16.0));
        assert_eq!(text.line_height, Some(1.5));
        let PenNode::Image(image) = &children[1] else {
            panic!("image")
        };
        assert!(image.src.as_str().starts_with("data:image/png"));
    }

    #[test]
    fn computed_order_box_shadow_parses() {
        let result = import_snapshot(SAMPLE, &HtmlImportOptions::default());
        let PenNode::Frame(root) = &result.nodes[0] else {
            panic!()
        };
        let PenNode::Frame(card) = &root.children.as_ref().unwrap()[0] else {
            panic!()
        };
        let effects = card.container.effects.as_ref().expect("shadow");
        assert!(
            matches!(&effects[0], jian_ops_schema::style::PenEffect::Shadow(shadow)
            if shadow.offset_y == 4.0 && shadow.blur == 8.0 && shadow.color == "#00000040")
        );
    }

    #[test]
    fn bad_version_and_bad_json_warn_not_panic() {
        let result = import_snapshot("{\"version\":2,\"root\":{}}", &HtmlImportOptions::default());
        assert!(result.nodes.is_empty());
        assert!(result.warnings[0].contains("version"));
        let malformed = import_snapshot("not json", &HtmlImportOptions::default());
        assert!(malformed.nodes.is_empty());
    }
}
