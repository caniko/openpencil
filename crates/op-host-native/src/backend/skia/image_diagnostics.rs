//! Opt-in, current-thread image paint counters used by headless verification.

use std::cell::Cell;

/// Cumulative image activity on the thread that enabled diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImagePaintDiagnostics {
    /// Thumbnail rasters that reached the canvas draw path.
    pub successful_thumbnail_draws: u64,
    /// Full-resolution raster-cache hits that reached the sharp draw path.
    pub sharp_raster_hits: u64,
    /// Calls to the canonical full decoder on this paint thread.
    pub paint_thread_full_decodes: u64,
}

thread_local! {
    static COUNTERS: Cell<Option<ImagePaintDiagnostics>> = const { Cell::new(None) };
}

/// Reset and enable image diagnostics for the calling thread.
pub fn begin_image_paint_diagnostics() {
    COUNTERS.set(Some(ImagePaintDiagnostics::default()));
}

/// Snapshot diagnostics without disabling them.
pub fn image_paint_diagnostics_snapshot() -> ImagePaintDiagnostics {
    COUNTERS.get().unwrap_or_default()
}

/// Disable diagnostics and return their final values.
pub fn end_image_paint_diagnostics() -> ImagePaintDiagnostics {
    COUNTERS.take().unwrap_or_default()
}

fn update(f: impl FnOnce(&mut ImagePaintDiagnostics)) {
    if let Some(mut counters) = COUNTERS.get() {
        f(&mut counters);
        COUNTERS.set(Some(counters));
    }
}

pub(super) fn record_successful_thumbnail_draw() {
    update(|counters| {
        counters.successful_thumbnail_draws = counters.successful_thumbnail_draws.saturating_add(1);
    });
}

pub(super) fn record_sharp_raster_hit() {
    update(|counters| {
        counters.sharp_raster_hits = counters.sharp_raster_hits.saturating_add(1);
    });
}

pub(super) fn record_full_decode() {
    update(|counters| {
        counters.paint_thread_full_decodes = counters.paint_thread_full_decodes.saturating_add(1);
    });
}
