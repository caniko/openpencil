use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use jian_ops_schema::node::{
    BoolOrExpression, CornerRadius, FontWeight, ImageFitMode, JustifyContent, LayoutMode,
    NumberOrExpression, Padding, PenNode, PenNodeBase, TextAlign, TextAlignVertical, TextContent,
    TextFontStyle, TextGrowth, TextNode,
};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
use jian_ops_schema::style::{
    FontStyleKind, PenEffect, PenFill, PenStroke, StrokeThickness, StyledTextSegment,
};
use jian_ops_schema::PenDocument;
use serde_json::{json, Map, Value};

use crate::access::{
    align, children_of, container_of, corner_of, effects_of, fills_of, index_nodes, is_flex,
    is_raster_leaf, justify, node_base, node_id, node_kind, node_size, page_index_of, select_root,
    stroke_of,
};
use crate::assets::{font_from_bytes, image_from_src, png_size, Sidecar};
use crate::{ExportError, ExportOptions, ExportResult, RasterCandidate};

struct Diag {
    code: &'static str,
    severity: &'static str,
    node_id: String,
    runtime_id: Option<String>,
    message: String,
    strategy: &'static str,
}

struct Cx<'a> {
    index: HashMap<String, &'a PenNode>,
    source_dir: &'a Path,
    strict: bool,
    nodes: BTreeMap<String, Value>,
    sidecars: Vec<Sidecar>,
    assets: BTreeMap<String, Value>,
    diags: Vec<Diag>,
    rasters: Vec<RasterCandidate>,
    page_index: usize,
    ref_stack: HashSet<String>,
}

