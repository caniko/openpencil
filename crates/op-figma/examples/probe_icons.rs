//! Paint-inventory probe for the Tesla service-icon regression batch.
//!
//! It inspects the raw Kiwi tree and prints every fill/stroke, mask flag,
//! blend mode, visibility flag, boolean operation, and geometry source in
//! each selected icon subtree. Instance override payloads are inventoried
//! in place when present.
//!
//! Usage:
//! `cargo run -p op-figma --example probe_icons -- <tesla.fig>`

#![allow(dead_code)]

#[path = "../src/container.rs"]
mod container;
#[path = "../src/corner_geometry.rs"]
mod corner_geometry;
#[path = "../src/figma_types.rs"]
mod figma_types;
#[path = "../src/kiwi.rs"]
mod kiwi;
#[path = "../src/tree.rs"]
mod tree;
#[path = "../src/vector_decoder.rs"]
mod vector_decoder;
#[path = "../src/zip_reader.rs"]
mod zip_reader;

use figma_types::{parse_fig_file, BlobOrString, FigColor, FigMatrix};
use kiwi::FigValue;
use std::collections::{BTreeMap, HashSet};
use tree::{build_tree, guid_to_string, TreeNode};
use vector_decoder::decode_figma_vector_path;

const BATCH_ROOT_GUID: &str = "43:3558";

#[derive(Clone, Copy)]
struct Target {
    class: &'static str,
    label: &'static str,
    guid: &'static str,
}

const TARGETS: &[Target] = &[
    Target {
        class: "HEALTHY",
        label: "换轮胎",
        guid: "43:2343",
    },
    Target {
        class: "A",
        label: "特斯拉专修",
        guid: "43:3422",
    },
    Target {
        class: "A",
        label: "换摆臂",
        guid: "43:3450",
    },
    Target {
        class: "A+C",
        label: "修主机电脑",
        guid: "43:3515",
    },
    Target {
        class: "A",
        label: "低压电池",
        guid: "43:3517",
    },
    Target {
        class: "A",
        label: "换刹车片",
        guid: "43:3545",
    },
    Target {
        class: "A",
        label: "空调养护",
        guid: "43:3611",
    },
    Target {
        class: "A",
        label: "四轮定位",
        guid: "43:3613",
    },
    Target {
        class: "B",
        label: "补胎续电",
        guid: "43:3641",
    },
    Target {
        class: "A",
        label: "给车充电",
        guid: "43:3655",
    },
    Target {
        class: "A",
        label: "换雨刷",
        guid: "43:3668",
    },
    Target {
        class: "A",
        label: "防晒衣",
        guid: "43:3749",
    },
    Target {
        class: "A",
        label: "特斯拉补漆",
        guid: "43:3751",
    },
    Target {
        class: "B",
        label: "撞车点我修",
        guid: "43:3779",
    },
    Target {
        class: "A",
        label: "标洗2次卡",
        guid: "43:3793",
    },
    Target {
        class: "A",
        label: "全部分类",
        guid: "43:3806",
    },
];

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: probe_icons <tesla.fig>");
    let bytes = std::fs::read(&path).expect("read fig");
    let decoded = parse_fig_file(&bytes).expect("parse fig");
    let root = build_tree(&decoded.node_changes).expect("document tree");

    println!(
        "input={path:?} node_changes={} blobs={}",
        decoded.node_changes.len(),
        decoded.blobs.len()
    );
    print_raw_binding_evidence(&decoded.node_changes);
    print_whole_file_frequencies(&root);

    let batch_path = find_path(&root, BATCH_ROOT_GUID).expect("batch root 43:3558");
    println!("\n== BATCH ANCHOR ==");
    println!("guid={BATCH_ROOT_GUID} path={}", path_names(&batch_path));
    println!("class_frequency selected_batch: A=13 B=2 C=1 HEALTHY=1");

    let mut dumped_contexts = HashSet::new();
    for target in TARGETS {
        let path = find_path(&root, target.guid)
            .unwrap_or_else(|| panic!("target {} ({})", target.label, target.guid));
        let label = path.last().expect("label node");
        let actual = display_text(&label.figma);
        println!("\n============================================================");
        println!(
            "TARGET class={} expected={:?} guid={} actual={:?}",
            target.class, target.label, target.guid, actual
        );
        println!("path={}", path_names(&path));
        if matches!(target.label, "特斯拉专修" | "换摆臂") {
            print_instance_chain(&path);
        }
        let context_index = icon_context_index(&path);
        let context = path[context_index];
        let context_guid = node_guid(context);
        println!(
            "label_geometry={}\ncontext_guid={} context_path={}",
            geometry_line(&label.figma, world_before(&path, path.len() - 1)),
            context_guid,
            path_names(&path[..=context_index])
        );
        if !dumped_contexts.insert(context_guid.clone()) {
            println!("context already dumped above; label geometry retained");
            continue;
        }
        let parent_world = world_before(&path, context_index);
        dump_subtree(context, parent_world, 0, &decoded.blobs);
    }
}

