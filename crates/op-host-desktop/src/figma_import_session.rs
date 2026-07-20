//! Background `.fig` import session — moves the multi-second
//! `parse_fig_binary` call off the main thread so the editor UI keeps
//! repainting (cursor moves, overlay animation) while a large Figma
//! file decodes.
//!
//! Lifecycle, mirrors `chat_session`:
//!   1. `spawn` — kick off file read + parse on a `std::thread`,
//!      returning a session handle holding the receiver.
//!   2. `pump` — called every `RedrawRequested`; non-blocking
//!      `try_recv` on the channel. Returns whether the host state
//!      changed (so the caller can mark the next frame dirty).
//!   3. `is_pending` — true while the worker is still running. The
//!      app handler reads this to schedule periodic wakes so the
//!      "正在解析…" overlay keeps repainting under `WaitUntil` flow.
//!
//! On result land:
//!   - `Ok(import)` → swap `EditorState` to the imported document and
//!     clear `figma_import_in_progress`.
//!   - `Err(e)` → show the native error dialog + clear the flag.

use op_editor_core::EditorState;
use op_host_native::WidgetHostNative;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::persistence::show_error_dialog_public;
use op_host_services::doc_io::ErrorKind;

mod image_sources;
use image_sources::{bind_import_thumbnails, PendingImportThumbs};

/// Successful worker output. Keep this Skia-free: building
/// `LayoutScene` runs text measurement, which can contend with the
/// main-thread painter and freeze the progress overlay.
pub struct PreparedImport {
    pub state: EditorState,
    pub warnings: Vec<String>,
}

/// One in-flight `.fig` parse — the source path (for the error
/// dialog) plus the worker-thread receiver.
pub struct FigmaImportSession {
    path: PathBuf,
    rx: Receiver<Result<PreparedImport, String>>,
}

impl FigmaImportSession {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Spawn a worker thread that reads `path`, parses it with
/// `op_figma::parse_fig_binary` in `Preserve` mode, and posts the
/// result back through a channel. Returns the session handle.
pub fn spawn(host: &mut WidgetHostNative, path: PathBuf) -> FigmaImportSession {
    let (tx, rx) = mpsc::channel();
    // Flip the overlay flag so paint shows "正在解析…" feedback as
    // soon as the next frame fires. We deliberately do NOT call
    // `mark_editor_state_dirty()` here: that would set
    // `editor_state_dirty=true`, which triggers
    // `refresh_layout_scene` on the next paint and rebuilds the
    // layout against the OLD document — wasted work since the
    // import is about to replace `editor_state` whole-cloth. The
    // overlay widget reads `editor_ui.figma_import_in_progress`
    // directly, not through `layout_scene`, so the cached layout
    // from before the spawn is fine to keep painting underneath.
    let ui = &mut host.editor_state_mut().editor_ui;
    ui.import_source = op_editor_core::figma_import_state::ImportSource::Figma;
    ui.figma_import_in_progress = true;

    let path_for_thread = path.clone();
    thread::Builder::new()
        .name("op-figma-import".into())
        .spawn(move || {
            let result = parse_path(&path_for_thread);
            // Recv side may be gone if the user closed the app
            // mid-parse; tolerate the SendError silently.
            let _ = tx.send(result);
        })
        .expect("spawn op-figma-import worker");

    FigmaImportSession { path, rx }
}

fn parse_path(path: &Path) -> Result<PreparedImport, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let file_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Figma Import");
    // Down-scale every referenced bitmap before it is embedded as
    // base64 — an image-heavy `.fig` can carry hundreds of MB of
    // full-resolution photos, which would otherwise bloat the
    // document, the scene rebuild, and every paint-time decode. This
    // is CPU raster work (decode/resample/encode); unlike LayoutScene
    // text measurement it does not touch FontMgr, so it is safe on
    // this worker thread.
    let pending_thumbs = RefCell::new(PendingImportThumbs::default());
    let transform = |bytes: &[u8]| {
        let prepared = crate::image_downscale::prepare_figma_import_image(bytes);
        if let Some(thumb) = prepared.thumbnail {
            let final_bytes = prepared.replacement.as_deref().unwrap_or(bytes);
            pending_thumbs.borrow_mut().record(final_bytes, thumb);
        }
        prepared.replacement
    };
    let import = op_figma::parse_fig_binary_with_images(
        &bytes,
        file_name,
        op_figma::FigLayoutMode::Preserve,
        Some(&transform),
    )
    .map_err(|e| e.to_string())?;
    let mut state = EditorState::from_document(import.document);
    bind_import_thumbnails(&state.doc, &mut pending_thumbs.borrow_mut());
    state.editor_ui.preserve_authored_geometry = true;
    Ok(PreparedImport {
        state,
        warnings: import.warnings,
    })
}

