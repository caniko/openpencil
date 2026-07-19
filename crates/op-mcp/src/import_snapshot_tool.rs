use std::{collections::BTreeMap, fs};

use jian_ops_schema::node::PenNode;
use op_editor_core::NodeId;
use op_html::{import_snapshot, HtmlImportOptions};

use super::write_tools::{parse_opt_i32, root_or_node_id};
use super::{EditorCommand, McpTool, ToolErrorCode, ToolOutcome};

pub struct ImportWebSnapshot;

impl McpTool for ImportWebSnapshot {
    fn name(&self) -> &str {
        "import_web_snapshot"
    }

    fn call(&self, args: &BTreeMap<String, String>) -> ToolOutcome {
        let snapshot = match args.get("snapshot") {
            Some(snapshot) => snapshot.clone(),
            None => {
                let Some(path) = args
                    .get("snapshotPath")
                    .or_else(|| args.get("snapshot_path"))
                else {
                    return ToolOutcome::Err(
                        ToolErrorCode::MissingArgument,
                        "snapshot or snapshotPath is required".into(),
                    );
                };
                match fs::read_to_string(path) {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        return ToolOutcome::Err(
                            ToolErrorCode::ToolFailed,
                            format!("failed to read snapshotPath {path:?}: {error}"),
                        );
                    }
                }
            }
        };
        if snapshot.trim().is_empty() {
            return ToolOutcome::Err(
                ToolErrorCode::InvalidArgument,
                "snapshot must not be empty".into(),
            );
        }
        let x = match parse_opt_i32(args, "x") {
            Ok(value) => value.unwrap_or(0),
            Err(error) => {
                return ToolOutcome::Err(ToolErrorCode::InvalidArgument, format!("x: {error}"));
            }
        };
        let y = match parse_opt_i32(args, "y") {
            Ok(value) => value.unwrap_or(0),
            Err(error) => {
                return ToolOutcome::Err(ToolErrorCode::InvalidArgument, format!("y: {error}"));
            }
        };
        let target_parent = args
            .get("parent")
            .or_else(|| args.get("parent_id"))
            .or_else(|| args.get("target_parent_id"))
            .map(|value| root_or_node_id(value))
            .unwrap_or(NodeId::NONE);
        let page_id = args
            .get("pageId")
            .or_else(|| args.get("page_id"))
            .or_else(|| args.get("page"))
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        let result = import_snapshot(&snapshot, &HtmlImportOptions::default());
        if result.nodes.is_empty() {
            let detail = result
                .warnings
                .first()
                .map(String::as_str)
                .unwrap_or("input produced no nodes");
            return ToolOutcome::Err(
                ToolErrorCode::InvalidArgument,
                format!("no importable content: {detail}"),
            );
        }
        let mut nodes = result.nodes;
        if x != 0 || y != 0 {
            if let PenNode::Frame(frame) = &mut nodes[0] {
                frame.base.x = Some(x as f64);
                frame.base.y = Some(y as f64);
            }
        }
        let mut output = BTreeMap::new();
        output.insert("wrote".into(), "true".into());
        output.insert("nodeCount".into(), count_nodes(&nodes).to_string());
        if !result.warnings.is_empty() {
            output.insert("warnings".into(), result.warnings.join("\n"));
        }
        ToolOutcome::OkWithCommand(
            output,
            EditorCommand::InsertSubtree {
                nodes,
                parent_id: target_parent,
                page_id,
            },
        )
    }
}

fn count_nodes(nodes: &[PenNode]) -> usize {
    nodes
        .iter()
        .map(|node| {
            1 + match node {
                PenNode::Frame(node) => node
                    .children
                    .as_deref()
                    .map(count_nodes)
                    .unwrap_or_default(),
                PenNode::Group(node) => node
                    .children
                    .as_deref()
                    .map(count_nodes)
                    .unwrap_or_default(),
                PenNode::Rectangle(node) => node
                    .children
                    .as_deref()
                    .map(count_nodes)
                    .unwrap_or_default(),
                _ => 0,
            }
        })
        .sum()
}

pub fn import_web_snapshot_tool() -> ImportWebSnapshot {
    ImportWebSnapshot
}