fn print_raw_binding_evidence(node_changes: &[FigValue]) {
    const RELEVANT_GUIDS: &[&str] = &[
        "43:3420", "43:3421", "43:3423", "43:3441", "43:3426", "43:3449", "43:3451", "43:3457",
        "43:3453",
    ];
    const BINDING_KEYS: &[&str] = &[
        "symbolData",
        "derivedSymbolData",
        "overriddenSymbolID",
        "componentPropertyReferences",
        "componentProperties",
        "variantProperties",
    ];

    println!("\n== RAW INSTANCE/BINDING EVIDENCE ==");
    for wanted in RELEVANT_GUIDS {
        let records = node_changes
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.get("guid").and_then(guid_to_string).as_deref() == Some(*wanted)
            })
            .collect::<Vec<_>>();
        println!("guid={wanted} rawRecords={}", records.len());
        for (index, record) in records {
            let bindings = BINDING_KEYS
                .iter()
                .filter_map(|key| record.get(key).map(|value| format!("{key}={value:?}")))
                .collect::<Vec<_>>();
            println!(
                "  record[{index}] phase={:?} type={:?} name={:?} visible={:?} bindingFields={} keys=[{}]",
                record.get_str("phase"),
                record.get_str("type"),
                record.get_str("name"),
                record.get_bool("visible"),
                if bindings.is_empty() { "none".to_string() } else { bindings.join(" ") },
                object_keys(record).join(","),
            );
        }
    }

    for key in BINDING_KEYS {
        let matches = node_changes
            .iter()
            .filter(|node| node.get(key).is_some())
            .count();
        println!("wholeFile topLevelField={key} records={matches}");
    }

    for node in node_changes {
        let owner = node
            .get("guid")
            .and_then(guid_to_string)
            .unwrap_or_else(|| "-".to_string());
        let override_entries = node
            .get("symbolData")
            .and_then(|value| value.get_array("symbolOverrides"))
            .unwrap_or(&[]);
        let derived_entries = node.get_array("derivedSymbolData").unwrap_or(&[]);
        for (kind, entries) in [("override", override_entries), ("derived", derived_entries)] {
            for entry in entries {
                let path = guid_path(entry);
                if RELEVANT_GUIDS
                    .iter()
                    .any(|wanted| path.split('/').any(|part| part == *wanted))
                {
                    println!(
                        "targetedEntry owner={owner} kind={kind} path={path} keys=[{}] visible={:?}",
                        object_keys(entry).join(","),
                        entry.get_bool("visible"),
                    );
                }
            }
        }
    }
}

