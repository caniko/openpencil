#![cfg(test)]

use crate::test_support::{rect, state_with};
use crate::{NodeId, PropertyFocus};
use jian_ops_schema::node::container::CornerRadius;
use jian_ops_schema::node::path::PathFillRule;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::style::PenEffect;

#[test]
fn corner_edit_expands_uniform_radius_to_schema_order() {
    let mut node = rect("r1", "Rect", 0.0, 0.0, 100.0, 80.0);
    let PenNode::Rectangle(rect) = &mut node else {
        unreachable!()
    };
    rect.container.corner_radius = Some(CornerRadius::Uniform(8.0));
    let mut state = state_with(vec![node]);
    state.set_single_selection(NodeId::new("r1"));
    state.editor_ui.corner_expand_open = true;

    assert!(state.commit_property_edit(PropertyFocus::CornerTR, 12.0));
    let PenNode::Rectangle(rect) = &state.active_children()[0] else {
        unreachable!()
    };
    assert_eq!(
        rect.container.corner_radius,
        Some(CornerRadius::PerCorner([8.0, 12.0, 8.0, 8.0]))
    );
    assert!(state.editor_ui.corner_expand_open);

    assert!(state.commit_property_edit(PropertyFocus::CornerTR, 8.0));
    let PenNode::Rectangle(rect) = &state.active_children()[0] else {
        unreachable!()
    };
    assert_eq!(
        rect.container.corner_radius,
        Some(CornerRadius::Uniform(8.0))
    );
    assert!(!state.editor_ui.corner_expand_open);
}

#[test]
fn path_fill_rule_and_effect_mutators_are_undoable() {
    let doc = jian_ops_schema::load_str(
        r##"{"version":"1.0.0","children":[
          {"type":"path","id":"p1","d":"M0 0L10 0L10 10Z","width":10,"height":10}
        ]}"##,
    )
    .unwrap()
    .value;
    let mut state = crate::EditorState::from_document(doc);
    state.set_single_selection(NodeId::new("p1"));

    assert!(state.set_selected_fill_rule(PathFillRule::Evenodd));
    assert_eq!(state.history.past.len(), 1);
    let PenNode::Path(path) = &state.active_children()[0] else {
        unreachable!()
    };
    assert_eq!(path.fill_rule, Some(PathFillRule::Evenodd));

    assert!(state.add_background_blur_to_selected());
    assert_eq!(state.history.past.len(), 2);
    let PenEffect::BackgroundBlur(blur) =
        &crate::fills::node_effects(&state.active_children()[0])[0]
    else {
        panic!("expected background blur")
    };
    assert_eq!(blur.radius, 10.0);
    assert_ne!(blur.visible, Some(false));

    assert!(state.set_selected_effect_visible(0, false));
    assert_eq!(state.history.past.len(), 3);
    let PenEffect::BackgroundBlur(blur) =
        &crate::fills::node_effects(&state.active_children()[0])[0]
    else {
        panic!("expected background blur")
    };
    assert_eq!(blur.visible, Some(false));

    assert!(state.remove_selected_effect(0));
    assert_eq!(state.history.past.len(), 4);
    assert!(crate::fills::node_effects(&state.active_children()[0]).is_empty());
}