/// Returns true when the session resolved and the host state changed
/// (caller should mark the next frame dirty). Returns false when the
/// worker is still running or no session is active.
///
/// On success this drains the receiver, applies the imported
/// document, refreshes the host title, and clears the
/// in-progress flag. On failure it pops the native error dialog. In
/// either case the `*session` slot becomes `None`.
pub fn pump(
    host: &mut WidgetHostNative,
    session: &mut Option<FigmaImportSession>,
    current_path: &mut Option<PathBuf>,
    window: Option<&winit::window::Window>,
) -> PumpOutcome {
    let Some(sess) = session.as_mut() else {
        return PumpOutcome::Idle;
    };
    match sess.rx.try_recv() {
        Ok(Ok(prepared)) => {
            for warning in &prepared.warnings {
                eprintln!("[import-figma] warning: {warning}");
            }
            // Swap in the parsed state. The worker deliberately did
            // not build a LayoutScene because that touches Skia /
            // FontMgr and can block the main-thread progress overlay.
            host.install_imported_state(prepared.state);
            // Imported docs have no `.op` path; next Save routes via
            // Save As — matches the synchronous import behaviour.
            *current_path = None;
            refresh_title(current_path, window);
            *session = None;
            PumpOutcome::CompletedOk
        }
        Ok(Err(e)) => {
            eprintln!("[import-figma] {e}");
            show_error_dialog_public(host, ErrorKind::Open, Some(&sess.path), &e);
            host.editor_state_mut().editor_ui.figma_import_in_progress = false;
            host.mark_editor_state_dirty();
            *session = None;
            PumpOutcome::CompletedErr
        }
        Err(TryRecvError::Empty) => PumpOutcome::StillPending,
        Err(TryRecvError::Disconnected) => {
            // Worker thread panicked or dropped without sending —
            // pop the same native dialog the explicit error path
            // uses so the user gets feedback instead of a silently
            // vanishing progress overlay.
            eprintln!("[import-figma] worker thread terminated without sending a result");
            let detail = "Figma import worker exited unexpectedly";
            show_error_dialog_public(host, ErrorKind::Open, Some(&sess.path), detail);
            host.editor_state_mut().editor_ui.figma_import_in_progress = false;
            host.mark_editor_state_dirty();
            *session = None;
            PumpOutcome::CompletedErr
        }
    }
}

/// Drop the active session (if any) and clear the in-progress flag —
/// called when another document-replacing action runs while a Figma
/// import is still parsing (File→New, File→Open, another File→Import
/// Figma). Without this guard, a stale worker would later overwrite
/// the user's freshly-opened document in `pump`.
///
/// The worker thread keeps running until it tries to `tx.send`; that
/// send becomes a no-op once we drop the receiver here. The thread
/// is short-lived (one parse + layout pass) so leaking it briefly is
/// fine.
pub fn cancel(host: &mut WidgetHostNative, session: &mut Option<FigmaImportSession>) {
    if session.is_some() {
        eprintln!("[import-figma] cancelling in-flight session — superseded");
        *session = None;
        if host.editor_state().editor_ui.figma_import_in_progress {
            host.editor_state_mut().editor_ui.figma_import_in_progress = false;
            host.mark_editor_state_dirty();
        }
    }
}

/// Outcome of `pump` — used by the caller to decide whether to mark
/// the next frame dirty and whether to reset the document-path state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpOutcome {
    /// No active session.
    Idle,
    /// Worker thread still running.
    StillPending,
    /// Worker finished and the document was applied.
    CompletedOk,
    /// Worker finished with an error (dialog already shown).
    CompletedErr,
}

