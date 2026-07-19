//! Diagnostic probe — classify vector-node decode failures in a real
#![allow(dead_code)]

//! `.fig` (raw fig-kiwi or zip). Mounts the crate's private decode
//! modules via #[path] so no library source changes are needed.
//!
//! Usage: cargo run -p op-figma --example probe_vec -- <canvas.fig>

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

use figma_types::{parse_fig_file, BlobOrString, FigMatrix};
use kiwi::FigValue;
use std::collections::BTreeMap;
use tree::{build_tree, guid_to_string, TreeNode};
use vector_decoder::decode_figma_vector_path;

fn keys(v: &FigValue) -> Vec<String> {
    match v {
        FigValue::Object(pairs) => pairs.iter().map(|(k, _)| k.clone()).collect(),
        _ => Vec::new(),
    }
}

fn bump(map: &mut BTreeMap<String, usize>, key: &str) {
    *map.entry(key.to_string()).or_insert(0) += 1;
}

fn geometry_dump(node: &FigValue, blobs: &[BlobOrString], indent: &str) {
    println!("{indent}keys: {:?}", keys(node));
    let size = node.get("size");
    println!(
        "{indent}size=({:?},{:?}) strokeWeight={:?} align={:?} cap={:?} join={:?}",
        size.and_then(|value| value.get_f64("x")),
        size.and_then(|value| value.get_f64("y")),
        node.get_f64("strokeWeight"),
        node.get_str("strokeAlign"),
        node.get_str("strokeCap"),
        node.get_str("strokeJoin")
    );
    if node.get_str("type") == Some("INSTANCE") {
        let symbol_data = node.get("symbolData");
        let symbol_id = symbol_data
            .and_then(|data| data.get("symbolID"))
            .and_then(guid_to_string);
        let overridden_symbol_id = node.get("overriddenSymbolID").and_then(guid_to_string);
        let overrides = symbol_data
            .and_then(|data| data.get_array("symbolOverrides"))
            .unwrap_or(&[]);
        let derived = node.get_array("derivedSymbolData").unwrap_or(&[]);
        println!(
            "{indent}instance symbolID={symbol_id:?} overriddenSymbolID={overridden_symbol_id:?} overrides={} derived={}",
            overrides.len(),
            derived.len()
        );
        if !overrides.is_empty() {
            println!("{indent}symbolOverrides: {overrides:?}");
        }
    }
    if let Some(boolean_operation) = node.get_str("booleanOperation") {
        println!("{indent}booleanOperation={boolean_operation}");
    }
    for paint_key in ["fillPaints", "strokePaints"] {
        if let Some(paints) = node.get_array(paint_key) {
            println!("{indent}{paint_key}: {paints:?}");
        }
    }
    for geometry_key in ["fillGeometry", "strokeGeometry"] {
        let Some(geometries) = node.get_array(geometry_key) else {
            continue;
        };
        println!("{indent}{geometry_key}: {} record(s)", geometries.len());
        for (geometry_index, geometry) in geometries.iter().enumerate() {
            let winding = geometry.get_str("windingRule").unwrap_or("(none)");
            let blob_index = geometry.get_f64("commandsBlob").map(|value| value as usize);
            match blob_index.and_then(|index| blobs.get(index).map(|blob| (index, blob))) {
                Some((index, BlobOrString::Bytes(bytes))) => {
                    println!(
                        "{indent}  [{geometry_index}] commandsBlob={index} len={} windingRule={winding} bytes={bytes:02x?}",
                        bytes.len()
                    );
                    println!(
                        "{indent}      opcode decode: {:?}",
                        vector_decoder::decode_figma_path_blob(bytes)
                    );
                }
                Some((index, BlobOrString::Str(value))) => println!(
                    "{indent}  [{geometry_index}] commandsBlob={index} string={value:?} windingRule={winding}"
                ),
                None => println!(
                    "{indent}  [{geometry_index}] commandsBlob={blob_index:?} missing windingRule={winding}"
                ),
            }
        }
    }
    let vector_blob = node
        .get("vectorData")
        .and_then(|data| data.get_f64("vectorNetworkBlob"))
        .map(|value| value as usize);
    if let Some(index) = vector_blob {
        match blobs.get(index) {
            Some(BlobOrString::Bytes(bytes)) => println!(
                "{indent}vectorNetworkBlob={index} len={} header={:?}",
                bytes.len(),
                (0..3)
                    .filter_map(|word| {
                        let offset = word * 4;
                        bytes.get(offset..offset + 4).map(|slice| {
                            u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]])
                        })
                    })
                    .collect::<Vec<_>>()
            ),
            Some(BlobOrString::Str(value)) => {
                println!("{indent}vectorNetworkBlob={index} string={value:?}")
            }
            None => println!("{indent}vectorNetworkBlob={index} missing"),
        }
    }
    let decoded = decode_figma_vector_path(node, blobs);
    println!("{indent}current decode: {decoded:?}");
}