fn print_instance_chain(path: &[&TreeNode]) {
    println!("instance_chain:");
    for (depth, node) in path.iter().enumerate() {
        let figma = &node.figma;
        println!(
            "  [{depth}] guid={} type={} name={:?} keys=[{}] overriddenSymbolID={:?} componentPropertyReferences={:?}",
            node_guid(node),
            figma.get_str("type").unwrap_or("(none)"),
            figma.get_str("name").unwrap_or(""),
            object_keys(figma).join(","),
            figma.get("overriddenSymbolID").and_then(guid_to_string),
            figma.get("componentPropertyReferences"),
        );
        if let Some(symbol_data) = figma.get("symbolData") {
            println!(
                "    symbolData keys=[{}] symbolID={:?} raw={symbol_data:?}",
                object_keys(symbol_data).join(","),
                symbol_data.get("symbolID").and_then(guid_to_string),
            );
            print_instance_overrides(figma, "    ");
        }
        if let Some(derived) = figma.get_array("derivedSymbolData") {
            println!("    derivedSymbolData: count={}", derived.len());
            for (index, entry) in derived.iter().enumerate() {
                println!(
                    "      derived[{index}] path={} keys=[{}] visible={:?}",
                    guid_path(entry),
                    object_keys(entry).join(","),
                    entry.get_bool("visible"),
                );
            }
        }
    }
}

fn guid_path(entry: &FigValue) -> String {
    entry
        .get("guidPath")
        .and_then(|value| value.get_array("guids"))
        .map(|guids| {
            guids
                .iter()
                .filter_map(guid_to_string)
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_else(|| "-".to_string())
}

fn print_whole_file_frequencies(root: &TreeNode) {
    #[derive(Default)]
    struct Frequency {
        occurrences: usize,
        contexts_with_hidden_gradient: usize,
        contexts_with_image: usize,
        contexts_with_multi_fill: usize,
        collapse_signature_contexts: usize,
    }

    #[derive(Default)]
    struct ContextMetrics {
        hidden_gradient: bool,
        image: bool,
        multi_fill: bool,
        visible_blue: bool,
        visible_neutral: bool,
    }

    fn inspect_context(node: &TreeNode, inherited_hidden: bool, metrics: &mut ContextMetrics) {
        let hidden = inherited_hidden || node.figma.get_bool("visible") == Some(false);
        for key in ["fillPaints", "strokePaints"] {
            let Some(paints) = node.figma.get_array(key) else {
                continue;
            };
            let visible = paints
                .iter()
                .filter(|paint| paint.get_bool("visible") != Some(false))
                .collect::<Vec<_>>();
            metrics.multi_fill |= visible.len() > 1;
            for paint in visible {
                let ty = paint.get_str("type").unwrap_or("");
                metrics.image |= ty == "IMAGE";
                metrics.hidden_gradient |= hidden && ty.starts_with("GRADIENT_");
                if !hidden && ty == "SOLID" && paint.get_f64("opacity").unwrap_or(1.0) > 0.0 {
                    if let Some(color) = paint.get("color").and_then(FigColor::from_value) {
                        let alpha = color.a.unwrap_or(1.0);
                        metrics.visible_blue |=
                            alpha > 0.0 && color.b > 0.5 && color.b > color.r + 0.2;
                        let min = color.r.min(color.g).min(color.b);
                        let max = color.r.max(color.g).max(color.b);
                        metrics.visible_neutral |= alpha > 0.0 && max - min < 0.08;
                    }
                }
            }
        }
        for child in &node.children {
            inspect_context(child, hidden, metrics);
        }
    }

    let mut counts: BTreeMap<&str, Frequency> = TARGETS
        .iter()
        .map(|target| (target.label, Frequency::default()))
        .collect();
    fn visit<'a>(
        node: &'a TreeNode,
        stack: &mut Vec<&'a TreeNode>,
        counts: &mut BTreeMap<&str, Frequency>,
    ) {
        stack.push(node);
        let name = node.figma.get_str("name").unwrap_or("");
        let text = display_text(&node.figma);
        for (target, frequency) in counts.iter_mut() {
            if name == *target || text == *target {
                frequency.occurrences += 1;
                let context_index = icon_context_index(stack);
                let mut metrics = ContextMetrics::default();
                inspect_context(stack[context_index], false, &mut metrics);
                frequency.contexts_with_hidden_gradient += usize::from(metrics.hidden_gradient);
                frequency.contexts_with_image += usize::from(metrics.image);
                frequency.contexts_with_multi_fill += usize::from(metrics.multi_fill);
                frequency.collapse_signature_contexts += usize::from(
                    metrics.hidden_gradient
                        && !metrics.image
                        && metrics.visible_blue
                        && metrics.visible_neutral,
                );
            }
        }
        for child in &node.children {
            visit(child, stack, counts);
        }
        stack.pop();
    }
    visit(root, &mut Vec::new(), &mut counts);
    println!("\n== WHOLE-FILE EXACT LABEL FREQUENCY ==");
    println!(
        "collapseSignature = hidden visible-gradient artwork + no image + visible blue and neutral solids"
    );
    for target in TARGETS {
        let frequency = &counts[target.label];
        println!(
            "class={:<7} label={:?} occurrences={} collapseSignatureContexts={} hiddenGradientContexts={} imageContexts={} multiFillContexts={}",
            target.class,
            target.label,
            frequency.occurrences,
            frequency.collapse_signature_contexts,
            frequency.contexts_with_hidden_gradient,
            frequency.contexts_with_image,
            frequency.contexts_with_multi_fill,
        );
    }
}

fn find_path<'a>(root: &'a TreeNode, wanted_guid: &str) -> Option<Vec<&'a TreeNode>> {
    fn visit<'a>(node: &'a TreeNode, wanted_guid: &str, stack: &mut Vec<&'a TreeNode>) -> bool {
        stack.push(node);
        if node_guid(node) == wanted_guid {
            return true;
        }
        for child in &node.children {
            if visit(child, wanted_guid, stack) {
                return true;
            }
        }
        stack.pop();
        false
    }
    let mut stack = Vec::new();
    visit(root, wanted_guid, &mut stack).then_some(stack)
}

