use super::*;
use crate::common::{clear_icon_lookup, set_icon_lookup};
use crate::converters::tests::{fresh_ctx, solid_paint, vector_node, LOOKUP_GUARD};
use jian_ops_schema::style::{PenFill, StrokeThickness};

fn rectangle_child(name: &str, fill: FigValue) -> TreeNode {
    let mut child = vector_node(name);
    child.figma.set("type", FigValue::Str("RECTANGLE".into()));
    child.figma.set("fillPaints", FigValue::Array(vec![fill]));
    child
}

fn group_child(name: &str) -> TreeNode {
    let mut child = vector_node(name);
    child.figma.set("type", FigValue::Str("GROUP".into()));
    child
}

fn empty_boolean(operation: &str) -> TreeNode {
    let mut parent = vector_node("empty boolean result");
    parent
        .figma
        .set("type", FigValue::Str("BOOLEAN_OPERATION".into()));
    parent
        .figma
        .set("booleanOperation", FigValue::Str(operation.into()));
    parent.figma.set(
        "fillPaints",
        FigValue::Array(vec![solid_paint(1.0, 1.0, 1.0)]),
    );
    parent.figma.set(
        "strokePaints",
        FigValue::Array(vec![solid_paint(1.0, 0.0, 0.0)]),
    );
    parent.figma.set("strokeWeight", FigValue::Float(3.0));
    parent.children = vec![
        rectangle_child("first operand", solid_paint(0.0, 0.0, 0.0)),
        rectangle_child("second operand", solid_paint(0.0, 1.0, 0.0)),
        group_child("container operand"),
    ];
    parent
}

fn assert_parent_paint(node: &PenNode) {
    let (fill, stroke) = match node {
        PenNode::Rectangle(node) => (&node.container.fill, &node.container.stroke),
        PenNode::Group(node) => (&node.container.fill, &node.container.stroke),
        _ => panic!("expected paintable fallback operand, got {node:?}"),
    };
    let Some(PenFill::Solid(fill)) = fill.as_ref().and_then(|fills| fills.first()) else {
        panic!("expected inherited solid fill");
    };
    assert_eq!(fill.color, "#ffffff");
    let stroke = stroke.as_ref().expect("inherited stroke");
    assert_eq!(stroke.thickness, StrokeThickness::Uniform(3.0));
    let Some(PenFill::Solid(stroke_fill)) = stroke.fill.as_ref().and_then(|fills| fills.first())
    else {
        panic!("expected inherited solid stroke paint");
    };
    assert_eq!(stroke_fill.color, "#ff0000");
}

#[test]
fn empty_union_boolean_falls_back_to_all_children_with_result_paint() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);
    let mut ctx = fresh_ctx();

    let node = convert_vector(&empty_boolean("UNION"), None, &mut ctx);

    let PenNode::Group(group) = node else {
        panic!("expected boolean fallback Group, got {node:?}");
    };
    let children = group.children.expect("converted union operands");
    assert_eq!(children.len(), 3);
    children.iter().for_each(assert_parent_paint);
    assert!(ctx.warnings.is_empty(), "warnings={:?}", ctx.warnings);
    clear_icon_lookup();
}

#[test]
fn non_union_empty_booleans_keep_only_base_child_and_warn() {
    let _guard = LOOKUP_GUARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    set_icon_lookup(|_| None);

    for operation in ["SUBTRACT", "INTERSECT"] {
        let mut ctx = fresh_ctx();
        let node = convert_vector(&empty_boolean(operation), None, &mut ctx);
        let PenNode::Group(group) = node else {
            panic!("expected {operation} fallback Group, got {node:?}");
        };
        let children = group.children.expect("converted base operand");
        assert_eq!(children.len(), 1, "operation={operation}");
        assert_parent_paint(&children[0]);
        assert!(
            ctx.warnings
                .iter()
                .any(|warning| warning.contains("empty geometry")),
            "operation={operation}, warnings={:?}",
            ctx.warnings
        );
        assert_eq!(ctx.warnings.len(), 1, "operation={operation}");
    }
    clear_icon_lookup();
}
