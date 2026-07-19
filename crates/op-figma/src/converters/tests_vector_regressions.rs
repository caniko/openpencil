//! Captured-byte vector regressions from the Tesla client file —
//! expanded-stroke classification, open-network fill suppression,
//! empty-preferred fallback, and single-child boolean recovery.
//! Split from `tests.rs` to honor the 800-line cap.

use super::tests::{captured_bytes, fresh_ctx, obj, solid_paint, vector_node, LOOKUP_GUARD};
use super::*;
use crate::common::{clear_icon_lookup, set_icon_lookup};
use crate::figma_types::BlobOrString;
use jian_ops_schema::node::PenNode;

const TESLA_BRAKE_FILL_GEOMETRY: &str = concat!(
    "01000000000000000002b76ddb3f00000000023dcff33f0000000002000080400000000002",
    "000080407877773f02000080400000004002000000000000004002000000001211913f020000",
    "00000000000000"
);

// tesla.fig blob 1481, Vector 711 expanded strokeGeometry (696 bytes).
const TESLA_BRAKE_STROKE_GEOMETRY: &str = concat!(
    "0100000000000000000200000000000000c002000000c0000000c002000000c0000000000200",
    "000000000000000001000000000000004002000000c00000004002000000c000008040020000",
    "00000000804002000000000000004000010000804000000040020000804000008040020000c0",
    "4000008040020000c0400000004002000080400000004000010000804000000000020000c040",
    "00000000020000c040000000c00200008040000000c00200008040000000000001b76ddb3f00",
    "00000002b76ddb3f000000c00200000000000000c00200000000000000000200000000000000",
    "4002b76ddb3f0000004002b76ddb3f000000000001000000000000000002000000c000000000",
    "02000000c01211913f02000000001211913f02000000401211913f0200000040000000000200",
    "000000000000000001000000001211913f02000000c01211913f02000000c000000040020000",
    "00000000004002000000400000004002000000401211913f02000000001211913f0001000000",
    "0000000040020000000000008040020000804000008040020000804000000040020000804000",
    "00000002000000000000000002000000000000004000013dcff33f00000000023dcff33f0000",
    "00c002b76ddb3f000000c002b76ddb3f0000000002b76ddb3f00000040023dcff33f00000040",
    "023dcff33f0000000000010000804000000040020000c04000000040020000c0407877773f02",
    "000080407877773f02000000407877773f020000004000000040020000804000000040000100",
    "0080407877773f020000c0407877773f020000c0400000000002000080400000000002000000",
    "400000000002000000407877773f02000080407877773f000100008040000000000200008040",
    "000000c0023dcff33f000000c0023dcff33f00000000023dcff33f0000004002000080400000",
    "004002000080400000000000"
);

