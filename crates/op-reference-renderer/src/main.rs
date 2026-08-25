use std::path::{Path, PathBuf};

use op_host_services::export::{export_node_raster_with_margin, RasterFormat};

const BUNDLED_FONTS: &[&[u8]] = &[
    include_bytes!("../../op-host-desktop/assets/fonts/Inter-VF.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/SpaceGrotesk-VF.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/Manrope-VF.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/Outfit-VF.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/DMSans-VF.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/DMSerifDisplay-Regular.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/DMMono-Regular.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/DMMono-Medium.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/InstrumentSerif-Regular.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/JetBrainsMono-VF.ttf"),
    include_bytes!("../../op-host-desktop/assets/fonts/CormorantGaramond-VF.ttf"),
];

fn main() {
    if let Err(error) = run() {
        eprintln!("op-reference-renderer: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .ok_or("usage: op-reference-renderer --render-shots FILE OUT [SCALE]")?;
    if first == "--version" {
        println!("op-reference-renderer {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if first != "--render-shots" {
        return Err("expected --render-shots".into());
    }
    let file = PathBuf::from(args.next().ok_or("missing FILE")?);
    let out = PathBuf::from(args.next().ok_or("missing OUT")?);
    let scale: f32 = args
        .next()
        .map(|v| v.parse())
        .transpose()
        .map_err(|_| "invalid SCALE")?
        .unwrap_or(1.0);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("invalid SCALE".into());
    }
    let margin: f32 = std::env::var("OPENPENCIL_RENDER_MARGIN")
        .ok()
        .map(|v| v.parse())
        .transpose()
        .map_err(|_| "invalid OPENPENCIL_RENDER_MARGIN")?
        .unwrap_or(0.0);
    if !margin.is_finite() || margin < 0.0 {
        return Err("invalid OPENPENCIL_RENDER_MARGIN".into());
    }
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    render(&file, &out, scale, margin)
}

fn render(file: &Path, out: &Path, scale: f32, margin: f32) -> Result<(), String> {
    let source = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let loaded = op_pen_loader::load_canonical(&source).map_err(|e| e.to_string())?;
    jian_skia::register_bundled_fonts(BUNDLED_FONTS.iter().map(|font| font.to_vec()).collect());
    let state = op_editor_core::EditorState::from_document(loaded.value);
    let scene = op_pen_loader::editor_state_to_layout_scene(&state);
    let page = scene.active_page().ok_or("no active page")?;
    if page.children.is_empty() {
        return Err("active page has no nodes".into());
    }
    let mut names = std::collections::HashSet::new();
    for node in &page.children {
        let name = sanitize(&node.id);
        if name.is_empty() || !names.insert(name) {
            return Err(format!("node id {} has an unsafe output name", node.id));
        }
    }
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    for node in &page.children {
        let target = out.join(format!("{}.png", sanitize(&node.id)));
        export_node_raster_with_margin(&scene, &node.id, &target, RasterFormat::Png, scale, margin)
            .map_err(|e| format!("{}: {e}", node.id))?;
    }
    Ok(())
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_is_repeatable() {
        let root = std::env::temp_dir().join(format!(
            "op-reference-renderer-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let source = root.join("rectangle.op");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &source,
            include_str!("../../../vendor/jian/crates/jian-ops-schema/tests/corpus/rectangle.op"),
        )
        .unwrap();

        render(&source, &root.join("first"), 1.0, 0.0).unwrap();
        render(&source, &root.join("second"), 1.0, 0.0).unwrap();
        assert_eq!(
            std::fs::read(root.join("first/rect-1.png")).unwrap(),
            std::fs::read(root.join("second/rect-1.png")).unwrap()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn output_names_are_path_safe() {
        assert_eq!(sanitize("../card/icon"), "___card_icon");
    }
}
