use std::collections::BTreeMap;

use super::import_snapshot_tool::{import_web_snapshot_tool, ImportWebSnapshot};
use super::{EditorCommand, McpTool, ToolOutcome};

const SAMPLE: &str = include_str!("../../op-html/tests/fixtures/snapshot_v1_sample.json");

#[test]
fn snapshot_import_returns_insert_subtree() {
    let tool: ImportWebSnapshot = import_web_snapshot_tool();
    let mut args = BTreeMap::new();
    args.insert("snapshot".to_string(), SAMPLE.to_string());
    args.insert("x".to_string(), "50".to_string());
    let ToolOutcome::OkWithCommand(map, EditorCommand::InsertSubtree { nodes, .. }) =
        tool.call(&args)
    else {
        panic!("expected OkWithCommand(InsertSubtree)")
    };
    assert_eq!(map.get("wrote").map(String::as_str), Some("true"));
    assert_eq!(nodes.len(), 1);
}

#[test]
fn invalid_snapshot_is_typed_error() {
    let tool = import_web_snapshot_tool();
    let mut args = BTreeMap::new();
    args.insert("snapshot".to_string(), "{\"version\":9}".to_string());
    assert!(matches!(tool.call(&args), ToolOutcome::Err(..)));
}
