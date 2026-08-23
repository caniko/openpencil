mod access;
mod assets;
mod fallback;
mod map;

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::assets::Sidecar;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Message(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

impl ExportError {
    pub(crate) fn msg(s: impl Into<String>) -> Self {
        Self::Message(s.into())
    }
}

pub struct ExportOptions<'a> {
    pub name: &'a str,
    pub item: Option<&'a str>,
    pub strict: bool,
    pub source_dir: &'a Path,
}

#[derive(Debug, Clone)]
pub struct RasterCandidate {
    pub source_id: String,
    pub scene_id: String,
    pub page_index: usize,
}

pub struct ExportResult {
    pub manifest: Value,
    sidecars: Vec<Sidecar>,
    pub raster_candidates: Vec<RasterCandidate>,
}

pub use fallback::apply_raster_fallbacks;

pub fn export_document(
    doc: &jian_ops_schema::PenDocument,
    opts: &ExportOptions<'_>,
) -> Result<ExportResult, ExportError> {
    map::export_document(doc, opts)
}

pub fn load_document(input: &Path) -> Result<jian_ops_schema::PenDocument, ExportError> {
    let src = fs::read_to_string(input)?;
    jian_ops_schema::load_str(&src)
        .map(|loaded| loaded.value)
        .map_err(|e| ExportError::msg(e.to_string()))
}

pub fn prepare_export(
    input: &Path,
    output: &Path,
    item: Option<&str>,
    strict: bool,
) -> Result<ExportResult, ExportError> {
    let doc = load_document(input)?;
    let name = output
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_suffix(".opui"))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ExportError::msg("output must end with .opui"))?;
    let source_dir = input.parent().unwrap_or(Path::new("."));
    export_document(
        &doc,
        &ExportOptions {
            name,
            item,
            strict,
            source_dir,
        },
    )
}

pub fn export_file(
    input: &Path,
    output: &Path,
    item: Option<&str>,
    strict: bool,
) -> Result<(), ExportError> {
    write_package(output, &prepare_export(input, output, item, strict)?)
}