pub(crate) fn export_document(
    doc: &PenDocument,
    opts: &ExportOptions<'_>,
) -> Result<ExportResult, ExportError> {
    if opts.name.is_empty() {
        return Err(ExportError::msg("package name is empty"));
    }
    let index = index_nodes(doc);
    let root = select_root(doc, opts.item, &index)?;
    let (vw, vh) = viewport_of(root).ok_or_else(|| {
        ExportError::msg(format!(
            "root {} has no authored numeric width/height",
            node_id(root)
        ))
    })?;
    let mut cx = Cx {
        index,
        source_dir: opts.source_dir,
        strict: opts.strict,
        nodes: BTreeMap::new(),
        sidecars: Vec::new(),
        assets: BTreeMap::new(),
        diags: Vec::new(),
        rasters: Vec::new(),
        page_index: page_index_of(doc, node_id(root))?,
        ref_stack: HashSet::new(),
    };
    let root_id = cx.emit(root, None, None, false)?;
    if let Some(Value::Object(node)) = cx.nodes.get_mut(&root_id) {
        if let Some(Value::Object(layout)) = node.get_mut("layout") {
            let fill = json!({ "type": "percent", "value": 100.0 });
            layout.insert("width".into(), fill.clone());
            layout.insert("height".into(), fill);
        }
    }
    cx.validate_visual_states()?;
    cx.record_duplicate_runtime_ids();
    cx.diags.sort_by(|a, b| {
        sev_rank(a.severity)
            .cmp(&sev_rank(b.severity))
            .then(a.code.cmp(b.code))
            .then(a.node_id.cmp(&b.node_id))
            .then(
                a.runtime_id
                    .as_deref()
                    .unwrap_or("")
                    .cmp(b.runtime_id.as_deref().unwrap_or("")),
            )
            .then(a.strategy.cmp(b.strategy))
            .then(a.message.cmp(&b.message))
    });
    let diagnostics = Value::Array(cx.diags.iter().map(diag_json).collect());
    let mut entrypoints = Map::new();
    entrypoints.insert("default".into(), json!(root_id));
    if let Some(authored) = &doc.runtime_entrypoints {
        let root_runtime_id = cx.nodes[&root_id].get("runtime_id").and_then(Value::as_str);
        for (name, runtime_id) in authored {
            if root_runtime_id != Some(runtime_id.as_str()) {
                return Err(ExportError::msg(format!(
                    "runtime entrypoint `{name}` targets `{runtime_id}`, but the exported root is `{}`",
                    root_runtime_id.unwrap_or("<missing runtimeId>")
                )));
            }
            entrypoints.insert(name.clone(), json!(root_id));
        }
    }
    let mut nodes = Map::new();
    for (k, v) in cx.nodes {
        nodes.insert(k, v);
    }
    let mut assets = Map::new();
    for (k, v) in cx.assets {
        assets.insert(k, v);
    }
    let manifest = json!({
        "format": "openpencil-runtime-ui",
        "schema_version": 1,
        "generator": {
            "name": "openpencil",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "document": {
            "name": opts.name,
            "reference_viewport": { "width": num(vw), "height": num(vh) },
        },
        "entrypoints": entrypoints,
        "assets": assets,
        "nodes": nodes,
        "diagnostics": diagnostics,
        "extensions": {},
    });
    Ok(ExportResult {
        manifest,
        sidecars: cx.sidecars,
        raster_candidates: cx.rasters,
    })
}

impl Cx<'_> {
    fn emit(
        &mut self,
        node: &PenNode,
        prefix: Option<&str>,
        scene_prefix: Option<&str>,
        parent_flex: bool,
    ) -> Result<String, ExportError> {
        if let PenNode::Ref(r) = node {
            return self.emit_ref(r, prefix, scene_prefix, parent_flex);
        }
        let id = qualify(node_id(node), prefix);
        let scene_id = qualify_scene(node_id(node), scene_prefix);
        self.record_raster(node, &id, &scene_id);
        if matches!(
            node,
            PenNode::Ellipse(_)
                | PenNode::Line(_)
                | PenNode::Polygon(_)
                | PenNode::Path(_)
                | PenNode::IconFont(_)
                | PenNode::TextInput(_)
                | PenNode::TextArea(_)
                | PenNode::Select(_)
                | PenNode::Switch(_)
                | PenNode::Checkbox(_)
                | PenNode::Slider(_)
                | PenNode::RadioGroup(_)
                | PenNode::NumberInput(_)
                | PenNode::Progress(_)
        ) {
            self.unsupported(&id, node_kind(node))?;
        }
        if matches!(node, PenNode::Tabs(_)) {
            self.unsupported(&id, "tabs")?;
        }
        let kids = children_of(node);
        let flex = is_flex(node);
        let mut child_ids = Vec::new();
        for child in kids {
            child_ids.push(self.emit(child, prefix, scene_prefix, flex)?);
        }
        let (typ, payload_key, payload) = match node {
            PenNode::Text(t) => ("text", "text", Some(self.text_payload(t)?)),
            PenNode::Image(img) => ("image", "image", Some(self.image_payload(&id, img)?)),
            _ => ("container", "container", Some(json!({}))),
        };
        let mut obj = Map::new();
        obj.insert("type".into(), json!(typ));
        obj.insert("source_id".into(), json!(id));
        let name = node_base(node).name.as_deref();
        if node_base(node)
            .runtime_id
            .as_deref()
            .is_some_and(|runtime_id| !is_runtime_id(runtime_id))
        {
            return Err(ExportError::msg(format!("invalid runtimeId on `{id}`")));
        }
        if let Some(rid) = runtime_id_of(&id, name, node_base(node).runtime_id.as_deref()) {
            obj.insert("runtime_id".into(), json!(rid));
        }
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            obj.insert("name".into(), json!(name));
        }
        obj.insert(
            "visible".into(),
            json!(node_base(node).visible.unwrap_or(true)),
        );
        obj.insert("children".into(), json!(child_ids));
        obj.insert("layout".into(), self.layout_of(node, parent_flex));
        obj.insert("style".into(), self.style_of(node, &id)?);
        obj.insert("extensions".into(), runtime_extensions(node_base(node)));
        if let Some(p) = payload {
            obj.insert(payload_key.into(), p);
        }
        self.nodes.insert(id.clone(), Value::Object(obj));
        Ok(id)
    }

    fn emit_ref(
        &mut self,
        r: &jian_ops_schema::node::RefNode,
        prefix: Option<&str>,
        scene_prefix: Option<&str>,
        parent_flex: bool,
    ) -> Result<String, ExportError> {
        let id = qualify(&r.base.id, prefix);
        let scene_id = qualify_scene(&r.base.id, scene_prefix);
        let target = self.index.get(&r.target).copied().ok_or_else(|| {
            ExportError::msg(format!("ref {} target {} not found", r.base.id, r.target))
        })?;
        if !self.ref_stack.insert(r.target.clone()) {
            return Err(ExportError::msg(format!(
                "cyclic ref {} -> {}",
                r.base.id, r.target
            )));
        }
        if r.descendants.as_ref().is_some_and(|d| !d.is_empty()) {
            self.unsupported(&id, "ref descendants")?;
        }
        self.record_raster(target, &id, &scene_id);
        let child_prefix = Some(id.as_str());
        let child_scene = Some(scene_id.as_str());
        let kids = r
            .children
            .as_deref()
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| children_of(target));
        let flex = is_flex(target);
        let mut child_ids = Vec::new();
        for child in kids {
            child_ids.push(self.emit(child, child_prefix, child_scene, flex)?);
        }
        self.ref_stack.remove(&r.target);
        let (typ, payload_key, payload) = match target {
            PenNode::Text(t) => ("text", "text", Some(self.text_payload(t)?)),
            PenNode::Image(img) => ("image", "image", Some(self.image_payload(&id, img)?)),
            _ => ("container", "container", Some(json!({}))),
        };
        let mut obj = Map::new();
        obj.insert("type".into(), json!(typ));
        obj.insert("source_id".into(), json!(id));
        let name = r
            .base
            .name
            .clone()
            .or_else(|| node_base(target).name.clone());
        if r.base
            .runtime_id
            .as_deref()
            .is_some_and(|runtime_id| !is_runtime_id(runtime_id))
        {
            return Err(ExportError::msg(format!("invalid runtimeId on `{id}`")));
        }
        if let Some(rid) = runtime_id_of(&id, name.as_deref(), r.base.runtime_id.as_deref()) {
            obj.insert("runtime_id".into(), json!(rid));
        }
        if let Some(name) = name.filter(|n| !n.is_empty()) {
            obj.insert("name".into(), json!(name));
        }
        obj.insert(
            "component".into(),
            json!({
                "role": "instance",
                "definition_source_id": r.target,
                "component_name": node_base(target).name.as_deref().unwrap_or(r.target.as_str()),
                "variant_properties": {},
            }),
        );
        obj.insert("visible".into(), json!(r.base.visible.unwrap_or(true)));
        obj.insert("children".into(), json!(child_ids));
        // Instance keeps the Ref's position; size/style come from the target.
        obj.insert(
            "layout".into(),
            merge_layout(
                self.layout_of(target, parent_flex),
                self.position_layout(&r.base, parent_flex),
            ),
        );
        obj.insert("style".into(), self.style_of(target, &id)?);
        obj.insert("extensions".into(), runtime_extensions(&r.base));
        if let Some(p) = payload {
            obj.insert(payload_key.into(), p);
        }
        self.nodes.insert(id.clone(), Value::Object(obj));
        Ok(id)
    }

    fn intern_font(&mut self) -> String {
        if let Some(id) = self
            .assets
            .iter()
            .find_map(|(k, v)| (v["kind"] == "font").then(|| k.clone()))
        {
            return id;
        }
        let sidecar = font_from_bytes(
            include_bytes!("../../op-host-desktop/assets/fonts/Inter-VF.ttf").to_vec(),
        );
        let id = sidecar.id();
        self.assets.insert(
            id.clone(),
            json!({
                "kind": sidecar.kind,
                "uri": sidecar.uri(),
                "mime_type": sidecar.mime,
                "sha256": sidecar.sha256,
                "byte_length": sidecar.bytes.len() as u64,
            }),
        );
        self.sidecars.push(sidecar);
        id
    }

    fn text_payload(&mut self, t: &TextNode) -> Result<Value, ExportError> {
        let (content, runs) = text_content(t);
        let color = first_solid_color(t.fill.as_deref()).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        let weight = font_weight(t.font_weight.as_ref());
        let size = t
            .font_size
            .filter(|s| s.is_finite() && *s >= 0.0)
            .unwrap_or(16.0);
        let family = t
            .font_family
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("Inter");
        let font_style = match t.font_style {
            Some(TextFontStyle::Italic) => "italic",
            _ => "normal",
        };
        let letter = t.letter_spacing.filter(|v| v.is_finite()).unwrap_or(0.0);
        let line_height = match t.layout_line_height_multiplier() {
            Some(m) => json!({ "type": "multiple", "value": num(m) }),
            None => json!({ "type": "normal" }),
        };
        let align_h = match t.text_align {
            Some(TextAlign::Center) => "center",
            Some(TextAlign::Right) => "end",
            Some(TextAlign::Justify) => "justify",
            _ => "start",
        };
        let align_v = match t.text_align_vertical {
            Some(TextAlignVertical::Middle) => "center",
            Some(TextAlignVertical::Bottom) => "end",
            _ => "start",
        };
        let wrap = match t.text_growth {
            Some(TextGrowth::FixedWidth | TextGrowth::FixedWidthHeight) => "wrap",
            _ => "nowrap",
        };
        Ok(json!({
            "content": content,
            "defaults": {
                "family": family,
                "font": self.intern_font(),
                "weight": weight,
                "font_style": font_style,
                "size": { "type": "px", "value": num(size) },
                "color": color_json(color),
                "letter_spacing": { "type": "px", "value": num(letter) },
                "line_height": line_height,
            },
            "runs": runs,
            "align_horizontal": align_h,
            "align_vertical": align_v,
            "wrap": wrap,
            "overflow": "clip",
        }))
    }

    fn image_payload(
        &mut self,
        id: &str,
        img: &jian_ops_schema::node::ImageNode,
    ) -> Result<Value, ExportError> {
        let sidecar = image_from_src(img.src.as_str(), self.source_dir)?;
        let (iw, ih) = png_size(&sidecar.bytes).unwrap_or_else(|| {
            (
                authored_px(img.width.as_ref()).unwrap_or(1.0).max(1.0) as u32,
                authored_px(img.height.as_ref()).unwrap_or(1.0).max(1.0) as u32,
            )
        });
        let fit = match img.object_fit {
            Some(ImageFitMode::Fill) => "fill",
            Some(ImageFitMode::Crop) => "cover",
            Some(ImageFitMode::Tile) => "none",
            _ => "contain",
        };
        let rec = json!({
            "kind": sidecar.kind,
            "uri": sidecar.uri(),
            "mime_type": sidecar.mime,
            "sha256": sidecar.sha256,
            "byte_length": sidecar.bytes.len() as u64,
        });
        let asset_id = sidecar.id();
        self.assets.insert(asset_id.clone(), rec);
        self.sidecars.push(sidecar);
        let _ = id;
        Ok(json!({
            "asset": asset_id,
            "intrinsic_width": iw,
            "intrinsic_height": ih,
            "fit": fit,
            "nine_slice": null,
            "tint": null,
        }))
    }

    fn layout_of(&self, node: &PenNode, parent_flex: bool) -> Value {
        let mut layout = Map::new();
        if let Some(c) = container_of(node) {
            match c.layout {
                Some(LayoutMode::Vertical) => {
                    layout.insert("display".into(), json!("flex"));
                    layout.insert("flex_direction".into(), json!("column"));
                }
                Some(LayoutMode::Horizontal) => {
                    layout.insert("display".into(), json!("flex"));
                    layout.insert("flex_direction".into(), json!("row"));
                }
                _ => {}
            }
            if let Some(j) = c.justify_content.as_ref() {
                if !matches!(j, JustifyContent::Start) {
                    layout.insert("justify_content".into(), json!(justify(j)));
                }
            }
            if let Some(a) = c.align_items.as_ref() {
                layout.insert("align_items".into(), json!(align(a)));
            }
            if let Some(NumberOrExpression::Number(g)) = &c.gap {
                if g.is_finite() && *g > 0.0 {
                    let px = json!({ "type": "px", "value": num(*g) });
                    layout.insert("gap".into(), json!({ "row": px, "column": px }));
                }
            }
            if let Some(pad) = padding_edges(c.padding.as_ref()) {
                layout.insert("padding".into(), pad);
            }
            if c.clip_content == Some(true) {
                layout.insert("overflow".into(), json!("hidden"));
            }
        }
        let (w, h) = node_size(node);
        if let Some(v) = sizing(w) {
            layout.insert("width".into(), v);
        }
        if let Some(v) = sizing(h) {
            layout.insert("height".into(), v);
        }
        merge_layout(
            Value::Object(layout),
            self.position_layout(node_base(node), parent_flex),
        )
    }

    fn position_layout(&self, base: &PenNodeBase, parent_flex: bool) -> Value {
        if parent_flex {
            return json!({});
        }
        let x = base.x.filter(|v| v.is_finite());
        let y = base.y.filter(|v| v.is_finite());
        if x.is_none() && y.is_none() {
            return json!({});
        }
        if x.unwrap_or(0.0) == 0.0 && y.unwrap_or(0.0) == 0.0 {
            return json!({});
        }
        json!({
            "position": "absolute",
            "inset": {
                "top": edge_or_auto(y),
                "right": { "type": "auto" },
                "bottom": { "type": "auto" },
                "left": edge_or_auto(x),
            }
        })
    }

    fn style_of(&mut self, node: &PenNode, id: &str) -> Result<Value, ExportError> {
        let mut style = Map::new();
        if let Some(NumberOrExpression::Number(o)) = &node_base(node).opacity {
            if o.is_finite() && *o != 1.0 {
                style.insert("opacity".into(), json!(num(o.clamp(0.0, 1.0))));
            }
        }
        if let Some(r) = node_base(node).rotation {
            if r.is_finite() && r != 0.0 {
                style.insert("rotation".into(), json!({ "degrees": num(r) }));
            }
        }
        let fx = node_base(node).flip_x.unwrap_or(false);
        let fy = node_base(node).flip_y.unwrap_or(false);
        if fx || fy {
            style.insert(
                "scale".into(),
                json!({
                    "x": if fx { -1.0 } else { 1.0 },
                    "y": if fy { -1.0 } else { 1.0 },
                }),
            );
        }
        if container_of(node).and_then(|c| c.clip_content) == Some(true) {
            style.insert("clipping".into(), json!(true));
        }
        if !matches!(node, PenNode::Text(_)) {
            if let Some(fills) = fills_of(node) {
                if let Some(fill) = self.first_fill_json(id, fills)? {
                    style.insert("fill".into(), fill);
                }
            }
        }
        if let Some(stroke) = stroke_of(node) {
            if let Some(border) = border_of(stroke) {
                style.insert("border".into(), border);
            } else {
                self.unsupported(id, "stroke")?;
            }
        }
        if matches!(node, PenNode::Ellipse(_)) {
            let half = || json!({ "type": "percent", "value": 50 });
            style.insert(
                "corner_radius".into(),
                json!({
                    "top_left": half(),
                    "top_right": half(),
                    "bottom_right": half(),
                    "bottom_left": half(),
                }),
            );
        } else if let Some(radius) = corner_of(node) {
            style.insert("corner_radius".into(), corner_json(radius));
        }
        if let Some(effects) = effects_of(node) {
            let mut shadows = Vec::new();
            for e in effects {
                match e {
                    PenEffect::Shadow(s) if s.inner != Some(true) => {
                        if let Some(c) = parse_hex(&s.color) {
                            shadows.push(json!({
                                "offset_x": { "type": "px", "value": num(s.offset_x as f64) },
                                "offset_y": { "type": "px", "value": num(s.offset_y as f64) },
                                "blur": { "type": "px", "value": num(s.blur.max(0.0) as f64) },
                                "spread": { "type": "px", "value": num(s.spread as f64) },
                                "color": color_json(c),
                            }));
                        }
                    }
                    _ => {
                        self.unsupported(id, "effect")?;
                    }
                }
            }
            if !shadows.is_empty() {
                style.insert("outer_shadows".into(), Value::Array(shadows));
            }
        }
        Ok(Value::Object(style))
    }

    fn unsupported(&mut self, id: &str, kind: &str) -> Result<(), ExportError> {
        if self.strict {
            return Err(ExportError::msg(format!(
                "unsupported {kind} on {id} (--strict)"
            )));
        }
        self.diags.push(Diag {
            code: "opui.unsupported_native",
            severity: "warning",
            node_id: id.to_string(),
            runtime_id: runtime_id_of(id, None, None),
            message: format!("{kind} approximated as container"),
            strategy: "native",
        });
        Ok(())
    }

    fn first_fill_json(
        &mut self,
        id: &str,
        fills: &[PenFill],
    ) -> Result<Option<Value>, ExportError> {
        // ponytail: v1 has exactly one fill; extra layers are a warning, first fill still emits
        let Some(first) = fills.first() else {
            return Ok(None);
        };
        let converted = one_fill_json(first);
        if converted.is_none() {
            self.unsupported(id, "unsupported fill")?;
        }
        if fills.len() > 1 {
            self.unsupported(id, "layered fill")?;
        }
        Ok(converted)
    }

    fn record_duplicate_runtime_ids(&mut self) {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        let mut dupes = Vec::new();
        for (id, node) in &self.nodes {
            let Some(rid) = node.get("runtime_id").and_then(Value::as_str) else {
                continue;
            };
            if let Some(prev) = seen.insert(rid, id.as_str()) {
                dupes.push(Diag {
                    code: "opui.duplicate_runtime_id",
                    severity: "error",
                    node_id: id.clone(),
                    runtime_id: Some(rid.to_string()),
                    message: format!("runtime_id `{rid}` already used by `{prev}`"),
                    strategy: "error",
                });
            }
        }
        self.diags.extend(dupes);
    }

    fn validate_visual_states(&self) -> Result<(), ExportError> {
        let runtime_ids = self
            .nodes
            .values()
            .filter_map(|node| node.get("runtime_id").and_then(Value::as_str))
            .collect::<HashSet<_>>();
        for node in self.nodes.values() {
            let Some(states) = node
                .pointer("/extensions/openpencil.runtime/visual_states")
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (state, target) in states {
                let target = target.as_str().unwrap_or_default();
                if !runtime_ids.contains(target) {
                    return Err(ExportError::msg(format!(
                        "visual state `{state}` targets missing runtimeId `{target}`"
                    )));
                }
            }
        }
        Ok(())
    }

    fn record_raster(&mut self, node: &PenNode, source_id: &str, scene_id: &str) {
        if node_base(node).visible.unwrap_or(true) && is_raster_leaf(node) {
            self.rasters.push(RasterCandidate {
                source_id: source_id.to_string(),
                scene_id: scene_id.to_string(),
                page_index: self.page_index,
            });
        }
    }
}

