use super::*;
use base64::Engine as _;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn export_item_maps_to_dedicated_command() {
    let parsed = parse_args(&args(&[
        "export",
        "--item",
        "page-2",
        "--output",
        "/tmp/page.png",
        "--format",
        "png",
        "--scale",
        "2",
    ]))
    .expect("parse export");
    assert_eq!(
        parsed.command,
        Command::Export {
            item_id: Some("page-2".into()),
            selection: false,
            output: "/tmp/page.png".into(),
            format: "png".into(),
            scale: Some("2".into()),
        }
    );
}

#[test]
fn export_without_item_means_live_selection() {
    let parsed = parse_args(&args(&[
        "export",
        "--output",
        "/tmp/selected.png",
        "--format",
        "png",
    ]))
    .expect("parse selection export");
    assert!(matches!(
        parsed.command,
        Command::Export {
            item_id: None,
            selection: false,
            ..
        }
    ));
}

#[test]
fn export_selection_flag_means_live_selection() {
    let parsed = parse_args(&args(&[
        "export",
        "--selection",
        "--output",
        "/tmp/selected.png",
    ]))
    .expect("parse --selection export");
    assert!(matches!(
        parsed.command,
        Command::Export {
            item_id: None,
            selection: true,
            ..
        }
    ));
}

#[test]
fn export_accepts_issue_formats_alias() {
    let parsed = parse_args(&args(&[
        "export",
        "--item",
        "page-1",
        "--output",
        "/tmp/page.png",
        "--formats",
        "png",
    ]))
    .expect("parse --formats alias");
    assert!(matches!(
        parsed.command,
        Command::Export { format, .. } if format == "png"
    ));
}

#[test]
fn export_rejects_conflicting_target_and_format_flags() {
    let target = parse_args(&args(&[
        "export",
        "--item",
        "n1",
        "--selection",
        "--output",
        "/tmp/node.png",
    ]));
    assert!(target.unwrap_err().contains("--item and --selection"));

    let format = parse_args(&args(&[
        "export",
        "--output",
        "/tmp/node.png",
        "--format",
        "png",
        "--formats",
        "jpeg",
    ]));
    assert!(format.unwrap_err().contains("--format and --formats"));
}

#[test]
fn write_export_response_decodes_png_to_exact_path() {
    let path =
        std::env::temp_dir().join(format!("op-cli-export-{}-selected.png", std::process::id()));
    let png = [0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let response = serde_json::json!({
        "itemId": "n1",
        "itemType": "node",
        "format": "png",
        "bytes_base64": base64::engine::general_purpose::STANDARD.encode(png),
    })
    .to_string();

    let output = export_cli::write_export_response(&response, &path).expect("write export");
    assert_eq!(std::fs::read(&path).expect("read export"), png);
    assert!(output.contains("\"itemType\":\"node\""), "{output}");
    std::fs::remove_file(path).ok();
}

#[test]
fn export_opui_maps_without_server() {
    let parsed = parse_args(&args(&[
        "export", "--file", "doc.op", "--format", "opui", "--output", "doc.opui", "--item", "root",
        "--strict",
    ]))
    .expect("parse opui export");
    assert_eq!(
        parsed.command,
        Command::ExportOpui {
            file: "doc.op".into(),
            item_id: Some("root".into()),
            output: "doc.opui".into(),
            strict: true,
            raster_native: false,
            watch: false,
            debounce_ms: 150,
        }
    );
}

#[test]
fn export_opui_requires_file_and_opui_suffix() {
    let missing = parse_args(&args(&[
        "export", "--format", "opui", "--output", "doc.opui",
    ]));
    assert!(missing.unwrap_err().contains("--file"), "need --file");

    let suffix = parse_args(&args(&[
        "export", "--file", "doc.op", "--format", "opui", "--output", "doc.png",
    ]));
    assert!(suffix.unwrap_err().contains(".opui"), "need .opui");

    let selection = parse_args(&args(&[
        "export",
        "--file",
        "doc.op",
        "--format",
        "opui",
        "--output",
        "doc.opui",
        "--selection",
    ]));
    assert!(selection.unwrap_err().contains("--selection"));
}

#[test]
fn export_opui_rejects_strict_with_raster() {
    let err = parse_args(&args(&[
        "export",
        "--file",
        "doc.op",
        "--format",
        "opui",
        "--output",
        "doc.opui",
        "--strict",
        "--raster-native",
    ]))
    .unwrap_err();
    assert!(err.contains("--strict"), "{err}");
}

#[test]
fn export_opui_raster_native_feature_gate() {
    let parsed = parse_args(&args(&[
        "export",
        "--file",
        "doc.op",
        "--format",
        "opui",
        "--output",
        "doc.opui",
        "--raster-native",
    ]));
    #[cfg(not(feature = "opui-raster"))]
    assert!(
        parsed.unwrap_err().contains("opui-raster"),
        "feature-off must fail closed"
    );
    #[cfg(feature = "opui-raster")]
    assert_eq!(
        parsed.expect("parse raster opui").command,
        Command::ExportOpui {
            file: "doc.op".into(),
            item_id: None,
            output: "doc.opui".into(),
            strict: false,
            raster_native: true,
            watch: false,
            debounce_ms: 150,
        }
    );
}

#[test]
fn export_opui_watch_parses_debounce() {
    let parsed = parse_args(&args(&[
        "export",
        "--file",
        "doc.op",
        "--format",
        "opui",
        "--output",
        "doc.opui",
        "--watch",
        "--debounce-ms",
        "75",
    ]))
    .unwrap();
    assert!(matches!(
        parsed.command,
        Command::ExportOpui {
            watch: true,
            debounce_ms: 75,
            ..
        }
    ));
}

#[test]
fn export_opui_watch_exports_a_settled_source_change() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("x.op");
    let output = dir.path().join("x.opui");
    std::fs::write(
        &source,
        r#"{"version":"0.8.1","children":[{"type":"text","id":"root","width":80,"height":40,"content":"Before"}]}"#,
    )
    .unwrap();
    let changed = source.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        std::fs::write(
            changed,
            r#"{"version":"0.8.1","children":[{"type":"text","id":"root","width":80,"height":40,"content":"After"}]}"#,
        )
        .unwrap();
    });

    export_cli::run_export_opui_watch_for(
        source.to_str().unwrap(),
        output.to_str().unwrap(),
        None,
        false,
        false,
        25,
        Some(1),
    )
    .unwrap();
    writer.join().unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(manifest["nodes"]["root"]["text"]["content"], "After");
}

