//! Image-blob resolution — ports `figma-image-resolver.ts`. Walks the
//! converted document and replaces `__blob:N` / `__hash:HEX` image-
//! fill placeholders with `data:` URLs.

use base64::Engine;
use jian_ops_schema::document::PenDocument;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::style::PenFill;
use std::collections::HashMap;
use std::rc::Rc;

/// Host-supplied rewrite of raw image bytes, applied lazily to each
/// referenced blob before it is base64-encoded into the document.
/// Return `Some(bytes)` to replace the payload (e.g. the desktop
/// host's down-scale + re-encode pass, which keeps a 500MB image set
/// from entering the document at full resolution), `None` to keep the
/// original. The data-URL MIME is re-sniffed from the returned bytes,
/// so a transform may change the encoding format. op-figma stays
/// wasm32-clean by taking the transform as a callback instead of
/// depending on an image codec itself.
pub type ImageTransform<'a> = dyn Fn(&[u8]) -> Option<Vec<u8>> + 'a;

/// Encode raw image bytes as a `data:` URL, sniffing the MIME type
/// from the leading magic bytes (PNG fallback).
fn blob_to_data_url(bytes: &[u8]) -> String {
    let mime = match bytes {
        [0xFF, 0xD8, ..] => "image/jpeg",
        [0x47, 0x49, ..] => "image/gif",
        [0x52, 0x49, ..] => "image/webp",
        _ => "image/png",
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

/// Lazy, memoized `__blob:` / `__hash:` → data-URL resolver. Encoding
/// (base64, which allocates + copies the whole payload) is deferred
/// until a fill actually references the blob, and the result is cached
/// as a shared `Rc<str>` — a never-referenced blob is never encoded at
/// all, and a blob referenced by several fills is encoded exactly once,
/// with every subsequent reference sharing the same allocation via a
/// cheap refcount bump instead of a fresh `String` copy.
struct BlobCache<'a> {
    image_blobs: &'a HashMap<u32, Vec<u8>>,
    image_files: &'a HashMap<String, Vec<u8>>,
    transform: Option<&'a ImageTransform<'a>>,
    blob_urls: HashMap<u32, Rc<str>>,
    hash_urls: HashMap<String, Rc<str>>,
    /// Number of times a blob was actually base64-encoded (cache
    /// misses only) — exposed for tests that verify the memoization +
    /// never-referenced-blob-skipped invariants.
    encode_calls: u32,
}

impl<'a> BlobCache<'a> {
    fn new(
        image_blobs: &'a HashMap<u32, Vec<u8>>,
        image_files: &'a HashMap<String, Vec<u8>>,
        transform: Option<&'a ImageTransform<'a>>,
    ) -> Self {
        Self {
            image_blobs,
            image_files,
            transform,
            blob_urls: HashMap::new(),
            hash_urls: HashMap::new(),
            encode_calls: 0,
        }
    }

    /// Apply the host transform (if any) then base64-encode. Runs on
    /// cache misses only, so a transform executes once per unique blob.
    fn encode(&mut self, bytes: &[u8]) -> String {
        self.encode_calls += 1;
        match self.transform.and_then(|t| t(bytes)) {
            Some(replaced) => blob_to_data_url(&replaced),
            None => blob_to_data_url(bytes),
        }
    }

    /// Resolve a `__blob:` / `__hash:` placeholder to a shared data
    /// URL, encoding + caching on first reference only.
    fn resolve(&mut self, src: &str) -> Option<Rc<str>> {
        if let Some(rest) = src.strip_prefix("__blob:") {
            let index: u32 = rest.parse().ok()?;
            if let Some(cached) = self.blob_urls.get(&index) {
                return Some(Rc::clone(cached));
            }
            let bytes = self.image_blobs.get(&index)?;
            let url: Rc<str> = Rc::from(self.encode(bytes));
            self.blob_urls.insert(index, Rc::clone(&url));
            return Some(url);
        }
        if let Some(hash) = src.strip_prefix("__hash:") {
            if let Some(cached) = self.hash_urls.get(hash) {
                return Some(Rc::clone(cached));
            }
            let bytes = self.image_files.get(hash)?;
            let url: Rc<str> = Rc::from(self.encode(bytes));
            self.hash_urls.insert(hash.to_string(), Rc::clone(&url));
            return Some(url);
        }
        None
    }
}

fn patch_fills(fills: &mut Option<Vec<PenFill>>, cache: &mut BlobCache) -> usize {
    let mut count = 0;
    if let Some(fills) = fills {
        for fill in fills {
            if let PenFill::Image(img) = fill {
                if img.url.starts_with("__blob:") || img.url.starts_with("__hash:") {
                    if let Some(url) = cache.resolve(&img.url) {
                        // Materialize the owned `ImageSrc` only here, at
                        // the schema boundary — everywhere upstream the
                        // resolved URL travels as a shared `Rc<str>`.
                        img.url = (&*url).into();
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Patch one node's image fills, then recurse into its children.
fn patch_node(node: &mut PenNode, cache: &mut BlobCache) -> usize {
    let mut count = 0;
    let children: Option<&mut Vec<PenNode>> = match node {
        PenNode::Frame(f) => {
            count += patch_fills(&mut f.container.fill, cache);
            f.children.as_mut()
        }
        PenNode::Group(g) => {
            count += patch_fills(&mut g.container.fill, cache);
            g.children.as_mut()
        }
        PenNode::Rectangle(r) => {
            count += patch_fills(&mut r.container.fill, cache);
            r.children.as_mut()
        }
        PenNode::Ellipse(e) => {
            count += patch_fills(&mut e.fill, cache);
            None
        }
        PenNode::Polygon(p) => {
            count += patch_fills(&mut p.fill, cache);
            None
        }
        PenNode::Path(p) => {
            count += patch_fills(&mut p.fill, cache);
            None
        }
        PenNode::Text(t) => {
            count += patch_fills(&mut t.fill, cache);
            None
        }
        PenNode::Ref(r) => r.children.as_mut(),
        _ => None,
    };
    if let Some(children) = children {
        for child in children {
            count += patch_node(child, cache);
        }
    }
    count
}

/// Replace every image-fill placeholder in `doc` with a data URL,
/// applying an optional host [`ImageTransform`] to each referenced
/// blob's bytes before encoding. Returns the number of replacements
/// made.
pub fn resolve_image_blobs_with(
    doc: &mut PenDocument,
    image_blobs: &HashMap<u32, Vec<u8>>,
    image_files: &HashMap<String, Vec<u8>>,
    transform: Option<&ImageTransform<'_>>,
) -> usize {
    if image_blobs.is_empty() && image_files.is_empty() {
        return 0;
    }
    let mut cache = BlobCache::new(image_blobs, image_files, transform);

    let mut count = 0;
    if let Some(pages) = &mut doc.pages {
        for page in pages {
            for child in &mut page.children {
                count += patch_node(child, &mut cache);
            }
        }
    }
    for child in &mut doc.children {
        count += patch_node(child, &mut cache);
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_mime_is_default() {
        assert!(blob_to_data_url(&[0x89, 0x50, 0x4e, 0x47]).starts_with("data:image/png;base64,"));
    }

    #[test]
    fn jpeg_mime_detected() {
        assert!(blob_to_data_url(&[0xFF, 0xD8, 0xFF, 0xE0]).starts_with("data:image/jpeg;base64,"));
    }

    fn png_bytes() -> Vec<u8> {
        vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0xAA]
    }

    #[test]
    fn resolve_blob_ref() {
        let mut blobs = HashMap::new();
        blobs.insert(3u32, png_bytes());
        let no_files = HashMap::new();
        let mut cache = BlobCache::new(&blobs, &no_files, None);
        let resolved = cache.resolve("__blob:3").expect("blob 3 resolves");
        assert!(resolved.starts_with("data:image/png;base64,"));
        assert!(cache.resolve("__blob:9").is_none());
    }

    #[test]
    fn resolve_hash_ref() {
        let mut files = HashMap::new();
        files.insert("abcd".to_string(), png_bytes());
        let no_blobs = HashMap::new();
        let mut cache = BlobCache::new(&no_blobs, &files, None);
        let resolved = cache.resolve("__hash:abcd").expect("hash abcd resolves");
        assert!(resolved.starts_with("data:image/png;base64,"));
    }

    /// A blob that no fill ever references must never be run through
    /// `blob_to_data_url` — the whole point of laziness is to skip the
    /// (potentially large) encode for image-heavy files where most
    /// blobs aren't actually used by any visible fill.
    #[test]
    fn unreferenced_blob_is_never_encoded() {
        let mut blobs = HashMap::new();
        blobs.insert(0u32, png_bytes());
        blobs.insert(1u32, png_bytes());
        let no_files = HashMap::new();
        let mut cache = BlobCache::new(&blobs, &no_files, None);

        cache.resolve("__blob:0").expect("blob 0 resolves");

        assert_eq!(cache.encode_calls, 1, "only the referenced blob is encoded");
        assert!(cache.blob_urls.contains_key(&0));
        assert!(
            !cache.blob_urls.contains_key(&1),
            "blob 1 was never referenced and must not be cached/encoded"
        );
    }

    /// A host-supplied transform (e.g. desktop down-scaling) rewrites
    /// the payload before base64 encoding — the data URL must carry
    /// the transformed bytes (and their sniffed MIME), and memoization
    /// must keep the transform to one call per unique blob.
    #[test]
    fn transform_rewrites_referenced_blob_before_encoding() {
        use std::cell::Cell;
        let mut blobs = HashMap::new();
        blobs.insert(0u32, png_bytes());
        let no_files = HashMap::new();
        let calls = Cell::new(0u32);
        let transform = |_bytes: &[u8]| {
            calls.set(calls.get() + 1);
            // Pretend the host re-encoded the image as JPEG.
            Some(vec![0xFF, 0xD8, 0xFF, 0xE0])
        };
        let mut cache = BlobCache::new(&blobs, &no_files, Some(&transform));

        let first = cache.resolve("__blob:0").expect("blob 0 resolves");
        let second = cache.resolve("__blob:0").expect("blob 0 resolves again");

        assert!(
            first.starts_with("data:image/jpeg;base64,"),
            "data URL carries the transformed bytes' MIME, got {first}"
        );
        assert!(Rc::ptr_eq(&first, &second), "memoized after transform");
        assert_eq!(calls.get(), 1, "transform runs once per unique blob");
    }

    /// A transform returning `None` keeps the original payload — the
    /// pass-through contract mirrors `maybe_downscale`'s "already
    /// small enough" case.
    #[test]
    fn transform_none_keeps_original_bytes() {
        let mut files = HashMap::new();
        files.insert("abcd".to_string(), png_bytes());
        let no_blobs = HashMap::new();
        let transform = |_bytes: &[u8]| None;
        let mut cache = BlobCache::new(&no_blobs, &files, Some(&transform));
        let resolved = cache.resolve("__hash:abcd").expect("hash abcd resolves");
        assert!(
            resolved.starts_with("data:image/png;base64,"),
            "original PNG payload survives a None transform"
        );
    }

    /// Laziness extends to the transform: a blob no fill references
    /// must never be transformed (the whole point for image-heavy
    /// files where some blobs are unused).
    #[test]
    fn unreferenced_blob_is_never_transformed() {
        use std::cell::Cell;
        let mut blobs = HashMap::new();
        blobs.insert(0u32, png_bytes());
        blobs.insert(1u32, png_bytes());
        let no_files = HashMap::new();
        let calls = Cell::new(0u32);
        let transform = |_bytes: &[u8]| {
            calls.set(calls.get() + 1);
            None
        };
        let mut cache = BlobCache::new(&blobs, &no_files, Some(&transform));
        cache.resolve("__blob:0").expect("blob 0 resolves");
        assert_eq!(calls.get(), 1, "only the referenced blob is transformed");
    }

    /// Two fills pointing at the same blob must share the encoding
    /// work: the base64 pass runs once, and both callers get the same
    /// `Rc` allocation back (a refcount bump, not a fresh string copy).
    #[test]
    fn shared_blob_is_encoded_exactly_once() {
        let mut blobs = HashMap::new();
        blobs.insert(0u32, png_bytes());
        let no_files = HashMap::new();
        let mut cache = BlobCache::new(&blobs, &no_files, None);

        let first = cache.resolve("__blob:0").expect("first fill resolves");
        let second = cache.resolve("__blob:0").expect("second fill resolves");

        assert_eq!(
            cache.encode_calls, 1,
            "encode runs once despite two references"
        );
        assert!(
            Rc::ptr_eq(&first, &second),
            "both references share the same Rc allocation"
        );
    }
}
