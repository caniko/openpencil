//! Probe vectorNetworkBlob binary layout on real failing nodes.
#![allow(dead_code)]

//! Tests two candidate layouts:
//!  A (current decoder): u32 V; V*(f32 x,f32 y); u32 S; S*(u32,u32,4*f32)
//!  B (fig2sketch):      u32 V; u32 S; u32 R; V*(u32 styleID,f32 x,f32 y);
//!                       S*(u32 styleID,u32 start,f32 tsx,f32 tsy,u32 end,f32 tex,f32 tey); regions...
//!
//! Usage: cargo run -p op-figma --example probe_vn -- <canvas.fig> [--dump-smallest N] [--dump-regions]

#[path = "../src/container.rs"]
mod container;
#[path = "../src/corner_geometry.rs"]
mod corner_geometry;
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
use vector_decoder::decode_figma_vector_path;

#[derive(Debug)]
struct RegionRecord {
    raw_style_and_winding: u32,
    loops: Vec<Vec<u32>>,
}

fn u32_le(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
fn f32_le(b: &[u8], o: usize) -> Option<f32> {
    b.get(o..o + 4)
        .map(|s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn keys(v: &FigValue) -> Vec<String> {
    match v {
        FigValue::Object(pairs) => pairs.iter().map(|(k, _)| k.clone()).collect(),
        _ => Vec::new(),
    }
}

fn parse_region_records(blob: &[u8]) -> Option<(usize, usize, Vec<RegionRecord>)> {
    let (Some(vertices), Some(segments), Some(regions)) =
        (u32_le(blob, 0), u32_le(blob, 4), u32_le(blob, 8))
    else {
        return None;
    };
    let mut off = 12usize
        .checked_add(vertices as usize * 12)
        .and_then(|value| value.checked_add(segments as usize * 28))?;
    let region_start = off;
    let mut decoded = Vec::new();
    for _ in 0..regions as usize {
        let raw_style_and_winding = u32_le(blob, off)?;
        let loop_count = u32_le(blob, off + 4)?;
        off += 8;
        let mut loops = Vec::new();
        for _ in 0..loop_count as usize {
            let index_count = u32_le(blob, off)?;
            off += 4;
            let mut indices = Vec::with_capacity(index_count as usize);
            for _ in 0..index_count as usize {
                let segment_index = u32_le(blob, off)?;
                if segment_index >= segments {
                    return None;
                }
                indices.push(segment_index);
                off += 4;
            }
            loops.push(indices);
        }
        decoded.push(RegionRecord {
            raw_style_and_winding,
            loops,
        });
    }
    (off <= blob.len()).then_some((region_start, off, decoded))
}

fn dump_region_records(blob: &[u8], blob_index: usize, name: &str) -> bool {
    let (Some(vertices), Some(segments), Some(regions)) =
        (u32_le(blob, 0), u32_le(blob, 4), u32_le(blob, 8))
    else {
        return false;
    };
    if regions == 0 {
        return false;
    }
    let Some((region_start, off, decoded)) = parse_region_records(blob) else {
        return false;
    };
    let Some(raw) = blob.get(region_start..off) else {
        return false;
    };
    println!(
        "REGION SAMPLE blobIdx={blob_index} node={name:?} V={vertices} S={segments} R={regions} regionOffset={region_start} parsedEnd={off} blobLen={}",
        blob.len()
    );
    println!("  raw region bytes: {raw:02x?}");
    for (region_index, region) in decoded.into_iter().enumerate() {
        let raw = region.raw_style_and_winding;
        println!(
            "  region[{region_index}] raw={raw} styleID={} lowBit={} loops={:?}",
            raw >> 1,
            raw & 1,
            region.loops
        );
    }
    println!("  exact consumption: {}", off == blob.len());
    true
}

fn dump_full_network(blob: &[u8], blob_index: usize, node: &FigValue, blobs: &[BlobOrString]) {
    let name = node.get_str("name").unwrap_or("");
    let ty = node.get_str("type").unwrap_or("");
    let (Some(vertices), Some(segments), Some(regions)) =
        (u32_le(blob, 0), u32_le(blob, 4), u32_le(blob, 8))
    else {
        println!("TARGET NETWORK blobIdx={blob_index} node={name:?} type={ty} invalid header");
        return;
    };
    println!(
        "\nTARGET NETWORK blobIdx={blob_index} node={name:?} type={ty} len={} V={vertices} S={segments} R={regions}",
        blob.len()
    );
    println!("  raw bytes: {blob:02x?}");
    let mut off = 12usize;
    for vertex_index in 0..vertices as usize {
        println!(
            "  vertex[{vertex_index}] style={} point=({:?},{:?})",
            u32_le(blob, off).unwrap_or_default(),
            f32_le(blob, off + 4),
            f32_le(blob, off + 8)
        );
        off += 12;
    }
    for segment_index in 0..segments as usize {
        println!(
            "  segment[{segment_index}] style={} start={} tangent=({:?},{:?}) end={} tangent=({:?},{:?})",
            u32_le(blob, off).unwrap_or_default(),
            u32_le(blob, off + 4).unwrap_or_default(),
            f32_le(blob, off + 8),
            f32_le(blob, off + 12),
            u32_le(blob, off + 16).unwrap_or_default(),
            f32_le(blob, off + 20),
            f32_le(blob, off + 24)
        );
        off += 28;
    }
    match parse_region_records(blob) {
        Some((region_start, parsed_end, records)) => {
            println!("  region bytes: {:02x?}", &blob[region_start..parsed_end]);
            for (region_index, region) in records.iter().enumerate() {
                println!(
                    "  region[{region_index}] raw={} styleID={} lowBit={} loops={:?}",
                    region.raw_style_and_winding,
                    region.raw_style_and_winding >> 1,
                    region.raw_style_and_winding & 1,
                    region.loops
                );
            }
            println!("  exact consumption: {}", parsed_end == blob.len());
        }
        None => println!("  region parse: failed"),
    }
    println!(
        "  current decode: {:?}",
        decode_figma_vector_path(node, blobs)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dump_smallest = args
        .windows(2)
        .find(|pair| pair[0] == "--dump-smallest")
        .and_then(|pair| pair[1].parse::<usize>().ok())
        .unwrap_or(0);
    let dump_regions = args.iter().any(|arg| arg == "--dump-regions");
    let targets: Vec<String> = args
        .windows(2)
        .filter(|pair| pair[0] == "--target")
        .map(|pair| pair[1].clone())
        .collect();
    let target_blobs: Vec<usize> = args
        .windows(2)
        .filter(|pair| pair[0] == "--blob")
        .filter_map(|pair| pair[1].parse().ok())
        .collect();
    let path = args
        .iter()
        .find(|arg| !arg.starts_with("--") && arg.parse::<usize>().is_err())
        .expect("usage: probe_vn <path> [--dump-smallest N]");
    let bytes = std::fs::read(path).expect("read");
    let decoded = parse_fig_file(&bytes).expect("parse");

    let mut layout_b_consistent = 0usize;
    let mut layout_a_consistent = 0usize;
    let mut neither = 0usize;
    let mut total = 0usize;
    let mut printed = 0usize;
    let mut failing_blobs: Vec<(usize, usize, String, Vec<u8>)> = Vec::new();
    let mut region_samples = 0usize;
    let mut correlation_nodes = 0usize;
    let mut correlation_samples = 0usize;
    let mut low_one_nonzero = 0usize;
    let mut low_one_odd = 0usize;
    let mut unpaired_regions = 0usize;
    let mut unpaired_fill_geometry = 0usize;

    for nc in &decoded.node_changes {
        let Some(vd) = nc.get("vectorData") else {
            continue;
        };
        let Some(idx) = vd.get_f64("vectorNetworkBlob") else {
            continue;
        };
        let Some(BlobOrString::Bytes(blob)) = decoded.blobs.get(idx as usize) else {
            continue;
        };
        let ty = nc.get_str("type").unwrap_or("");
        let name = nc.get_str("name").unwrap_or("");

        if (!targets.is_empty() || !target_blobs.is_empty())
            && !targets.iter().any(|target| target == name)
            && !target_blobs.contains(&(idx as usize))
        {
            continue;
        }

        if !targets.is_empty() || !target_blobs.is_empty() {
            dump_full_network(blob, idx as usize, nc, &decoded.blobs);
        }

        if let (Some((_, _, regions)), Some(fill_geometry)) =
            (parse_region_records(blob), nc.get_array("fillGeometry"))
        {
            let rules: Vec<Option<&str>> = fill_geometry
                .iter()
                .map(|geometry| geometry.get_str("windingRule"))
                .collect();
            if !regions.is_empty() && rules.iter().any(Option::is_some) {
                correlation_nodes += 1;
                let paired = regions.len().min(rules.len());
                unpaired_regions += regions.len() - paired;
                unpaired_fill_geometry += rules.len() - paired;
                for (region_index, (region, rule)) in regions.iter().zip(rules.iter()).enumerate() {
                    let Some(rule) = rule else { continue };
                    let low_bit = region.raw_style_and_winding & 1;
                    let is_nonzero = rule.eq_ignore_ascii_case("NONZERO");
                    let is_odd = rule.eq_ignore_ascii_case("ODD");
                    correlation_samples += 1;
                    low_one_nonzero += usize::from((low_bit == 1) == is_nonzero);
                    low_one_odd += usize::from((low_bit == 1) == is_odd);
                    if (low_bit == 1) != is_nonzero {
                        println!(
                            "CORRELATION MISMATCH node={:?} type={ty} blobIdx={} region={region_index} raw={} styleID={} lowBit={low_bit} windingRule={rule}",
                            nc.get_str("name").unwrap_or(""),
                            idx as usize,
                            region.raw_style_and_winding,
                            region.raw_style_and_winding >> 1,
                        );
                    }
                }
            }
        }

        if dump_regions
            && region_samples < 5
            && dump_region_records(blob, idx as usize, nc.get_str("name").unwrap_or(""))
        {
            let fill_rules: Vec<&str> = nc
                .get_array("fillGeometry")
                .unwrap_or(&[])
                .iter()
                .filter_map(|geometry| geometry.get_str("windingRule"))
                .collect();
            println!("  fillGeometry windingRule values: {fill_rules:?}");
            region_samples += 1;
        }

        // Only look at nodes the current pipeline FAILS on (no geometry decode).
        let ok = decode_figma_vector_path(nc, &decoded.blobs)
            .map(|d| !d.is_empty())
            .unwrap_or(false);
        if ok {
            continue;
        }
        total += 1;
        failing_blobs.push((
            blob.len(),
            idx as usize,
            nc.get_str("name").unwrap_or("").to_string(),
            blob.clone(),
        ));

        let len = blob.len();
        // Layout A consistency: 4 + V*8 + 4 + S*24 == len (exactly or <=)
        let a_ok = (|| {
            let v = u32_le(blob, 0)? as usize;
            if v > 100_000 {
                return None;
            }
            let seg_off = 4 + v * 8;
            let s = u32_le(blob, seg_off)? as usize;
            let end = seg_off + 4 + s * 24;
            Some(end == len)
        })()
        .unwrap_or(false);

        // Layout B consistency: 12 + V*12 + S*28 <= len
        let b = (|| {
            let v = u32_le(blob, 0)? as usize;
            let s = u32_le(blob, 4)? as usize;
            let r = u32_le(blob, 8)? as usize;
            if v > 100_000 || s > 100_000 || r > 100_000 {
                return None;
            }
            let min = 12 + v * 12 + s * 28;
            Some((v, s, r, min <= len, min == len))
        })();

        if a_ok {
            layout_a_consistent += 1;
        }
        let b_ok = matches!(b, Some((_, _, _, true, _)));
        if b_ok {
            layout_b_consistent += 1;
        }
        if !a_ok && !b_ok {
            neither += 1;
        }

        if printed < 8 {
            printed += 1;
            let name = nc.get_str("name").unwrap_or("");
            println!(
                "node {:?} ({ty}) blobIdx={} len={} vdKeys={:?}",
                name,
                idx as usize,
                len,
                keys(vd)
            );
            println!(
                "  first u32s: {:?}",
                (0..6)
                    .filter_map(|i| u32_le(blob, i * 4))
                    .collect::<Vec<_>>()
            );
            if let Some((v, s, r, fits, exact)) = b {
                println!("  layoutB: V={v} S={s} R={r} fits={fits} exact={exact}");
                if fits {
                    // Dump first 3 vertices under layout B.
                    for vi in 0..v.min(3) {
                        let o = 12 + vi * 12;
                        println!(
                            "    vtx[{vi}]: styleID={} x={:?} y={:?}",
                            u32_le(blob, o).unwrap_or(0),
                            f32_le(blob, o + 4),
                            f32_le(blob, o + 8)
                        );
                    }
                    for si in 0..s.min(3) {
                        let o = 12 + v * 12 + si * 28;
                        println!(
                            "    seg[{si}]: styleID={} start={} ts=({:?},{:?}) end={} te=({:?},{:?})",
                            u32_le(blob, o).unwrap_or(0),
                            u32_le(blob, o + 4).unwrap_or(0),
                            f32_le(blob, o + 8),
                            f32_le(blob, o + 12),
                            u32_le(blob, o + 16).unwrap_or(0),
                            f32_le(blob, o + 20),
                            f32_le(blob, o + 24)
                        );
                    }
                }
            }
            // size + normalizedSize for scale sanity
            let sz = nc.get("size");
            println!(
                "  node size=({:?},{:?}) normalizedSize=({:?},{:?})",
                sz.and_then(|s| s.get_f64("x")),
                sz.and_then(|s| s.get_f64("y")),
                vd.get("normalizedSize").and_then(|n| n.get_f64("x")),
                vd.get("normalizedSize").and_then(|n| n.get_f64("y")),
            );
        }
    }

    println!("\nfailing nodes with vectorNetworkBlob: {total}");
    println!("  layout A (current) length-consistent: {layout_a_consistent}");
    println!("  layout B (V,S,R header) length-consistent: {layout_b_consistent}");
    println!("  neither: {neither}");
    println!("\nwinding-bit correlation across all vector-network nodes:");
    println!("  qualifying nodes: {correlation_nodes}");
    println!("  correlating region/fillGeometry samples: {correlation_samples}");
    println!("  lowBit=1 <-> NONZERO matches: {low_one_nonzero}/{correlation_samples}");
    println!("  lowBit=1 <-> ODD matches: {low_one_odd}/{correlation_samples}");
    println!("  unpaired regions: {unpaired_regions}");
    println!("  unpaired fillGeometry entries: {unpaired_fill_geometry}");

    if dump_smallest > 0 {
        failing_blobs.sort_by_key(|(len, idx, _, _)| (*len, *idx));
        for (fixture_index, (len, idx, name, blob)) in
            failing_blobs.iter().take(dump_smallest).enumerate()
        {
            let fixture_name = (b'A' + fixture_index as u8) as char;
            println!("\n// blob index {idx}, {len} bytes, node {name:?}");
            println!("const REAL_VN_BLOB_{fixture_name}: &[u8] = &{blob:?};");
        }
    }
}
// (appended) count windingRule values across fill/strokeGeometry entries
