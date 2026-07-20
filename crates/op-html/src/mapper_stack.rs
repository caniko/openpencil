use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::PenNode;

pub(super) const Z_INDEX_HINT: &str = "__op_html_z_index:";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PaintKey {
    Negative(i32),
    Normal,
    Zero,
    Positive(i32),
}

enum PaintLayer {
    Node(Box<PenNode>),
    FlowBand(Vec<PenNode>),
}

pub(super) fn layer_absolute_children(children: Vec<PenNode>) -> Vec<PenNode> {
    let mut normal = Vec::new();
    let mut flow_key: Option<PaintKey> = None;
    let mut flow_key_is_uniform = true;
    let mut flow_source_order = 0;
    let mut layers = Vec::new();
    for (source_order, mut node) in children.into_iter().enumerate() {
        let positioned = has_explicit_position(&node);
        let z_index = take_z_index(&mut node);
        // Only out-of-flow children may move around the ordinary child list.
        // Reordering an in-flow z-index item also reorders Jian auto-layout,
        // changing geometry even though CSS z-index only changes paint order.
        if positioned {
            layers.push((
                paint_key(Some(z_index.unwrap_or(0))),
                source_order,
                PaintLayer::Node(Box::new(node)),
            ));
        } else {
            let key = paint_key(z_index);
            if flow_key.is_some_and(|current| current != key) {
                flow_key_is_uniform = false;
            } else if flow_key.is_none() {
                flow_key = Some(key);
            }
            flow_source_order = source_order;
            normal.push(node);
        }
    }
    // Jian uses child order for both auto-layout and paint order. Keep all
    // in-flow children in one source-ordered block so z-index never changes
    // their geometry. A shared paint key can place a uniform band among the
    // absolute layers; splitting it would reorder auto-layout siblings. A
    // heterogeneous band must stay at the normal-flow level: promoting the
    // whole band because one relative child has a positive z-index would also
    // promote every unrelated ordinary sibling. When every item has the same
    // stacking key (or there is only one item), applying that key is safe.
    if let Some(key) = flow_key {
        let key = if flow_key_is_uniform {
            key
        } else {
            PaintKey::Normal
        };
        layers.push((key, flow_source_order, PaintLayer::FlowBand(normal)));
    }
    // Jian paints the first child on top. CSS paints later positioned
    // siblings on top at the same stacking level. The key order is positive,
    // zero/auto, normal flow, then negative.
    layers.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
    let mut ordered = Vec::new();
    for (_, _, layer) in layers {
        match layer {
            PaintLayer::Node(node) => ordered.push(*node),
            PaintLayer::FlowBand(nodes) => ordered.extend(nodes),
        }
    }
    ordered
}

fn paint_key(z_index: Option<i32>) -> PaintKey {
    match z_index {
        Some(z_index) if z_index > 0 => PaintKey::Positive(z_index),
        Some(0) => PaintKey::Zero,
        Some(z_index) => PaintKey::Negative(z_index),
        None => PaintKey::Normal,
    }
}

fn take_z_index(node: &mut PenNode) -> Option<i32> {
    let base = node_base_mut(node);
    let hint = base.explain.take()?;
    if let Some(value) = hint
        .strip_prefix(Z_INDEX_HINT)
        .and_then(|value| value.parse().ok())
    {
        Some(value)
    } else {
        base.explain = Some(hint);
        None
    }
}

fn node_base_mut(node: &mut PenNode) -> &mut PenNodeBase {
    match node {
        PenNode::Frame(node) => &mut node.base,
        PenNode::Group(node) => &mut node.base,
        PenNode::Rectangle(node) => &mut node.base,
        PenNode::Ellipse(node) => &mut node.base,
        PenNode::Line(node) => &mut node.base,
        PenNode::Polygon(node) => &mut node.base,
        PenNode::Path(node) => &mut node.base,
        PenNode::Text(node) => &mut node.base,
        PenNode::TextInput(node) => &mut node.base,
        PenNode::Image(node) => &mut node.base,
        PenNode::IconFont(node) => &mut node.base,
        PenNode::TextArea(node) => &mut node.base,
        PenNode::Select(node) => &mut node.base,
        PenNode::Switch(node) => &mut node.base,
        PenNode::Checkbox(node) => &mut node.base,
        PenNode::Slider(node) => &mut node.base,
        PenNode::RadioGroup(node) => &mut node.base,
        PenNode::NumberInput(node) => &mut node.base,
        PenNode::Progress(node) => &mut node.base,
        PenNode::Tabs(node) => &mut node.base,
        PenNode::Ref(node) => &mut node.base,
    }
}

