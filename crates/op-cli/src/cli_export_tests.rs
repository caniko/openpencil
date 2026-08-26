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
