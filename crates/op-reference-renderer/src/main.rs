use std::path::{Path, PathBuf};

use jian_ops_schema::node::PenNode;
use jian_ops_schema::sizing::SizingBehavior;
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
        .ok_or("usage: op-reference-renderer --render-shots FILE OUT [SCALE [WIDTH HEIGHT]]")?;
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
    let viewport = args
        .next()
        .map(|width| {
            let height = args.next().ok_or("missing HEIGHT")?;
            let width = width.parse::<u32>().map_err(|_| "invalid WIDTH")?;
            let height = height.parse::<u32>().map_err(|_| "invalid HEIGHT")?;
            if width == 0 || height == 0 {
                return Err("viewport dimensions must be positive");
            }
            Ok((width, height))
        })
        .transpose()?;
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    render(&file, &out, scale, margin, viewport)
}

fn render(
    file: &Path,
    out: &Path,
    scale: f32,
    margin: f32,
    viewport: Option<(u32, u32)>,
) -> Result<(), String> {
    let source = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut loaded = op_pen_loader::load_canonical(&source).map_err(|e| e.to_string())?;
    if let Some((width, height)) = viewport {
        resize_frames(&mut loaded.value.children, width, height)?;
        for page in loaded.value.pages.iter_mut().flatten() {
            resize_frames(&mut page.children, width, height)?;
        }
    }
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

fn resize_frames(nodes: &mut [PenNode], width: u32, height: u32) -> Result<(), String> {
    for node in nodes {
        let PenNode::Frame(frame) = node else {
            return Err("viewport override requires top-level frames".into());
        };
        frame.container.width = Some(SizingBehavior::Number(width.into()));
        frame.container.height = Some(SizingBehavior::Number(height.into()));
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

        render(&source, &root.join("first"), 1.0, 0.0, None).unwrap();
        render(&source, &root.join("second"), 1.0, 0.0, None).unwrap();
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

    #[test]
    fn viewport_override_changes_output_dimensions() {
        let root =
            std::env::temp_dir().join(format!("op-reference-viewport-{}", std::process::id()));
        let source = root.join("rectangle.op");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &source,
            r##"{"version":"0.8.0","children":[{"type":"frame","id":"artboard","width":100,"height":100,"fill":[{"type":"solid","color":"#000000"}]}]}"##,
        )
        .unwrap();

        render(&source, &root.join("out"), 1.0, 0.0, Some((320, 180))).unwrap();
        let image = std::fs::read(root.join("out/artboard.png")).unwrap();
        assert_eq!(&image[16..24], &[0, 0, 1, 64, 0, 0, 0, 180]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