fn viewport_of(node: &PenNode) -> Option<(f64, f64)> {
    let (w, h) = node_size(node);
    Some((authored_px(w)?, authored_px(h)?))
}

fn authored_px(s: Option<&SizingBehavior>) -> Option<f64> {
    match s {
        Some(SizingBehavior::Number(n)) if n.is_finite() && *n > 0.0 => Some(*n),
        _ => None,
    }
}

fn sizing(s: Option<&SizingBehavior>) -> Option<Value> {
    match s {
        Some(SizingBehavior::Number(n)) if n.is_finite() && *n >= 0.0 => {
            Some(json!({ "type": "px", "value": num(*n) }))
        }
        Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)) => {
            Some(json!({ "type": "fill", "weight": 1 }))
        }
        Some(SizingBehavior::Expression(raw)) => percent_length(raw),
        _ => None,
    }
}

fn padding_edges(p: Option<&Padding>) -> Option<Value> {
    let (t, r, b, l) = match p {
        Some(Padding::Uniform(n)) if n.is_finite() => (*n, *n, *n, *n),
        Some(Padding::XY([y, x])) if y.is_finite() && x.is_finite() => (*y, *x, *y, *x),
        Some(Padding::LtrB([l, t, r, b]))
            if l.is_finite() && t.is_finite() && r.is_finite() && b.is_finite() =>
        {
            (*t, *r, *b, *l)
        }
        _ => return None,
    };
    if t == 0.0 && r == 0.0 && b == 0.0 && l == 0.0 {
        return None;
    }
    Some(json!({
        "top": { "type": "px", "value": num(t) },
        "right": { "type": "px", "value": num(r) },
        "bottom": { "type": "px", "value": num(b) },
        "left": { "type": "px", "value": num(l) },
    }))
}

