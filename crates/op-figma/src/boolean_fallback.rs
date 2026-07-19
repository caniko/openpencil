//! Visible approximations for boolean nodes whose result geometry is absent.

use crate::common::{common_props, resolve_height, resolve_width, ConversionContext};
use crate::converters::convert_children;
use crate::kiwi::FigValue;
use crate::mappers::{map_figma_effects, map_figma_fills, map_figma_stroke};
use crate::node_build::group_node;
use crate::tree::TreeNode;
use jian_ops_schema::node::container::ContainerProps;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::style::{PenFill, PenStroke};

const STROKE_STYLE_KEYS: [&str; 10] = [
    "strokeWeight",
    "borderStrokeWeightsIndependent",
    "borderTopWeight",
    "borderRightWeight",
    "borderBottomWeight",
    "borderLeftWeight",
    "strokeAlign",
    "strokeJoin",
    "strokeCap",
    "dashPattern",
];

/// Approximate an empty boolean result with its painted operands.
///
/// UNION can be represented reasonably by painting every operand. The exact
/// geometry of SUBTRACT, INTERSECT, and other operations is unavailable, so
/// they retain only the first operand as a visible base and emit a warning.
/// This is deliberately approximate, but it is strictly more useful than the
/// previous invisible-path fallback.
pub fn convert_empty_boolean_group(
    tree: &TreeNode,
    parent_stack_mode: Option<&str>,
    id: String,
    ctx: &mut ConversionContext,
) -> Option<PenNode> {
    if tree.figma.get_str("type") != Some("BOOLEAN_OPERATION") || tree.children.is_empty() {
        return None;
    }

    let operation = tree.figma.get_str("booleanOperation").unwrap_or("UNKNOWN");
    let mut synthetic = tree.clone();
    if operation != "UNION" {
        synthetic.children.truncate(1);
    }
    for child in &mut synthetic.children {
        inherit_result_paint(&tree.figma, &mut child.figma);
    }

    let mut children = convert_children(&synthetic, ctx);
    if children.is_empty() {
        return None;
    }
    let fill = map_figma_fills(tree.figma.get_array("fillPaints"));
    let stroke = map_figma_stroke(&tree.figma);
    for child in &mut children {
        apply_result_paint(child, &fill, &stroke);
    }
    if operation != "UNION" {
        ctx.warnings.push(format!(
            "Boolean node \"{}\" approximated from first child ({operation}; empty geometry)",
            tree.figma.get_str("name").unwrap_or("")
        ));
    }

    let container = ContainerProps {
        width: Some(resolve_width(&tree.figma, parent_stack_mode, ctx)),
        height: Some(resolve_height(&tree.figma, parent_stack_mode, ctx)),
        effects: map_figma_effects(tree.figma.get_array("effects")),
        ..ContainerProps::default()
    };
    Some(group_node(
        common_props(&tree.figma, id),
        container,
        Some(children),
    ))
}

fn inherit_result_paint(parent: &FigValue, child: &mut FigValue) {
    for key in ["fillPaints", "strokePaints"] {
        let paints = parent.get_array(key).unwrap_or_default().to_vec();
        child.set(key, FigValue::Array(paints));
    }
    child.set("backgroundPaints", FigValue::Array(Vec::new()));
    for key in STROKE_STYLE_KEYS {
        if let Some(value) = parent.get(key) {
            child.set(key, value.clone());
        }
    }
}

fn apply_result_paint(node: &mut PenNode, fill: &Option<Vec<PenFill>>, stroke: &Option<PenStroke>) {
    match node {
        PenNode::Frame(node) => {
            node.container.fill = fill.clone();
            node.container.stroke = stroke.clone();
        }
        PenNode::Group(node) => {
            node.container.fill = fill.clone();
            node.container.stroke = stroke.clone();
        }
        PenNode::Rectangle(node) => {
            node.container.fill = fill.clone();
            node.container.stroke = stroke.clone();
        }
        PenNode::Ellipse(node) => {
            node.fill = fill.clone();
            node.stroke = stroke.clone();
        }
        PenNode::Polygon(node) => {
            node.fill = fill.clone();
            node.stroke = stroke.clone();
        }
        PenNode::Path(node) => {
            node.fill = fill.clone();
            node.stroke = stroke.clone();
        }
        PenNode::Text(node) => node.fill = fill.clone(),
        PenNode::Line(node) => node.stroke = stroke.clone(),
        _ => {}
    }
}
