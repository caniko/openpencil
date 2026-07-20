use crate::layout_scene::{stable_image_source_id, SceneNode};
use crate::widgets::canvas_viewport_overlay::scaled_non_uniform_corner_radii;
use crate::widgets::PaintCx;
use crate::Rect;
use crate::{Color, Point2D};
use std::sync::{Arc, Mutex, OnceLock};

mod decode_registry;

#[cfg(test)]
pub(crate) use decode_registry::lock_decode_registry_for_tests;
pub use decode_registry::{
    has_pending_decodes, mark_decode_done, mark_decode_failed, note_pending_decode,
    pending_decode_count, take_pending_decodes,
};

/// Byte budget for cached encoded-image payloads. The cache must hold
/// the working set of an image-heavy document (a Figma import easily
/// carries hundreds of bitmaps) — a small entry cap caused every frame
/// to re-run the base64 decode for whatever fell off the end. Budget
/// by bytes instead: wasm gets a smaller budget to respect the 32-bit
/// heap.
#[cfg(target_arch = "wasm32")]
const DATA_URL_CACHE_BYTE_BUDGET: usize = 96 * 1024 * 1024;
#[cfg(not(target_arch = "wasm32"))]
const DATA_URL_CACHE_BYTE_BUDGET: usize = 256 * 1024 * 1024;
/// Safety cap on entry count — bounds accumulation of tiny payloads.
const DATA_URL_CACHE_MAX_ENTRIES: usize = 4096;

struct DataUrlCache {
    entries: std::collections::HashMap<u64, (Arc<[u8]>, u64)>,
    /// Sum of payload lengths across `entries`.
    bytes: usize,
    /// Monotonic use counter driving LRU eviction.
    tick: u64,
}

impl DataUrlCache {
    fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            bytes: 0,
            tick: 0,
        }
    }

    fn get(&mut self, id: u64) -> Option<Arc<[u8]>> {
        self.tick += 1;
        let tick = self.tick;
        let (bytes, last_used) = self.entries.get_mut(&id)?;
        *last_used = tick;
        Some(Arc::clone(bytes))
    }

    /// Presence check that does not refresh the entry's LRU position.
    fn contains(&self, id: u64) -> bool {
        self.entries.contains_key(&id)
    }

    fn insert(&mut self, id: u64, bytes: Arc<[u8]>) {
        self.tick += 1;
        let tick = self.tick;
        if let Some((old, _)) = self.entries.insert(id, (bytes, tick)) {
            self.bytes = self.bytes.saturating_sub(old.len());
        }
        if let Some((fresh, _)) = self.entries.get(&id) {
            self.bytes += fresh.len();
        }
        self.evict_over(DATA_URL_CACHE_BYTE_BUDGET, DATA_URL_CACHE_MAX_ENTRIES);
    }

    /// Evict least-recently-used entries until the cache fits both
    /// `byte_budget` and `max_entries`. Separated from `insert` so
    /// tests can exercise eviction with small budgets.
    fn evict_over(&mut self, byte_budget: usize, max_entries: usize) {
        while self.bytes > byte_budget || self.entries.len() > max_entries {
            let Some((&oldest, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, last_used))| *last_used)
            else {
                break;
            };
            if let Some((evicted, _)) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(evicted.len());
            }
        }
    }
}

static DATA_URL_CACHE: OnceLock<Mutex<DataUrlCache>> = OnceLock::new();

fn data_url_cache() -> &'static Mutex<DataUrlCache> {
    DATA_URL_CACHE.get_or_init(|| Mutex::new(DataUrlCache::new()))
}

/// Most remote misses a single frame can queue before the rest wait
/// for the next paint (the host drains a few per frame anyway).
const REMOTE_MISS_QUEUE_CAP: usize = 32;
/// Negative-cache bound — failed source ids beyond this evict oldest
/// (an evicted failure may refetch once; it just fails again).
const REMOTE_FAILED_CAP: usize = 256;

/// Paint-time registry for remote (`http(s)`) image sources. The
/// platform-free painter can only RECORD a cache miss here; a host
/// with network access (desktop) drains [`take_remote_image_requests`]
/// per frame, fetches, and stores the bytes back into the shared
/// data-url cache via [`store_remote_image_bytes`]. Hosts without a
/// fetch path (web, for now) simply never drain — the bounded queue
/// caps memory and the placeholder keeps painting.
#[derive(Default)]
struct RemoteImageRegistry {
    /// Misses recorded by paint, in first-seen order, awaiting a host.
    pending: std::collections::VecDeque<(u64, String)>,
    /// Ids a host has taken (in-flight) — paint must not re-queue.
    requested: std::collections::HashSet<u64>,
    /// Ids whose fetch failed — permanent placeholder, never re-queued.
    failed: std::collections::HashSet<u64>,
    failed_order: std::collections::VecDeque<u64>,
}