#[test]
fn export_opui_watch_recovers_from_invalid_initial_source() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("x.op");
    let output = dir.path().join("x.opui");
    std::fs::write(
        &source,
        r#"{"version":"0.8.1","children":[{"type":"text","id":"root","width":80,"height":40,"content":"Last good"}]}"#,
    )
    .unwrap();
    export_cli::run_export_opui(
        source.to_str().unwrap(),
        output.to_str().unwrap(),
        None,
        false,
        false,
    )
    .unwrap();
    let last_good = std::fs::read(&output).unwrap();
    std::fs::write(&source, "not json").unwrap();
    let changed = source.clone();
    let retained = output.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert_eq!(std::fs::read(retained).unwrap(), last_good);
        std::fs::write(
            changed,
            r#"{"version":"0.8.1","children":[{"type":"text","id":"root","width":80,"height":40,"content":"Repaired"}]}"#,
        )
        .unwrap();
    });

    export_cli::run_export_opui_watch_for(
        source.to_str().unwrap(),
        output.to_str().unwrap(),
        None,
        false,
        false,
        25,
        Some(1),
    )
    .unwrap();
    writer.join().unwrap();
    let manifest: Value = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(manifest["nodes"]["root"]["text"]["content"], "Repaired");
}

#[test]
fn runtime_ui_entrypoint_is_authored_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("app.op");
    std::fs::write(&source, r#"{"version":"0.8.1","name":"App","children":[]}"#).unwrap();

    export_cli::set_runtime_entrypoint(source.to_str().unwrap(), "app=app.root").unwrap();
    let document = op_runtime_ui::load_document(&source).unwrap();
    assert_eq!(document.name.as_deref(), Some("App"));
    assert_eq!(document.runtime_entrypoints.unwrap()["app"], "app.root");
    assert!(
        export_cli::set_runtime_entrypoint(source.to_str().unwrap(), "bad name=app.root").is_err()
    );
}

