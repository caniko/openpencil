use super::{to_sk_rect, NativeBackend};
use op_editor_ui::{ImageBlendMode, Rect};

impl NativeBackend {
    /// Isolate subsequent draws and composite the finished layer with `mode`.
    pub fn push_blend_layer(&self, canvas: &skia_safe::Canvas, mode: ImageBlendMode) {
        if mode == ImageBlendMode::Normal {
            canvas.save();
            return;
        }
        let mut paint = skia_safe::Paint::default();
        paint.set_blend_mode(to_skia_blend_mode(mode));
        canvas.save_layer(&skia_safe::canvas::SaveLayerRec::default().paint(&paint));
    }

    /// Begin a bounded, always-isolated layer that applies opacity and blend
    /// exactly once when restored. Even opaque source-over uses saveLayer:
    /// callers rely on the transparent intermediate backdrop for fill blends.
    pub fn push_composite_layer(
        &self,
        canvas: &skia_safe::Canvas,
        bounds: Rect,
        opacity: f32,
        mode: ImageBlendMode,
    ) {
        let bounds = to_sk_rect(bounds);
        let mut paint = skia_safe::Paint::default();
        paint.set_alpha_f(opacity.clamp(0.0, 1.0));
        paint.set_blend_mode(to_skia_blend_mode(mode));
        let rec = skia_safe::canvas::SaveLayerRec::default()
            .bounds(&bounds)
            .paint(&paint);
        canvas.save_layer(&rec);
    }
}

pub(super) fn to_skia_blend_mode(mode: ImageBlendMode) -> skia_safe::BlendMode {
    match mode {
        ImageBlendMode::Normal => skia_safe::BlendMode::SrcOver,
        ImageBlendMode::Darken => skia_safe::BlendMode::Darken,
        ImageBlendMode::Multiply => skia_safe::BlendMode::Multiply,
        ImageBlendMode::Screen => skia_safe::BlendMode::Screen,
        ImageBlendMode::Overlay => skia_safe::BlendMode::Overlay,
        ImageBlendMode::Lighten => skia_safe::BlendMode::Lighten,
        ImageBlendMode::Difference => skia_safe::BlendMode::Difference,
        ImageBlendMode::Hue => skia_safe::BlendMode::Hue,
        ImageBlendMode::Saturation => skia_safe::BlendMode::Saturation,
        ImageBlendMode::Color => skia_safe::BlendMode::Color,
        ImageBlendMode::Luminosity => skia_safe::BlendMode::Luminosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(surface: &mut skia_safe::Surface, x: i32, y: i32) -> skia_safe::Color {
        surface
            .image_snapshot()
            .peek_pixels()
            .expect("raster pixels")
            .get_color((x, y))
    }

    #[test]
    fn composite_layer_applies_group_opacity_once_across_overlapping_draws() {
        let backend = NativeBackend::with_dpi(1.0);
        let mut surface = skia_safe::surfaces::raster_n32_premul((20, 20)).unwrap();
        surface.canvas().clear(skia_safe::Color::TRANSPARENT);
        let bounds = Rect::xywh(0.0, 0.0, 20.0, 20.0);
        backend.push_composite_layer(surface.canvas(), bounds, 0.5, ImageBlendMode::Normal);
        let red = skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 0.0, 0.0, 1.0), None);
        let blue = skia_safe::Paint::new(skia_safe::Color4f::new(0.0, 0.0, 1.0, 1.0), None);
        surface.canvas().draw_rect(to_sk_rect(bounds), &red);
        surface
            .canvas()
            .draw_rect(skia_safe::Rect::from_xywh(10.0, 0.0, 10.0, 20.0), &blue);
        surface.canvas().restore();

        let left = pixel(&mut surface, 5, 10);
        let overlap = pixel(&mut surface, 15, 10);
        assert!((120..=136).contains(&left.a()));
        assert!((120..=136).contains(&overlap.a()));
    }

    #[test]
    fn outer_layer_keeps_inner_blend_from_sampling_canvas_backdrop() {
        let backend = NativeBackend::with_dpi(1.0);
        let mut surface = skia_safe::surfaces::raster_n32_premul((20, 20)).unwrap();
        surface.canvas().clear(skia_safe::Color::BLUE);
        let bounds = Rect::xywh(0.0, 0.0, 20.0, 20.0);
        backend.push_composite_layer(surface.canvas(), bounds, 1.0, ImageBlendMode::Normal);
        backend.push_composite_layer(surface.canvas(), bounds, 1.0, ImageBlendMode::Multiply);
        let red = skia_safe::Paint::new(skia_safe::Color4f::new(1.0, 0.0, 0.0, 1.0), None);
        surface.canvas().draw_rect(to_sk_rect(bounds), &red);
        surface.canvas().restore();
        surface.canvas().restore();

        let center = pixel(&mut surface, 10, 10);
        assert!(
            center.r() > 240,
            "isolated fill should remain red: {center:?}"
        );
        assert!(
            center.b() < 16,
            "canvas blue must not enter blend: {center:?}"
        );
    }
}
