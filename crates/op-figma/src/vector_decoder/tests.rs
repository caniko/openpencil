//! Vector-decoder tests.

use super::*;

// Captured from two real vector-network nodes in the client fixture.
const REAL_VN_BLOB_A: &[u8] = &[
    2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0,
    65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0,
];
const REAL_VN_BLOB_B: &[u8] = &[
    2, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 143, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0,
];

fn obj(pairs: Vec<(&str, FigValue)>) -> FigValue {
    FigValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn push_f32(buf: &mut Vec<u8>, v: f32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

#[test]
fn decodes_command_blob() {
    let mut blob = Vec::new();
    blob.push(0x01); // M
    push_f32(&mut blob, 1.0);
    push_f32(&mut blob, 2.0);
    blob.push(0x02); // L
    push_f32(&mut blob, 3.0);
    push_f32(&mut blob, 4.0);
    blob.push(0x00); // Z
    assert_eq!(
        decode_figma_path_blob(&blob).as_deref(),
        Some("M1 2 L3 4 Z")
    );
}

#[test]
fn decodes_cubic_command() {
    let mut blob = Vec::new();
    blob.push(0x01);
    push_f32(&mut blob, 0.0);
    push_f32(&mut blob, 0.0);
    blob.push(0x04); // C
    for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
        push_f32(&mut blob, v);
    }
    assert_eq!(
        decode_figma_path_blob(&blob).as_deref(),
        Some("M0 0 C1 2 3 4 5 6")
    );
}

#[test]
fn short_blob_is_none() {
    assert!(decode_figma_path_blob(&[0x01, 0, 0]).is_none());
}

#[test]
fn path_bounds_from_coordinates() {
    let b = compute_svg_path_bounds("M1 2 L3 4 L-5 10 Z").expect("bounds");
    assert_eq!(b.min_x, -5.0);
    assert_eq!(b.min_y, 2.0);
    assert_eq!(b.max_x, 3.0);
    assert_eq!(b.max_y, 10.0);
}

#[test]
fn path_bounds_empty_string_none() {
    assert!(compute_svg_path_bounds("").is_none());
}

#[test]
fn vector_path_from_fill_geometry() {
    let mut blob = Vec::new();
    blob.push(0x01);
    push_f32(&mut blob, 0.0);
    push_f32(&mut blob, 0.0);
    blob.push(0x02);
    push_f32(&mut blob, 8.0);
    push_f32(&mut blob, 0.0);
    let node = obj(vec![
        (
            "fillPaints",
            FigValue::Array(vec![obj(vec![("type", FigValue::Str("SOLID".into()))])]),
        ),
        (
            "fillGeometry",
            FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(0))])]),
        ),
    ]);
    let blobs = [BlobOrString::Bytes(blob)];
    assert_eq!(
        decode_figma_vector_path(&node, &blobs)
            .as_ref()
            .map(|decoded| decoded.d.as_str()),
        Some("M0 0 L8 0")
    );
}

#[test]
fn fill_geometry_winding_rule_is_exposed_on_decode_result() {
    let mut blob = Vec::new();
    blob.push(0x01);
    push_f32(&mut blob, 0.0);
    push_f32(&mut blob, 0.0);
    blob.push(0x02);
    push_f32(&mut blob, 8.0);
    push_f32(&mut blob, 0.0);
    let geometry = |winding_rule: Option<&str>| {
        let mut fields = vec![("commandsBlob", FigValue::Uint(0))];
        if let Some(rule) = winding_rule {
            fields.push(("windingRule", FigValue::Str(rule.into())));
        }
        obj(fields)
    };

    for (winding_rule, expected) in [
        (
            Some("ODD"),
            Some(jian_ops_schema::node::PathFillRule::Evenodd),
        ),
        (Some("NONZERO"), None),
        (None, None),
    ] {
        let node = obj(vec![
            (
                "fillPaints",
                FigValue::Array(vec![obj(vec![("type", FigValue::Str("SOLID".into()))])]),
            ),
            (
                "fillGeometry",
                FigValue::Array(vec![geometry(winding_rule)]),
            ),
        ]);
        let decoded = decode_figma_vector_path(&node, &[BlobOrString::Bytes(blob.clone())])
            .expect("geometry decodes");
        assert_eq!(decoded.fill_rule, expected);
    }
}

fn vector_network_node(blob: &[u8]) -> (FigValue, Vec<BlobOrString>) {
    let node = obj(vec![(
        "vectorData",
        obj(vec![("vectorNetworkBlob", FigValue::Uint(0))]),
    )]);
    (node, vec![BlobOrString::Bytes(blob.to_vec())])
}