fn edge_or_auto(v: Option<f64>) -> Value {
    match v {
        Some(n) => json!({ "type": "px", "value": num(n) }),
        None => json!({ "type": "auto" }),
    }
}

fn merge_layout(a: Value, b: Value) -> Value {
    let mut out = match a {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    if let Value::Object(m) = b {
        for (k, v) in m {
            out.insert(k, v);
        }
    }
    Value::Object(out)
}

fn text_content(t: &TextNode) -> (String, Vec<Value>) {
    match &t.content {
        TextContent::Plain(s) => (s.clone(), Vec::new()),
        TextContent::Styled(segs) => styled_runs(segs),
    }
}

fn styled_runs(segs: &[StyledTextSegment]) -> (String, Vec<Value>) {
    let mut content = String::new();
    let mut runs = Vec::new();
    for seg in segs {
        let start = content.len() as u64;
        content.push_str(&seg.text);
        let end = content.len() as u64;
        if start == end {
            continue;
        }
        let mut style = Map::new();
        if let Some(f) = seg.font_family.as_deref().filter(|s| !s.is_empty()) {
            style.insert("family".into(), json!(f));
        }
        if let Some(w) = seg.font_weight {
            style.insert("weight".into(), json!(w.clamp(1, 1000)));
        }
        if let Some(FontStyleKind::Italic) = seg.font_style {
            style.insert("font_style".into(), json!("italic"));
        }
        if let Some(sz) = seg.font_size {
            if sz.is_finite() && sz >= 0.0 {
                style.insert(
                    "size".into(),
                    json!({ "type": "px", "value": num(sz as f64) }),
                );
            }
        }
        if let Some(fill) = &seg.fill {
            if let Some(c) = parse_hex(fill) {
                style.insert("color".into(), color_json(c));
            }
        }
        if style.is_empty() {
            continue;
        }
        runs.push(json!({ "start": start, "end": end, "style": style }));
    }
    (content, runs)
}

fn font_weight(w: Option<&FontWeight>) -> u32 {
    match w {
        Some(FontWeight::Number(n)) => (*n).clamp(1, 1000),
        Some(FontWeight::Keyword(k)) => match k.trim().to_ascii_lowercase().as_str() {
            "thin" | "hairline" => 100,
            "extralight" | "extra_light" | "ultralight" | "ultra_light" => 200,
            "light" => 300,
            "medium" => 500,
            "semibold" | "semi_bold" | "demibold" | "demi_bold" => 600,
            "bold" => 700,
            "extrabold" | "extra_bold" | "ultrabold" | "ultra_bold" => 800,
            "black" | "heavy" => 900,
            s if s.parse::<u32>().is_ok() => s.parse::<u32>().unwrap().clamp(1, 1000),
            _ => 400,
        },
        None => 400,
    }
}

fn first_solid_color(fills: Option<&[PenFill]>) -> Option<[f64; 4]> {
    for f in fills.unwrap_or(&[]) {
        if let PenFill::Solid(body) = f {
            let mut c = parse_hex(&body.color)?;
            if let Some(o) = body.opacity {
                c[3] = (c[3] * o as f64).clamp(0.0, 1.0);
            }
            return Some(c);
        }
    }
    None
}

fn one_fill_json(fill: &PenFill) -> Option<Value> {
    match fill {
        PenFill::Solid(body) => {
            let mut c = parse_hex(&body.color)?;
            if let Some(o) = body.opacity {
                c[3] = (c[3] * o as f64).clamp(0.0, 1.0);
            }
            Some(json!({ "type": "solid", "color": color_json(c) }))
        }
        PenFill::LinearGradient(body) => {
            let stops = gradient_stops(&body.stops, body.opacity)?;
            let angle = body.angle.unwrap_or(0.0) as f64;
            if !angle.is_finite() {
                return None;
            }
            // CSS / OPUI: 0deg is up, clockwise, y-down border box
            let rad = angle.to_radians();
            let (dx, dy) = (rad.sin(), -rad.cos());
            Some(json!({
                "type": "linear",
                "start": { "x": num(0.5 - dx * 0.5), "y": num(0.5 - dy * 0.5) },
                "end": { "x": num(0.5 + dx * 0.5), "y": num(0.5 + dy * 0.5) },
                "stops": stops,
            }))
        }
        PenFill::RadialGradient(body) => {
            let stops = gradient_stops(&body.stops, body.opacity)?;
            let cx = body.cx.unwrap_or(0.5) as f64;
            let cy = body.cy.unwrap_or(0.5) as f64;
            let r = body.radius.unwrap_or(0.5) as f64;
            if !(cx.is_finite() && cy.is_finite() && r.is_finite() && r > 0.0) {
                return None;
            }
            Some(json!({
                "type": "radial",
                "center": { "x": num(cx), "y": num(cy) },
                "radius": { "x": num(r), "y": num(r) },
                "stops": stops,
            }))
        }
        _ => None,
    }
}

fn gradient_stops(
    stops: &[jian_ops_schema::style::GradientStop],
    opacity: Option<f32>,
) -> Option<Vec<Value>> {
    if stops.len() < 2 {
        return None;
    }
    let mul = opacity.unwrap_or(1.0) as f64;
    if !mul.is_finite() {
        return None;
    }
    let mut out = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for stop in stops {
        let offset = f64::from(stop.offset);
        if !offset.is_finite() || !(0.0..=1.0).contains(&offset) || offset < last {
            return None;
        }
        last = offset;
        let mut c = parse_hex(&stop.color)?;
        c[3] = (c[3] * mul).clamp(0.0, 1.0);
        out.push(json!({ "offset": num(offset), "color": color_json(c) }));
    }
    Some(out)
}

fn percent_length(raw: &str) -> Option<Value> {
    let s = raw.trim();
    let digits = s.strip_suffix('%')?.trim();
    let value: f64 = digits.parse().ok()?;
    if !value.is_finite() {
        return None;
    }
    Some(json!({ "type": "percent", "value": num(value) }))
}

fn runtime_id_of(id: &str, name: Option<&str>, explicit: Option<&str>) -> Option<String> {
    explicit
        .filter(|n| is_runtime_id(n))
        .or_else(|| name.filter(|n| is_runtime_id(n)))
        .map(str::to_string)
        .or_else(|| is_runtime_id(id).then(|| id.to_string()))
}

fn runtime_extensions(base: &PenNodeBase) -> Value {
    let mut runtime = Map::new();
    if let Some(role) = base.role.as_deref().filter(|value| !value.is_empty()) {
        runtime.insert("role".into(), json!(role));
    }
    if let Some(label) = base
        .accessibility_label
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        runtime.insert("accessibility_label".into(), json!(label));
    }
    if let Some(tab_index) = base.tab_index {
        runtime.insert("tab_index".into(), json!(tab_index));
    }
    if let Some(BoolOrExpression::Bool(enabled)) = &base.enabled {
        runtime.insert("enabled".into(), json!(enabled));
    }
    if let Some(states) = &base.visual_states {
        runtime.insert("visual_states".into(), json!(states));
    }
    if runtime.is_empty() {
        json!({})
    } else {
        json!({ "openpencil.runtime": runtime })
    }
}

