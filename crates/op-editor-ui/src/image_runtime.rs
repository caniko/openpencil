//! Host-facing image cache and decode-queue facade.
//!
//! Canvas painting records image work inside the widget layer, while native
//! and web renderers perform the platform-specific decode. Hosts must use this
//! module for that handoff instead of reaching into `widgets` directly.

pub use crate::widgets::canvas_viewport_image::{
    cached_bytes_for, has_pending_decodes, mark_decode_done, mark_decode_failed,
    note_pending_decode, store_remote_image_bytes, take_pending_decodes, PendingDecode,
};
