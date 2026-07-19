//! Figma vector geometry decoder — ports `figma-vector-decoder.ts`.
//! Decodes the two Figma path blob formats — the opcode command
//! stream (`fillGeometry` / `strokeGeometry`) and the vertex/segment
//! vector-network table — into SVG path `d` strings.
//!
//! Vector-network regions follow the segment table as
//! `u32 packed_style_and_winding; u32 loop_count;` followed by each
//! loop's `u32 segment_count` and segment indices. The packed word is
//! `style_id = raw >> 1`; its low bit is 1 for NONZERO and 0 for ODD.
//! That winding interpretation was empirically confirmed against all
//! 6,698 correlating region/fillGeometry samples in `tesla.fig`
//! (6,698/6,698 matches; zero matches for the inverted mapping).

use crate::corner_geometry::rounded_polyline_path;
use crate::figma_types::BlobOrString;
use crate::kiwi::FigValue;
use jian_ops_schema::node::PathFillRule;
use std::collections::HashMap;

/// Approximate path bounding box (control points included).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathBounds {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

/// SVG geometry decoded from a Figma vector plus the fill rule that
/// must be used when painting its subpaths.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DecodedVectorPath {
    pub d: String,
    pub fill_rule: Option<PathFillRule>,
    /// Whether the decoded geometry contains a region that Figma
    /// intends to paint as a fill. Open vector-network chains have no
    /// fill region even when the node carries a fill paint.
    pub allows_fill: bool,
    /// Whether `d` actually came from the node's `strokeGeometry`
    /// stream (Figma's pre-expanded stroke outline). Only such paths
    /// may be reclassified as "fill the expansion, drop the stroke" —
    /// a vector-network fallback is a CENTERLINE and must keep its
    /// stroke even when the strokeGeometry array is non-empty but
    /// undecodable.
    pub from_stroke_geometry: bool,
}

impl std::ops::Deref for DecodedVectorPath {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.d
    }
}

impl std::fmt::Display for DecodedVectorPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.d)
    }
}

/// Format a coordinate: snap near-zero to `0`, else 4-decimal round
/// with trailing zeros stripped.
fn r(n: f64) -> String {
    if n.abs() < 5e-5 {
        return "0".to_string();
    }
    let rounded: f64 = format!("{n:.4}").parse().unwrap_or(n);
    format!("{rounded}")
}

fn f32_le(blob: &[u8], off: usize) -> Option<f64> {
    let s = blob.get(off..off + 4)?;
    Some(f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64)
}

