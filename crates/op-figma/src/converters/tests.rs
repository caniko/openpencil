//! `convert_vector` icon-lookup tests — exercise the host-resolver
//! branch (`set_icon_lookup` → name match → emit `Path` with `icon_id`).
//! Tests serialise on `LOOKUP_GUARD` because the resolver is
//! process-global state.

use super::*;
use crate::common::{clear_icon_lookup, set_icon_lookup, IconLookupResult, IconStyle};
use crate::figma_types::BlobOrString;
use jian_ops_schema::node::PenNode;
use std::collections::HashMap;
use std::sync::Mutex;

/// Serialise tests that touch the process-global icon-lookup so they
/// don't race with each other under cargo's parallel runner.
static LOOKUP_GUARD: Mutex<()> = Mutex::new(());

fn obj(pairs: Vec<(&str, FigValue)>) -> FigValue {
    FigValue::Object(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn vector_node(name: &str) -> TreeNode {
    TreeNode {
        figma: obj(vec![
            ("type", FigValue::Str("VECTOR".into())),
            (
                "guid",
                obj(vec![
                    ("sessionID", FigValue::Uint(1)),
                    ("localID", FigValue::Uint(10)),
                ]),
            ),
            ("name", FigValue::Str(name.into())),
            (
                "size",
                obj(vec![
                    ("x", FigValue::Float(24.0)),
                    ("y", FigValue::Float(24.0)),
                ]),
            ),
        ]),
        children: vec![],
    }
}

fn fresh_ctx() -> ConversionContext {
    ConversionContext {
        component_map: HashMap::new(),
        symbol_tree: HashMap::new(),
        warnings: Vec::new(),
        id_counter: 1,
        blobs: Vec::new(),
        layout_mode: FigLayoutMode::OpenPencil,
        instance_assignments: HashMap::new(),
    }
}

fn captured_bytes(hex: &str) -> Vec<u8> {
    assert_eq!(hex.len() % 2, 0, "captured fixture must contain byte pairs");
    (0..hex.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&hex[offset..offset + 2], 16).expect("fixture hex"))
        .collect()
}

fn solid_paint(r: f32, g: f32, b: f32) -> FigValue {
    obj(vec![
        ("type", FigValue::Str("SOLID".into())),
        (
            "color",
            obj(vec![
                ("r", FigValue::Float(r)),
                ("g", FigValue::Float(g)),
                ("b", FigValue::Float(b)),
                ("a", FigValue::Float(1.0)),
            ]),
        ),
        ("visible", FigValue::Bool(true)),
    ])
}

// tesla.fig blob 1480, Vector 711 fillGeometry (82 bytes).
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

#[test]
fn smoothed_rounded_rectangle_imports_as_bounded_cubic_path() {
    let tree = TreeNode {
        figma: obj(vec![
            ("type", FigValue::Str("ROUNDED_RECTANGLE".into())),
            ("name", FigValue::Str("smooth card".into())),
            (
                "size",
                obj(vec![
                    ("x", FigValue::Float(100.0)),
                    ("y", FigValue::Float(60.0)),
                ]),
            ),
            ("cornerRadius", FigValue::Float(20.0)),
            ("cornerSmoothing", FigValue::Float(1.0)),
        ]),
        children: vec![],
    };
    let mut ctx = fresh_ctx();

    let node = convert_rectangle(&tree, None, &mut ctx);
    let PenNode::Path(path) = node else {
        panic!("expected smoothed rectangle to bake into a Path");
    };
    let d = path.d.expect("smoothed path data");
    assert!(d.contains('C'), "squircle corners must be cubic: {d}");
    assert!(
        !d.contains('A'),
        "squircle must not retain arc commands: {d}"
    );
    let bounds = compute_svg_path_bounds(&d).expect("smoothed path bounds");
    assert_eq!(
        (bounds.min_x, bounds.min_y, bounds.max_x, bounds.max_y),
        (0.0, 0.0, 100.0, 60.0)
    );
}

#[test]
fn convert_vector_uses_icon_lookup_when_set() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|name| {
        if name == "menu" {
            Some(IconLookupResult {
                d: "M3 12h18".into(),
                icon_id: Some("menu".into()),
                style: Some(IconStyle::Stroke),
            })
        } else {
            None
        }
    });

    let tree = vector_node("menu");
    let mut ctx = fresh_ctx();
    let node = convert_vector(&tree, None, &mut ctx);

    match node {
        PenNode::Path(p) => {
            assert_eq!(p.icon_id.as_deref(), Some("menu"));
            assert_eq!(p.d.as_deref(), Some("M3 12h18"));
            assert!(p.stroke.is_some());
            assert!(p.fill.is_none());
        }
        other => panic!("expected Path, got {other:?}"),
    }

    clear_icon_lookup();
}

