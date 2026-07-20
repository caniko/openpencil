//! Shared image-fill preview helpers for the Fill row and popover.

use crate::widgets::canvas_viewport_image::{image_source_bytes, note_pending_decode};
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
    if !cx.backend.image_decoded(id, bytes.as_ref()) {
        note_pending_decode(id);
        return true;
    }
    cx.backend
        .draw_image_with_options(rect, id, bytes.as_ref(), mode, adjustments, 1.0, 0.0);
    true
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

    #[test]
    fn preview_source_id_uses_the_canonical_paint_id() {
        let src = "data:image/png;base64,QUJD";
        assert_eq!(
            src_hash(src),
            jian_ops_schema::node::image_src::paint_image_id(src)
        );
    }
}
