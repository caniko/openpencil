//! Background `.html` import session — mirrors
//! `figma_import_session`: moves the op-html parse (CSS cascade +
//! node mapping + local resource embedding) off the main thread so
//! the editor UI keeps repainting while a page converts.
//!
//! Reuses `figma_import_session::{PreparedImport, PumpOutcome}` and
//! the same `figma_import_in_progress` overlay flag so the paint
//! side needs no new UI state.

use op_editor_core::EditorState;
use op_host_native::WidgetHostNative;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use crate::figma_import_session::{PreparedImport, PumpOutcome};
use crate::persistence::show_error_dialog_public;
use op_host_services::doc_io::ErrorKind;

/// Synthetic origin for resolving a file-import's relative resource
/// references — same convention as `op-cli`'s `html_cli.rs`.
const LOCAL_RESOURCE_ORIGIN: &str = "https://openpencil.local/";

/// One in-flight `.html` parse — the source path (for the error
/// dialog) plus the worker-thread receiver.
pub struct HtmlImportSession {
    path: PathBuf,
    rx: Receiver<Result<PreparedImport, String>>,
}

/// Spawn a worker thread that reads `path`, converts it with
/// `op_html::import_html_document` (resolving same-directory
/// relative resources from disk), and posts the result back through
/// a channel. Returns the session handle.
pub fn spawn(host: &mut WidgetHostNative, path: PathBuf) -> HtmlImportSession {
    let (tx, rx) = mpsc::channel();
    // Same overlay flag + same "no dirty-mark" rationale as the
    // Figma session: the import replaces `editor_state` whole-cloth,
    // so rebuilding the old layout would be wasted work.
    host.editor_state_mut().editor_ui.figma_import_in_progress = true;

    let path_for_thread = path.clone();
    thread::Builder::new()
        .name("op-html-import".into())
        .spawn(move || {
            let result = parse_path(&path_for_thread);
            let _ = tx.send(result);
        })
        .expect("spawn op-html-import worker");

    HtmlImportSession { path, rx }
}

fn parse_path(path: &Path) -> Result<PreparedImport, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let file_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("HTML Import");
    let base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    // Relative references only resolve when the importer has a base
    // URL, so file import uses the same synthetic local origin as the
    // CLI (`html_cli.rs`): resolved URLs come back prefixed with the
    // origin and are stripped to same-directory disk lookups. Real
    // remote URLs keep their own scheme, fail the strip, and are
    // rejected by `local_resource_fetch` — file import never touches
    // the network (the importer records a warning instead).
    let fetcher = move |url: &str| {
        let href = url.strip_prefix(LOCAL_RESOURCE_ORIGIN).unwrap_or(url);
        local_resource_fetch(&base_dir, href)
    };
    // Down-scale embedded bitmaps exactly like the Figma import path.
    let transform =
        |bytes: &[u8]| crate::image_downscale::maybe_downscale(bytes).map(|(_mime, out)| out);
    let opts = op_html::HtmlImportOptions {
        document_name: Some(file_name.to_string()),
        base_url: Some(format!("{LOCAL_RESOURCE_ORIGIN}document.html")),
        ..Default::default()
    };
    let result = op_html::import_html_document(&source, &opts, Some(&fetcher), Some(&transform));
    if result.document.children.is_empty() {
        return Err(result
            .warnings
            .first()
            .cloned()
            .unwrap_or_else(|| "no importable content".to_string()));
    }
    let state = EditorState::from_document(result.document);
    Ok(PreparedImport {
        state,
        warnings: result.warnings,
    })
}

/// Resolve a relative resource reference against the HTML file's
/// directory. Refuses anything that is not a plain same-tree
/// relative path: remote URLs, `data:`, absolute paths, and `..`
/// escapes all return `None` (the importer degrades with a warning).
pub(crate) fn local_resource_fetch(dir: &Path, href: &str) -> Option<Vec<u8>> {
    if href.starts_with("http://")
        || href.starts_with("https://")
        || href.starts_with("//")
        || href.starts_with("data:")
        || Path::new(href).is_absolute()
    {
        return None;
    }
    let candidate = dir.join(href);
    let resolved = candidate.canonicalize().ok()?;
    let dir_resolved = dir.canonicalize().ok()?;
    if !resolved.starts_with(&dir_resolved) {
        return None;
    }
    std::fs::read(resolved).ok()
}

