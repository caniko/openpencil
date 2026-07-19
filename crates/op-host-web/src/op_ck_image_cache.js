// CanvasKit image residency and thumbnail drawing.
//
// Full document images and blur-up thumbnails intentionally use independent
// LRU budgets. A large document therefore cannot evict every thumbnail, and
// a thumbnail-heavy document cannot consume the full-resolution raster budget.

const FULL_IMAGE_CACHE_CAP = 4096;
const FULL_IMAGE_CACHE_BYTE_BUDGET = 384 * 1024 * 1024;
const THUMBNAIL_CACHE_CAP = 1024;
const THUMBNAIL_CACHE_BYTE_BUDGET = 4 * 1024 * 1024;
const THUMBNAIL_ENCODED_BYTE_LIMIT = 4 * 1024;
const THUMBNAIL_MAX_EDGE = 32;
const THUMBNAIL_FAILURE_CAP = 1024;
const JPEG_SOF_MARKERS = new Set([
  0xc0, 0xc1, 0xc2, 0xc3, 0xc5, 0xc6, 0xc7,
  0xc9, 0xca, 0xcb, 0xcd, 0xce, 0xcf,
]);

// Read JPEG SOF metadata before CanvasKit expands pixels. The persisted table
// is untrusted input: a very compressible, multi-megapixel JPEG can fit under
// the 4 KiB encoded limit but is not safe to decode synchronously in paint.
const jpegFitsThumbnailBounds = (jpeg) => {
  if (!jpeg || jpeg.byteLength < 4 || jpeg[0] !== 0xff || jpeg[1] !== 0xd8) {
    return false;
  }
  let offset = 2;
  while (offset < jpeg.byteLength) {
    while (offset < jpeg.byteLength && jpeg[offset] === 0xff) offset += 1;
    if (offset >= jpeg.byteLength) return false;
    const marker = jpeg[offset++];
    if (marker === 0xd9 || marker === 0xda) return false;
    if (marker === 0x01 || marker === 0xd8 || (marker >= 0xd0 && marker <= 0xd7)) {
      continue;
    }
    if (offset + 1 >= jpeg.byteLength) return false;
    const length = (jpeg[offset] << 8) | jpeg[offset + 1];
    if (length < 2 || offset + length > jpeg.byteLength) return false;
    if (JPEG_SOF_MARKERS.has(marker)) {
      if (length < 7) return false;
      const height = (jpeg[offset + 3] << 8) | jpeg[offset + 4];
      const width = (jpeg[offset + 5] << 8) | jpeg[offset + 6];
      return (
        width > 0 &&
        height > 0 &&
        width <= THUMBNAIL_MAX_EDGE &&
        height <= THUMBNAIL_MAX_EDGE
      );
    }
    offset += length;
  }
  return false;
};

const imageKey = (lo, hi) => String(hi >>> 0) + ':' + String(lo >>> 0);

const copyBytes = (bytes) =>
  bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);

const decodedRasterBytes = (image) => {
  const width = image.width();
  const height = image.height();
  if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
    return 0;
  }
  return width * height * 4;
};

const deleteImage = (image) => {
  if (image && image.delete) image.delete();
};