// tesla.fig vectorNetwork blobs 249, 302, and 304.
const TESLA_TAB_MAINTENANCE_NETWORK: &str = concat!(
    "060000000500000000000000020000000000000000000000020000000000a041",
    "00000000020000000000a04100009c41020000000000000000009c4100000000",
    "0000a04000000000000000000000704100000000000000000100000000000000",
    "0000000002000000000000000000000000000000020000000000000000000000",
    "0300000000000000000000000000000003000000000000000000000000000000",
    "0000000000000000000000000000000000000000000000000400000000000000",
    "0000000000000000050000000000000000000000010000000000000000000000"
);
const TESLA_TAB_STORE_NETWORK: &str = concat!(
    "08000000080000000100000000000000cc1cc141cc1c414101000000cc1cc141",
    "cc1cc141010000004aaded3fcc1cc141000000004aaded3fcc1c414100000000",
    "00000000cc1c4141010000004aad6d400000000001000000f841b24100000000",
    "00000000a1f7cf41cc1c41410000000000000000000000000000000001000000",
    "0000000000000000000000000100000000000000000000000200000000000000",
    "0000000000000000020000000000000000000000030000000000000000000000",
    "0000000003000000000000000000000004000000000000000000000000000000",
    "0400000000000000000000000500000000000000000000000000000005000000",
    "0000000000000000060000000000000000000000000000000600000000000000",
    "0000000007000000000000000000000000000000070000000000000000000000",
    "0000000000000000000000000000000001000000080000000000000001000000",
    "020000000300000004000000050000000600000007000000"
);
const TESLA_TAB_PROFILE_NETWORK: &str = concat!(
    "0b0000000b00000000000000070000000000000084dd884107000000f841b241",
    "84dd884107000000f841b241b724c1410700000000000000b724c14100000000",
    "568c9441000000000000000088ad6d40f3620b4100000000568c9441f8620b41",
    "0000000005423241000000000000000088ad6d403faded4005000000a58def40",
    "b6cb6e410500000034bd6c41bccb6e4101000000010000000000000000000000",
    "0200000000000000000000000100000002000000000000000000000003000000",
    "0000000000000000010000000300000000000000000000000000000000000000",
    "0000000000000000070000000000000000000000040000000000000000000000",
    "0000000004000000000000000000000006000000000000000000000000000000",
    "0500000000000000000000000800000000000000000000000000000007000000",
    "ec4383c0000000800800000000000080ec4383c00000000009000000d2750fc0",
    "e45b95bf050000000000000068eb2c4001000000000000000000000000000000",
    "09000000000000000000000000000000060000000000000078eb2c400a000000",
    "e5750f40ca5b95bf010000000a00000000000000000000000100000000000000",
    "00000000"
);

#[test]
fn tesla_brake_expanded_stroke_geometry_is_filled_not_stroked_again() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);
    let mut tree = vector_node("Vector 711");
    tree.figma.set(
        "size",
        obj(vec![
            ("x", FigValue::Float(4.0)),
            ("y", FigValue::Float(2.0)),
        ]),
    );
    tree.figma.set(
        "strokePaints",
        FigValue::Array(vec![solid_paint(0.105882354, 0.16862746, 0.3372549)]),
    );
    tree.figma.set("strokeWeight", FigValue::Float(2.0));
    tree.figma.set(
        "fillGeometry",
        FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(0))])]),
    );
    tree.figma.set(
        "strokeGeometry",
        FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(1))])]),
    );
    let mut ctx = fresh_ctx();
    ctx.blobs = vec![
        BlobOrString::Bytes(captured_bytes(TESLA_BRAKE_FILL_GEOMETRY)),
        BlobOrString::Bytes(captured_bytes(TESLA_BRAKE_STROKE_GEOMETRY)),
    ];

    let PenNode::Path(path) = convert_vector(&tree, None, &mut ctx) else {
        panic!("expected Path");
    };
    assert!(path.d.as_deref().is_some_and(|d| d.contains("L0 -2")));
    assert!(
        path.fill.is_some(),
        "expanded stroke outline must be filled"
    );
    assert!(
        path.stroke.is_none(),
        "expanded outline must not be stroked twice"
    );
    clear_icon_lookup();
}