fn has_explicit_position(node: &PenNode) -> bool {
    let base = match node {
        PenNode::Frame(node) => &node.base,
        PenNode::Group(node) => &node.base,
        PenNode::Rectangle(node) => &node.base,
        PenNode::Ellipse(node) => &node.base,
        PenNode::Line(node) => &node.base,
        PenNode::Polygon(node) => &node.base,
        PenNode::Path(node) => &node.base,
        PenNode::Text(node) => &node.base,
        PenNode::TextInput(node) => &node.base,
        PenNode::Image(node) => &node.base,
        PenNode::IconFont(node) => &node.base,
        PenNode::TextArea(node) => &node.base,
        PenNode::Select(node) => &node.base,
        PenNode::Switch(node) => &node.base,
        PenNode::Checkbox(node) => &node.base,
        PenNode::Slider(node) => &node.base,
        PenNode::RadioGroup(node) => &node.base,
        PenNode::NumberInput(node) => &node.base,
        PenNode::Progress(node) => &node.base,
        PenNode::Tabs(node) => &node.base,
        PenNode::Ref(node) => &node.base,
    };
    base.x.is_some() || base.y.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: &str, positioned: bool, z_index: Option<i32>) -> PenNode {
        let mut node: PenNode =
            serde_json::from_str(&format!(r#"{{"type":"frame","id":"{id}","children":[]}}"#))
                .unwrap();
        let base = node_base_mut(&mut node);
        base.x = positioned.then_some(0.0);
        base.explain = z_index.map(|z| format!("{Z_INDEX_HINT}{z}"));
        node
    }

    fn id(node: &PenNode) -> &str {
        match node {
            PenNode::Frame(node) => &node.base.id,
            _ => unreachable!(),
        }
    }

    #[test]
    fn absolute_layers_surround_normal_flow_without_reordering_flow_items() {
        let children = vec![
            frame("flow", false, None),
            frame("negative", true, Some(-1)),
            frame("auto", true, None),
            frame("positive", true, Some(10)),
            frame("relative", false, Some(1)),
        ];
        let layered = layer_absolute_children(children);
        assert_eq!(
            layered.iter().map(id).collect::<Vec<_>>(),
            ["positive", "auto", "flow", "relative", "negative"]
        );
        assert!(layered.iter().all(|node| match node {
            PenNode::Frame(node) => node.base.explain.is_none(),
            _ => false,
        }));
    }

    #[test]
    fn mixed_zero_z_flow_band_stays_at_the_normal_flow_level() {
        let children = vec![
            frame("absolute-before", true, None),
            frame("flow", false, None),
            frame("relative-zero", false, Some(0)),
            frame("absolute-after", true, None),
        ];
        let layered = layer_absolute_children(children);
        assert_eq!(
            layered.iter().map(id).collect::<Vec<_>>(),
            ["absolute-after", "absolute-before", "flow", "relative-zero"]
        );
    }

    #[test]
    fn mixed_flow_band_does_not_promote_ordinary_siblings_with_relative_z_index() {
        let children = vec![
            frame("absolute-before", true, Some(2)),
            frame("flow", false, None),
            frame("relative", false, Some(2)),
            frame("absolute-after", true, Some(2)),
        ];
        let layered = layer_absolute_children(children);
        assert_eq!(
            layered.iter().map(id).collect::<Vec<_>>(),
            ["absolute-after", "absolute-before", "flow", "relative"]
        );
    }

    #[test]
    fn uniform_flow_band_can_use_its_shared_positive_z_index() {
        let children = vec![
            frame("absolute", true, Some(1)),
            frame("relative-a", false, Some(2)),
            frame("relative-b", false, Some(2)),
        ];
        let layered = layer_absolute_children(children);
        assert_eq!(
            layered.iter().map(id).collect::<Vec<_>>(),
            ["relative-a", "relative-b", "absolute"]
        );
    }
}