fn refresh_title(current_path: &Option<PathBuf>, window: Option<&winit::window::Window>) {
    let Some(window) = window else { return };
    let title = match current_path.as_ref().and_then(|p| p.file_name()) {
        Some(name) => format!("{} — OpenPencil", name.to_string_lossy()),
        None => "OpenPencil".to_string(),
    };
    window.set_title(&title);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct ByteCounter(u64);

    impl std::io::Write for ByteCounter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 += bytes.len() as u64;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct SavedTableProbe {
        #[serde(default)]
        images: std::collections::BTreeMap<String, serde::de::IgnoredAny>,
        #[serde(default)]
        image_thumbs: std::collections::BTreeMap<String, String>,
    }

    fn compact_json_size(value: &impl serde::Serialize) -> u64 {
        let mut counter = ByteCounter::default();
        serde_json::to_writer(&mut counter, value).expect("count compact JSON bytes");
        counter.0
    }

    fn probe_saved_tables(path: &Path) -> SavedTableProbe {
        let file = std::fs::File::open(path).expect("open saved document");
        let mut deserializer = serde_json::Deserializer::from_reader(std::io::BufReader::new(file));
        deserializer.disable_recursion_limit();
        <SavedTableProbe as serde::Deserialize>::deserialize(&mut deserializer)
            .expect("probe saved tables")
    }

    fn no_thumbs_path(path: &Path) -> PathBuf {
        let stem = path
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("openpencil-tesla-blurup");
        path.with_file_name(format!("{stem}-no-thumbs.op"))
    }

    fn select_summary_page(state: &mut EditorState) -> usize {
        let pages = state.doc.pages.as_ref().expect("Tesla document has pages");
        let page_index = pages
            .iter()
            .position(|page| page.name.ends_with("汇总稿"))
            .unwrap_or_else(|| {
                panic!(
                    "汇总稿 missing; pages={:?}",
                    pages
                        .iter()
                        .map(|page| page.name.as_str())
                        .collect::<Vec<_>>()
                )
            });
        assert!(state.set_active_page(page_index));
        page_index
    }

    fn paint_and_pump_decode_frame(
        host: &mut WidgetHostNative,
        backend: &mut op_host_native::NativeBackend,
        image_decodes: &mut crate::image_decode_host::ImageDecodeHost,
        surface: &mut skia_safe::Surface,
        started: std::time::Instant,
    ) {
        const WIDTH: f32 = 1440.0;
        const HEIGHT: f32 = 900.0;
        surface.canvas().restore_to_count(1);
        surface.canvas().reset_matrix();
        surface.canvas().clear(skia_safe::Color::BLACK);
        host.set_now_ms(started.elapsed().as_millis() as u64);
        {
            let mut frame = op_host_native::NativeFrameBackend::new(backend, surface.canvas());
            host.paint(&mut frame, WIDTH, HEIGHT);
        }
        image_decodes.pump(backend);
    }

    fn write_surface_png(surface: &mut skia_safe::Surface, path: &Path) {
        let png = surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .expect("encode verification frame as PNG");
        std::fs::write(path, png.as_bytes()).expect("write verification frame PNG");
    }

    fn run_zoomed_out_decode_probe(mut state: EditorState, mut host: WidgetHostNative) {
        use op_editor_ui::widgets::canvas_viewport_image::pending_decode_count;
        use std::time::{Duration, Instant};

        const WIDTH: f32 = 1440.0;
        const HEIGHT: f32 = 900.0;
        const FRAME_PERIOD: Duration = Duration::from_millis(16);
        const SETTLE_DEADLINE: Duration = Duration::from_secs(120);
        const STABLE_IDLE: Duration = Duration::from_secs(3);
        const IDLE_SAMPLE: Duration = Duration::from_secs(5);

        let page_index = select_summary_page(&mut state);
        host.install_imported_state(state);
        host.fit_content_to_viewport(WIDTH, HEIGHT);
        let mut backend = op_host_native::NativeBackend::with_dpi(1.0);
        let mut surface = skia_safe::surfaces::raster_n32_premul((WIDTH as i32, HEIGHT as i32))
            .expect("offscreen raster surface");
        let mut image_decodes = crate::image_decode_host::ImageDecodeHost::new();
        let started = Instant::now();
        let mut frames = 0u64;
        let mut idle_candidate = None;
        let mut settled = false;

        op_host_native::begin_image_paint_diagnostics();
        paint_and_pump_decode_frame(
            &mut host,
            &mut backend,
            &mut image_decodes,
            &mut surface,
            started,
        );
        frames += 1;
        let first_frame = op_host_native::image_paint_diagnostics_snapshot();
        assert!(
            first_frame.successful_thumbnail_draws > 0,
            "the reopened first frame must draw persisted blur-up thumbnails"
        );
        assert_eq!(
            first_frame.sharp_raster_hits, 0,
            "the reopened first frame starts before full rasters are installed"
        );
        let blur_capture = Path::new("/private/tmp/openpencil-tesla-blur-first.png");
        write_surface_png(&mut surface, blur_capture);

        while started.elapsed() < SETTLE_DEADLINE {
            let frame_started = Instant::now();
            paint_and_pump_decode_frame(
                &mut host,
                &mut backend,
                &mut image_decodes,
                &mut surface,
                started,
            );
            frames += 1;
            let queue_empty = !image_decodes.is_pending() && pending_decode_count() == 0;
            if queue_empty {
                let idle_since = idle_candidate.get_or_insert_with(Instant::now);
                if idle_since.elapsed() >= STABLE_IDLE {
                    settled = true;
                    break;
                }
            } else {
                idle_candidate = None;
            }
            if let Some(remaining) = FRAME_PERIOD.checked_sub(frame_started.elapsed()) {
                std::thread::sleep(remaining);
            }
        }

        let settled_at = started.elapsed();
        let before_idle = image_decodes
            .stats_snapshot()
            .expect("set OP_IMAGE_DECODE_STATS=1 for the real-file decode probe");
        let idle_started = Instant::now();
        while idle_started.elapsed() < IDLE_SAMPLE {
            let frame_started = Instant::now();
            paint_and_pump_decode_frame(
                &mut host,
                &mut backend,
                &mut image_decodes,
                &mut surface,
                started,
            );
            frames += 1;
            if let Some(remaining) = FRAME_PERIOD.checked_sub(frame_started.elapsed()) {
                std::thread::sleep(remaining);
            }
        }
        let idle_elapsed = idle_started.elapsed();
        let after_idle = image_decodes
            .stats_snapshot()
            .expect("decode telemetry remains active");
        let queue_pending = pending_decode_count();
        let diagnostics = op_host_native::end_image_paint_diagnostics();
        let sharp_capture = Path::new("/private/tmp/openpencil-tesla-sharp-terminal.png");
        write_surface_png(&mut surface, sharp_capture);
        let idle_installs = after_idle.0.saturating_sub(before_idle.0);
        let idle_reinstalls = after_idle.1.saturating_sub(before_idle.1);
        let sample_secs = idle_elapsed.as_secs_f64();
        assert!(after_idle.0 > 0, "the harness must install sharp rasters");
        assert!(
            diagnostics.sharp_raster_hits > 0,
            "later frames must sharpen at least one blur-up thumbnail"
        );
        assert_eq!(
            diagnostics.paint_thread_full_decodes, 0,
            "full images must never decode synchronously on the paint thread"
        );
        eprintln!(
            "fig_decode_probe: page_index={page_index}, frames={frames}, settled={settled}, \
             settle_window={:.1}s, terminal_sample={sample_secs:.1}s, terminal_installs={idle_installs}, \
             terminal_reinstalls={idle_reinstalls}, terminal_installs_per_sec={:.1}, \
             terminal_reinstalls_per_sec={:.1}, installs_total={}, reinstalls_total={}, in_flight={}, \
             pending={}, state={}, first_thumb_draws={}, first_sharp_hits={}, thumb_draws_total={}, \
             sharp_hits_total={}, paint_sync_full_decodes={}, blur_capture={}, sharp_capture={}",
            settled_at.as_secs_f64(),
            idle_installs as f64 / sample_secs,
            idle_reinstalls as f64 / sample_secs,
            after_idle.0,
            after_idle.1,
            after_idle.2,
            queue_pending,
            after_idle.4,
            first_frame.successful_thumbnail_draws,
            first_frame.sharp_raster_hits,
            diagnostics.successful_thumbnail_draws,
            diagnostics.sharp_raster_hits,
            diagnostics.paint_thread_full_decodes,
            blur_capture.display(),
            sharp_capture.display(),
        );
    }

    #[test]
    fn deferred_thumbnail_binding_uses_the_final_source_string() {
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine as _;
        use jian_ops_schema::node::image_src::paint_image_id;

        let provisional = b"raw import bytes that are replaced";
        let final_bytes = b"final transformed bytes";
        let final_src = format!("data:image/jpeg;base64,{}", B64.encode(final_bytes));
        let provisional_src = format!("data:image/png;base64,{}", B64.encode(provisional));
        let doc = serde_json::from_value(serde_json::json!({
            "version": "0.8.2",
            "children": [{
                "type": "rectangle",
                "id": "image-fill",
                "fill": [{"type": "image", "url": final_src}]
            }]
        }))
        .expect("test document");
        let jpeg = vec![0xff, 0xd8, 0xff, 0xd9];
        let mut pending = PendingImportThumbs::default();
        pending.record(final_bytes, jpeg.clone());

        bind_import_thumbnails(&doc, &mut pending);

        let final_id = paint_image_id(&final_src);
        assert_eq!(
            &*jian_ops_schema::image_thumbs::thumb_for(final_id).expect("final id is bound"),
            jpeg
        );
        assert!(
            jian_ops_schema::image_thumbs::thumb_for(paint_image_id(&provisional_src)).is_none(),
            "the callback's provisional source is never persisted"
        );
    }

    /// Manual large-file bench for the exact worker code path
    /// (`parse_path`, including the down-scale transform). Point
    /// `OP_FIG_BENCH` at a `.fig` file and run:
    ///
    /// ```sh
    /// OP_IMAGE_DECODE_STATS=1 OP_FIG_BENCH=/path/to/big.fig \
    ///   cargo test -p op-host-desktop --release fig_import_bench -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "manual bench — needs OP_FIG_BENCH pointing at a local .fig"]
    fn fig_import_bench() {
        let Ok(path) = std::env::var("OP_FIG_BENCH") else {
            eprintln!("OP_FIG_BENCH not set — skipping");
            return;
        };
        let output = std::env::var_os("OP_FIG_BENCH_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(format!(
                    "/private/tmp/openpencil-tesla-blurup-{}.op",
                    std::process::id()
                ))
            });
        let started = std::time::Instant::now();
        let prepared = parse_path(Path::new(&path)).expect("bench file parses");
        let import_elapsed = started.elapsed();
        let PreparedImport {
            mut state,
            warnings,
        } = prepared;
        let warning_count = warnings.len();
        let page_index = select_summary_page(&mut state);
        let inline_bytes = compact_json_size(&state.doc);

        let save_started = std::time::Instant::now();
        op_host_services::doc_io::save_to_path(&state, &output).expect("production save succeeds");
        let save_elapsed = save_started.elapsed();
        let saved_bytes = std::fs::metadata(&output).expect("saved metadata").len();
        let probe = probe_saved_tables(&output);
        assert!(!probe.image_thumbs.is_empty(), "saved imageThumbs table");
        assert!(
            probe
                .image_thumbs
                .keys()
                .all(|paint_id| paint_id.parse::<u64>().is_ok()),
            "imageThumbs keys are decimal paint ids"
        );
        let first_thumb_id = probe
            .image_thumbs
            .keys()
            .next()
            .expect("one thumbnail id")
            .parse::<u64>()
            .expect("decimal thumbnail id");
        let thumb_table_bytes = compact_json_size(&probe.image_thumbs);

        jian_ops_schema::image_thumbs::clear_registry();
        assert!(jian_ops_schema::image_thumbs::thumb_for(first_thumb_id).is_none());
        let baseline = no_thumbs_path(&output);
        op_host_services::doc_io::save_to_path(&state, &baseline)
            .expect("no-thumbnail baseline save succeeds");
        let baseline_probe = probe_saved_tables(&baseline);
        assert!(
            baseline_probe.image_thumbs.is_empty(),
            "cleared registry omits imageThumbs"
        );
        let baseline_bytes = std::fs::metadata(&baseline)
            .expect("baseline metadata")
            .len();
        let thumbnail_delta = saved_bytes as i128 - baseline_bytes as i128;
        drop(state);

        // The running desktop host exists before a document is opened. Build
        // it before reload so its empty default document cannot clear the
        // thumbnail table that the production load path is about to seed.
        let host = WidgetHostNative::new();
        let reload_started = std::time::Instant::now();
        let reloaded =
            op_host_services::doc_io::load_editor_state(&output, op_editor_core::Locale::EnUs)
                .expect("production reopen succeeds");
        let reload_elapsed = reload_started.elapsed();
        assert!(
            jian_ops_schema::image_thumbs::thumb_for(first_thumb_id).is_some(),
            "reopen seeds the persisted thumbnail registry"
        );
        eprintln!(
            "fig_import_bench: page_index={page_index}, parse+downscale={:.1}s, \
             save={:.1}s, reload={:.1}s, inline={:.1} MB, saved={:.1} MB, \
             no_thumbs={:.1} MB, thumbnail_delta={:.1} MB, thumb_table={:.1} MB, \
             images={}, imageThumbs={}, warnings={warning_count}, output={}, baseline={}",
            import_elapsed.as_secs_f64(),
            save_elapsed.as_secs_f64(),
            reload_elapsed.as_secs_f64(),
            inline_bytes as f64 / 1e6,
            saved_bytes as f64 / 1e6,
            baseline_bytes as f64 / 1e6,
            thumbnail_delta as f64 / 1e6,
            thumb_table_bytes as f64 / 1e6,
            probe.images.len(),
            probe.image_thumbs.len(),
            output.display(),
            baseline.display(),
        );
        run_zoomed_out_decode_probe(reloaded, host);
        if warning_count != 356 {
            eprintln!(
                "fig_import_bench premise drift: expected 356 warnings, observed {warning_count}"
            );
        }
    }
}