fn dump_subtree(node: &TreeNode, blobs: &[BlobOrString], depth: usize, max_depth: usize) {
    let indent = "  ".repeat(depth);
    let figma = &node.figma;
    let guid = figma
        .get("guid")
        .and_then(guid_to_string)
        .unwrap_or_else(|| "(none)".to_string());
    println!(
        "{indent}- guid={guid} type={} name={:?} children={}",
        figma.get_str("type").unwrap_or("(none)"),
        figma.get_str("name").unwrap_or(""),
        node.children.len()
    );
    geometry_dump(figma, blobs, &format!("{indent}  "));
    if depth < max_depth {
        for child in &node.children {
            dump_subtree(child, blobs, depth + 1, max_depth);
        }
    }
}

fn dump_named_contexts(
    root: &TreeNode,
    blobs: &[BlobOrString],
    page_name: &str,
    targets: &[String],
    ancestor_levels: usize,
    max_depth: usize,
) {
    struct Options<'a> {
        blobs: &'a [BlobOrString],
        page_name: &'a str,
        targets: &'a [String],
        ancestor_levels: usize,
        max_depth: usize,
    }

    fn visit<'a>(
        node: &'a TreeNode,
        options: &Options<'_>,
        current_page: Option<&'a str>,
        stack: &mut Vec<&'a TreeNode>,
    ) {
        let current_page = if node.figma.get_str("type") == Some("CANVAS") {
            node.figma.get_str("name")
        } else {
            current_page
        };
        stack.push(node);
        let name = node.figma.get_str("name").unwrap_or("");
        let characters = node
            .figma
            .get("textData")
            .and_then(|data| data.get_str("characters"))
            .unwrap_or("");
        let matched_target = options
            .targets
            .iter()
            .find(|target| target.as_str() == name || target.as_str() == characters);
        if let Some(target) = matched_target
            .filter(|_| options.page_name == "*" || current_page == Some(options.page_name))
        {
            let context_index = stack.len().saturating_sub(1 + options.ancestor_levels);
            let context = stack[context_index];
            let path = stack
                .iter()
                .map(|entry| entry.figma.get_str("name").unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" / ");
            println!(
                "\n== TARGET {target:?} (node name={name:?}, characters={characters:?}) on page {:?} ==",
                current_page.unwrap_or("(outside canvas)")
            );
            println!("tree path: {path}");
            println!("context ancestor levels: {}", options.ancestor_levels);
            dump_subtree(context, options.blobs, 0, options.max_depth);
        }
        for child in &node.children {
            visit(child, options, current_page, stack);
        }
        stack.pop();
    }

    let options = Options {
        blobs,
        page_name,
        targets,
        ancestor_levels,
        max_depth,
    };
    visit(root, &options, None, &mut Vec::new());
}

fn layout_value(node: &FigValue, key: &str) -> String {
    if let Some(value) = node.get_str(key) {
        return value.to_string();
    }
    if let Some(value) = node.get_f64(key) {
        return format!("{value:.2}");
    }
    if let Some(value) = node.get_bool(key) {
        return value.to_string();
    }
    "-".to_string()
}