fn u32_le(blob: &[u8], off: usize) -> Option<u32> {
    let s = blob.get(off..off + 4)?;
    Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn join_parts(parts: &[String]) -> Option<String> {
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

/// Decode the opcode command stream — `0x00`=Z, `0x01`=M, `0x02`=L,
/// `0x03`=Q (quadratic), `0x04`=C (cubic); operands are f32-LE. A
/// truncated operand buffer or unknown opcode returns the prefix
/// decoded so far.
pub fn decode_figma_path_blob(blob: &[u8]) -> Option<String> {
    if blob.len() < 9 {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    let mut off = 0usize;
    while off < blob.len() {
        let cmd = blob[off];
        off += 1;
        match cmd {
            0x00 => parts.push("Z".to_string()),
            0x01 | 0x02 => {
                let (Some(x), Some(y)) = (f32_le(blob, off), f32_le(blob, off + 4)) else {
                    return join_parts(&parts);
                };
                off += 8;
                if x.is_finite() && y.is_finite() {
                    let letter = if cmd == 0x01 { "M" } else { "L" };
                    parts.push(format!("{letter}{} {}", r(x), r(y)));
                }
            }
            0x03 => {
                let coords: Option<Vec<f64>> = (0..4).map(|i| f32_le(blob, off + i * 4)).collect();
                let Some(c) = coords else {
                    return join_parts(&parts);
                };
                off += 16;
                if c.iter().all(|v| v.is_finite()) {
                    parts.push(format!("Q{} {} {} {}", r(c[0]), r(c[1]), r(c[2]), r(c[3])));
                }
            }
            0x04 => {
                let coords: Option<Vec<f64>> = (0..6).map(|i| f32_le(blob, off + i * 4)).collect();
                let Some(c) = coords else {
                    return join_parts(&parts);
                };
                off += 24;
                if c.iter().all(|v| v.is_finite()) {
                    parts.push(format!(
                        "C{} {} {} {} {} {}",
                        r(c[0]),
                        r(c[1]),
                        r(c[2]),
                        r(c[3]),
                        r(c[4]),
                        r(c[5])
                    ));
                }
            }
            _ => return join_parts(&parts),
        }
    }
    join_parts(&parts)
}

/// Scan signed decimal numbers (`-?\d+\.?\d*`) out of a path-command
/// body. Lone `-` and exponents are not matched — `r()` never emits
/// either.
fn scan_numbers(body: &str) -> Vec<f64> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == b'-' || c.is_ascii_digit() {
            let start = i;
            if c == b'-' {
                i += 1;
            }
            let digit_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i > digit_start && i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i > digit_start {
                if let Ok(v) = body[start..i].parse::<f64>() {
                    out.push(v);
                }
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Approximate the bounding box of an SVG path `d` string using its
/// raw coordinate pairs (control points included; no extrema math).
pub fn compute_svg_path_bounds(d: &str) -> Option<PathBounds> {
    let is_cmd = |c: char| matches!(c, 'M' | 'L' | 'C' | 'Q' | 'Z' | 'm' | 'l' | 'c' | 'q' | 'z');
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    let mut letter: Option<char> = None;
    let mut body = String::new();
    let flush = |letter: Option<char>,
                 body: &str,
                 mnx: &mut f64,
                 mny: &mut f64,
                 mxx: &mut f64,
                 mxy: &mut f64| {
        let Some(l) = letter else { return };
        if l.eq_ignore_ascii_case(&'Z') {
            return;
        }
        let nums = scan_numbers(body);
        let mut i = 0;
        while i + 1 < nums.len() {
            let (x, y) = (nums[i], nums[i + 1]);
            if x.is_finite() && y.is_finite() {
                *mnx = mnx.min(x);
                *mny = mny.min(y);
                *mxx = mxx.max(x);
                *mxy = mxy.max(y);
            }
            i += 2;
        }
    };

    for c in d.chars() {
        if is_cmd(c) {
            flush(
                letter, &body, &mut min_x, &mut min_y, &mut max_x, &mut max_y,
            );
            letter = Some(c);
            body.clear();
        } else {
            body.push(c);
        }
    }
    flush(
        letter, &body, &mut min_x, &mut min_y, &mut max_x, &mut max_y,
    );

    if min_x.is_finite() {
        Some(PathBounds {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    } else {
        None
    }
}

/// Whether any paint in the array is visible (`visible != false`).
fn any_visible(paints: Option<&[FigValue]>) -> bool {
    paints
        .map(|p| p.iter().any(|x| x.get_bool("visible") != Some(false)))
        .unwrap_or(false)
}

/// Decode a Figma vector node into an SVG path string. Prefers
/// geometry blobs (expanded stroke outlines for stroke-only shapes),
/// falling back to the vector-network table.
pub fn decode_figma_vector_path(
    node: &FigValue,
    blobs: &[BlobOrString],
) -> Option<DecodedVectorPath> {
    let has_fills = any_visible(node.get_array("fillPaints"));
    let has_strokes = any_visible(node.get_array("strokePaints"));

    let (geometries, from_stroke_geometry) = if !has_fills && has_strokes {
        match node.get_array("strokeGeometry").filter(|g| !g.is_empty()) {
            Some(g) => (Some(g), true),
            None => (node.get_array("fillGeometry"), false),
        }
    } else {
        match node.get_array("fillGeometry").filter(|g| !g.is_empty()) {
            Some(g) => (Some(g), false),
            None => (node.get_array("strokeGeometry"), true),
        }
    };

    let Some(geometries) = geometries.filter(|g| !g.is_empty()) else {
        return decode_vector_network_blob(node, blobs);
    };

    let mut path_parts: Vec<String> = Vec::new();
    for geom in geometries {
        let Some(idx) = geom.get_f64("commandsBlob") else {
            continue;
        };
        if let Some(BlobOrString::Bytes(bytes)) = blobs.get(idx as usize) {
            if let Some(decoded) = decode_figma_path_blob(bytes) {
                path_parts.push(decoded);
            }
        }
    }

    if path_parts.is_empty() {
        return decode_vector_network_blob(node, blobs);
    }
    // Geometry coords are already node-local — no scaling.
    Some(DecodedVectorPath {
        d: path_parts.join(" "),
        fill_rule: fill_geometry_rule(node),
        // A geometry stream is already the paint-specific shape Figma
        // selected (fillGeometry or expanded strokeGeometry).
        allows_fill: true,
        from_stroke_geometry,
    })
}

fn fill_geometry_rule(node: &FigValue) -> Option<PathFillRule> {
    node.get_array("fillGeometry")?
        .iter()
        .filter_map(|geometry| geometry.get_str("windingRule"))
        .any(|rule| rule.eq_ignore_ascii_case("ODD"))
        .then_some(PathFillRule::Evenodd)
}

struct VnSegment {
    start: usize,
    end: usize,
    ts: (f64, f64),
    te: (f64, f64),
}

struct VnRegion {
    _style_id: u32,
    nonzero_winding: bool,
    loops: Vec<Vec<usize>>,
}

struct VnPathContext<'a> {
    segments: &'a [VnSegment],
    vertices: &'a [(f64, f64)],
    sx: f64,
    sy: f64,
    corner_radius: f64,
    corner_smoothing: f64,
}

fn parse_vn_regions(
    blob: &[u8],
    mut off: usize,
    region_count: usize,
    segment_count: usize,
) -> Option<Vec<VnRegion>> {
    let mut regions = Vec::with_capacity(region_count);
    for _ in 0..region_count {
        let raw_style_and_winding = u32_le(blob, off)?;
        let loop_count = u32_le(blob, off.checked_add(4)?)? as usize;
        off = off.checked_add(8)?;
        if loop_count > blob.len().saturating_sub(off) / 4 {
            return None;
        }

        let mut loops = Vec::with_capacity(loop_count);
        for _ in 0..loop_count {
            let index_count = u32_le(blob, off)? as usize;
            off = off.checked_add(4)?;
            let index_bytes = index_count.checked_mul(4)?;
            if index_bytes > blob.len().saturating_sub(off) {
                return None;
            }
            let mut indices = Vec::with_capacity(index_count);
            for _ in 0..index_count {
                let index = u32_le(blob, off)? as usize;
                if index >= segment_count {
                    return None;
                }
                indices.push(index);
                off = off.checked_add(4)?;
            }
            loops.push(indices);
        }
        regions.push(VnRegion {
            _style_id: raw_style_and_winding >> 1,
            nonzero_winding: raw_style_and_winding & 1 == 1,
            loops,
        });
    }
    Some(regions)
}

/// Decode the vertex/segment vector-network blob — the fallback when
/// no geometry blob is present. Coordinates are scaled by
/// `nodeSize / normalizedSize`; tangents are start/end-relative.
pub fn decode_vector_network_blob(
    node: &FigValue,
    blobs: &[BlobOrString],
) -> Option<DecodedVectorPath> {
    let vector_data = node.get("vectorData")?;
    let blob_idx = vector_data.get_f64("vectorNetworkBlob")? as usize;
    let BlobOrString::Bytes(blob) = blobs.get(blob_idx)? else {
        return None;
    };
    if blob.len() < 12 {
        return None;
    }

    let vertex_count = u32_le(blob, 0)? as usize;
    let segment_count = u32_le(blob, 4)? as usize;
    let region_count = u32_le(blob, 8)? as usize;
    if vertex_count > 100_000 || segment_count > 100_000 || region_count > 100_000 {
        return None;
    }

    let vertex_bytes = vertex_count.checked_mul(12)?;
    let segment_bytes = segment_count.checked_mul(28)?;
    let vertices_end = 12usize.checked_add(vertex_bytes)?;
    let segments_end = vertices_end.checked_add(segment_bytes)?;
    if segments_end > blob.len() {
        return None;
    }

    let mut off = 12usize;
    let mut vertices: Vec<(f64, f64)> = Vec::with_capacity(vertex_count);
    for _ in 0..vertex_count {
        let _style_id = u32_le(blob, off)?;
        let x = f32_le(blob, off + 4)?;
        let y = f32_le(blob, off + 8)?;
        off += 12;
        vertices.push((x, y));
    }

    let mut segments: Vec<VnSegment> = Vec::with_capacity(segment_count);
    for _ in 0..segment_count {
        let _style_id = u32_le(blob, off)?;
        let start = u32_le(blob, off + 4)? as usize;
        let ts = (f32_le(blob, off + 8)?, f32_le(blob, off + 12)?);
        let end = u32_le(blob, off + 16)? as usize;
        let te = (f32_le(blob, off + 20)?, f32_le(blob, off + 24)?);
        off += 28;
        if start >= vertex_count || end >= vertex_count {
            return None;
        }
        segments.push(VnSegment { start, end, ts, te });
    }
    if segments.is_empty() || vertices.is_empty() {
        return None;
    }
    let regions = parse_vn_regions(blob, segments_end, region_count, segment_count)?;

    let norm = vector_data.get("normalizedSize");
    let norm_w = norm.and_then(|n| n.get_f64("x")).unwrap_or(1.0);
    let norm_h = norm.and_then(|n| n.get_f64("y")).unwrap_or(1.0);
    let size = node.get("size");
    let node_w = size.and_then(|s| s.get_f64("x")).unwrap_or(norm_w);
    let node_h = size.and_then(|s| s.get_f64("y")).unwrap_or(norm_h);
    let sx = if norm_w > 0.001 { node_w / norm_w } else { 1.0 };
    let sy = if norm_h > 0.001 { node_h / norm_h } else { 1.0 };
    let corner_radius = node.get_f64("cornerRadius").unwrap_or(0.0).max(0.0);
    let corner_smoothing = node
        .get_f64("cornerSmoothing")
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let context = VnPathContext {
        segments: &segments,
        vertices: &vertices,
        sx,
        sy,
        corner_radius,
        corner_smoothing,
    };

    let mut parts: Vec<String> = Vec::new();
    let fill_rule = if regions.is_empty() {
        assemble_greedy_paths(&context, &mut parts)?;
        None
    } else {
        assemble_region_paths(&regions, &context, &mut parts)?;
        regions
            .iter()
            .any(|region| !region.nonzero_winding)
            .then_some(PathFillRule::Evenodd)
    };

    let result = parts.join(" ");
    if result.is_empty() {
        None
    } else {
        let allows_fill = result.contains('Z');
        Some(DecodedVectorPath {
            d: result,
            fill_rule,
            allows_fill,
            // Networks are centerlines, never expanded outlines.
            from_stroke_geometry: false,
        })
    }
}

fn assemble_greedy_paths(context: &VnPathContext<'_>, parts: &mut Vec<String>) -> Option<()> {
    let mut adj: HashMap<usize, Vec<usize>> = HashMap::new();
    for (i, seg) in context.segments.iter().enumerate() {
        adj.entry(seg.start).or_default().push(i);
    }

    let mut used = vec![false; context.segments.len()];
    for i in 0..context.segments.len() {
        if used[i] {
            continue;
        }
        let seg = &context.segments[i];
        let mut oriented = vec![(i, false)];
        used[i] = true;
        let chain_start = seg.start;
        let mut current = seg.end;
        loop {
            let Some(nexts) = adj.get(&current) else {
                break;
            };
            let Some(&next) = nexts.iter().find(|&&index| !used[index]) else {
                break;
            };
            used[next] = true;
            oriented.push((next, false));
            current = context.segments[next].end;
        }
        emit_oriented_chain(&oriented, current == chain_start, context, parts)?;
    }
    Some(())
}

fn assemble_region_paths(
    regions: &[VnRegion],
    context: &VnPathContext<'_>,
    parts: &mut Vec<String>,
) -> Option<()> {
    for region in regions {
        for segment_indices in &region.loops {
            let (&first_index, rest) = segment_indices.split_first()?;
            let first = context.segments.get(first_index)?;
            let reverse_first = if let Some(&second_index) = rest.first() {
                let second = context.segments.get(second_index)?;
                let forward_connects = first.end == second.start || first.end == second.end;
                let reverse_connects = first.start == second.start || first.start == second.end;
                if !forward_connects && !reverse_connects {
                    return None;
                }
                !forward_connects
            } else {
                false
            };
            let loop_start = if reverse_first {
                first.end
            } else {
                first.start
            };
            let mut oriented = vec![(first_index, reverse_first)];
            let mut current = if reverse_first {
                first.start
            } else {
                first.end
            };

            for &segment_index in rest {
                let segment = context.segments.get(segment_index)?;
                let reverse = if segment.start == current {
                    false
                } else if segment.end == current {
                    true
                } else {
                    return None;
                };
                oriented.push((segment_index, reverse));
                current = if reverse { segment.start } else { segment.end };
            }
            // Figma also uses region index lists for stroke-only open
            // networks (for example a single line segment). Close only
            // when the listed chain actually returns to its first vertex.
            emit_oriented_chain(&oriented, current == loop_start, context, parts)?;
        }
    }
    Some(())
}

fn emit_oriented_chain(
    oriented: &[(usize, bool)],
    closed: bool,
    context: &VnPathContext<'_>,
    parts: &mut Vec<String>,
) -> Option<()> {
    let &(first_index, reverse_first) = oriented.first()?;
    let first = context.segments.get(first_index)?;
    let first_vertex = if reverse_first {
        first.end
    } else {
        first.start
    };

    if context.corner_radius > 0.0
        && oriented
            .iter()
            .all(|&(index, _)| context.segments.get(index).is_some_and(segment_is_straight))
    {
        let mut points = Vec::with_capacity(oriented.len() + 1);
        let start = context.vertices[first_vertex];
        points.push((start.0 * context.sx, start.1 * context.sy));
        for &(index, reverse) in oriented {
            let segment = context.segments.get(index)?;
            let end_index = if reverse { segment.start } else { segment.end };
            let end = context.vertices[end_index];
            points.push((end.0 * context.sx, end.1 * context.sy));
        }
        if closed {
            points.pop();
        }
        if let Some(path) = rounded_polyline_path(
            &points,
            closed,
            context.corner_radius,
            context.corner_smoothing,
        ) {
            parts.push(path);
            return Some(());
        }
    }

    let sv = context.vertices[first_vertex];
    parts.push(format!(
        "M{} {}",
        r(sv.0 * context.sx),
        r(sv.1 * context.sy)
    ));
    for &(index, reverse) in oriented {
        emit_oriented_segment(
            context.segments.get(index)?,
            reverse,
            context.vertices,
            context.sx,
            context.sy,
            parts,
        );
    }
    if closed {
        parts.push("Z".to_string());
    }
    Some(())
}

fn segment_is_straight(seg: &VnSegment) -> bool {
    seg.ts.0.abs() < 1e-4 && seg.ts.1.abs() < 1e-4 && seg.te.0.abs() < 1e-4 && seg.te.1.abs() < 1e-4
}

fn emit_oriented_segment(
    seg: &VnSegment,
    reverse: bool,
    vertices: &[(f64, f64)],
    sx: f64,
    sy: f64,
    parts: &mut Vec<String>,
) {
    if reverse {
        let reversed = VnSegment {
            start: seg.end,
            end: seg.start,
            ts: seg.te,
            te: seg.ts,
        };
        emit_oriented_segment(&reversed, false, vertices, sx, sy, parts);
        return;
    }
    let sv = vertices[seg.start];
    let ev = vertices[seg.end];
    if segment_is_straight(seg) {
        parts.push(format!("L{} {}", r(ev.0 * sx), r(ev.1 * sy)));
    } else {
        let cp1x = (sv.0 + seg.ts.0) * sx;
        let cp1y = (sv.1 + seg.ts.1) * sy;
        let cp2x = (ev.0 + seg.te.0) * sx;
        let cp2y = (ev.1 + seg.te.1) * sy;
        parts.push(format!(
            "C{} {} {} {} {} {}",
            r(cp1x),
            r(cp1y),
            r(cp2x),
            r(cp2y),
            r(ev.0 * sx),
            r(ev.1 * sy)
        ));
    }
}

#[cfg(test)]
#[path = "vector_decoder/tests.rs"]
mod tests;
