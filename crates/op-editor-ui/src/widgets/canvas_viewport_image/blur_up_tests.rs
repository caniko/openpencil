use super::*;
use crate::layout_scene::NodeKind;
use crate::{RenderBackend, TextLayout};

#[derive(Default)]
struct BlurCaptureBackend {
    ops: Vec<&'static str>,
}

impl RenderBackend for BlurCaptureBackend {
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
    fn fill_rect(&mut self, _: Rect, _: Color) {
        self.ops.push("fill");
    }
    fn stroke_rect(&mut self, _: Rect, _: Color, _: f32) {}
    fn draw_text(&mut self, _: &TextLayout, _: Point2D) {}
    fn clip_rect(&mut self, _: Rect) {}
    fn save(&mut self) {}
    fn restore(&mut self) {}
    fn translate(&mut self, _: Point2D) {}
    fn stroke_line(&mut self, _: Point2D, _: Point2D, _: Color, _: f32) {
        self.ops.push("glyph");
    }
    fn fill_round_rect(&mut self, _: Rect, _: f32, _: Color) {
        self.ops.push("fill");
    }
    fn stroke_round_rect(&mut self, _: Rect, _: f32, _: Color, _: f32) {}
    fn stroke_svg_path(&mut self, _: &str, _: Point2D, _: f32, _: Color, _: f32) {}
    fn image_decoded(&mut self, _: u64, _: &[u8], _: u32) -> bool {
        false
    }
    fn image_resident(&mut self, _: u64) -> bool {
        false
    }
    fn draw_image_thumb(&mut self, _: Rect, _: u64, _: &[u8]) {
        self.ops.push("thumb");
    }
    fn resize(&mut self, _: u32, _: u32) {}
    fn dpi_scale(&self) -> f32 {
        1.0
    }
}

#[test]
fn undecoded_image_paints_thumb_before_fill_and_glyph() {
    let _guard = lock_decode_registry_for_tests();
    clear_data_url_cache_for_tests();
    clear_remote_registry_for_tests();
    let src = "data:image/png;base64,QUJD";
    let id = stable_image_source_id(src);
    jian_ops_schema::image_thumbs::store_thumb(id, b"small jpeg".to_vec());
    let mut node = SceneNode::leaf("image", NodeKind::Rect);
    node.image_src_id = id;
    node.fill = Some(Color::WHITE);
    let mut backend = BlurCaptureBackend::default();

    paint_image_node(
        &mut PaintCx {
            backend: &mut backend,
        },
        &node,
        Rect::xywh(0.0, 0.0, 100.0, 50.0),
        1.0,
        src,
        true,
    );

    assert_eq!(&backend.ops[..2], &["thumb", "fill"]);
    assert!(backend.ops[2..].contains(&"glyph"));
}