#[test]
fn runtime_ui_metadata_is_authored_and_schema_valid() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("app.op");
    let spec = dir.path().join("runtime.json");
    std::fs::write(
        &source,
        r#"{"version":"0.8.1","children":[{"type":"frame","id":"play","name":"Play","width":100,"height":40,"children":[{"type":"frame","id":"play-default","name":"Play default","width":100,"height":40},{"type":"text","id":"play-label","name":"Play","width":100,"height":40,"content":"Play"},{"type":"frame","id":"play-hover","name":"Play hover","width":100,"height":40}]}]}"#,
    )
    .unwrap();
    std::fs::write(
        &spec,
        r#"{"entrypoints":{"app":"main.play"},"nodes":[{"name":"Play","type":"frame","runtimeId":"main.play","role":"button","accessibilityLabel":"Play game","tabIndex":0,"visualStates":{"default":"Play default","hover":"Play hover"}}]}"#,
    )
    .unwrap();

    let response =
        export_cli::apply_runtime_metadata(source.to_str().unwrap(), spec.to_str().unwrap())
            .unwrap();
    assert!(response.contains("legacy name selector"));
    let document = op_runtime_ui::load_document(&source).unwrap();
    let value = serde_json::to_value(document).unwrap();
    assert_eq!(value["runtimeEntrypoints"]["app"], "main.play");
    assert_eq!(value["children"][0]["runtimeId"], "main.play");
    assert_eq!(value["children"][0]["role"], "button");
    assert_eq!(value["children"][0]["accessibilityLabel"], "Play game");
    assert_eq!(value["children"][0]["tabIndex"], 0);
    assert_eq!(
        value["children"][0]["visualStates"]["hover"],
        "main.play.hover"
    );
    assert_eq!(
        value["children"][0]["children"][2]["runtimeId"],
        "main.play.hover"
    );
}

#[test]
fn runtime_ui_metadata_prefers_exact_ids_and_rejects_stale_specs() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("app.op");
    let spec = dir.path().join("runtime.json");
    std::fs::write(
        &source,
        r#"{"version":"0.8.1","children":[{"type":"frame","id":"play","name":"Duplicate","width":100,"height":40,"children":[{"type":"frame","id":"play-default","name":"Duplicate","width":100,"height":40}]}]}"#,
    )
    .unwrap();
    let digest = op_runtime_ui::source_sha256(&source).unwrap();
    std::fs::write(
        &spec,
        format!(
            r#"{{"sourceSha256":"{digest}","nodes":[{{"nodeId":"play","runtimeId":"main.play","role":"button","visualStates":{{"default":{{"nodeId":"play-default"}}}}}}]}}"#
        ),
    )
    .unwrap();

    let response =
        export_cli::apply_runtime_metadata(source.to_str().unwrap(), spec.to_str().unwrap())
            .unwrap();
    assert!(response.contains(r#""warnings":[]"#));
    let document = op_runtime_ui::load_document(&source).unwrap();
    let value = serde_json::to_value(document).unwrap();
    assert_eq!(value["children"][0]["runtimeId"], "main.play");
    assert_eq!(
        value["children"][0]["children"][0]["runtimeId"],
        "main.play.default"
    );

    let error =
        export_cli::apply_runtime_metadata(source.to_str().unwrap(), spec.to_str().unwrap())
            .unwrap_err();
    assert!(error.contains("spec is stale"));

    let current = op_runtime_ui::source_sha256(&source).unwrap();
    std::fs::write(
        &spec,
        format!(
            r#"{{"sourceSha256":"{current}","nodes":[{{"nodeId":"missing","runtimeId":"main.missing"}}]}}"#
        ),
    )
    .unwrap();
    let error =
        export_cli::apply_runtime_metadata(source.to_str().unwrap(), spec.to_str().unwrap())
            .unwrap_err();
    assert!(error.contains("nodeId selector `missing` matched 0 nodes"));
}

#[cfg(feature = "opui-raster")]
#[test]
fn export_opui_raster_native_writes_fallback_png() {
    let dir = std::env::temp_dir().join(format!("op-cli-opui-e2e-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let input = dir.join("e.op");
    let output = dir.join("e.opui");
    std::fs::write(
        &input,
        r##"{"version":"0.8.0","children":[{"type":"ellipse","id":"e","width":40,"height":40,"fill":[{"type":"solid","color":"#ff0000"}]}]}"##,
    )
    .unwrap();
    export_cli::run_export_opui(
        input.to_str().unwrap(),
        output.to_str().unwrap(),
        None,
        false,
        true,
    )
    .expect("raster export");
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&output).unwrap()).unwrap();
    assert_eq!(v["nodes"]["e"]["type"], "fallback");
    let pngs = std::fs::read_dir(dir.join("e.opui.assets/fallback"))
        .unwrap()
        .count();
    assert_eq!(pngs, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn write_export_response_rejects_invalid_payloads() {
    let path = std::env::temp_dir().join("op-cli-export-invalid.png");
    assert!(export_cli::write_export_response("not-json", &path).is_err());
    assert!(export_cli::write_export_response(
        r#"{"itemId":"n1","itemType":"node","format":"png","bytes_base64":"%%%"}"#,
        &path,
    )
    .is_err());
}