fn push_straight_segment(blob: &mut Vec<u8>, start: u32, end: u32) {
    push_u32(blob, 0); // segment style
    push_u32(blob, start);
    push_f32(blob, 0.0);
    push_f32(blob, 0.0); // start tangent
    push_u32(blob, end);
    push_f32(blob, 0.0);
    push_f32(blob, 0.0); // end tangent
}

fn push_region(blob: &mut Vec<u8>, raw_style_and_winding: u32, loops: &[&[u32]]) {
    push_u32(blob, raw_style_and_winding);
    push_u32(blob, loops.len() as u32);
    for indices in loops {
        push_u32(blob, indices.len() as u32);
        for &index in *indices {
            push_u32(blob, index);
        }
    }
}

#[test]
fn vn_layout_header_and_strides_decode() {
    let mut blob = Vec::new();
    push_u32(&mut blob, 2); // vertexCount
    push_u32(&mut blob, 1); // segmentCount
    push_u32(&mut blob, 0); // regionCount
    for (x, y) in [(0.0, 0.0), (10.0, 0.0)] {
        push_u32(&mut blob, 0); // vertex style
        push_f32(&mut blob, x);
        push_f32(&mut blob, y);
    }
    push_u32(&mut blob, 0); // segment style
    push_u32(&mut blob, 0); // start
    push_f32(&mut blob, 0.0);
    push_f32(&mut blob, 0.0); // start tangent
    push_u32(&mut blob, 1); // end
    push_f32(&mut blob, 0.0);
    push_f32(&mut blob, 0.0); // end tangent

    let (node, blobs) = vector_network_node(&blob);
    let path = decode_vector_network_blob(&node, &blobs).expect("decodes");
    assert!(path.starts_with('M'), "emits a moveto: {path}");
    assert!(
        path.contains('L') || path.contains('C'),
        "emits the segment: {path}"
    );
    assert_eq!(path.d, "M0 0 L10 0");
}

#[test]
fn real_captured_blobs_decode() {
    for blob in [REAL_VN_BLOB_A, REAL_VN_BLOB_B] {
        let (node, blobs) = vector_network_node(blob);
        assert!(decode_vector_network_blob(&node, &blobs).is_some());
    }
}

#[test]
fn vn_regions_assemble_closed_subpaths_in_region_order_and_expose_winding() {
    let mut blob = Vec::new();
    push_u32(&mut blob, 8); // vertexCount
    push_u32(&mut blob, 8); // segmentCount
    push_u32(&mut blob, 2); // regionCount
    for (x, y) in [
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 10.0),
        (0.0, 10.0),
        (20.0, 0.0),
        (30.0, 0.0),
        (30.0, 10.0),
        (20.0, 10.0),
    ] {
        push_u32(&mut blob, 0); // vertex style
        push_f32(&mut blob, x);
        push_f32(&mut blob, y);
    }
    for (start, end) in [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
    ] {
        push_straight_segment(&mut blob, start, end);
    }
    // Packed record: style_id = raw >> 1; low bit 1 = NONZERO, 0 = ODD.
    push_region(&mut blob, 5, &[&[4, 5, 6, 7]]); // style 2, NONZERO
    push_region(&mut blob, 2, &[&[0, 1, 2, 3]]); // style 1, ODD

    let (node, blobs) = vector_network_node(&blob);
    let decoded = decode_vector_network_blob(&node, &blobs).expect("regions decode");
    assert_eq!(
        decoded.d,
        "M20 0 L30 0 L30 10 L20 10 L20 0 Z M0 0 L10 0 L10 10 L0 10 L0 0 Z"
    );
    assert_eq!(decoded.fill_rule, Some(PathFillRule::Evenodd));
}

#[test]
fn vn_corner_radius_rounds_straight_region_vertices() {
    let mut blob = Vec::new();
    push_u32(&mut blob, 4);
    push_u32(&mut blob, 4);
    push_u32(&mut blob, 1);
    for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
        push_u32(&mut blob, 0);
        push_f32(&mut blob, x);
        push_f32(&mut blob, y);
    }
    for (start, end) in [(0, 1), (1, 2), (2, 3), (3, 0)] {
        push_straight_segment(&mut blob, start, end);
    }
    push_region(&mut blob, 1, &[&[0, 1, 2, 3]]);
    let (mut node, blobs) = vector_network_node(&blob);
    node.set("cornerRadius", FigValue::Float(2.0));

    let decoded = decode_vector_network_blob(&node, &blobs).expect("rounded network decodes");
    assert_eq!(
        decoded.d,
        "M2 0 L8 0 Q10 0 10 2 L10 8 Q10 10 8 10 L2 10 Q0 10 0 8 L0 2 Q0 0 2 0 Z"
    );
}

#[test]
fn r_strips_trailing_zeros() {
    assert_eq!(r(1.5), "1.5");
    assert_eq!(r(2.0), "2");
    assert_eq!(r(0.00001), "0");
    assert_eq!(r(-0.0), "0");
}
