use super::{contain_rect, to_sk_rect, NativeBackend};
use op_editor_ui::{ImageAdjustments, ImageDrawMode, Point2D, Rect};

const THUMB_CACHE_BYTE_BUDGET: usize = 4 * 1024 * 1024;
const THUMB_CACHE_MAX_ENTRIES: usize = 4096;
const THUMB_ENCODED_BYTE_LIMIT: usize = 4 * 1024;
const THUMB_MAX_EDGE: i32 = 32;

#[derive(Default)]
pub(super) struct ThumbCache {
    entries: std::collections::HashMap<u64, ThumbCacheEntry>,
    bytes: usize,
    tick: u64,
}

struct ThumbCacheEntry {
    image: Option<skia_safe::Image>,
    bytes: usize,
    last_used: u64,
}

/// Aspect-cover (`fill` / `crop`) `img_w × img_h` over `outer`, centered.
/// The result fully covers `outer`; the caller clips to `outer`.
pub(super) fn cover_rect(outer: Rect, img_w: f32, img_h: f32) -> Rect {
    if img_w <= 0.0 || img_h <= 0.0 || outer.size.x <= 0.0 || outer.size.y <= 0.0 {
        return outer;
    }
    let scale = (outer.size.x / img_w).max(outer.size.y / img_h);
    let w = img_w * scale;
    let h = img_h * scale;
    Rect {
        origin: Point2D::new(
            outer.origin.x + (outer.size.x - w) / 2.0,
            outer.origin.y + (outer.size.y - h) / 2.0,
        ),
        size: Point2D::new(w, h),
    }
}

impl NativeBackend {
    /// Decode and draw a bounded blur-up JPEG. Unlike full images, these
    /// <=32px thumbnails are explicitly allowed to decode during paint and
    /// live in a dedicated small LRU so they cannot displace sharp rasters.
    pub fn draw_image_thumb(
        &mut self,
        canvas: &skia_safe::Canvas,
        rect: Rect,
        id: u64,
        jpeg: &[u8],
    ) {
        let Some(image) = self.thumbnail_image(id, jpeg) else {
            return;
        };
        let dst = cover_rect(rect, image.width() as f32, image.height() as f32);
        let paint = skia_safe::Paint::default();
        let sampling = skia_safe::SamplingOptions::new(
            skia_safe::FilterMode::Linear,
            skia_safe::MipmapMode::None,
        );
        let save = canvas.save();
        canvas.clip_rect(to_sk_rect(rect), None, Some(true));
        canvas.draw_image_rect_with_sampling_options(
            &image,
            None,
            to_sk_rect(dst),
            sampling,
            &paint,
        );
        canvas.restore_to_count(save);
        super::image_diagnostics::record_successful_thumbnail_draw();
    }

    fn thumbnail_image(&mut self, id: u64, jpeg: &[u8]) -> Option<skia_safe::Image> {
        self.thumb_cache.tick = self.thumb_cache.tick.wrapping_add(1);
        let tick = self.thumb_cache.tick;
        if let Some(hit) = self.thumb_cache.entries.get_mut(&id) {
            hit.last_used = tick;
            return hit.image.clone();
        }

        let image = (jpeg.len() <= THUMB_ENCODED_BYTE_LIMIT)
            .then(|| skia_safe::Image::from_encoded(skia_safe::Data::new_copy(jpeg)))
            .flatten()
            // Dimensions come from encoded metadata and are checked before
            // Skia expands pixels on the paint thread. A highly compressible
            // large JPEG can otherwise fit under the 4 KiB payload bound.
            .filter(|lazy| {
                lazy.width() > 0
                    && lazy.height() > 0
                    && lazy.width() <= THUMB_MAX_EDGE
                    && lazy.height() <= THUMB_MAX_EDGE
            })
            .and_then(|lazy| lazy.make_raster_image(None, None));
        let bytes = image
            .as_ref()
            .map(|image| {
                (image.width().max(0) as usize)
                    .saturating_mul(image.height().max(0) as usize)
                    .saturating_mul(4)
            })
            .unwrap_or(0);
        self.thumb_cache.bytes = self.thumb_cache.bytes.saturating_add(bytes);
        self.thumb_cache.entries.insert(
            id,
            ThumbCacheEntry {
                image: image.clone(),
                bytes,
                last_used: tick,
            },
        );
        self.evict_thumbnails_over_budget();
        image
    }