static REMOTE_IMAGES: OnceLock<Mutex<RemoteImageRegistry>> = OnceLock::new();

fn remote_images() -> &'static Mutex<RemoteImageRegistry> {
    REMOTE_IMAGES.get_or_init(|| Mutex::new(RemoteImageRegistry::default()))
}

/// Record a paint-time miss for a remote image source. Deduplicated
/// against already-queued / in-flight / failed ids, and bounded — a
/// dropped miss is simply re-noted on a later paint.
pub fn note_remote_image_miss(id: u64, url: &str) {
    let Ok(mut reg) = remote_images().lock() else {
        return;
    };
    if reg.failed.contains(&id)
        || reg.requested.contains(&id)
        || reg.pending.iter().any(|(queued, _)| *queued == id)
        || reg.pending.len() >= REMOTE_MISS_QUEUE_CAP
    {
        return;
    }
    reg.pending.push_back((id, url.to_string()));
}

/// Host-side drain: pop up to `max` recorded misses and mark them
/// in-flight so paint stops re-queuing them while the fetch runs.
pub fn take_remote_image_requests(max: usize) -> Vec<(u64, String)> {
    let Ok(mut reg) = remote_images().lock() else {
        return Vec::new();
    };
    let take = max.min(reg.pending.len());
    let mut out = Vec::with_capacity(take);
    for _ in 0..take {
        let Some((id, url)) = reg.pending.pop_front() else {
            break;
        };
        reg.requested.insert(id);
        out.push((id, url));
    }
    out
}

/// Host-side store: put fetched image bytes into the SAME cache the
/// data-url decode path uses, keyed by the source id — the next paint
/// hits the cache and draws the bitmap. Empty bytes are treated as a
/// failed fetch. Clears the in-flight mark so a later cache eviction
/// re-queues (and refetches) the source instead of sticking on the
/// placeholder forever.
pub fn store_remote_image_bytes(id: u64, bytes: Vec<u8>) {
    if bytes.is_empty() {
        mark_remote_image_failed(id);
        return;
    }
    if let Ok(mut cache) = data_url_cache().lock() {
        cache.insert(id, Arc::from(bytes.into_boxed_slice()));
    }
    if let Ok(mut reg) = remote_images().lock() {
        reg.requested.remove(&id);
    }
}

/// Host-side negative cache: a failed fetch keeps the placeholder
/// permanently — paint never re-queues a failed id, so a dead URL
/// doesn't refetch every frame.
pub fn mark_remote_image_failed(id: u64) {
    let Ok(mut reg) = remote_images().lock() else {
        return;
    };
    reg.requested.remove(&id);
    if reg.failed.insert(id) {
        reg.failed_order.push_back(id);
        while reg.failed.len() > REMOTE_FAILED_CAP {
            match reg.failed_order.pop_front() {
                Some(oldest) => {
                    reg.failed.remove(&oldest);
                }
                None => break,
            }
        }
    }
}

/// Whether the shared byte cache holds an entry for `id` — lets hosts
/// (and their tests) verify a stored fetch result without re-running
/// a paint pass.
pub fn has_cached_image_bytes(id: u64) -> bool {
    data_url_cache()
        .lock()
        .map(|cache| cache.contains(id))
        .unwrap_or(false)
}

pub fn cached_bytes_for(id: u64) -> Option<Arc<[u8]>> {
    data_url_cache().lock().ok()?.get(id)
}

/// Whether paint has recorded remote misses no host has taken yet —
/// the desktop host uses this to keep its event loop waking until the
/// queue is drained.
pub fn has_pending_remote_image_requests() -> bool {
    remote_images()
        .lock()
        .map(|reg| !reg.pending.is_empty())
        .unwrap_or(false)
}

