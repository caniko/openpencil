//! TS-compatible `update_node(data)` patch detection.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{ToolErrorCode, ToolOutcome};

#[allow(clippy::result_large_err)]
pub(super) fn ts_update_patch_json(
    args: &BTreeMap<String, String>,
) -> Result<Option<String>, ToolOutcome> {
    let Some(raw) = args.get("data") else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(raw).map_err(|e| {
        ToolOutcome::Err(
            ToolErrorCode::InvalidArgument,
            format!("data must be a JSON object: {e}"),
        )
    })?;
    if !value.is_object() {
        return Err(ToolOutcome::Err(
            ToolErrorCode::InvalidArgument,
            "data must be a JSON object".into(),
        ));
    };
    ts_update_patch_json_value(&value)
        .map_err(|message| ToolOutcome::Err(ToolErrorCode::InvalidArgument, message))
}

pub(super) fn ts_update_patch_json_value(value: &Value) -> Result<Option<String>, String> {
    let Some(obj) = value.as_object() else {
        return Ok(None);
    };
    let is_rich = obj.is_empty()
        || obj.keys().any(|key| !is_flat_update_data_key(key))
        || uses_sizing_keyword(obj);
    if !is_rich {
        return Ok(None);
    }

    let mut patch = value.clone();
    canonicalize_fill_hex_shortcut(&mut patch)?;
    Ok(Some(patch.to_string()))
}

/// `UpdateNode` stores width and height as integers, while canonical node data
/// also accepts content/fill sizing keywords. Keep literal numeric geometry on
/// the lightweight command, but route keyword sizing through the rich shallow
/// patch so serde can preserve its `SizingBehavior` variant.
fn uses_sizing_keyword(obj: &serde_json::Map<String, Value>) -> bool {
    ["width", "height"].into_iter().any(|key| {
        matches!(
            obj.get(key),
            Some(Value::String(value))
                if matches!(value.as_str(), "fit_content" | "fill_container")
        )
    })
}

/// `fill_hex` / `fillHex` belong to the lightweight update tool, not the
/// canonical PenNode schema. Once any rich field moves the update onto
/// `PatchNodeData`, rewrite the shortcut to a canonical solid fill so it does
/// not disappear during PenNode deserialization.
fn canonicalize_fill_hex_shortcut(value: &mut Value) -> Result<(), String> {
    let Some(obj) = value.as_object_mut() else {
        return Ok(());
    };
    let shortcut = obj.remove("fill_hex");
    let camel_shortcut = obj.remove("fillHex");
    let Some(shortcut) = shortcut.or(camel_shortcut) else {
        return Ok(());
    };
    let Some(hex) = shortcut.as_str() else {
        return Err("fill_hex must be a string".into());
    };
    if !validate_hex(hex) {
        return Err(format!(
            "fill_hex must be #rgb/#rrggbb/#rrggbbaa, got {hex:?}"
        ));
    }
    obj.insert(
        "fill".into(),
        serde_json::json!([{ "type": "solid", "color": hex }]),
    );
    Ok(())
}

fn validate_hex(value: &str) -> bool {
    let Some(hex) = value.trim().strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_flat_update_data_key(key: &str) -> bool {
    matches!(
        key,
        "name" | "x" | "y" | "width" | "height" | "fill" | "fill_hex" | "fillHex"
    )
}
