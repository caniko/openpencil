//! TS-style `update_node(data)` parity tests.

use super::test_fixtures::sample;
use super::write_tools::update_node_snapshot;
use super::{EditorCommand, McpTool, ToolOutcome};
use op_editor_core::NodeId;
use serde_json::Value;
use std::collections::BTreeMap;

#[test]
fn update_node_data_with_ts_text_fields_emits_patch_command() {
    let mut args = BTreeMap::new();
    args.insert("nodeId".into(), "n11".into());
    args.insert(
        "data".into(),
        r##"{"content":"Updated","fontSize":24}"##.into(),
    );
    args.insert("pageId".into(), "page-2".into());

    let ToolOutcome::OkWithCommand(
        _,
        EditorCommand::PatchNodeData {
            node_id,
            patch_json,
            page_id,
        },
    ) = update_node_snapshot().call(&args)
    else {
        panic!("expected PatchNodeData command for TS text patch");
    };
    let patch: Value = serde_json::from_str(&patch_json).expect("patch json");
    assert_eq!(node_id.as_str(), "n11");
    assert_eq!(patch["content"], "Updated");
    assert_eq!(patch["fontSize"], 24);
    assert_eq!(page_id.as_deref(), Some("page-2"));
}

#[test]
fn update_node_data_with_sizing_keyword_emits_patch_command_and_applies() {
    let mut state = sample();
    let mut args = BTreeMap::new();
    args.insert("nodeId".into(), "n10".into());
    args.insert("data".into(), r##"{"height":"fit_content"}"##.into());

    let ToolOutcome::OkWithCommand(_, command @ EditorCommand::PatchNodeData { .. }) =
        update_node_snapshot().call(&args)
    else {
        panic!("expected PatchNodeData command for sizing keyword");
    };
    assert!(state.apply(command), "sizing keyword patch must apply");
    let node = op_editor_core::walkers::find_node(state.active_children(), &NodeId::new("n10"))
        .expect("updated frame");
    let value = serde_json::to_value(node).expect("updated node json");
    assert_eq!(value["height"], "fit_content");
}