fn icon_context_index(path: &[&TreeNode]) -> usize {
    let label_index = path.len() - 1;
    for index in (0..label_index).rev() {
        let candidate = path[index];
        let direct_label = candidate
            .children
            .iter()
            .any(|child| node_guid(child) == node_guid(path[label_index]));
        let has_visual_sibling = candidate.children.iter().any(|child| {
            node_guid(child) != node_guid(path[label_index])
                && child.figma.get_str("type") != Some("TEXT")
        });
        if direct_label && has_visual_sibling {
            return index;
        }
    }
    label_index.saturating_sub(1)
}

fn dump_subtree(node: &TreeNode, parent_world: Matrix, depth: usize, blobs: &[BlobOrString]) {
    let indent = "  ".repeat(depth);
    let local = matrix_of(&node.figma);
    let world = parent_world.then(local);
    println!(
        "{indent}- guid={} type={} name={:?} text={:?} children={} {}",
        node_guid(node),
        node.figma.get_str("type").unwrap_or("(none)"),
        node.figma.get_str("name").unwrap_or(""),
        display_text(&node.figma),
        node.children.len(),
        geometry_line(&node.figma, parent_world)
    );
    print_node_flags(&node.figma, &indent);
    print_paints(&node.figma, "fillPaints", &indent);
    print_paints(&node.figma, "strokePaints", &indent);
    print_geometry_inventory(&node.figma, blobs, &indent);
    print_instance_overrides(&node.figma, &indent);

    for child in &node.children {
        dump_subtree(child, world, depth + 1, blobs);
    }
}

fn print_node_flags(node: &FigValue, indent: &str) {
    let fields = [
        "visible",
        "opacity",
        "blendMode",
        "isMask",
        "maskType",
        "mask",
        "maskIsOutline",
        "frameMaskDisabled",
        "booleanOperation",
        "strokeWeight",
        "strokeAlign",
        "strokeCap",
        "strokeJoin",
    ];
    let values = fields
        .iter()
        .filter_map(|key| node.get(key).map(|value| format!("{key}={value:?}")))
        .collect::<Vec<_>>();
    println!(
        "{indent}  flags: {}",
        if values.is_empty() {
            "(defaults)".to_string()
        } else {
            values.join(" ")
        }
    );
    let refs = ["styleIdForFill", "styleIdForStrokeFill", "styleIdForText"]
        .iter()
        .filter_map(|key| node.get(key).map(|value| format!("{key}={value:?}")))
        .collect::<Vec<_>>();
    if !refs.is_empty() {
        println!("{indent}  style_refs: {}", refs.join(" "));
    }
}