fn is_runtime_id(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('A'..='Z' | 'a'..='z'))
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

fn border_of(stroke: &PenStroke) -> Option<Value> {
    let width = match &stroke.thickness {
        StrokeThickness::Uniform(n) if n.is_finite() && *n >= 0.0 => f64::from(*n),
        StrokeThickness::PerSide([n, _, _, _]) if n.is_finite() && *n >= 0.0 => f64::from(*n),
        StrokeThickness::Sided(s) => {
            let n = s.top.or(s.right).or(s.bottom).or(s.left)?;
            if n.is_finite() && n >= 0.0 {
                f64::from(n)
            } else {
                return None;
            }
        }
        _ => return None,
    };
    let color = first_solid_color(stroke.fill.as_deref())?;
    Some(json!({
        "width": { "type": "px", "value": num(width) },
        "style": "solid",
        "color": color_json(color),
    }))
}

fn corner_json(r: &CornerRadius) -> Value {
    let (tl, tr, br, bl) = match r {
        CornerRadius::Uniform(n) => (*n, *n, *n, *n),
        CornerRadius::PerCorner([tl, tr, br, bl]) => (*tl, *tr, *br, *bl),
    };
    let px = |n: f64| json!({ "type": "px", "value": num(n.max(0.0)) });
    json!({
        "top_left": px(tl),
        "top_right": px(tr),
        "bottom_right": px(br),
        "bottom_left": px(bl),
    })
}