/// A stroke-only node whose strokeGeometry key is present but EMPTY
/// must go to the vector-network fallback — NOT to fillGeometry (the
/// opposite paint's outline), which would paint the wrong shape.
#[test]
fn empty_preferred_geometry_falls_to_network_not_opposite_stream() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);
    let mut tree = vector_node("Empty Preferred");
    tree.figma.set(
        "size",
        obj(vec![
            ("x", FigValue::Float(20.0)),
            ("y", FigValue::Float(19.5)),
        ]),
    );
    tree.figma.set(
        "strokePaints",
        FigValue::Array(vec![solid_paint(0.2, 0.3, 0.4)]),
    );
    tree.figma.set("strokeWeight", FigValue::Float(2.0));
    // Preferred (stroke) geometry present but EMPTY; a decodable
    // fillGeometry exists that must NOT be chosen.
    tree.figma.set("strokeGeometry", FigValue::Array(vec![]));
    tree.figma.set(
        "fillGeometry",
        FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(1))])]),
    );
    tree.figma.set(
        "vectorData",
        obj(vec![
            ("vectorNetworkBlob", FigValue::Uint(0)),
            (
                "normalizedSize",
                obj(vec![
                    ("x", FigValue::Float(20.0)),
                    ("y", FigValue::Float(19.5)),
                ]),
            ),
        ]),
    );
    let network = captured_bytes(TESLA_TAB_MAINTENANCE_NETWORK);
    // Blob 1: a decodable fill outline (M/L/Z rect) that the selector
    // must skip in favour of the network fallback.
    let mut rect_blob = Vec::new();
    rect_blob.push(0x01);
    for v in [0f32, 0f32] {
        rect_blob.extend(v.to_le_bytes());
    }
    rect_blob.push(0x02);
    for v in [20f32, 0f32] {
        rect_blob.extend(v.to_le_bytes());
    }
    rect_blob.push(0x00);
    let mut ctx = fresh_ctx();
    ctx.blobs = vec![BlobOrString::Bytes(network), BlobOrString::Bytes(rect_blob)];

    let PenNode::Path(path) = convert_vector(&tree, None, &mut ctx) else {
        panic!("expected Path");
    };
    // The maintenance network is open (no Z); the fill rect blob would
    // have contained one — proves the network was chosen.
    assert!(path.d.as_deref().is_some_and(|d| !d.contains('Z')));
    assert!(path.stroke.is_some(), "network fallback keeps the stroke");
}

/// A stroke-only node whose strokeGeometry array is present but
/// UNDECODABLE falls back to the vector network — a CENTERLINE. It
/// must keep its stroke instead of being reclassified as an expanded
/// outline and painted as a fill.
#[test]
fn network_fallback_keeps_stroke_despite_nonempty_stroke_geometry() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);
    let mut tree = vector_node("Centerline Icon");
    tree.figma.set(
        "size",
        obj(vec![
            ("x", FigValue::Float(20.0)),
            ("y", FigValue::Float(19.5)),
        ]),
    );
    tree.figma.set(
        "strokePaints",
        FigValue::Array(vec![solid_paint(0.2, 0.3, 0.4)]),
    );
    tree.figma.set("strokeWeight", FigValue::Float(2.0));
    // Non-empty strokeGeometry whose commandsBlob is degenerate (4
    // bytes) — decode fails and the vector network takes over.
    tree.figma.set(
        "strokeGeometry",
        FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(1))])]),
    );
    tree.figma.set(
        "vectorData",
        obj(vec![
            ("vectorNetworkBlob", FigValue::Uint(0)),
            (
                "normalizedSize",
                obj(vec![
                    ("x", FigValue::Float(20.0)),
                    ("y", FigValue::Float(19.5)),
                ]),
            ),
        ]),
    );
    let network = captured_bytes(TESLA_TAB_MAINTENANCE_NETWORK);
    let mut ctx = fresh_ctx();
    ctx.blobs = vec![
        BlobOrString::Bytes(network),
        BlobOrString::Bytes(vec![0u8; 4]),
    ];

    let PenNode::Path(path) = convert_vector(&tree, None, &mut ctx) else {
        panic!("expected Path");
    };
    assert!(path.stroke.is_some(), "centerline network keeps its stroke");
    assert!(
        path.fill.is_none(),
        "centerline must not be filled as an expanded outline"
    );
}

