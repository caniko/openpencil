//! Shared image-fill preview helpers for the Fill row and popover.

use crate::widgets::canvas_viewport_image::{
    image_source_bytes, note_pending_decode, required_raster_edge,
};
use crate::widgets::PaintCx;
use crate::{ImageAdjustments, ImageDrawMode, Rect};

pub(crate) fn paint_image_preview(
    cx: &mut PaintCx<'_>,
    rect: Rect,
    src: &str,
    summary: &op_editor_core::ImageFillSummary,
) -> bool {
    cx.backend.save();
    cx.backend.clip_rect(rect);
    let has_source = paint_image_source(
        cx,
        rect,
        src,
        mode_to_draw_mode(summary.mode),
        summary_adjustments(summary),
    );
    cx.backend.restore();
    has_source
}

/// Paint an image source that participates in the shared asynchronous decode
/// pipeline. A resolved source returns `true` while its raster is pending so
/// callers keep their neutral placeholder instead of flashing missing-image UI.
pub(crate) fn paint_image_source(
    cx: &mut PaintCx<'_>,
    rect: Rect,
    src: &str,
    mode: ImageDrawMode,
    adjustments: ImageAdjustments,
) -> bool {
    let id = src_hash(src);
    let Some(bytes) = image_source_bytes(src, id) else {
        return false;
    };
    let max_edge_px = required_raster_edge(rect, cx.backend.dpi_scale());
    let sharp_enough = cx.backend.image_decoded(id, bytes.as_ref(), max_edge_px);
    if !sharp_enough {
        note_pending_decode(id, max_edge_px);
    }
    // Refinement must not blank a preview that already has a coarser raster.
    // Match the canvas path: keep drawing the resident image while the larger
    // decode requested above completes in the background.
    let resident = !sharp_enough && cx.backend.image_resident(id);
    if preview_raster_ready(sharp_enough, resident) {
        cx.backend
            .draw_image_with_options(rect, id, bytes.as_ref(), mode, adjustments, 1.0, 0.0);
    }
    true
}

fn preview_raster_ready(sharp_enough: bool, resident: bool) -> bool {
    sharp_enough || resident
}

fn summary_adjustments(summary: &op_editor_core::ImageFillSummary) -> ImageAdjustments {
    ImageAdjustments {
        exposure: summary.exposure,
        contrast: summary.contrast,
        saturation: summary.saturation,
        temperature: summary.temperature,
        tint: summary.tint,
        highlights: summary.highlights,
        shadows: summary.shadows,
    }
}

fn mode_to_draw_mode(mode: op_editor_core::ImageFillMode) -> ImageDrawMode {
    match mode {
        op_editor_core::ImageFillMode::Fill => ImageDrawMode::Fill,
        op_editor_core::ImageFillMode::Fit => ImageDrawMode::Fit,
        op_editor_core::ImageFillMode::Crop => ImageDrawMode::Crop,
        op_editor_core::ImageFillMode::Tile => ImageDrawMode::Tile,
    }
}

fn src_hash(src: &str) -> u64 {
    jian_ops_schema::node::image_src::paint_image_id(src)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::canvas_viewport_image::{
        lock_decode_registry_for_tests, mark_decode_done, take_pending_decodes, PendingDecode,
    };
    use crate::widgets::property_panel_test_support::CountingBackend;

    #[test]
    fn preview_source_id_uses_the_canonical_paint_id() {
        let src = "data:image/png;base64,QUJD";
        assert_eq!(
            src_hash(src),
            jian_ops_schema::node::image_src::paint_image_id(src)
        );
    }

    #[test]
    fn resident_raster_remains_visible_while_refinement_is_pending() {
        let _guard = lock_decode_registry_for_tests();
        let src = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=";
        let id = src_hash(src);
        let mut backend = CountingBackend {
            image_decode_ready: Some(false),
            image_resident_ready: Some(true),
            ..Default::default()
        };

        let has_source = paint_image_source(
            &mut PaintCx {
                backend: &mut backend,
            },
            Rect::xywh(0.0, 0.0, 20.0, 20.0),
            src,
            ImageDrawMode::Crop,
            ImageAdjustments::default(),
        );

        assert!(has_source);
        assert_eq!(backend.images.len(), 1, "resident raster stays visible");
        assert_eq!(backend.images[0].1, id);
        assert_eq!(backend.image_modes, vec![ImageDrawMode::Crop]);
        assert_eq!(
            take_pending_decodes(8),
            vec![PendingDecode {
                id,
                max_edge_px: 64,
            }],
            "a sharper raster is still queued"
        );
        mark_decode_done(id);
    }
}
