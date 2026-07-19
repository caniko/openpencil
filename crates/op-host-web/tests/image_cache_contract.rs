//! Structural coverage for the CanvasKit image-cache bridge.
//!
//! The CanvasKit implementation executes in JavaScript, so native Cargo tests
//! pin the source-level contract that the wasm FFI and browser smoke exercise.

use std::fs;
use std::path::PathBuf;

fn source(path: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path);
    fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

#[test]
fn full_image_cache_accounts_decoded_rgba_bytes() {
    let cache = source("src/op_ck_image_cache.js");

    assert!(cache.contains("const decodedRasterBytes = (image) =>"));
    assert!(cache.contains("image.width()"));
    assert!(cache.contains("image.height()"));
    assert!(cache.contains("* 4"));
    assert!(cache.contains("return fullImageCache.has(key)"));
    assert!(
        !cache.contains("encoded.byteLength") && !cache.contains("encoded.length"),
        "full-image residency must be charged as decoded RGBA, not payload bytes"
    );
}

#[test]
fn thumbnail_cache_is_dedicated_bounded_and_negative_caches_corruption() {
    let cache = source("src/op_ck_image_cache.js");

    assert!(cache.contains("const thumbnailCache = new Map()"));
    assert!(cache.contains("let thumbnailCacheBytes = 0"));
    assert!(cache.contains("const thumbnailFailures = new Set()"));
    assert!(cache.contains("THUMBNAIL_CACHE_BYTE_BUDGET"));
    assert!(cache.contains("THUMBNAIL_ENCODED_BYTE_LIMIT"));
    assert!(cache.contains("THUMBNAIL_FAILURE_CAP"));
    assert!(cache.contains("bytes > THUMBNAIL_CACHE_BYTE_BUDGET"));
    assert!(cache.contains("thumbnailFailures.has(key)"));
    assert!(cache.contains("thumbnailFailures.add(key)"));
    assert!(
        cache.contains("catch"),
        "a corrupt JPEG must not escape into paint"
    );
}

#[test]
fn thumbnail_decode_rejects_oversized_dimensions_before_canvaskit_decode() {
    let cache = source("src/op_ck_image_cache.js");

    assert!(cache.contains("const jpegFitsThumbnailBounds = (jpeg) =>"));
    assert!(cache.contains("THUMBNAIL_MAX_EDGE"));
    assert!(cache.contains("if (!jpegFitsThumbnailBounds(jpeg))"));
    let guard = cache
        .find("if (!jpegFitsThumbnailBounds(jpeg))")
        .expect("dimension guard");
    let decode = cache
        .find("CK.MakeImageFromEncoded(copyBytes(jpeg))")
        .expect("CanvasKit decode");
    assert!(
        guard < decode,
        "dimension metadata must be checked before CanvasKit expands pixels"
    );
}

#[test]
fn thumbnail_draw_is_linear_sampled_aspect_cover() {
    let cache = source("src/op_ck_image_cache.js");

    assert!(cache.contains("drawThumbnailCover"));
    assert!(cache.contains("Math.max(w / imageW, h / imageH)"));
    assert!(cache.contains("canvas.clipRect"));
    assert!(cache.contains("CK.FilterMode.Linear"));
    assert!(cache.contains("CK.MipmapMode.None"));
}

#[test]
fn rust_and_js_expose_the_narrow_thumbnail_hook() {
    let rust = source("src/canvaskit.rs");
    let bridge = source("src/op_ck_bridge.js");

    assert!(rust.contains("js_name = drawImageThumb"));
    assert!(rust.contains("fn draw_image_thumb"));
    assert!(rust.contains("self.ck.draw_image_thumb"));
    assert!(bridge.contains("createWebImageCaches"));
    assert!(bridge.contains("drawImageThumb("));
    assert!(bridge.contains("imageCaches.drawThumbnailCover"));
}

#[test]
fn wasm_bindgen_packages_the_extracted_cache_module() {
    let rust = source("src/canvaskit.rs");
    let bridge = source("src/op_ck_bridge.js");

    assert!(rust.contains("module = \"/src/op_ck_image_cache.js\""));
    assert!(rust.contains("web_image_cache_factory()"));
    assert!(rust.contains("set_image_cache_factory(&image_cache_factory)"));
    assert!(rust.contains("op_ck_init(canvas_id)?"));
    assert!(bridge.contains("export function setImageCacheFactory"));
    assert!(
        !bridge.contains("from './op_ck_image_cache.js'"),
        "wasm-bindgen does not recursively copy relative imports from local modules"
    );
}