pub fn write_package(output: &Path, result: &ExportResult) -> Result<(), ExportError> {
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    fs::write(output, opui::canonical_bytes(&result.manifest))?;
    if !result.sidecars.is_empty() {
        let root = opui::asset_root_for(output);
        for sidecar in &result.sidecars {
            sidecar.write_under(&root)?;
        }
    }
    let diags = opui::check_path(output, &opui::CheckOptions::for_path(output));
    if diags.iter().any(|d| d.severity == "error") {
        return Err(ExportError::msg(opui::format_diagnostics(&diags)));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn corpus(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../vendor/jian/crates/jian-ops-schema/tests/corpus")
            .join(name)
    }

    fn tmp_opui(stem: &str) -> PathBuf {
        std::env::temp_dir().join(format!("op-runtime-ui-{stem}-{}.opui", std::process::id()))
    }

    #[test]
    fn nested_frame_is_check_clean() {
        let out = tmp_opui("nested");
        export_file(&corpus("nested-frame.op"), &out, None, false).unwrap();
        let diags = opui::check_path(&out, &opui::CheckOptions::for_path(&out));
        assert!(
            diags.iter().all(|d| d.severity != "error"),
            "{}",
            opui::format_diagnostics(&diags)
        );
        let _ = fs::remove_file(&out);
    }

    #[test]
    fn image_item_writes_sidecar() {
        let out = tmp_opui("img");
        export_file(&corpus("image.op"), &out, Some("img-data"), false).unwrap();
        let root = opui::asset_root_for(&out);
        assert!(root.join("images").exists(), "missing {}", root.display());
        let _ = fs::remove_file(&out);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn missing_viewport_fails() {
        let src = r#"{"version":"0.8.0","children":[{"type":"text","id":"t","content":"hi"}]}"#;
        let doc = jian_ops_schema::load_str(src).unwrap().value;
        let err = export_document(
            &doc,
            &ExportOptions {
                name: "x",
                item: None,
                strict: false,
                source_dir: Path::new("."),
            },
        )
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("width/height"), "{err}");
    }

    #[test]
    fn multiple_roots_need_item() {
        let err = export_file(&corpus("image.op"), &tmp_opui("multi"), None, false).unwrap_err();
        assert!(err.to_string().contains("--item"), "{err}");
    }

    fn tiny_png() -> Vec<u8> {
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJrqD0AAAAAElFTkSuQmCC",
        )
        .unwrap()
    }

    #[test]
    fn ellipse_is_raster_candidate() {
        let src = r#"{"version":"0.8.0","children":[{"type":"ellipse","id":"e","width":40,"height":40}]}"#;
        let doc = jian_ops_schema::load_str(src).unwrap().value;
        let result = export_document(
            &doc,
            &ExportOptions {
                name: "x",
                item: None,
                strict: false,
                source_dir: Path::new("."),
            },
        )
        .unwrap();
        assert_eq!(result.raster_candidates.len(), 1);
        assert_eq!(result.raster_candidates[0].source_id, "e");
        assert_eq!(result.raster_candidates[0].scene_id, "e");
        assert_eq!(result.raster_candidates[0].page_index, 0);
    }

    #[test]
    fn ref_child_scene_id_uses_double_underscore() {
        let src = r#"{
            "version":"0.8.0",
            "children":[
                {"type":"frame","id":"proto","width":20,"height":20,"children":[
                    {"type":"ellipse","id":"dot","width":10,"height":10}
                ]},
                {"type":"frame","id":"root","width":100,"height":100,"children":[
                    {"type":"ref","id":"inst","ref":"proto"}
                ]}
            ]
        }"#;
        let doc = jian_ops_schema::load_str(src).unwrap().value;
        let result = export_document(
            &doc,
            &ExportOptions {
                name: "x",
                item: Some("root"),
                strict: false,
                source_dir: Path::new("."),
            },
        )
        .unwrap();
        let c = result
            .raster_candidates
            .iter()
            .find(|c| c.source_id == "inst/dot")
            .expect("ref child candidate");
        assert_eq!(c.scene_id, "inst__dot");
    }

    #[test]
    fn page_index_follows_item() {
        let src = r#"{
            "version":"0.8.0",
            "pages":[
                {"id":"p0","name":"A","children":[{"type":"frame","id":"a","width":10,"height":10}]},
                {"id":"p1","name":"B","children":[{"type":"ellipse","id":"e","width":10,"height":10}]}
            ]
        }"#;
        let doc = jian_ops_schema::load_str(src).unwrap().value;
        let result = export_document(
            &doc,
            &ExportOptions {
                name: "x",
                item: Some("e"),
                strict: false,
                source_dir: Path::new("."),
            },
        )
        .unwrap();
        assert_eq!(result.raster_candidates[0].page_index, 1);
    }

    #[test]
    fn apply_raster_fallbacks_rewrites_leaf() {
        let src = r#"{"version":"0.8.0","children":[{"type":"ellipse","id":"e","width":40,"height":40}]}"#;
        let doc = jian_ops_schema::load_str(src).unwrap().value;
        let mut result = export_document(
            &doc,
            &ExportOptions {
                name: "x",
                item: None,
                strict: false,
                source_dir: Path::new("."),
            },
        )
        .unwrap();
        apply_raster_fallbacks(&mut result, &[("e".into(), tiny_png())]).unwrap();
        let node = &result.manifest["nodes"]["e"];
        assert_eq!(node["type"], "fallback");
        assert_eq!(node["children"], serde_json::json!([]));
        assert_eq!(node["fallback"]["intrinsic_width"], 1);
        let diags = result.manifest["diagnostics"].as_array().unwrap();
        let raster: Vec<_> = diags
            .iter()
            .filter(|d| d["code"] == "opui.rasterization")
            .collect();
        assert_eq!(raster.len(), 1);
        assert_eq!(raster[0]["strategy"], "raster_fallback");
        assert!(diags
            .iter()
            .all(|d| { !(d["node_id"] == "e" && d["strategy"] == "native") }));
    }
}