export function createWebImageCaches(CK) {
  const fullImageCache = new Map();
  let fullImageCacheBytes = 0;
  const thumbnailCache = new Map();
  let thumbnailCacheBytes = 0;
  const thumbnailFailures = new Set();

  const evictFullImages = () => {
    while (
      fullImageCache.size > FULL_IMAGE_CACHE_CAP ||
      fullImageCacheBytes > FULL_IMAGE_CACHE_BYTE_BUDGET
    ) {
      const oldestKey = fullImageCache.keys().next().value;
      const oldest = fullImageCache.get(oldestKey);
      if (!oldest) break;
      fullImageCache.delete(oldestKey);
      fullImageCacheBytes -= oldest.bytes;
      deleteImage(oldest.image);
    }
  };

  const evictThumbnails = () => {
    while (
      thumbnailCache.size > THUMBNAIL_CACHE_CAP ||
      thumbnailCacheBytes > THUMBNAIL_CACHE_BYTE_BUDGET
    ) {
      const oldestKey = thumbnailCache.keys().next().value;
      const oldest = thumbnailCache.get(oldestKey);
      if (!oldest) break;
      thumbnailCache.delete(oldestKey);
      thumbnailCacheBytes -= oldest.bytes;
      deleteImage(oldest.image);
    }
  };

  const rememberThumbnailFailure = (key) => {
    if (thumbnailFailures.has(key)) return;
    if (thumbnailFailures.size >= THUMBNAIL_FAILURE_CAP) {
      thumbnailFailures.delete(thumbnailFailures.values().next().value);
    }
    thumbnailFailures.add(key);
  };

  const hasFullImage = (lo, hi) => fullImageCache.has(imageKey(lo, hi));

  const fullImage = (lo, hi) => {
    const key = imageKey(lo, hi);
    const hit = fullImageCache.get(key);
    if (!hit) return null;
    fullImageCache.delete(key);
    fullImageCache.set(key, hit);
    return hit.image;
  };

  const installFullImage = (lo, hi, encoded) => {
    const key = imageKey(lo, hi);
    if (fullImageCache.has(key)) return true;

    let image = null;
    try {
      image = CK.MakeImageFromEncoded(copyBytes(encoded));
      if (!image) return false;
      const bytes = decodedRasterBytes(image);
      if (!(bytes > 0)) {
        deleteImage(image);
        return false;
      }
      fullImageCache.set(key, { image, bytes });
      fullImageCacheBytes += bytes;
      evictFullImages();
      return fullImageCache.has(key);
    } catch (_error) {
      deleteImage(image);
      return false;
    }
  };

  const thumbnailImage = (lo, hi, jpeg) => {
    const key = imageKey(lo, hi);
    const hit = thumbnailCache.get(key);
    if (hit) {
      thumbnailCache.delete(key);
      thumbnailCache.set(key, hit);
      return hit.image;
    }
    if (thumbnailFailures.has(key)) return null;
    if (!jpeg || jpeg.byteLength > THUMBNAIL_ENCODED_BYTE_LIMIT) {
      rememberThumbnailFailure(key);
      return null;
    }
    if (!jpegFitsThumbnailBounds(jpeg)) {
      rememberThumbnailFailure(key);
      return null;
    }

    let image = null;
    try {
      image = CK.MakeImageFromEncoded(copyBytes(jpeg));
      if (!image) {
        rememberThumbnailFailure(key);
        return null;
      }
      const bytes = decodedRasterBytes(image);
      if (!(bytes > 0) || bytes > THUMBNAIL_CACHE_BYTE_BUDGET) {
        deleteImage(image);
        rememberThumbnailFailure(key);
        return null;
      }
      thumbnailCache.set(key, { image, bytes });
      thumbnailCacheBytes += bytes;
      evictThumbnails();
      return image;
    } catch (_error) {
      deleteImage(image);
      rememberThumbnailFailure(key);
      return null;
    }
  };

  const discardThumbnail = (key) => {
    const entry = thumbnailCache.get(key);
    if (!entry) return;
    thumbnailCache.delete(key);
    thumbnailCacheBytes -= entry.bytes;
    deleteImage(entry.image);
  };

  const drawThumbnailCover = (canvas, lo, hi, jpeg, x, y, w, h) => {
    if (!(w > 0) || !(h > 0)) return false;
    const image = thumbnailImage(lo, hi, jpeg);
    if (!image) return false;

    const imageW = image.width();
    const imageH = image.height();
    if (!(imageW > 0) || !(imageH > 0)) return false;
    const scale = Math.max(w / imageW, h / imageH);
    const drawW = imageW * scale;
    const drawH = imageH * scale;
    const src = CK.LTRBRect(0, 0, imageW, imageH);
    const dst = CK.LTRBRect(
      x + (w - drawW) / 2,
      y + (h - drawH) / 2,
      x + (w + drawW) / 2,
      y + (h + drawH) / 2,
    );
    const clip = CK.LTRBRect(x, y, x + w, y + h);
    const paint = new CK.Paint();
    paint.setAntiAlias(true);

    let saved = false;
    try {
      canvas.save();
      saved = true;
      canvas.clipRect(clip, CK.ClipOp.Intersect, true);
      if (canvas.drawImageRectOptions) {
        canvas.drawImageRectOptions(
          image,
          src,
          dst,
          CK.FilterMode.Linear,
          CK.MipmapMode.None,
          paint,
        );
      } else {
        canvas.drawImageRect(image, src, dst, paint, false);
      }
    } catch (_error) {
      const key = imageKey(lo, hi);
      discardThumbnail(key);
      rememberThumbnailFailure(key);
      return false;
    } finally {
      if (saved) canvas.restore();
      paint.delete();
    }
    return true;
  };

  return {
    hasFullImage,
    fullImage,
    installFullImage,
    drawThumbnailCover,
  };
}

// wasm-bindgen only copies local modules named by Rust attributes; it does not
// recursively package relative imports from another local module. Rust obtains
// this function object and passes it into the bridge during initialization.
export function webImageCacheFactory() {
  return createWebImageCaches;
}
