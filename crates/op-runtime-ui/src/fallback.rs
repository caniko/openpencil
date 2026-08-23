use serde_json::{json, Value};

use crate::assets::raster_fallback_from_png;
use crate::{ExportError, ExportResult};

const PAINTED_STYLE: &[&str] = &["fill", "border", "clipping", "corner_radius", "opacity"];

pub fn apply_raster_fallbacks(
    result: &mut ExportResult,
    rasters: &[(String, Vec<u8>)],
) -> Result<(), ExportError> {
    for (source_id, bytes) in rasters {
        apply_one(result, source_id, bytes.clone())?;
    }
    sort_diagnostics(result);
    Ok(())
}

fn apply_one(
    result: &mut ExportResult,
    source_id: &str,
    bytes: Vec<u8>,
) -> Result<(), ExportError> {
    let sidecar = raster_fallback_from_png(bytes)?;
    let (iw, ih) = crate::assets::png_size(&sidecar.bytes).unwrap();
    let asset_id = sidecar.id();
    let rec = json!({
        "kind": sidecar.kind,
        "uri": sidecar.uri(),
        "mime_type": sidecar.mime,
        "sha256": sidecar.sha256,
        "byte_length": sidecar.bytes.len() as u64,
    });
    let nodes = result
        .manifest
        .get_mut("nodes")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ExportError::msg("manifest missing nodes"))?;
    let node = nodes
        .get_mut(source_id)
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ExportError::msg(format!("raster node {source_id} not in manifest")))?;
    node.insert("type".into(), json!("fallback"));
    node.remove("container");
    node.remove("text");
    node.remove("image");
    node.insert("children".into(), json!([]));
    node.insert(
        "fallback".into(),
        json!({
            "asset": asset_id,
            "intrinsic_width": iw,
            "intrinsic_height": ih,
        }),
    );
    if let Some(style) = node.get_mut("style").and_then(Value::as_object_mut) {
        for key in PAINTED_STYLE {
            style.remove(*key);
        }
    }
    let assets = result
        .manifest
        .get_mut("assets")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| ExportError::msg("manifest missing assets"))?;
    assets.insert(asset_id, rec);
    result.sidecars.push(sidecar);

    let diags = result
        .manifest
        .get_mut("diagnostics")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| ExportError::msg("manifest missing diagnostics"))?;
    diags.retain(|d| {
        !(d.get("node_id").and_then(Value::as_str) == Some(source_id)
            && d.get("strategy").and_then(Value::as_str) == Some("native"))
    });
    diags.push(json!({
        "code": "opui.rasterization",
        "severity": "warning",
        "node_id": source_id,
        "runtime_id": null,
        "message": "rasterized unsupported native node",
        "strategy": "raster_fallback",
        "details": {},
    }));
    Ok(())
}

fn sort_diagnostics(result: &mut ExportResult) {
    let Some(diags) = result
        .manifest
        .get_mut("diagnostics")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    diags.sort_by(|a, b| {
        sev_rank(a)
            .cmp(&sev_rank(b))
            .then(diag_str(a, "code").cmp(diag_str(b, "code")))
            .then(diag_id(a, "node_id").cmp(diag_id(b, "node_id")))
            .then(diag_id(a, "runtime_id").cmp(diag_id(b, "runtime_id")))
            .then(diag_str(a, "strategy").cmp(diag_str(b, "strategy")))
            .then(diag_str(a, "message").cmp(diag_str(b, "message")))
    });
}

fn sev_rank(d: &Value) -> u8 {
    match d.get("severity").and_then(Value::as_str) {
        Some("error") => 0,
        Some("warning") => 1,
        _ => 2,
    }
}

fn diag_str<'a>(d: &'a Value, key: &str) -> &'a str {
    d.get(key).and_then(Value::as_str).unwrap_or("")
}

fn diag_id<'a>(d: &'a Value, key: &str) -> &'a str {
    match d.get(key) {
        Some(Value::String(s)) => s.as_str(),
        _ => "",
    }
}