fn layout_dump(node: &TreeNode, depth: usize, max_depth: usize) {
    let indent = "  ".repeat(depth);
    let figma = &node.figma;
    let guid = figma
        .get("guid")
        .and_then(guid_to_string)
        .unwrap_or_else(|| "(none)".to_string());
    let transform = figma.get("transform").and_then(FigMatrix::from_value);
    let (m00, m01, m02, m10, m11, m12) = transform
        .map(|m| (m.m00, m.m01, m.m02, m.m10, m.m11, m.m12))
        .unwrap_or((0.0, 0.0, 0.0, 0.0, 0.0, 0.0));
    let size = figma.get("size");
    let w = size.and_then(|value| value.get_f64("x")).unwrap_or(0.0);
    let h = size.and_then(|value| value.get_f64("y")).unwrap_or(0.0);
    let characters = figma
        .get("textData")
        .and_then(|data| data.get_str("characters"))
        .unwrap_or("");
    println!(
        "{indent}- guid={guid} type={} name={:?} text={characters:?} children={}",
        figma.get_str("type").unwrap_or("(none)"),
        figma.get_str("name").unwrap_or(""),
        node.children.len()
    );
    println!(
        "{indent}  transform=[{m00:.4} {m01:.4} {m02:.2}; {m10:.4} {m11:.4} {m12:.2}] size=({w:.2},{h:.2})"
    );
    println!(
        "{indent}  stackMode={} spacing={} primarySizing={} counterSizing={} primaryAlign={} counterAlign={}",
        layout_value(figma, "stackMode"),
        layout_value(figma, "stackSpacing"),
        layout_value(figma, "stackPrimarySizing"),
        layout_value(figma, "stackCounterSizing"),
        layout_value(figma, "stackPrimaryAlignItems"),
        layout_value(figma, "stackCounterAlignItems")
    );
    println!(
        "{indent}  padding=[top:{} right:{} bottom:{} left:{} uniform:{} h:{} v:{}] textAutoResize={} constraints=({}, {})",
        layout_value(figma, "stackPaddingTop"),
        layout_value(figma, "stackPaddingRight"),
        layout_value(figma, "stackPaddingBottom"),
        layout_value(figma, "stackPaddingLeft"),
        layout_value(figma, "stackPadding"),
        layout_value(figma, "stackHorizontalPadding"),
        layout_value(figma, "stackVerticalPadding"),
        layout_value(figma, "textAutoResize"),
        layout_value(figma, "horizontalConstraint"),
        layout_value(figma, "verticalConstraint")
    );
    if figma.get_str("type") == Some("TEXT") {
        let font_name = figma.get("fontName");
        println!(
            "{indent}  textStyle=family:{:?} style:{:?} size:{} lineHeight:{:?} letterSpacing:{:?} align=({},{})",
            font_name.and_then(|value| value.get_str("family")),
            font_name.and_then(|value| value.get_str("style")),
            layout_value(figma, "fontSize"),
            figma.get("lineHeight"),
            figma.get("letterSpacing"),
            layout_value(figma, "textAlignHorizontal"),
            layout_value(figma, "textAlignVertical")
        );
    }
    if let Some(image) = figma.get_array("fillPaints").and_then(|paints| {
        paints
            .iter()
            .find(|paint| paint.get_str("type") == Some("IMAGE"))
    }) {
        let transform = image.get("transform").and_then(FigMatrix::from_value);
        println!(
            "{indent}  image mode={} original=({},{}) transform={:?}",
            layout_value(image, "imageScaleMode"),
            layout_value(image, "originalImageWidth"),
            layout_value(image, "originalImageHeight"),
            transform.map(|m| [m.m00, m.m01, m.m02, m.m10, m.m11, m.m12])
        );
    }
    if depth < max_depth {
        for child in &node.children {
            layout_dump(child, depth + 1, max_depth);
        }
    }
}

struct LayoutProbeOptions<'a> {
    page_name: &'a str,
    targets: &'a [String],
    guids: &'a [String],
    target_size: Option<(f64, f64)>,
    ancestor_levels: usize,
    max_depth: usize,
    match_limit: usize,
}

