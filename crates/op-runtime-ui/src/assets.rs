use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::ExportError;

pub(crate) struct Sidecar {
    pub kind: &'static str,
    pub ext: &'static str,
    pub mime: &'static str,
    pub sha256: String,
    pub bytes: Vec<u8>,
}

impl Sidecar {
    pub(crate) fn id(&self) -> String {
        format!("{}-{}", self.kind, self.sha256)
    }

    pub(crate) fn uri(&self) -> String {
        format!("{}/{}.{}", dir_for(self.kind), self.sha256, self.ext)
    }

    pub(crate) fn write_under(&self, asset_root: &Path) -> Result<(), ExportError> {
        let path = asset_root
            .join(dir_for(self.kind))
            .join(format!("{}.{}", self.sha256, self.ext));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &self.bytes)?;
        Ok(())
    }
}

fn dir_for(kind: &str) -> &'static str {
    match kind {
        "image" => "images",
        "font" => "fonts",
        "vector" => "vectors",
        "raster_fallback" => "fallback",
        _ => "fallback",
    }
}

pub(crate) fn raster_fallback_from_png(bytes: Vec<u8>) -> Result<Sidecar, ExportError> {
    if png_size(&bytes).is_none() {
        return Err(ExportError::msg("raster fallback is not a PNG"));
    }
    Ok(Sidecar {
        kind: "raster_fallback",
        ext: "png",
        mime: "image/png",
        sha256: hex_sha256(&bytes),
        bytes,
    })
}

pub(crate) fn image_from_src(src: &str, source_dir: &Path) -> Result<Sidecar, ExportError> {
    if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("//") {
        return Err(ExportError::msg(format!(
            "network image uri is not allowed: {src}"
        )));
    }
    if let Some(rest) = src.strip_prefix("data:") {
        return decode_data_url(rest);
    }
    let path = if Path::new(src).is_absolute() {
        PathBuf::from(src)
    } else {
        source_dir.join(src)
    };
    let bytes = fs::read(&path)
        .map_err(|e| ExportError::msg(format!("cannot read image {}: {e}", path.display())))?;
    sidecar_from_bytes(bytes, ext_of(&path))
}

fn decode_data_url(rest: &str) -> Result<Sidecar, ExportError> {
    let (meta, payload) = rest
        .split_once(',')
        .ok_or_else(|| ExportError::msg("invalid data: image url"))?;
    if !meta.contains(";base64") {
        return Err(ExportError::msg("data: image url must be base64"));
    }
    let mime = meta
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let ext = match mime.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/svg+xml" => {
            return Err(ExportError::msg(
                "svg data: images are not exported as image assets",
            ))
        }
        other => return Err(ExportError::msg(format!("unsupported data: mime {other}"))),
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|e| ExportError::msg(format!("invalid data: base64: {e}")))?;
    sidecar_from_bytes(bytes, ext)
}

fn sidecar_from_bytes(bytes: Vec<u8>, ext: &str) -> Result<Sidecar, ExportError> {
    let (kind, mime) = match ext {
        "png" => ("image", "image/png"),
        "jpg" => ("image", "image/jpeg"),
        "webp" => ("image", "image/webp"),
        other => return Err(ExportError::msg(format!("unsupported image ext .{other}"))),
    };
    let sha256 = hex_sha256(&bytes);
    Ok(Sidecar {
        kind,
        ext: match ext {
            "jpg" => "jpg",
            "webp" => "webp",
            _ => "png",
        },
        mime,
        sha256,
        bytes,
    })
}

fn ext_of(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "jpg",
        "webp" => "webp",
        _ => "png",
    }
}

pub(crate) fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    if &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if w == 0 || h == 0 {
        None
    } else {
        Some((w, h))
    }
}

pub(crate) fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_size_reads_ihdr() {
        let png = base64::engine::general_purpose::STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJrqD0AAAAAElFTkSuQmCC")
            .unwrap();
        assert_eq!(png_size(&png), Some((1, 1)));
    }
}
