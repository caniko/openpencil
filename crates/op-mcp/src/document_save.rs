//! Document save tool used by the Rust HTTP CLI transport.

use std::collections::BTreeMap;
use std::path::PathBuf;

use op_editor_core::EditorState;

use super::{McpTool, ToolErrorCode, ToolOutcome};

pub struct SaveDocument {
    document_json: String,
}

impl McpTool for SaveDocument {
    fn name(&self) -> &str {
        "save_document"
    }

    fn call(&self, args: &BTreeMap<String, String>) -> ToolOutcome {
        let Some(path) = args.get("filePath").filter(|path| !path.trim().is_empty()) else {
            return ToolOutcome::Err(
                ToolErrorCode::MissingArgument,
                "filePath is required".into(),
            );
        };
        let path = resolve_path(path);
        if let Err(e) = std::fs::write(&path, &self.document_json) {
            return ToolOutcome::Err(
                ToolErrorCode::ToolFailed,
                format!("save document failed: {e}"),
            );
        }
        let mut out = BTreeMap::new();
        out.insert("ok".into(), "true".into());
        out.insert("filePath".into(), path.display().to_string());
        ToolOutcome::Ok(out)
    }
}

pub fn save_document_snapshot(state: &EditorState) -> SaveDocument {
    // Dedup shared image payloads into the `images` table so the
    // written `.op` matches the desktop save format (loader resolves
    // the refs back to inline on open).
    let document_json = serde_json::to_value(&state.doc)
        .map(|mut value| {
            jian_ops_schema::image_table::externalize_images(&mut value);
            value.to_string()
        })
        .unwrap_or_else(|_| "{}".into());
    SaveDocument { document_json }
}

fn resolve_path(raw: &str) -> PathBuf {
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}