#[test]
fn convert_vector_falls_back_when_no_match() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|_| None);

    let tree = vector_node("unmatched-name");
    let mut ctx = fresh_ctx();
    let node = convert_vector(&tree, None, &mut ctx);

    // No icon match — the synthetic fixture has no geometry, so it
    // remains present in the layer tree as an invisible path.
    assert_invisible_degenerate_path(&node, "unmatched-name");
    assert!(ctx.warnings.iter().any(|w| w.contains("empty geometry")));

    clear_icon_lookup();
}

fn assert_invisible_degenerate_path(node: &PenNode, expected_name: &str) {
    let PenNode::Path(path) = node else {
        panic!("expected Path, got {node:?}");
    };
    assert_eq!(path.base.name.as_deref(), Some(expected_name));
    assert!(path.d.as_deref().unwrap_or_default().is_empty());
    assert!(path.anchors.as_deref().unwrap_or_default().is_empty());
    assert!(path.fill.is_none());
    assert!(path.stroke.is_none());
    assert_eq!(path.width, Some(SizingBehavior::Number(24.0)));
    assert_eq!(path.height, Some(SizingBehavior::Number(24.0)));
}

#[test]
fn degenerate_commands_blob_imports_as_invisible_path() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|_| None);

    let mut tree = vector_node("four-byte-vector");
    let FigValue::Object(pairs) = &mut tree.figma else {
        unreachable!();
    };
    pairs.push((
        "fillGeometry".into(),
        FigValue::Array(vec![obj(vec![("commandsBlob", FigValue::Uint(0))])]),
    ));
    let mut ctx = fresh_ctx();
    ctx.blobs = vec![BlobOrString::Bytes(vec![0; 4])];

    let node = convert_vector(&tree, None, &mut ctx);

    assert_invisible_degenerate_path(&node, "four-byte-vector");
    assert!(ctx.warnings.iter().any(|w| w.contains("empty geometry")));
    clear_icon_lookup();
}

#[test]
fn degenerate_boolean_without_geometry_imports_as_invisible_path() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|_| None);

    let mut tree = vector_node("empty-boolean");
    let FigValue::Object(pairs) = &mut tree.figma else {
        unreachable!();
    };
    let node_type = pairs
        .iter_mut()
        .find(|(key, _)| key == "type")
        .expect("type field");
    node_type.1 = FigValue::Str("BOOLEAN_OPERATION".into());
    let mut ctx = fresh_ctx();

    let node = convert_vector(&tree, None, &mut ctx);

    assert_invisible_degenerate_path(&node, "empty-boolean");
    assert!(ctx.warnings.iter().any(|w| w.contains("empty geometry")));
    clear_icon_lookup();
}

#[test]
fn odd_fill_geometry_sets_evenodd_path_fill_rule() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|_| None);

    let mut tree = vector_node("ring");
    let FigValue::Object(pairs) = &mut tree.figma else {
        unreachable!();
    };
    pairs.push((
        "fillGeometry".into(),
        FigValue::Array(vec![obj(vec![
            ("commandsBlob", FigValue::Uint(0)),
            ("windingRule", FigValue::Str("ODD".into())),
        ])]),
    ));
    let mut blob = vec![0x01];
    blob.extend_from_slice(&0.0f32.to_le_bytes());
    blob.extend_from_slice(&0.0f32.to_le_bytes());
    blob.push(0x02);
    blob.extend_from_slice(&10.0f32.to_le_bytes());
    blob.extend_from_slice(&0.0f32.to_le_bytes());
    let mut ctx = fresh_ctx();
    ctx.blobs = vec![BlobOrString::Bytes(blob)];

    let node = convert_vector(&tree, None, &mut ctx);
    let PenNode::Path(path) = node else {
        panic!("expected Path");
    };
    assert_eq!(
        path.fill_rule,
        Some(jian_ops_schema::node::PathFillRule::Evenodd)
    );

    clear_icon_lookup();
}

#[test]
fn icon_stroke_thickness_scales_for_small_icons() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|_| {
        Some(IconLookupResult {
            d: "M3 12h18".into(),
            icon_id: Some("menu".into()),
            style: Some(IconStyle::Stroke),
        })
    });

    // 12×12 vector → icon_scale = 12/24 = 0.5 → thickness 1.5 × 0.5 = 0.75.
    let mut tree = vector_node("menu");
    if let FigValue::Object(pairs) = &mut tree.figma {
        for (k, v) in pairs.iter_mut() {
            if k == "size" {
                *v = obj(vec![
                    ("x", FigValue::Float(12.0)),
                    ("y", FigValue::Float(12.0)),
                ]);
            }
        }
    }

    let mut ctx = fresh_ctx();
    let node = convert_vector(&tree, None, &mut ctx);

    match node {
        PenNode::Path(p) => {
            let stroke = p.stroke.expect("stroke present");
            match stroke.thickness {
                jian_ops_schema::style::StrokeThickness::Uniform(t) => {
                    assert!(
                        (t - 0.75).abs() < 0.001,
                        "expected scaled thickness 0.75 got {t}"
                    );
                }
                other => panic!("expected Uniform thickness, got {other:?}"),
            }
        }
        other => panic!("expected Path, got {other:?}"),
    }

    clear_icon_lookup();
}