fn parse_hex(s: &str) -> Option<[f64; 4]> {
    let s = s.trim().trim_start_matches('#');
    let (r, g, b, a) = match s.len() {
        3 => (
            u8::from_str_radix(&s[0..1].repeat(2), 16).ok()?,
            u8::from_str_radix(&s[1..2].repeat(2), 16).ok()?,
            u8::from_str_radix(&s[2..3].repeat(2), 16).ok()?,
            255u8,
        ),
        6 => (
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
            255u8,
        ),
        8 => (
            u8::from_str_radix(&s[0..2], 16).ok()?,
            u8::from_str_radix(&s[2..4], 16).ok()?,
            u8::from_str_radix(&s[4..6], 16).ok()?,
            u8::from_str_radix(&s[6..8], 16).ok()?,
        ),
        _ => return None,
    };
    Some([
        r as f64 / 255.0,
        g as f64 / 255.0,
        b as f64 / 255.0,
        a as f64 / 255.0,
    ])
}

fn color_json(c: [f64; 4]) -> Value {
    json!({
        "space": "srgb",
        "r": num(c[0]),
        "g": num(c[1]),
        "b": num(c[2]),
        "a": num(c[3]),
    })
}

fn num(v: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

fn qualify(id: &str, prefix: Option<&str>) -> String {
    match prefix {
        Some(p) => format!("{p}/{id}"),
        None => id.to_string(),
    }
}

fn qualify_scene(id: &str, prefix: Option<&str>) -> String {
    match prefix {
        Some(p) => format!("{p}__{id}"),
        None => id.to_string(),
    }
}

fn sev_rank(s: &str) -> u8 {
    match s {
        "error" => 0,
        "warning" => 1,
        _ => 2,
    }
}

fn diag_json(d: &Diag) -> Value {
    json!({
        "code": d.code,
        "severity": d.severity,
        "node_id": d.node_id,
        "runtime_id": d.runtime_id,
        "message": d.message,
        "strategy": d.strategy,
        "details": {},
    })
}