fn print_paints(node: &FigValue, key: &str, indent: &str) {
    let Some(paints) = node.get_array(key) else {
        println!("{indent}  {key}: absent");
        return;
    };
    println!("{indent}  {key}: count={}", paints.len());
    for (index, paint) in paints.iter().enumerate() {
        let ty = paint.get_str("type").unwrap_or("(none)");
        let keys = object_keys(paint).join(",");
        let color = paint
            .get("color")
            .and_then(FigColor::from_value)
            .map(format_color)
            .unwrap_or_else(|| "-".to_string());
        let stops = paint.get_array("stops").unwrap_or(&[]);
        let image = paint.get("image");
        let hash = image
            .and_then(|value| value.get("hash"))
            .and_then(|value| value.as_bytes())
            .map(hex)
            .unwrap_or_else(|| "-".to_string());
        let data_blob = image.and_then(|value| value.get_f64("dataBlob"));
        println!(
            "{indent}    [{index}] type={ty} visible={:?} opacity={:?} blendMode={:?} color={color} stops={} imageHashPresent={} imageHash={} dataBlob={data_blob:?} imageScaleMode={:?} original=({:?},{:?}) keys=[{keys}]",
            paint.get_bool("visible"),
            paint.get_f64("opacity"),
            paint.get_str("blendMode"),
            stops.len(),
            hash != "-",
            hash,
            paint.get_str("imageScaleMode"),
            paint.get_f64("originalImageWidth"),
            paint.get_f64("originalImageHeight"),
        );
        for (stop_index, stop) in stops.iter().enumerate() {
            let stop_color = stop
                .get("color")
                .and_then(FigColor::from_value)
                .map(format_color)
                .unwrap_or_else(|| "-".to_string());
            println!(
                "{indent}      stop[{stop_index}] position={:?} color={stop_color}",
                stop.get_f64("position")
            );
        }
    }
}

fn print_geometry_inventory(node: &FigValue, blobs: &[BlobOrString], indent: &str) {
    let fill_geometry = node.get_array("fillGeometry").map_or(0, <[FigValue]>::len);
    let stroke_geometry = node
        .get_array("strokeGeometry")
        .map_or(0, <[FigValue]>::len);
    let network = node
        .get("vectorData")
        .and_then(|value| value.get_f64("vectorNetworkBlob"));
    if fill_geometry == 0 && stroke_geometry == 0 && network.is_none() {
        return;
    }
    let decoded = decode_figma_vector_path(node, blobs);
    println!(
        "{indent}  geometry: fillRecords={fill_geometry} strokeRecords={stroke_geometry} vectorNetworkBlob={network:?} decoded={}",
        decoded.as_ref().map_or_else(
            || "none".to_string(),
            |value| format!(
                "dLen:{} fillRule:{:?} allowsFill:{} fromStroke:{}",
                value.d.len(), value.fill_rule, value.allows_fill, value.from_stroke_geometry
            )
        )
    );
}

fn print_instance_overrides(node: &FigValue, indent: &str) {
    let Some(overrides) = node
        .get("symbolData")
        .and_then(|value| value.get_array("symbolOverrides"))
    else {
        return;
    };
    println!("{indent}  symbolOverrides: count={}", overrides.len());
    for (index, entry) in overrides.iter().enumerate() {
        let path = guid_path(entry);
        println!(
            "{indent}    override[{index}] path={path} keys=[{}] visible={:?} opacity={:?} blendMode={:?}",
            object_keys(entry).join(","),
            entry.get_bool("visible"),
            entry.get_f64("opacity"),
            entry.get_str("blendMode")
        );
        print_paints(entry, "fillPaints", &format!("{indent}    "));
        print_paints(entry, "strokePaints", &format!("{indent}    "));
    }
}