#[test]
fn icon_fill_style_emits_fill_no_stroke() {
    let _g = LOOKUP_GUARD.lock().unwrap();
    set_icon_lookup(|_| {
        Some(IconLookupResult {
            d: "M3 3h18v18H3z".into(),
            icon_id: Some("solid-square".into()),
            style: Some(IconStyle::Fill),
        })
    });
    let tree = {
        let mut t = vector_node("solid-square");
        if let FigValue::Object(pairs) = &mut t.figma {
            pairs.push((
                "fillPaints".into(),
                FigValue::Array(vec![obj(vec![
                    ("type", FigValue::Str("SOLID".into())),
                    (
                        "color",
                        obj(vec![
                            ("r", FigValue::Float(0.0)),
                            ("g", FigValue::Float(0.0)),
                            ("b", FigValue::Float(0.0)),
                        ]),
                    ),
                ])]),
            ));
        }
        t
    };
    let mut ctx = fresh_ctx();
    let node = convert_vector(&tree, None, &mut ctx);
    match node {
        PenNode::Path(p) => {
            assert!(p.fill.is_some(), "fill style must populate fill");
            assert!(p.stroke.is_none());
        }
        other => panic!("expected Path, got {other:?}"),
    }
    clear_icon_lookup();
}

// ── order_children: auto-layout flow order ────────────────────────

fn rect_child(name: &str, local_id: u32, position: &str) -> TreeNode {
    TreeNode {
        figma: obj(vec![
            ("type", FigValue::Str("RECTANGLE".into())),
            (
                "guid",
                obj(vec![
                    ("sessionID", FigValue::Uint(1)),
                    ("localID", FigValue::Uint(local_id)),
                ]),
            ),
            ("name", FigValue::Str(name.into())),
            (
                "parentIndex",
                obj(vec![("position", FigValue::Str(position.into()))]),
            ),
            (
                "size",
                obj(vec![
                    ("x", FigValue::Float(10.0)),
                    ("y", FigValue::Float(10.0)),
                ]),
            ),
        ]),
        children: vec![],
    }
}

fn auto_layout_frame(children: Vec<TreeNode>) -> TreeNode {
    TreeNode {
        figma: obj(vec![
            ("type", FigValue::Str("FRAME".into())),
            (
                "guid",
                obj(vec![
                    ("sessionID", FigValue::Uint(1)),
                    ("localID", FigValue::Uint(100)),
                ]),
            ),
            ("name", FigValue::Str("stack".into())),
            ("stackMode", FigValue::Str("VERTICAL".into())),
            (
                "size",
                obj(vec![
                    ("x", FigValue::Float(100.0)),
                    ("y", FigValue::Float(50.0)),
                ]),
            ),
        ]),
        // Tree-builder order: DESCENDING parentIndex.position, i.e.
        // z-order with the topmost (= last layout item) first.
        children,
    }
}

fn child_names(node: &PenNode) -> Vec<String> {
    match node {
        PenNode::Frame(f) => f
            .children
            .as_ref()
            .map(|cs| {
                cs.iter()
                    .map(|c| match c {
                        PenNode::Rectangle(r) => r.base.name.clone().unwrap_or_default(),
                        _ => String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        other => panic!("expected Frame, got {other:?}"),
    }
}

#[test]
fn openpencil_mode_orders_auto_layout_children_in_flow_order() {
    // Flow (layout) order is A then B; the tree builder hands the
    // converter [B, A] (descending z). OpenPencil mode feeds a flex
    // solver, so the emitted children MUST be flow order [A, B].
    let tree = auto_layout_frame(vec![rect_child("B", 2, "\""), rect_child("A", 1, "!")]);
    let mut ctx = fresh_ctx();
    let node = convert_frame(&tree, None, &mut ctx);
    assert_eq!(child_names(&node), vec!["A".to_string(), "B".to_string()]);
}

#[test]
fn preserve_mode_orders_auto_layout_children_in_flow_order() {
    let tree = auto_layout_frame(vec![rect_child("B", 2, "\""), rect_child("A", 1, "!")]);
    let mut ctx = fresh_ctx();
    ctx.layout_mode = FigLayoutMode::Preserve;
    let node = convert_frame(&tree, None, &mut ctx);
    assert_eq!(child_names(&node), vec!["A".to_string(), "B".to_string()]);
}
