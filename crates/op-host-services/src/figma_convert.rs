//! Offline `.fig` → `.op` conversion for the managed daemon: the VS Code
//! extension cannot parse fig-kiwi, so it POSTs the raw bytes here and
//! boots the returned document JSON through the normal open-document push.

use base64::Engine;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConvertRequest {
    name: String,
    bytes_b64: String,
}

fn decode_b64(s: &str) -> Result<Vec<u8>, String> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("bad base64: {e}"))
}

/// Same STANDARD alphabet `op_figma::image_resolver::blob_to_data_url` uses
/// for its data-url encoding. Only the tests below need to encode base64
/// (production traffic sends it pre-encoded from the VS Code extension), so
/// this stays `#[cfg(test)]`-gated rather than dead-code-linted in release
/// builds.
#[cfg(test)]
fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(crate) fn convert_fig_json(body: &str) -> Result<String, String> {
    let req: ConvertRequest =
        serde_json::from_str(body).map_err(|e| format!("bad convert request: {e}"))?;
    let bytes = decode_b64(&req.bytes_b64)?;
    let import = op_figma::parse_fig_binary(&bytes, &req.name, op_figma::FigLayoutMode::Preserve)
        .map_err(|e| format!("parse {}: {e:?}", req.name))?;
    let mut doc = serde_json::to_value(&import.document).map_err(|e| e.to_string())?;
    jian_ops_schema::image_table::externalize_images(&mut doc);
    Ok(serde_json::json!({ "ok": true, "doc": doc, "warnings": import.warnings }).to_string())
}

// Happy-path coverage: op-figma's only fig-kiwi fixture builder
// (`binary_e2e_tests::build_fig`) is a private `#[cfg(test)]` helper with
// no `pub` export, so it isn't reachable from here and isn't worth
// duplicating (~200 lines of hand-rolled Kiwi wire encoding) just for this
// route. The happy path (a real `.fig` converting end-to-end through this
// endpoint) is covered by the Task 10 manual matrix instead.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_json_and_bad_base64_and_non_fig_bytes() {
        assert!(convert_fig_json("not json").is_err());
        assert!(convert_fig_json(r#"{"name":"a.fig","bytesB64":"!!!"}"#).is_err());
        let not_fig = base64_encode(b"plain text, not fig-kiwi");
        let body = format!(r#"{{"name":"a.fig","bytesB64":"{not_fig}"}}"#);
        assert!(convert_fig_json(&body).is_err());
    }
}