#[derive(Clone, Copy)]
struct Matrix {
    m00: f64,
    m01: f64,
    m02: f64,
    m10: f64,
    m11: f64,
    m12: f64,
}

impl Matrix {
    const IDENTITY: Self = Self {
        m00: 1.0,
        m01: 0.0,
        m02: 0.0,
        m10: 0.0,
        m11: 1.0,
        m12: 0.0,
    };

    fn then(self, rhs: Self) -> Self {
        Self {
            m00: self.m00 * rhs.m00 + self.m01 * rhs.m10,
            m01: self.m00 * rhs.m01 + self.m01 * rhs.m11,
            m02: self.m00 * rhs.m02 + self.m01 * rhs.m12 + self.m02,
            m10: self.m10 * rhs.m00 + self.m11 * rhs.m10,
            m11: self.m10 * rhs.m01 + self.m11 * rhs.m11,
            m12: self.m10 * rhs.m02 + self.m11 * rhs.m12 + self.m12,
        }
    }

    fn point(self, x: f64, y: f64) -> (f64, f64) {
        (
            self.m00 * x + self.m01 * y + self.m02,
            self.m10 * x + self.m11 * y + self.m12,
        )
    }
}

fn matrix_of(node: &FigValue) -> Matrix {
    node.get("transform")
        .and_then(FigMatrix::from_value)
        .map(|matrix| Matrix {
            m00: matrix.m00,
            m01: matrix.m01,
            m02: matrix.m02,
            m10: matrix.m10,
            m11: matrix.m11,
            m12: matrix.m12,
        })
        .unwrap_or(Matrix::IDENTITY)
}

fn world_before(path: &[&TreeNode], index: usize) -> Matrix {
    path.iter()
        .take(index)
        .fold(Matrix::IDENTITY, |world, node| {
            world.then(matrix_of(&node.figma))
        })
}

fn geometry_line(node: &FigValue, parent_world: Matrix) -> String {
    let local = matrix_of(node);
    let world = parent_world.then(local);
    let size = node.get("size");
    let width = size.and_then(|value| value.get_f64("x")).unwrap_or(0.0);
    let height = size.and_then(|value| value.get_f64("y")).unwrap_or(0.0);
    let corners = [
        world.point(0.0, 0.0),
        world.point(width, 0.0),
        world.point(0.0, height),
        world.point(width, height),
    ];
    let min_x = corners
        .iter()
        .map(|point| point.0)
        .fold(f64::INFINITY, f64::min);
    let max_x = corners
        .iter()
        .map(|point| point.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = corners
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = corners
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    format!(
        "localTransform=[{:.4} {:.4} {:.2}; {:.4} {:.4} {:.2}] size=({width:.2},{height:.2}) worldAabb=({min_x:.2},{min_y:.2},{:.2},{:.2})",
        local.m00,
        local.m01,
        local.m02,
        local.m10,
        local.m11,
        local.m12,
        max_x - min_x,
        max_y - min_y
    )
}

fn display_text(node: &FigValue) -> &str {
    node.get("textData")
        .and_then(|value| value.get_str("characters"))
        .unwrap_or("")
}

fn node_guid(node: &TreeNode) -> String {
    node.figma
        .get("guid")
        .and_then(guid_to_string)
        .unwrap_or_else(|| "(none)".to_string())
}

fn path_names(path: &[&TreeNode]) -> String {
    path.iter()
        .map(|node| node.figma.get_str("name").unwrap_or("(unnamed)"))
        .collect::<Vec<_>>()
        .join(" / ")
}

fn object_keys(value: &FigValue) -> Vec<&str> {
    match value {
        FigValue::Object(pairs) => pairs.iter().map(|(key, _)| key.as_str()).collect(),
        _ => Vec::new(),
    }
}

fn format_color(color: FigColor) -> String {
    format!(
        "rgba({:.4},{:.4},{:.4},{:.4})",
        color.r,
        color.g,
        color.b,
        color.a.unwrap_or(1.0)
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