    fn evict_thumbnails_over_budget(&mut self) {
        while self.thumb_cache.bytes > THUMB_CACHE_BYTE_BUDGET
            || self.thumb_cache.entries.len() > THUMB_CACHE_MAX_ENTRIES
        {
            let Some((&oldest, _)) = self
                .thumb_cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
            else {
                break;
            };
            if let Some(entry) = self.thumb_cache.entries.remove(&oldest) {
                self.thumb_cache.bytes = self.thumb_cache.bytes.saturating_sub(entry.bytes);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn thumb_cache_len(&self) -> usize {
        self.thumb_cache.entries.len()
    }

    #[cfg(test)]
    pub(crate) fn thumb_raster_cache_len(&self) -> usize {
        self.thumb_cache
            .entries
            .values()
            .filter(|entry| entry.image.is_some())
            .count()
    }

    /// Draw the image identified by `id`, aspect-fit + centered
    /// inside `rect`. Kept as the legacy/default image path for chat
    /// attachments and UI previews.
    pub fn draw_image(&mut self, canvas: &skia_safe::Canvas, rect: Rect, id: u64, encoded: &[u8]) {
        self.draw_image_with_mode(canvas, rect, id, encoded, ImageDrawMode::Fit);
    }

    /// Draw the image identified by `id` using the same placement
    /// modes as the TS renderer's image fill path.
    pub fn draw_image_with_mode(
        &mut self,
        canvas: &skia_safe::Canvas,
        rect: Rect,
        id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
    ) {
        self.draw_image_with_options(
            canvas,
            rect,
            id,
            encoded,
            mode,
            ImageAdjustments::default(),
            1.0,
            0.0,
        );
    }

    /// Draw the image identified by `id` using placement and
    /// adjustment controls from the image-fill popover.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_with_options(
        &mut self,
        canvas: &skia_safe::Canvas,
        rect: Rect,
        id: u64,
        encoded: &[u8],
        mode: ImageDrawMode,
        adjustments: ImageAdjustments,
        opacity: f32,
        corner_radius: f32,
    ) {
        self.draw_image_with_options_and_transform(
            canvas,
            rect,
            id,
            encoded,
            mode,
            adjustments,
            opacity,
            corner_radius,
            None,
        );
    }

    /// Draw an image with an optional Figma image-fill transform. The affine
    /// maps the node unit square into image UV; Skia image shaders expect the
    /// inverse mapping (image pixels into node-local coordinates).
    #[allow(clippy::too_many_arguments)]
    pub fn draw_image_with_options_and_transform(
        &mut self,
        canvas: &skia_safe::Canvas,
        rect: Rect,
        id: u64,
        _encoded: &[u8],
        mode: ImageDrawMode,
        adjustments: ImageAdjustments,
        opacity: f32,
        corner_radius: f32,
        transform: Option<[f32; 6]>,
    ) {
        let Some(image) = self.raster_image(id) else {
            return;
        };
        super::image_diagnostics::record_sharp_raster_hit();
        // Rounded image nodes clip the BITMAP, not just the
        // placeholder fill/stroke (TS `clipRRect` parity,
        // node-renderer.ts:1093-1104).
        let clip_round = corner_radius > 0.5;
        if clip_round {
            canvas.save();
            let rrect = skia_safe::RRect::new_rect_xy(
                skia_safe::Rect::from_xywh(rect.origin.x, rect.origin.y, rect.size.x, rect.size.y),
                corner_radius,
                corner_radius,
            );
            canvas.clip_rrect(rrect, skia_safe::ClipOp::Intersect, true);
        }
        let mut paint = skia_safe::Paint::default();
        paint.set_anti_alias(true);
        // Node-level opacity dims the raster (rasters carry no fill
        // colour to bake the opacity into at scene-build).
        paint.set_alpha_f(opacity.clamp(0.0, 1.0));
        if let Some(matrix) = image_adjustment_matrix(adjustments) {
            paint.set_color_filter(skia_safe::color_filters::matrix_row_major(&matrix, None));
        }

        // Figma commonly serializes UI-cropped paints as STRETCH plus an
        // affine transform (the Tesla 191×236 card does this). Apply the
        // transform before mode fallback so explicit CROP and transformed
        // STRETCH paints share Fill's exact sampling path.
        if let Some(local) = transform.and_then(|affine| {
            figma_image_local_matrix(rect, image.width() as f32, image.height() as f32, affine)
        }) {
            let local_matrix = skia_safe::Matrix::new_all(
                local[0], local[1], local[2], local[3], local[4], local[5], 0.0, 0.0, 1.0,
            );
            let sampling = skia_safe::SamplingOptions::new(
                skia_safe::FilterMode::Linear,
                skia_safe::MipmapMode::None,
            );
            if let Some(shader) = image.to_shader(
                (skia_safe::TileMode::Decal, skia_safe::TileMode::Decal),
                sampling,
                &local_matrix,
            ) {
                paint.set_shader(shader);
                let save = canvas.save();
                canvas.clip_rect(to_sk_rect(rect), None, Some(true));
                canvas.draw_rect(to_sk_rect(rect), &paint);
                canvas.restore_to_count(save);
                if clip_round {
                    canvas.restore();
                }
                return;
            }
        }

        match mode {
            ImageDrawMode::Fit => {
                let dst = contain_rect(rect, image.width() as f32, image.height() as f32);
                canvas.draw_image_rect(&image, None, to_sk_rect(dst), &paint);
            }
            ImageDrawMode::Stretch => {
                canvas.draw_image_rect(&image, None, to_sk_rect(rect), &paint);
            }
            ImageDrawMode::Tile => {
                draw_tiled_image(canvas, rect, &image, &paint);
            }
            ImageDrawMode::Fill | ImageDrawMode::Crop => {
                let dst = cover_rect(rect, image.width() as f32, image.height() as f32);
                let save = canvas.save();
                canvas.clip_rect(to_sk_rect(rect), None, Some(true));
                canvas.draw_image_rect(&image, None, to_sk_rect(dst), &paint);
                canvas.restore_to_count(save);
            }
        }
        if clip_round {
            canvas.restore();
        }
    }
}

/// Build Skia's image-shader local matrix from a Figma fill transform.
///
/// Figma maps normalized node coordinates to normalized image UV. A Skia
/// image shader's local matrix maps image pixels to local coordinates, so the
/// order is `node_rect * inverse(figma) * inverse(image_dimensions)`.
pub(super) fn figma_image_local_matrix(
    rect: Rect,
    img_w: f32,
    img_h: f32,
    transform: [f32; 6],
) -> Option<[f32; 6]> {
    if rect.size.x <= 0.0
        || rect.size.y <= 0.0
        || img_w <= 0.0
        || img_h <= 0.0
        || !transform.iter().all(|v| v.is_finite())
    {
        return None;
    }
    let [a, b, tx, c, d, ty] = transform;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() <= f32::EPSILON {
        return None;
    }
    let inv_det = 1.0 / det;
    let ia = d * inv_det;
    let ib = -b * inv_det;
    let ic = -c * inv_det;
    let id = a * inv_det;
    let itx = (b * ty - d * tx) * inv_det;
    let ity = (c * tx - a * ty) * inv_det;
    Some([
        rect.size.x * ia / img_w,
        rect.size.x * ib / img_h,
        rect.origin.x + rect.size.x * itx,
        rect.size.y * ic / img_w,
        rect.size.y * id / img_h,
        rect.origin.y + rect.size.y * ity,
    ])
}

pub(super) fn image_adjustment_matrix(adjustments: ImageAdjustments) -> Option<[f32; 20]> {
    let exp = adjustments.exposure / 100.0;
    let con = adjustments.contrast / 100.0;
    let sat = adjustments.saturation / 100.0;
    let temp = adjustments.temperature / 100.0;
    let tint = adjustments.tint / 100.0;
    let hi = adjustments.highlights / 100.0;
    let sh = adjustments.shadows / 100.0;
    if adjustments.is_neutral() {
        return None;
    }

    let e = 1.0 + exp * 1.5;
    let c = 1.0 + con;
    let c_off = 0.5 * (1.0 - c);
    let s = 1.0 + sat;
    let (lr, lg, lb) = (0.2126, 0.7152, 0.0722);
    let sr = (1.0 - s) * lr;
    let sg = (1.0 - s) * lg;
    let sb = (1.0 - s) * lb;
    let f = c * e;
    let off_r = c_off + temp * 0.15 + (hi + sh * 0.5) * 0.1;
    let off_g = c_off + tint * 0.15 + (hi + sh * 0.5) * 0.1;
    let off_b = c_off - temp * 0.15 + (hi + sh * 0.5) * 0.1;

    Some([
        f * (sr + s),
        f * sg,
        f * sb,
        0.0,
        off_r,
        f * sr,
        f * (sg + s),
        f * sb,
        0.0,
        off_g,
        f * sr,
        f * sg,
        f * (sb + s),
        0.0,
        off_b,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
    ])
}

fn draw_tiled_image(
    canvas: &skia_safe::Canvas,
    rect: Rect,
    image: &skia_safe::Image,
    paint: &skia_safe::Paint,
) {
    let tile_w = image.width() as f32;
    let tile_h = image.height() as f32;
    if tile_w <= 0.0 || tile_h <= 0.0 || rect.size.x <= 0.0 || rect.size.y <= 0.0 {
        return;
    }

    let save = canvas.save();
    canvas.clip_rect(to_sk_rect(rect), None, Some(true));

    let mut start_x = rect.origin.x + (rect.size.x - tile_w) / 2.0;
    let mut start_y = rect.origin.y + (rect.size.y - tile_h) / 2.0;
    while start_x > rect.origin.x {
        start_x -= tile_w;
    }
    while start_y > rect.origin.y {
        start_y -= tile_h;
    }

    let right = rect.origin.x + rect.size.x;
    let bottom = rect.origin.y + rect.size.y;
    let mut y = start_y;
    while y < bottom {
        let mut x = start_x;
        while x < right {
            canvas.draw_image_rect(
                image,
                None,
                to_sk_rect(Rect::xywh(x, y, tile_w, tile_h)),
                paint,
            );
            x += tile_w;
        }
        y += tile_h;
    }

    canvas.restore_to_count(save);
}

#[cfg(test)]
mod crop_transform_tests {
    use super::*;

    fn render(mode: ImageDrawMode, transform: Option<[f32; 6]>) -> Vec<u8> {
        let mut source = skia_safe::surfaces::raster_n32_premul((4, 2)).unwrap();
        source.canvas().clear(skia_safe::Color::RED);
        let blue = skia_safe::Paint::new(skia_safe::Color4f::new(0.0, 0.0, 1.0, 1.0), None);
        source
            .canvas()
            .draw_rect(skia_safe::Rect::from_xywh(2.0, 0.0, 2.0, 2.0), &blue);

        let mut backend = NativeBackend::with_dpi(1.0);
        backend.install_raster_image(1, source.image_snapshot());
        let mut target = skia_safe::surfaces::raster_n32_premul((20, 20)).unwrap();
        target.canvas().clear(skia_safe::Color::TRANSPARENT);
        backend.draw_image_with_options_and_transform(
            target.canvas(),
            Rect::xywh(0.0, 0.0, 20.0, 20.0),
            1,
            &[],
            mode,
            ImageAdjustments::default(),
            1.0,
            0.0,
            transform,
        );
        target
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .unwrap()
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn crop_uses_figma_transform_identically_to_fill() {
        let transform = Some([0.5, 0.0, 0.0, 0.0, 1.0, 0.0]);
        let transformed_crop = render(ImageDrawMode::Crop, transform);
        assert_eq!(
            transformed_crop,
            render(ImageDrawMode::Fill, transform),
            "Crop and Fill must share Figma's affine sampling path"
        );
        assert_ne!(
            transformed_crop,
            render(ImageDrawMode::Crop, None),
            "the parity assertion must prove the transform affects pixels"
        );
    }
}
