use std::collections::BTreeMap;

use super::import_html_tool::{import_html_snapshot, ImportHtml};
use super::{EditorCommand, McpTool, ToolOutcome};

#[test]
fn import_html_returns_insert_subtree_command() {
    let tool: ImportHtml = import_html_snapshot();
    let mut args = BTreeMap::new();
    args.insert(
        "html".to_string(),
        "<div style=\"display:flex\"><p>hi</p></div>".to_string(),
    );
    args.insert("x".to_string(), "100".to_string());
    let outcome = tool.call(&args);
    let ToolOutcome::OkWithCommand(result, EditorCommand::InsertSubtree { nodes, page_id, .. }) =
        outcome
    else {
        panic!("expected OkWithCommand(InsertSubtree)")
    };
    assert_eq!(result.get("wrote").map(String::as_str), Some("true"));
    assert_eq!(nodes.len(), 1);
    assert!(page_id.is_none());
}

#[test]
fn missing_html_is_typed_error() {
    let tool = import_html_snapshot();
    let outcome = tool.call(&BTreeMap::new());
    assert!(matches!(outcome, ToolOutcome::Err(..)));
}