#[test]
fn tesla_open_tab_network_does_not_implicitly_close_visible_fill() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);
    let mut tree = vector_node("Rectangle 11152");
    tree.figma.set(
        "size",
        obj(vec![
            ("x", FigValue::Float(20.0)),
            ("y", FigValue::Float(19.5)),
        ]),
    );
    tree.figma.set("cornerRadius", FigValue::Float(2.0));
    tree.figma.set(
        "fillPaints",
        FigValue::Array(vec![solid_paint(1.0, 0.0, 0.0)]),
    );
    tree.figma.set(
        "strokePaints",
        FigValue::Array(vec![solid_paint(0.29411766, 0.32941177, 0.4)]),
    );
    tree.figma.set("strokeWeight", FigValue::Float(2.0));
    tree.figma.set(
        "vectorData",
        obj(vec![
            ("vectorNetworkBlob", FigValue::Uint(0)),
            (
                "normalizedSize",
                obj(vec![
                    ("x", FigValue::Float(20.0)),
                    ("y", FigValue::Float(19.5)),
                ]),
            ),
        ]),
    );
    let network = captured_bytes(TESLA_TAB_MAINTENANCE_NETWORK);
    assert_eq!(network.len(), 224);
    let mut ctx = fresh_ctx();
    ctx.blobs = vec![BlobOrString::Bytes(network)];

    let PenNode::Path(path) = convert_vector(&tree, None, &mut ctx) else {
        panic!("expected Path");
    };
    assert!(path.d.as_deref().is_some_and(|d| !d.contains('Z')));
    assert!(
        path.fill.is_none(),
        "an open R=0 network must not close into a red box"
    );
    assert!(path.stroke.is_some());
    clear_icon_lookup();
}

#[test]
fn tesla_empty_tab_booleans_use_their_single_child_real_network() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);
    for (name, width, height, fixture, expected) in [
        (
            "store Combined Shape",
            25.995913,
            24.139061,
            TESLA_TAB_STORE_NETWORK,
            "L3.7137 0 L22.2822 0",
        ),
        (
            "profile Combined Shape",
            22.282211,
            24.142927,
            TESLA_TAB_PROFILE_NETWORK,
            "C7.0391 0 3.7137 3.3254 3.7137 7.4274",
        ),
    ] {
        let mut parent = vector_node(name);
        parent
            .figma
            .set("type", FigValue::Str("BOOLEAN_OPERATION".into()));
        parent.figma.set(
            "size",
            obj(vec![
                ("x", FigValue::Float(width)),
                ("y", FigValue::Float(height)),
            ]),
        );
        parent.figma.set(
            "fillPaints",
            FigValue::Array(vec![solid_paint(0.29411766, 0.32941177, 0.4)]),
        );
        parent.figma.set(
            "fillGeometry",
            FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(0))])]),
        );

        let mut child = vector_node("real child Path");
        child.figma.set(
            "size",
            obj(vec![
                ("x", FigValue::Float(width)),
                ("y", FigValue::Float(height)),
            ]),
        );
        child.figma.set(
            "strokePaints",
            FigValue::Array(vec![solid_paint(0.4, 0.4, 0.4)]),
        );
        child.figma.set("strokeWeight", FigValue::Float(2.0));
        child
            .figma
            .set("strokeAlign", FigValue::Str("INSIDE".into()));
        child.figma.set(
            "vectorData",
            obj(vec![
                ("vectorNetworkBlob", FigValue::Uint(1)),
                (
                    "normalizedSize",
                    obj(vec![
                        ("x", FigValue::Float(width)),
                        ("y", FigValue::Float(height)),
                    ]),
                ),
            ]),
        );
        parent.children = vec![child];

        let mut ctx = fresh_ctx();
        ctx.blobs = vec![
            BlobOrString::Bytes(Vec::new()),
            BlobOrString::Bytes(captured_bytes(fixture)),
        ];
        let PenNode::Path(path) = convert_vector(&parent, None, &mut ctx) else {
            panic!("expected Path");
        };
        assert!(
            path.d.as_deref().is_some_and(|d| d.contains(expected)),
            "{name} must use its child vector network: {:?}",
            path.d
        );
        assert!(path.fill.is_none());
        assert!(path.stroke.is_some());
    }
    clear_icon_lookup();
}