fn is_remote_image_url(src: &str) -> bool {
    let lower = src.get(..8).unwrap_or(src).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// Resolve the encoded image bytes for `src`: cache first (covers both
/// decoded data URLs and host-fetched remote bytes), then an inline
/// `data:` decode; a remote `http(s)` miss is recorded for the host
/// fetcher and paints the placeholder this frame.
pub(crate) fn image_source_bytes(src: &str, image_src_id: u64) -> Option<Arc<[u8]>> {
    let id = if image_src_id == 0 {
        stable_image_source_id(src)
    } else {
        image_src_id
    };
    if let Ok(mut cache) = data_url_cache().lock() {
        if let Some(bytes) = cache.get(id) {
            return Some(bytes);
        }
    }

    if is_remote_image_url(src) {
        note_remote_image_miss(id, src);
        return None;
    }
    let decoded = decode_data_url_bytes(src)?;
    if let Ok(mut cache) = data_url_cache().lock() {
        cache.insert(id, decoded.clone());
    }
    Some(decoded)
}

fn decode_data_url_bytes(src: &str) -> Option<Arc<[u8]>> {
    let after_scheme = src.strip_prefix("data:")?;
    let comma = after_scheme.find(',')?;
    let meta = &after_scheme[..comma];
    let payload = &after_scheme[comma + 1..];
    if !meta.contains(";base64") {
        return None;
    }

    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine as _;
    let decoded = if payload.bytes().any(|b| b.is_ascii_whitespace()) {
        let clean: Vec<u8> = payload
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .collect();
        B64.decode(clean.as_slice()).ok()?
    } else {
        B64.decode(payload.as_bytes()).ok()?
    };
    Some(Arc::from(decoded.into_boxed_slice()))
}

/// Paint a raster image inside `world_rect`. The source bytes and decoded
/// backend image are both cached, so repeated canvas paints do not re-decode
/// data URLs while importing or panning a Figma-heavy document.
pub(super) fn paint_image_node(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    world_rect: Rect,
    zoom: f32,
    src: &str,
) {
    let bytes = image_source_bytes(src, node.image_src_id);
    let id = if node.image_src_id == 0 {
        stable_image_source_id(src)
    } else {
        node.image_src_id
    };
    let decode_ready = bytes
        .as_deref()
        .is_some_and(|encoded| cx.backend.image_decoded(id, encoded));
    if bytes.is_some() && !decode_ready {
        note_pending_decode(id);
    }
    let r = node.corner_radius * zoom;
    let use_round = r > 0.5;
    let per_corner = scaled_non_uniform_corner_radii(node, zoom);
    if !decode_ready {
        // Keep every placeholder layer inside the image node's authored
        // corners. The thumb is deliberately first: a translucent node fill
        // can tint it before the neutral placeholder art lands on top.
        if let Some(radii) = per_corner {
            cx.backend.save();
            cx.backend.clip_round_rect_per_corner(world_rect, radii);
        } else if use_round {
            cx.backend.save();
            cx.backend.clip_round_rect(world_rect, r);
        }
        if let Some(thumb) = jian_ops_schema::image_thumbs::thumb_for(id) {
            cx.backend.draw_image_thumb(world_rect, id, thumb.as_ref());
        }
        if let Some(fill) = node.fill {
            if let Some(radii) = per_corner {
                cx.backend
                    .fill_round_rect_per_corner(world_rect, radii, fill);
            } else if use_round {
                cx.backend.fill_round_rect(world_rect, r, fill);
            } else {
                cx.backend.fill_rect(world_rect, fill);
            }
        }
        // Missing / undecodable / still-fetching source — dashed border +
        // a centred picture glyph so the node reads as "image placeholder",
        // not a plain grey box. Also the terminal look for a FAILED image
        // search (`placeholder://` sentinel src). Placeholder art adapts to
        // the slot's own fill: a readable mid-grey on light slots, a lifted
        // grey on dark ones — one fixed grey was invisible on dark designs.
        let luminance = node
            .fill
            .map(|fill| 0.299 * fill.r + 0.587 * fill.g + 0.114 * fill.b)
            .unwrap_or(0.9);
        let placeholder = if luminance >= 0.5 {
            Color {
                r: 0.45,
                g: 0.48,
                b: 0.53,
                a: 1.0,
            }
        } else {
            Color {
                r: 0.55,
                g: 0.58,
                b: 0.64,
                a: 1.0,
            }
        };
        super::canvas_viewport::paint_dashed_rect(cx, world_rect, placeholder, 1.0);
        paint_picture_glyph(cx, world_rect, placeholder);
        if per_corner.is_some() || use_round {
            cx.backend.restore();
        }
    }
    if decode_ready {
        let bytes = bytes.expect("decode-ready image has encoded bytes");
        if let Some(radii) = per_corner {
            cx.backend.save();
            cx.backend.clip_round_rect_per_corner(world_rect, radii);
        }
        cx.backend.draw_image_with_options_and_transform(
            world_rect,
            id,
            bytes.as_ref(),
            node.image_fit.to_draw_mode(),
            node.image_adjustments,
            node.opacity,
            if per_corner.is_none() && use_round {
                r
            } else {
                0.0
            },
            node.image_transform,
        );
        if per_corner.is_some() {
            cx.backend.restore();
        }
    }
    if let Some(stroke) = node.stroke {
        let width = stroke.width * zoom;
        if let Some(radii) = per_corner {
            cx.backend
                .stroke_round_rect_per_corner(world_rect, radii, stroke.color, width);
        } else if use_round {
            cx.backend
                .stroke_round_rect(world_rect, r, stroke.color, width);
        } else {
            cx.backend.stroke_rect(world_rect, stroke.color, width);
        }
    }
}

#[cfg(test)]
fn clear_data_url_cache_for_tests() {
    if let Ok(mut cache) = data_url_cache().lock() {
        *cache = DataUrlCache::new();
    }
}

#[cfg(test)]
fn data_url_cache_len_for_tests() -> usize {
    data_url_cache()
        .lock()
        .map(|cache| cache.entries.len())
        .unwrap_or(0)
}

#[cfg(test)]
fn clear_remote_registry_for_tests() {
    if let Ok(mut reg) = remote_images().lock() {
        *reg = RemoteImageRegistry::default();
    }
}

/// Minimal "picture" glyph — frame + sun + mountain strokes scaled
/// into the centre of `rect` (24px reference art, like the lucide
/// `image` icon but hand-stroked to avoid an icon-catalog dependency
/// in the paint path).
fn paint_picture_glyph(cx: &mut PaintCx<'_>, rect: Rect, color: Color) {
    let size = (rect.size.x.min(rect.size.y) * 0.4).clamp(12.0, 48.0);
    let cx0 = rect.origin.x + rect.size.x / 2.0 - size / 2.0;
    let cy0 = rect.origin.y + rect.size.y / 2.0 - size / 2.0;
    let w = 1.5;
    // Frame.
    cx.backend.stroke_round_rect(
        Rect {
            origin: Point2D::new(cx0, cy0),
            size: Point2D::new(size, size),
        },
        size * 0.12,
        color,
        w,
    );
    // Sun — small circle approximated by a tight round-rect.
    let sun = size * 0.16;
    cx.backend.stroke_round_rect(
        Rect {
            origin: Point2D::new(cx0 + size * 0.2, cy0 + size * 0.2),
            size: Point2D::new(sun, sun),
        },
        sun / 2.0,
        color,
        w,
    );
    // Mountain.
    cx.backend.stroke_line(
        Point2D::new(cx0 + size * 0.12, cy0 + size * 0.85),
        Point2D::new(cx0 + size * 0.45, cy0 + size * 0.45),
        color,
        w,
    );
    cx.backend.stroke_line(
        Point2D::new(cx0 + size * 0.45, cy0 + size * 0.45),
        Point2D::new(cx0 + size * 0.7, cy0 + size * 0.7),
        color,
        w,
    );
    cx.backend.stroke_line(
        Point2D::new(cx0 + size * 0.7, cy0 + size * 0.7),
        Point2D::new(cx0 + size * 0.88, cy0 + size * 0.52),
        color,
        w,
    );
}

#[cfg(test)]
mod blur_up_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_scene::NodeKind;
    use crate::{ImageAdjustments, ImageDrawMode, RenderBackend, TextLayout};

    /// The caches + registry are process-wide statics; serialize the
    /// tests that mutate them so parallel test threads don't race.
    fn lock_statics() -> std::sync::MutexGuard<'static, ()> {
        let guard = lock_decode_registry_for_tests();
        clear_data_url_cache_for_tests();
        clear_remote_registry_for_tests();
        guard
    }

    #[derive(Default)]
    struct ImageRadiusCaptureBackend {
        clips: Vec<[f32; 4]>,
        image_corner_radii: Vec<f32>,
        image_transforms: Vec<Option<[f32; 6]>>,
        decode_ready: Option<bool>,
    }

    impl RenderBackend for ImageRadiusCaptureBackend {
        fn begin_frame(&mut self) {}
        fn end_frame(&mut self) {}
        fn fill_rect(&mut self, _: Rect, _: Color) {}
        fn stroke_rect(&mut self, _: Rect, _: Color, _: f32) {}
        fn draw_text(&mut self, _: &TextLayout, _: Point2D) {}
        fn clip_rect(&mut self, _: Rect) {}
        fn clip_round_rect_per_corner(&mut self, _: Rect, radii: [f32; 4]) {
            self.clips.push(radii);
        }
        fn save(&mut self) {}
        fn restore(&mut self) {}
        fn translate(&mut self, _: Point2D) {}
        fn stroke_line(&mut self, _: Point2D, _: Point2D, _: Color, _: f32) {}
        fn fill_round_rect(&mut self, _: Rect, _: f32, _: Color) {}
        fn stroke_round_rect(&mut self, _: Rect, _: f32, _: Color, _: f32) {}
        fn stroke_svg_path(&mut self, _: &str, _: Point2D, _: f32, _: Color, _: f32) {}
        fn image_decoded(&mut self, _: u64, _: &[u8]) -> bool {
            self.decode_ready.unwrap_or(true)
        }
        fn draw_image_with_options_and_transform(
            &mut self,
            _: Rect,
            _: u64,
            _: &[u8],
            _: ImageDrawMode,
            _: ImageAdjustments,
            _: f32,
            corner_radius: f32,
            transform: Option<[f32; 6]>,
        ) {
            self.image_corner_radii.push(corner_radius);
            self.image_transforms.push(transform);
        }
        fn resize(&mut self, _: u32, _: u32) {}
        fn dpi_scale(&self) -> f32 {
            1.0
        }
    }

    #[test]
    fn image_fill_uses_per_corner_clip_instead_of_scalar_radius() {
        let _guard = lock_statics();
        let mut node = SceneNode::leaf("image", NodeKind::Rect);
        node.corner_radius = 8.0;
        node.corner_radii = Some([8.0, 0.0, 8.0, 0.0]);
        let mut backend = ImageRadiusCaptureBackend::default();

        paint_image_node(
            &mut PaintCx {
                backend: &mut backend,
            },
            &node,
            Rect::xywh(0.0, 0.0, 100.0, 50.0),
            1.0,
            "data:image/png;base64,QUJD",
        );

        assert_eq!(backend.clips, vec![[8.0, 0.0, 8.0, 0.0]]);
        assert_eq!(backend.image_corner_radii, vec![0.0]);
        assert_eq!(backend.image_transforms, vec![None]);
    }

    #[test]
    fn image_fill_forwards_figma_transform_to_backend() {
        let _guard = lock_statics();
        let transform = [0.9999718, 0.0, 0.00001408411, 0.0, 0.602706, 0.12054121];
        let mut node = SceneNode::leaf("image", NodeKind::Rect);
        node.image_transform = Some(transform);
        let mut backend = ImageRadiusCaptureBackend::default();

        paint_image_node(
            &mut PaintCx {
                backend: &mut backend,
            },
            &node,
            Rect::xywh(0.0, 0.0, 375.0, 490.0),
            1.0,
            "data:image/png;base64,QUJD",
        );

        assert_eq!(backend.image_transforms, vec![Some(transform)]);
    }

    #[test]
    fn data_url_cache_reuses_decoded_bytes() {
        let _guard = lock_statics();
        let src = "data:image/png;base64,QUJD";

        let first = image_source_bytes(src, 7).expect("first decode");
        assert_eq!(first.as_ref(), b"ABC");
        assert_eq!(data_url_cache_len_for_tests(), 1);

        let second = image_source_bytes(src, 7).expect("cached decode");
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert_eq!(data_url_cache_len_for_tests(), 1);
    }

    #[test]
    fn undecoded_image_is_queued_without_drawing_encoded_bytes() {
        let _guard = lock_statics();
        let mut node = SceneNode::leaf("image", NodeKind::Rect);
        node.image_src_id = 777;
        let mut backend = ImageRadiusCaptureBackend {
            decode_ready: Some(false),
            ..Default::default()
        };

        paint_image_node(
            &mut PaintCx {
                backend: &mut backend,
            },
            &node,
            Rect::xywh(0.0, 0.0, 100.0, 50.0),
            1.0,
            "data:image/png;base64,QUJD",
        );

        assert!(backend.image_transforms.is_empty());
        assert_eq!(take_pending_decodes(8), vec![777]);
        assert_eq!(cached_bytes_for(777).as_deref(), Some(b"ABC".as_slice()));
    }

    /// Eviction is LRU by byte budget: a payload set over budget must
    /// drop the least-recently-USED entry, not the least-recently
    /// inserted one — with hundreds of images painting every frame,
    /// FIFO eviction of a still-visible image re-decodes it each paint.
    #[test]
    fn data_url_cache_evicts_least_recently_used_over_byte_budget() {
        let _guard = lock_statics();
        let payload = |b: u8| Arc::from(vec![b; 8].into_boxed_slice());
        let mut cache = DataUrlCache::new();
        cache.insert(1, payload(1));
        cache.insert(2, payload(2));
        cache.insert(3, payload(3));
        assert_eq!(cache.bytes, 24);
        // Touch id 1 so id 2 becomes least-recently-used.
        assert!(cache.get(1).is_some());
        cache.evict_over(16, usize::MAX);
        assert!(cache.contains(1), "recently used entry survives");
        assert!(!cache.contains(2), "LRU entry is evicted");
        assert!(cache.contains(3));
        assert_eq!(cache.bytes, 16, "byte accounting tracks eviction");
        // Entry cap applies independently of the byte budget.
        cache.evict_over(usize::MAX, 1);
        assert_eq!(cache.entries.len(), 1);
        // Re-inserting an existing id replaces its bytes, not doubles.
        let survivor = *cache.entries.keys().next().expect("one entry");
        cache.insert(survivor, Arc::from(vec![9u8; 4].into_boxed_slice()));
        assert_eq!(cache.bytes, 4, "replacement swaps byte accounting");
    }

    #[test]
    fn remote_miss_is_queued_once_and_drained_by_the_host() {
        let _guard = lock_statics();
        let url = "https://example.com/cat.png";

        // First paint records the miss; repeated paints don't dup it.
        assert!(image_source_bytes(url, 101).is_none());
        assert!(image_source_bytes(url, 101).is_none());
        assert!(has_pending_remote_image_requests());
        let taken = take_remote_image_requests(8);
        assert_eq!(taken, vec![(101, url.to_string())]);
        assert!(!has_pending_remote_image_requests());

        assert!(image_source_bytes(url, 101).is_none());
        assert!(take_remote_image_requests(8).is_empty());
    }

    #[test]
    fn stored_remote_bytes_hit_the_shared_cache_on_next_paint() {
        let _guard = lock_statics();
        let url = "https://example.com/dog.png";

        assert!(image_source_bytes(url, 102).is_none());
        let _ = take_remote_image_requests(8);
        store_remote_image_bytes(102, b"PNGBYTES".to_vec());

        let bytes = image_source_bytes(url, 102).expect("cache hit after store");
        assert_eq!(bytes.as_ref(), b"PNGBYTES");
        assert!(!has_pending_remote_image_requests());
    }

    #[test]
    fn failed_fetch_is_negative_cached_and_never_requeued() {
        let _guard = lock_statics();
        let url = "https://example.com/404.png";

        assert!(image_source_bytes(url, 103).is_none());
        let _ = take_remote_image_requests(8);
        mark_remote_image_failed(103);

        // Paint keeps missing but the miss never re-queues.
        assert!(image_source_bytes(url, 103).is_none());
        assert!(!has_pending_remote_image_requests());
        assert!(take_remote_image_requests(8).is_empty());
    }

    #[test]
    fn empty_stored_bytes_count_as_failure() {
        let _guard = lock_statics();
        let url = "https://example.com/empty.png";

        assert!(image_source_bytes(url, 104).is_none());
        let _ = take_remote_image_requests(8);
        store_remote_image_bytes(104, Vec::new());

        assert!(image_source_bytes(url, 104).is_none());
        assert!(!has_pending_remote_image_requests());
    }

    #[test]
    fn miss_queue_is_bounded() {
        let _guard = lock_statics();
        for i in 0..(REMOTE_MISS_QUEUE_CAP as u64 + 10) {
            note_remote_image_miss(200 + i, "https://example.com/n.png");
        }
        let taken = take_remote_image_requests(usize::MAX);
        assert_eq!(taken.len(), REMOTE_MISS_QUEUE_CAP);
    }
}