/// Non-blocking drain — same contract as
/// `figma_import_session::pump`.
pub fn pump(
    host: &mut WidgetHostNative,
    session: &mut Option<HtmlImportSession>,
    current_path: &mut Option<PathBuf>,
    window: Option<&winit::window::Window>,
) -> PumpOutcome {
    let Some(sess) = session.as_mut() else {
        return PumpOutcome::Idle;
    };
    match sess.rx.try_recv() {
        Ok(Ok(prepared)) => {
            for warning in &prepared.warnings {
                eprintln!("[import-html] warning: {warning}");
            }
            host.install_imported_state(prepared.state);
            // Imported docs have no `.op` path; next Save routes via
            // Save As — matches the Figma import behaviour.
            *current_path = None;
            refresh_title(window);
            *session = None;
            PumpOutcome::CompletedOk
        }
        Ok(Err(e)) => {
            eprintln!("[import-html] {e}");
            show_error_dialog_public(host, ErrorKind::Open, Some(&sess.path), &e);
            host.editor_state_mut().editor_ui.figma_import_in_progress = false;
            host.mark_editor_state_dirty();
            *session = None;
            PumpOutcome::CompletedErr
        }
        Err(TryRecvError::Empty) => PumpOutcome::StillPending,
        Err(TryRecvError::Disconnected) => {
            eprintln!("[import-html] worker thread terminated without sending a result");
            let detail = "HTML import worker exited unexpectedly";
            show_error_dialog_public(host, ErrorKind::Open, Some(&sess.path), detail);
            host.editor_state_mut().editor_ui.figma_import_in_progress = false;
            host.mark_editor_state_dirty();
            *session = None;
            PumpOutcome::CompletedErr
        }
    }
}

/// Drop the active session (if any) and clear the in-progress flag —
/// called when another document-replacing action starts while an
/// HTML import is still parsing. Mirrors
/// `figma_import_session::cancel`.
pub fn cancel(host: &mut WidgetHostNative, session: &mut Option<HtmlImportSession>) {
    if session.is_some() {
        eprintln!("[import-html] cancelling in-flight session — superseded");
        *session = None;
        if host.editor_state().editor_ui.figma_import_in_progress {
            host.editor_state_mut().editor_ui.figma_import_in_progress = false;
            host.mark_editor_state_dirty();
        }
    }
}

fn refresh_title(window: Option<&winit::window::Window>) {
    let Some(window) = window else { return };
    window.set_title("OpenPencil");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_fetches_relative_stylesheet() {
        let dir = std::env::temp_dir().join("op_html_session_test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("s.css"), b".x { color: #ff0000 }").unwrap();
        std::fs::write(
            dir.join("page.html"),
            b"<html><head><link rel=\"stylesheet\" href=\"s.css\"></head>\
              <body><p class=\"x\">t</p></body></html>",
        )
        .unwrap();
        let prepared = parse_path(&dir.join("page.html")).expect("parse");
        assert!(
            prepared
                .warnings
                .iter()
                .all(|w| !w.contains("external stylesheet skipped")),
            "relative stylesheet must be fetched: {:?}",
            prepared.warnings
        );
    }

    #[test]
    fn local_fetch_confines_to_directory() {
        let dir = std::env::temp_dir().join("op_html_fetch_test");
        let _ = std::fs::create_dir_all(dir.join("sub"));
        std::fs::write(dir.join("a.css"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("b.css"), b"y").unwrap();
        assert_eq!(
            local_resource_fetch(&dir, "a.css").as_deref(),
            Some(b"x".as_ref())
        );
        assert_eq!(
            local_resource_fetch(&dir, "sub/b.css").as_deref(),
            Some(b"y".as_ref())
        );
        assert!(local_resource_fetch(&dir, "../outside.css").is_none());
        assert!(local_resource_fetch(&dir, "/etc/hosts").is_none());
        assert!(local_resource_fetch(&dir, "https://a.dev/x.css").is_none());
        assert!(local_resource_fetch(&dir, "data:text/css,x").is_none());
    }
}