fn dump_layout_contexts(root: &TreeNode, options: &LayoutProbeOptions<'_>) {
    fn visit<'a>(
        node: &'a TreeNode,
        options: &LayoutProbeOptions<'_>,
        current_page: Option<&'a str>,
        stack: &mut Vec<&'a TreeNode>,
        matches: &mut usize,
    ) {
        if *matches >= options.match_limit {
            return;
        }
        let current_page = if node.figma.get_str("type") == Some("CANVAS") {
            node.figma.get_str("name")
        } else {
            current_page
        };
        stack.push(node);
        let name = node.figma.get_str("name").unwrap_or("");
        let characters = node
            .figma
            .get("textData")
            .and_then(|data| data.get_str("characters"))
            .unwrap_or("");
        let guid = node.figma.get("guid").and_then(guid_to_string);
        let size = node.figma.get("size");
        let size_matches = options.target_size.is_some_and(|(target_w, target_h)| {
            let w = size
                .and_then(|value| value.get_f64("x"))
                .unwrap_or(f64::NAN);
            let h = size
                .and_then(|value| value.get_f64("y"))
                .unwrap_or(f64::NAN);
            (w - target_w).abs() < 0.01 && (h - target_h).abs() < 0.01
        });
        let matched = options
            .targets
            .iter()
            .any(|target| target == name || target == characters)
            || guid
                .as_ref()
                .is_some_and(|guid| options.guids.iter().any(|target| target == guid))
            || size_matches;
        if matched && (options.page_name == "*" || current_page == Some(options.page_name)) {
            *matches += 1;
            let context_index = stack.len().saturating_sub(1 + options.ancestor_levels);
            let context = stack[context_index];
            let path = stack
                .iter()
                .map(|entry| entry.figma.get_str("name").unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" / ");
            println!(
                "\n== LAYOUT MATCH {} on page {:?} ==",
                guid.as_deref().unwrap_or(name),
                current_page.unwrap_or("(outside canvas)")
            );
            println!("tree path: {path}");
            layout_dump(context, 0, options.max_depth);
        }
        for child in &node.children {
            visit(child, options, current_page, stack, matches);
        }
        stack.pop();
    }

    visit(root, options, None, &mut Vec::new(), &mut 0);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .expect("usage: probe_vec <path> [--page NAME] [--target NAME]...");
    let page_name = args
        .windows(2)
        .find(|pair| pair[0] == "--page")
        .map(|pair| pair[1].as_str())
        .unwrap_or("v1.0");
    let targets: Vec<String> = args
        .windows(2)
        .filter(|pair| pair[0] == "--target")
        .map(|pair| pair[1].clone())
        .collect();
    let guids: Vec<String> = args
        .windows(2)
        .filter(|pair| pair[0] == "--guid")
        .map(|pair| pair[1].clone())
        .collect();
    let target_size = args
        .windows(2)
        .find(|pair| pair[0] == "--size")
        .and_then(|pair| pair[1].split_once('x'))
        .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
    let layout_only = args.iter().any(|arg| arg == "--layout");
    let blob_indices: Vec<usize> = args
        .windows(2)
        .filter(|pair| pair[0] == "--blob")
        .filter_map(|pair| pair[1].parse().ok())
        .collect();
    let ancestor_levels = args
        .windows(2)
        .find(|pair| pair[0] == "--ancestor")
        .and_then(|pair| pair[1].parse().ok())
        .unwrap_or(1);
    let max_depth = args
        .windows(2)
        .find(|pair| pair[0] == "--depth")
        .and_then(|pair| pair[1].parse().ok())
        .unwrap_or(4);
    let match_limit = args
        .windows(2)
        .find(|pair| pair[0] == "--limit")
        .and_then(|pair| pair[1].parse().ok())
        .unwrap_or(20);
    let bytes = std::fs::read(path).expect("read");
    let decoded = match parse_fig_file(&bytes) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("parse error: {e:?}");
            std::process::exit(1);
        }
    };
    println!(
        "node_changes: {}  blobs: {}",
        decoded.node_changes.len(),
        decoded.blobs.len()
    );
    let mut blob_bytes = 0usize;
    let mut blob_strs = 0usize;
    for b in &decoded.blobs {
        match b {
            BlobOrString::Bytes(_) => blob_bytes += 1,
            BlobOrString::Str(_) => blob_strs += 1,
        }
    }
    println!("blob kinds: bytes={blob_bytes} str={blob_strs}");

    if !blob_indices.is_empty() {
        for index in blob_indices {
            match decoded.blobs.get(index) {
                Some(BlobOrString::Bytes(bytes)) => {
                    let hex = bytes
                        .iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<String>();
                    println!("blob[{index}] len={} hex={hex}", bytes.len());
                }
                Some(BlobOrString::Str(value)) => println!("blob[{index}] string={value:?}"),
                None => println!("blob[{index}] missing"),
            }
        }
        return;
    }

    if layout_only && (!targets.is_empty() || !guids.is_empty() || target_size.is_some()) {
        let root = build_tree(&decoded.node_changes).expect("document tree");
        let options = LayoutProbeOptions {
            page_name,
            targets: &targets,
            guids: &guids,
            target_size,
            ancestor_levels,
            max_depth,
            match_limit,
        };
        dump_layout_contexts(&root, &options);
        return;
    }

    if !targets.is_empty() {
        let root = build_tree(&decoded.node_changes).expect("document tree");
        dump_named_contexts(
            &root,
            &decoded.blobs,
            page_name,
            &targets,
            ancestor_levels,
            max_depth,
        );
        return;
    }

    // Global tallies.
    let mut type_tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut effect_tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut corner_tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut bool_op_tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut arc_data_count = 0usize;
    let mut ellipse_count = 0usize;

    // Vector-node failure classification.
    let mut vec_total = 0usize;
    let mut vec_class: BTreeMap<String, usize> = BTreeMap::new();
    let mut fail_samples: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut fail_field_tally: BTreeMap<String, usize> = BTreeMap::new();

    const VEC_TYPES: [&str; 4] = ["VECTOR", "STAR", "REGULAR_POLYGON", "BOOLEAN_OPERATION"];

    for nc in &decoded.node_changes {
        let ty = nc.get_str("type").unwrap_or("(none)").to_string();
        bump(&mut type_tally, &ty);

        if let Some(effects) = nc.get_array("effects") {
            for e in effects {
                let ety = e.get_str("type").unwrap_or("(untyped)");
                let vis = if e.get_bool("visible") == Some(false) {
                    "hidden"
                } else {
                    "visible"
                };
                bump(&mut effect_tally, &format!("{ety}/{vis}"));
            }
        }

        if nc.get_f64("cornerRadius").map(|v| v > 0.0) == Some(true) {
            bump(&mut corner_tally, &format!("{ty}/cornerRadius>0"));
        }
        if nc.get_bool("rectangleCornerRadiiIndependent") == Some(true) {
            bump(&mut corner_tally, &format!("{ty}/perCornerRadii"));
        }
        if nc.get_f64("cornerSmoothing").map(|v| v > 0.0) == Some(true) {
            bump(&mut corner_tally, &format!("{ty}/cornerSmoothing>0"));
        }

        if ty == "ELLIPSE" {
            ellipse_count += 1;
            if nc.get("arcData").is_some() {
                arc_data_count += 1;
            }
        }
        if ty == "BOOLEAN_OPERATION" {
            let op = nc.get_str("booleanOperation").unwrap_or("(none)");
            bump(&mut bool_op_tally, op);
        }

        if !VEC_TYPES.contains(&ty.as_str()) {
            continue;
        }
        vec_total += 1;

        let fill_geo = nc.get_array("fillGeometry");
        let stroke_geo = nc.get_array("strokeGeometry");
        let vn = nc
            .get("vectorData")
            .and_then(|v| v.get("vectorNetworkBlob"))
            .is_some();
        let fill_n = fill_geo.map(|g| g.len()).unwrap_or(0);
        let stroke_n = stroke_geo.map(|g| g.len()).unwrap_or(0);

        for g in fill_geo
            .unwrap_or(&[])
            .iter()
            .chain(stroke_geo.unwrap_or(&[]))
        {
            let wr = g.get_str("windingRule").unwrap_or("(none)");
            bump(&mut corner_tally, &format!("windingRule={wr}"));
        }

        // Geometry-entry field check: does any entry carry commandsBlob?
        let mut geo_has_commands = false;
        let mut geo_blob_oob = false;
        let mut geo_blob_is_str = false;
        let mut geo_blob_short = false;
        for g in fill_geo
            .unwrap_or(&[])
            .iter()
            .chain(stroke_geo.unwrap_or(&[]))
        {
            if let Some(idx) = g.get_f64("commandsBlob") {
                geo_has_commands = true;
                match decoded.blobs.get(idx as usize) {
                    Some(BlobOrString::Bytes(b)) => {
                        if b.len() < 9 {
                            geo_blob_short = true;
                        }
                    }
                    Some(BlobOrString::Str(_)) => geo_blob_is_str = true,
                    None => geo_blob_oob = true,
                }
            }
        }

        let decoded_path = decode_figma_vector_path(nc, &decoded.blobs);
        let ok = decoded_path.as_deref().map(|d| !d.is_empty()) == Some(true);

        let geo_desc = format!(
            "fillGeo={} strokeGeo={} cmdBlob={} vn={}",
            fill_n, stroke_n, geo_has_commands, vn
        );
        let class = if ok {
            format!("OK ({ty}) [{geo_desc}]")
        } else {
            let reason = if fill_n == 0 && stroke_n == 0 && !vn {
                "FAIL: no geometry arrays + no vectorNetworkBlob"
            } else if fill_n + stroke_n > 0 && !geo_has_commands {
                "FAIL: geometry entries lack commandsBlob field"
            } else if geo_blob_oob {
                "FAIL: commandsBlob index out of blob range"
            } else if geo_blob_is_str {
                "FAIL: commandsBlob points at string blob"
            } else if geo_blob_short {
                "FAIL: blob < 9 bytes (decoder minimum)"
            } else if vn {
                "FAIL: vectorNetworkBlob present but decode failed"
            } else {
                "FAIL: other"
            };
            format!("{reason} ({ty})")
        };
        bump(&mut vec_class, &class);

        if !ok && fail_samples.len() < 12 {
            let name = nc.get_str("name").unwrap_or("").to_string();
            fail_samples.push((ty.clone(), name, keys(nc)));
        }
        if !ok {
            for k in keys(nc) {
                bump(&mut fail_field_tally, &k);
            }
        }
    }

    println!("\n== node type tally (top 25) ==");
    let mut tv: Vec<_> = type_tally.iter().collect();
    tv.sort_by(|a, b| b.1.cmp(a.1));
    for (k, v) in tv.iter().take(25) {
        println!("  {v:>6}  {k}");
    }

    println!("\n== vector-family nodes: {vec_total} ==");
    for (k, v) in &vec_class {
        println!("  {v:>6}  {k}");
    }

    println!("\n== effects tally ==");
    for (k, v) in &effect_tally {
        println!("  {v:>6}  {k}");
    }

    println!("\n== corner tally ==");
    for (k, v) in &corner_tally {
        println!("  {v:>6}  {k}");
    }

    println!("\n== boolean ops ==");
    for (k, v) in &bool_op_tally {
        println!("  {v:>6}  {k}");
    }
    println!("\nellipses: {ellipse_count} (with arcData: {arc_data_count})");

    println!("\n== failing-node field frequency (top 30) ==");
    let mut fv: Vec<_> = fail_field_tally.iter().collect();
    fv.sort_by(|a, b| b.1.cmp(a.1));
    for (k, v) in fv.iter().take(30) {
        println!("  {v:>6}  {k}");
    }

    println!("\n== failing samples (first 12) ==");
    for (ty, name, ks) in &fail_samples {
        println!("  [{ty}] {name:?}");
        println!("      keys: {}", ks.join(","));
    }
}
