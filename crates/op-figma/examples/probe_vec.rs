//! Diagnostic probe — classify vector-node decode failures in a real
#![allow(dead_code)]

//! `.fig` (raw fig-kiwi or zip). Mounts the crate's private decode
//! modules via #[path] so no library source changes are needed.
//!
//! Usage: cargo run -p op-figma --example probe_vec -- <canvas.fig>

#[path = "../src/container.rs"]
mod container;
#[path = "../src/figma_types.rs"]
mod figma_types;
#[path = "../src/kiwi.rs"]
mod kiwi;
#[path = "../src/vector_decoder.rs"]
mod vector_decoder;
#[path = "../src/zip_reader.rs"]
mod zip_reader;

use figma_types::{parse_fig_file, BlobOrString};
use kiwi::FigValue;
use std::collections::BTreeMap;
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

fn main() {
    let path = std::env::args().nth(1).expect("usage: probe_vec <path>");
    let bytes = std::fs::read(&path).expect("read");
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
