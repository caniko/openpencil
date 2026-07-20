//! Regression tests for direct `U()` operation command selection.

use op_editor_core::{EditorCommand, NodeId};

use super::batch_direct_ops::parse_single_direct_operation;
use super::test_fixtures::sample;

#[test]
fn direct_update_routes_sizing_keywords_to_patch_and_applies() {
    let mut state = sample();
    let command = parse_single_direct_operation(
        r##"U("n10", {"width":"fill_container","height":"fit_content","fill_hex":"#112233"})"##,
    )
    .expect("valid direct update")
    .expect("update command");

    let EditorCommand::PatchNodeData {
        node_id,
        patch_json,
        page_id,
    } = &command
    else {
        panic!("sizing keywords must use PatchNodeData, got {command:?}");
    };
    assert_eq!(node_id.as_str(), "n10");
    assert_eq!(page_id, &None);
    let patch: serde_json::Value = serde_json::from_str(patch_json).expect("patch json");
    assert_eq!(patch["width"], "fill_container");
    assert_eq!(patch["height"], "fit_content");
    assert_eq!(patch["fill"][0]["color"], "#112233");
    assert!(patch.get("fill_hex").is_none());

    assert!(state.apply(command), "keyword patch must apply");
    let node = op_editor_core::walkers::find_node(state.active_children(), &NodeId::new("n10"))
        .expect("updated frame");
    let value = serde_json::to_value(node).expect("updated node json");
    assert_eq!(value["width"], "fill_container");
    assert_eq!(value["height"], "fit_content");
    assert_eq!(value["fill"][0]["color"], "#112233");
}

#[test]
fn direct_update_keeps_numeric_sizes_on_flat_command() {
    let command = parse_single_direct_operation(r#"U("n10", {"width":320,"height":480})"#)
        .expect("valid direct update")
        .expect("update command");

    match command {
        EditorCommand::UpdateNode { width, height, .. } => {
            assert_eq!(width, Some(320));
            assert_eq!(height, Some(480));
        }
        other => panic!("numeric sizes must stay on UpdateNode, got {other:?}"),
    }
}
