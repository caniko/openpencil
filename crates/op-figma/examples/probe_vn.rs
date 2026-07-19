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

fn dump_region_records(blob: &[u8], blob_index: usize, name: &str) -> bool {
    let (Some(vertices), Some(segments), Some(regions)) =
        (u32_le(blob, 0), u32_le(blob, 4), u32_le(blob, 8))
    else {
        return false;
    };
    if regions == 0 {
        return false;
    }
    let Some(mut off) = 12usize
        .checked_add(vertices as usize * 12)
        .and_then(|value| value.checked_add(segments as usize * 28))
    else {
        return false;
    };
    let region_start = off;
    let mut decoded = Vec::new();
    for region_index in 0..regions as usize {
        let Some(winding) = u32_le(blob, off) else {
            return false;
        };
        let Some(loop_count) = u32_le(blob, off + 4) else {
            return false;
        };
        off += 8;
        let mut loops = Vec::new();
        for _ in 0..loop_count as usize {
            let Some(index_count) = u32_le(blob, off) else {
                return false;
            };
            off += 4;
            let mut indices = Vec::with_capacity(index_count as usize);
            for _ in 0..index_count as usize {
                let Some(segment_index) = u32_le(blob, off) else {
                    return false;
                };
                if segment_index >= segments {
                    return false;
                }
                indices.push(segment_index);
                off += 4;
            }
            loops.push(indices);
        }
        decoded.push((region_index, winding, loops));
    }
    let Some(raw) = blob.get(region_start..off) else {
        return false;
    };
    println!(
        "REGION SAMPLE blobIdx={blob_index} node={name:?} V={vertices} S={segments} R={regions} regionOffset={region_start} parsedEnd={off} blobLen={}",
        blob.len()
    );
    println!("  raw region bytes: {raw:02x?}");
    for (region_index, winding, loops) in decoded {
        println!("  region[{region_index}] winding={winding} loops={loops:?}");
    }
    println!("  exact consumption: {}", off == blob.len());
    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dump_smallest = args
        .windows(2)
        .find(|pair| pair[0] == "--dump-smallest")
        .and_then(|pair| pair[1].parse::<usize>().ok())
        .unwrap_or(0);
    let dump_regions = args.iter().any(|arg| arg == "--dump-regions");
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

    for nc in &decoded.node_changes {
        let ty = nc.get_str("type").unwrap_or("");
        if !["VECTOR", "STAR", "REGULAR_POLYGON", "BOOLEAN_OPERATION"].contains(&ty) {
            continue;
        }
        let Some(vd) = nc.get("vectorData") else {
            continue;
        };
        let Some(idx) = vd.get_f64("vectorNetworkBlob") else {
            continue;
        };
        let Some(BlobOrString::Bytes(blob)) = decoded.blobs.get(idx as usize) else {
            continue;
        };

        if dump_regions
            && region_samples < 5
            && dump_region_records(
                blob,
                idx as usize,
                nc.get_str("name").unwrap_or(""),
            )
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
