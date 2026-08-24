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

    fn export_src(
        src: &str,
        item: Option<&str>,
        strict: bool,
    ) -> Result<ExportResult, ExportError> {
        export_document(
            &jian_ops_schema::load_str(src).unwrap().value,
            &ExportOptions {
                name: "x",
                item,
                strict,
                source_dir: Path::new("."),
            },
        )
    }

    #[test]
    fn hidden_and_widget_leaves_are_not_raster_candidates() {
        let hidden = export_src(
            r#"{"version":"0.8.0","children":[{"type":"ellipse","id":"e","width":40,"height":40,"visible":false}]}"#,
            None,
            false,
        )
        .unwrap();
        assert!(hidden.raster_candidates.is_empty());
        let widget = export_src(
            r#"{"version":"0.8.0","children":[{"type":"text_input","id":"ti","width":40,"height":20}]}"#,
            None,
            false,
        )
        .unwrap();
        assert!(widget.raster_candidates.is_empty());
    }

    #[test]
    fn align_items_start_is_emitted() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"r","width":40,"height":40,"layout":"vertical","alignItems":"start"}]}"#,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["r"]["layout"]["align_items"],
            "start"
        );
    }

    #[test]
    fn text_fill_is_glyph_color_not_background() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"frame","id":"r","width":80,"height":20,"children":[{"type":"text","id":"t","width":80,"height":20,"content":"hi","fill":[{"type":"solid","color":"#ff0000"}]}]}]}"##,
            None,
            false,
        )
        .unwrap();
        assert!(result.manifest["nodes"]["t"]["style"].get("fill").is_none());
        assert_eq!(
            result.manifest["nodes"]["t"]["text"]["defaults"]["color"]["r"],
            1.0
        );
    }

    #[test]
    fn text_emits_inter_font_asset() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"r","width":80,"height":20,"children":[{"type":"text","id":"t","width":80,"height":20,"content":"hi","fontFamily":"Inter"}]}]}"#,
            None,
            false,
        )
        .unwrap();
        let font = result.manifest["nodes"]["t"]["text"]["defaults"]["font"]
            .as_str()
            .unwrap();
        assert!(font.starts_with("font-"), "{font}");
        assert!(result.manifest["assets"]
            .as_object()
            .unwrap()
            .contains_key(font));
    }

    #[test]
    fn root_fills_host_mount() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"r","width":1280,"height":720}]}"#,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["r"]["layout"]["width"]["type"],
            "percent"
        );
        assert_eq!(
            result.manifest["nodes"]["r"]["layout"]["width"]["value"],
            100.0
        );
        assert_eq!(
            result.manifest["nodes"]["r"]["layout"]["height"]["value"],
            100.0
        );
        assert_eq!(
            result.manifest["document"]["reference_viewport"]["width"],
            1280.0
        );
    }

    #[test]
    fn linear_gradient_is_native() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"linear_gradient","angle":0,"stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}]}]}"##,
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["r"]["style"]["fill"]["type"],
            "linear"
        );
        assert_eq!(result.manifest["nodes"]["r"]["runtime_id"], "r");
    }

    fn linear_ends(src: &str) -> (f64, f64, f64, f64) {
        let fill = &export_src(src, None, false).unwrap().manifest["nodes"]["r"]["style"]["fill"];
        (
            fill["start"]["x"].as_f64().unwrap(),
            fill["start"]["y"].as_f64().unwrap(),
            fill["end"]["x"].as_f64().unwrap(),
            fill["end"]["y"].as_f64().unwrap(),
        )
    }

    #[test]
    fn linear_gradient_angles_are_css_zero_up() {
        let g = |angle| {
            format!(
                r##"{{"version":"0.8.0","children":[{{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{{"type":"linear_gradient","angle":{angle},"stops":[{{"offset":0,"color":"#000000"}},{{"offset":1,"color":"#ffffff"}}]}}]}}]}}"##
            )
        };
        let (sx, sy, ex, ey) = linear_ends(&g(0));
        assert!((sx - 0.5).abs() < 1e-9 && (ex - 0.5).abs() < 1e-9);
        assert!(sy > 1.0 && ey < 0.0);
        let (sx, sy, ex, ey) = linear_ends(&g(90));
        assert!(sx < 0.0 && ex > 1.0);
        assert!((sy - 0.5).abs() < 1e-9 && (ey - 0.5).abs() < 1e-9);
        let (sx, sy, ex, ey) = linear_ends(&g(180));
        assert!((sx - 0.5).abs() < 1e-9 && (ex - 0.5).abs() < 1e-9);
        assert!(sy < 0.0 && ey > 1.0);
        let (sx, sy, ex, ey) = linear_ends(&g(270));
        assert!(sx > 1.0 && ex < 0.0);
        assert!((sy - 0.5).abs() < 1e-9 && (ey - 0.5).abs() < 1e-9);
    }

    #[test]
    fn first_fill_wins_and_layered_fill_is_diagnosed() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"solid","color":"#ff0000"},{"type":"solid","color":"#00ff00"}]}]}"##,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["r"]["style"]["fill"]["color"]["r"],
            1.0
        );
        assert!(
            result.manifest["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["message"].as_str().unwrap().contains("layered fill")),
            "{}",
            result.manifest["diagnostics"]
        );
    }

    #[test]
    fn decreasing_gradient_stops_are_unsupported() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"linear_gradient","angle":0,"stops":[{"offset":1,"color":"#000000"},{"offset":0,"color":"#ffffff"}]}]}]}"##,
            None,
            false,
        )
        .unwrap();
        assert!(result.manifest["nodes"]["r"]["style"].get("fill").is_none());
    }

    #[test]
    fn duplicate_offsets_keep_source_order() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"linear_gradient","angle":0,"stops":[{"offset":0.5,"color":"#000000"},{"offset":0.5,"color":"#ffffff"}]}]}]}"##,
            None,
            false,
        )
        .unwrap();
        let stops = result.manifest["nodes"]["r"]["style"]["fill"]["stops"]
            .as_array()
            .unwrap();
        assert_eq!(stops[0]["color"]["r"], 0.0);
        assert_eq!(stops[1]["color"]["r"], 1.0);
    }

    #[test]
    fn fill_opacity_multiplies_stop_alpha() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"linear_gradient","angle":0,"opacity":0.5,"stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}]}]}"##,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["r"]["style"]["fill"]["stops"][0]["color"]["a"],
            0.5
        );
    }

    #[test]
    fn radial_invalid_radius_is_unsupported() {
        let result = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"radial_gradient","radius":0,"stops":[{"offset":0,"color":"#000000"},{"offset":1,"color":"#ffffff"}]}]}]}"##,
            None,
            false,
        )
        .unwrap();
        assert!(result.manifest["nodes"]["r"]["style"].get("fill").is_none());
    }

    #[test]
    fn duplicate_semantic_names_are_hard_errors() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"root","width":40,"height":40,"children":[{"type":"frame","id":"a","name":"play","width":10,"height":10},{"type":"frame","id":"b","name":"play","width":10,"height":10}]}]}"#,
            None,
            false,
        )
        .unwrap();
        assert!(
            result.manifest["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "opui.duplicate_runtime_id" && d["severity"] == "error"),
            "{}",
            result.manifest["diagnostics"]
        );
    }

    #[test]
    fn source_id_collides_with_semantic_name() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"root","width":40,"height":40,"children":[{"type":"frame","id":"play","width":10,"height":10},{"type":"frame","id":"other","name":"play","width":10,"height":10}]}]}"#,
            None,
            false,
        )
        .unwrap();
        assert!(
            result.manifest["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "opui.duplicate_runtime_id"),
            "{}",
            result.manifest["diagnostics"]
        );
    }

    #[test]
    fn invalid_identifier_is_not_a_runtime_id() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"1root","width":40,"height":20,"name":"1bad"}]}"#,
            None,
            false,
        )
        .unwrap();
        assert!(result.manifest["nodes"]["1root"]
            .get("runtime_id")
            .is_none());
        assert_eq!(result.manifest["nodes"]["1root"]["name"], "1bad");
    }

    #[test]
    fn instance_runtime_ids_collide() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"proto","name":"btn","width":20,"height":20},{"type":"frame","id":"root","width":40,"height":40,"children":[{"type":"ref","id":"a","name":"cta","ref":"proto"},{"type":"ref","id":"b","name":"cta","ref":"proto"}]}]}"#,
            Some("root"),
            false,
        )
        .unwrap();
        assert!(
            result.manifest["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["code"] == "opui.duplicate_runtime_id"),
            "{}",
            result.manifest["diagnostics"]
        );
    }

    #[test]
    fn runtime_ids_are_deterministic() {
        let src = r#"{"version":"0.8.0","children":[{"type":"frame","id":"uuid-1","name":"main_menu.play","width":40,"height":20}]}"#;
        let a = export_src(src, None, false).unwrap();
        let b = export_src(src, None, false).unwrap();
        assert_eq!(
            a.manifest["nodes"]["uuid-1"]["runtime_id"],
            b.manifest["nodes"]["uuid-1"]["runtime_id"]
        );
        assert_eq!(
            a.manifest["nodes"]["uuid-1"]["runtime_id"],
            "main_menu.play"
        );
    }

    #[test]
    fn raster_fallback_keeps_runtime_id() {
        let mut result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"ellipse","id":"badge_orb","width":40,"height":40}]}"#,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["badge_orb"]["runtime_id"],
            "badge_orb"
        );
        apply_raster_fallbacks(&mut result, &[("badge_orb".into(), tiny_png())]).unwrap();
        assert_eq!(
            result.manifest["nodes"]["badge_orb"]["runtime_id"],
            "badge_orb"
        );
        assert_eq!(
            result.manifest["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .find(|d| d["code"] == "opui.rasterization")
                .unwrap()["runtime_id"],
            "badge_orb"
        );
    }

    #[test]
    fn semantic_name_becomes_runtime_id() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"uuid-1","name":"main_menu.play","width":40,"height":20}]}"#,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["uuid-1"]["runtime_id"],
            "main_menu.play"
        );
    }

    #[test]
    fn percent_expression_maps() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"root","width":200,"height":80,"children":[{"type":"frame","id":"r","width":"50%","height":40}]}]}"#,
            None,
            false,
        )
        .unwrap();
        assert_eq!(
            result.manifest["nodes"]["r"]["layout"]["width"]["type"],
            "percent"
        );
        assert_eq!(
            result.manifest["nodes"]["r"]["layout"]["width"]["value"],
            50.0
        );
    }

    #[test]
    fn percent_boundaries_and_invalid() {
        let ok = |raw: &str, expect: f64| {
            let src = format!(
                r#"{{"version":"0.8.0","children":[{{"type":"frame","id":"root","width":200,"height":80,"children":[{{"type":"frame","id":"r","width":"{raw}","height":40}}]}}]}}"#
            );
            let result = export_src(&src, None, false).unwrap();
            assert_eq!(
                result.manifest["nodes"]["r"]["layout"]["width"]["type"],
                "percent"
            );
            assert_eq!(
                result.manifest["nodes"]["r"]["layout"]["width"]["value"],
                expect
            );
        };
        ok("0%", 0.0);
        ok("100%", 100.0);
        ok("200%", 200.0);
        ok("-10%", -10.0);
        let bad = |raw: &str| {
            let src = format!(
                r#"{{"version":"0.8.0","children":[{{"type":"frame","id":"root","width":200,"height":80,"children":[{{"type":"frame","id":"r","width":"{raw}","height":40}}]}}]}}"#
            );
            let result = export_src(&src, None, false).unwrap();
            assert!(
                result.manifest["nodes"]["r"]["layout"]
                    .get("width")
                    .is_none(),
                "{raw}: {}",
                result.manifest["nodes"]["r"]["layout"]
            );
        };
        bad("foo");
        bad("%");
        bad("nan%");
        bad("inf%");
    }

    #[test]
    fn strict_rejects_image_fill() {
        let err = export_src(
            r##"{"version":"0.8.0","children":[{"type":"rectangle","id":"r","width":10,"height":10,"fill":[{"type":"image","url":"x.png"}]}]}"##,
            None,
            true,
        )
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("--strict"), "{err}");
    }

    #[test]
    fn cyclic_ref_fails() {
        let err = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"proto","width":20,"height":20,"children":[{"type":"ref","id":"loop","ref":"proto"}]},{"type":"frame","id":"root","width":40,"height":40,"children":[{"type":"ref","id":"inst","ref":"proto"}]}]}"#,
            Some("root"),
            false,
        )
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("cyclic ref"), "{err}");
    }

    #[test]
    fn ref_uses_slot_children() {
        let result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"frame","id":"proto","width":20,"height":20,"children":[{"type":"rectangle","id":"slot","width":10,"height":10}]},{"type":"frame","id":"root","width":40,"height":40,"children":[{"type":"ref","id":"inst","ref":"proto","children":[{"type":"ellipse","id":"dot","width":10,"height":10}]}]}]}"#,
            Some("root"),
            false,
        )
        .unwrap();
        assert!(result
            .raster_candidates
            .iter()
            .any(|c| c.source_id == "inst/dot"));
        assert!(!result.manifest["nodes"]
            .as_object()
            .unwrap()
            .contains_key("inst/slot"));
    }

    #[test]
    fn fallback_keeps_outer_shadows() {
        let mut result = export_src(
            r#"{"version":"0.8.0","children":[{"type":"ellipse","id":"e","width":40,"height":40}]}"#,
            None,
            false,
        )
        .unwrap();
        result.manifest["nodes"]["e"]["style"]["outer_shadows"] = serde_json::json!([{
            "offset_x": {"type":"px","value":1.0},
            "offset_y": {"type":"px","value":1.0},
            "blur": {"type":"px","value":2.0},
            "spread": {"type":"px","value":0.0},
            "color": {"hex":"#000000","alpha":1.0}
        }]);
        apply_raster_fallbacks(&mut result, &[("e".into(), tiny_png())]).unwrap();
        assert!(result.manifest["nodes"]["e"]["style"]
            .get("outer_shadows")
            .is_some());
    }

    #[test]
    fn page_index_errors_when_item_not_under_pages() {
        let err = export_src(
            r#"{"version":"0.8.0","pages":[{"id":"p0","name":"A","children":[{"type":"frame","id":"a","width":10,"height":10}]}],"children":[{"type":"ellipse","id":"e","width":10,"height":10}]}"#,
            Some("e"),
            false,
        )
        .map(|_| ())
        .unwrap_err();
        assert!(err.to_string().contains("not under pages"), "{err}");
    }
}
