//! Headless web-canvas daemon — serves the document to the Rust WASM web shell
//! (`op-host-web`, which runs in a browser and can't bind a socket) and to
//! external MCP/CLI clients. It is the Rust analog of the TS web app's
//! `apps/web/server/api/mcp/*` Nitro routes + `setSyncDocument`: it owns the
//! canonical document in memory and answers the same whole-document REST sync
//! shape, so a JSON-RPC/REST client (e.g. the `op` CLI or any MCP client) can
//! drive the Rust *web* canvas the same way it drives the desktop canvas.
//!
//! This module ships the request-handling CORE (`handle_web_canvas_request`,
//! fully unit-testable without a socket) plus a runnable loop
//! (`run_web_canvas`, behind
//! `openpencil-desktop --serve-web <port> [doc] [--host <addr>]`).
//! Layered on top: an SSE endpoint that streams `version` bumps to connected
//! shells, static serving of the host page + WASM bundle (`crate::web_static`
//! — `GET /` and `GET /pkg/*`), and a token-authed `openpencil/shutdown`
//! (same contract as `--mcp-http`) so `op stop` works against this daemon.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use base64::Engine as _;
use op_editor_core::agent_settings::{AcpAgentConnectOutcome, ProviderConnectOutcome};
use op_editor_core::{AgentSettings, EditorState};

use crate::web_credential_policy::WebCredentialPersistence;

/// Slow/stalled-peer bound — bodies can be large (whole documents with embedded
/// images), so a connection that opens and dribbles must not pin a thread.
const IO_TIMEOUT: Duration = Duration::from_secs(30);

/// Max concurrent connections (SSE streams are long-lived, so bound them).
const MAX_CONNS: usize = 64;

/// SSE keep-alive cadence — also how quickly a disconnected SSE client is
/// detected (the heartbeat write fails once the socket is gone).
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);

/// Broadcast hub for SSE subscribers. Each `GET /api/mcp/events` connection
/// registers a channel; a document mutation broadcasts the new version to all
/// of them, and each SSE connection thread writes it to its socket. Senders to
/// disconnected clients are pruned on the next broadcast.
#[derive(Default)]
pub struct SseHub {
    subscribers: Mutex<Vec<mpsc::Sender<u64>>>,
}

impl SseHub {
    /// Register a subscriber; the SSE connection thread blocks on the returned
    /// receiver for version bumps.
    pub(crate) fn subscribe(&self) -> Receiver<u64> {
        let (tx, rx) = mpsc::channel();
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(tx);
        rx
    }

    /// Broadcast a version bump to all live subscribers, pruning any whose
    /// receiver was dropped (client disconnected).
    pub(crate) fn broadcast(&self, version: u64) {
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|tx| tx.send(version).is_ok());
    }

    #[cfg(test)]
    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

/// RAII decrement for the connection counter — `Drop` runs on normal exit AND
/// panic unwind, so a panicking connection can't leak its `MAX_CONNS` slot.
struct ConnGuard(Arc<AtomicUsize>);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// In-memory document authority for the web-canvas daemon — the Rust mirror of
/// the TS `mcp-sync-state` (document + monotonic version). The browser shell
/// mirrors this (over the SSE endpoint, layered on later); external MCP clients
/// read/replace it over `/api/mcp/document`.
pub struct WebCanvasState {
    pub(crate) editor: EditorState,
    /// Immutable deployment policy for browser-supplied credentials. Public
    /// demo deployments fail closed to browser-only persistence.
    pub(crate) credential_persistence: WebCredentialPersistence,
    /// Local file backing this daemon document, when `--serve-web` was launched
    /// with a path or the user opened a recent local file through the daemon.
    pub(crate) current_path: Option<PathBuf>,
    /// Monotonic sync version, bumped on every document mutation — the key the
    /// browser shell uses to detect that the live document changed.
    pub(crate) version: u64,
    /// The bound port, reported by `GET /api/mcp/server` (TS `server.get.ts`
    /// parity).
    pub(crate) port: u16,
    /// Managed-mode per-instance auth token (see `ServeWebOptions::managed`
    /// / `run_web_canvas`). `None` outside managed mode. Read by `serve_one`
    /// (via `RequestAuth`) to gate every privileged request.
    pub(crate) managed_token: Option<String>,
    /// Managed-mode `--allow-origin` allowlist. Empty outside managed mode.
    /// Read by `serve_one` (via `cors_origin_for`) to decide which `Origin`
    /// is echoed back in `Access-Control-Allow-Origin`.
    pub(crate) allow_origins: Vec<String>,
    /// Idempotence guard for `POST /api/mcp/sync-reset`: set once the FIRST
    /// successful reset in this process's lifetime completes. A failed reset
    /// does not set it (retryable); once set, further sync-reset calls are a
    /// no-op `{"ok":true,"skipped":true,"version":<current>}` reply instead
    /// of touching the document again.
    pub(crate) reset_consumed: bool,
}

impl WebCanvasState {
    #[cfg(test)]
    pub(crate) fn new(editor: EditorState, port: u16) -> Self {
        Self::new_with_path(editor, port, None)
    }

    #[cfg(test)]
    pub(crate) fn new_with_policy(
        editor: EditorState,
        port: u16,
        credential_persistence: WebCredentialPersistence,
    ) -> Self {
        Self::new_with_path_and_policy(editor, port, None, credential_persistence)
    }

    #[cfg(test)]
    pub(crate) fn new_with_path(
        editor: EditorState,
        port: u16,
        current_path: Option<PathBuf>,
    ) -> Self {
        Self::new_with_path_and_policy(
            editor,
            port,
            current_path,
            WebCredentialPersistence::BrowserOnly,
        )
    }

    fn new_with_path_and_policy(
        mut editor: EditorState,
        port: u16,
        current_path: Option<PathBuf>,
        credential_persistence: WebCredentialPersistence,
    ) -> Self {
        if !credential_persistence.server_persistence() {
            let _ = crate::web_credentials::remove_browser_owned_credentials(&mut editor);
        }
        Self {
            editor,
            credential_persistence,
            current_path,
            version: 0,
            port,
            managed_token: None,
            allow_origins: Vec::new(),
            reset_consumed: false,
        }
    }

    /// Replace the whole document (an already-loaded `POST /api/mcp/document`
    /// body), bump and return the new version.
    pub(crate) fn replace_document(&mut self, doc: jian_ops_schema::PenDocument) -> u64 {
        self.editor.replace_document(doc);
        self.version += 1;
        self.version
    }

    /// Clear the transient web-sync document back to the same starter document a
    /// fresh browser shell paints before the daemon applies updates.
    pub(crate) fn reset_document(&mut self) -> Result<u64, String> {
        if let Some(path) = self.current_path.clone() {
            let mut next = crate::mcp_serve::load_editor_state(&path)?;
            preserve_web_canvas_preferences(&self.editor, &mut next);
            set_file_name_display(&mut next, &path);
            self.editor = next;
        } else {
            self.editor.replace_document(EditorState::starter().doc);
        }
        self.version += 1;
        Ok(self.version)
    }

    /// Idempotent wrapper around [`Self::reset_document`] for
    /// `POST /api/mcp/sync-reset`: the first SUCCESSFUL reset in this
    /// process's lifetime consumes `reset_consumed`; every call after that
    /// is a no-op (`skipped: true`) that neither reloads nor bumps the
    /// version. A failed reset does NOT consume the guard — it stays
    /// retryable, and the document/version are left exactly as
    /// `reset_document`'s own `?`-early-return already guarantees (nothing
    /// touched before the fallible load succeeds).
    pub(crate) fn reset_document_guarded(&mut self) -> Result<ResetOutcome, String> {
        if self.reset_consumed {
            return Ok(ResetOutcome { skipped: true });
        }
        self.reset_document()?;
        self.reset_consumed = true;
        Ok(ResetOutcome { skipped: false })
    }

    /// Push a whole-document replacement (`POST /api/mcp/document` body).
    /// Consolidates the route's former inline parse-then-replace logic into
    /// one testable entry point. `base_version_override` is test-only — the
    /// real route always passes `None`, in which case an optional top-level
    /// `baseVersion` field inside `body` itself is consulted (extracted via
    /// serde, not a hand-rolled scan). When a base version is present and
    /// does not match the current version, the write is rejected WITHOUT
    /// mutating anything: that's a version verdict conveyed through
    /// `PushOutcome::applied == false`, not an `Err`. `Err` is reserved for
    /// the existing malformed-JSON / schema-validation failures, which stay
    /// client-fault → HTTP 400 at the route exactly as before.
    pub(crate) fn apply_document_push(
        &mut self,
        body: &str,
        base_version_override: Option<u64>,
    ) -> Result<PushOutcome, String> {
        let document_json = crate::mcp_serve::parse_document_sync_body(body)?;
        let base_version = base_version_override.or_else(|| extract_base_version(body));
        if let Some(expected) = base_version {
            if expected != self.version {
                return Ok(PushOutcome {
                    applied: false,
                    current_version: self.version,
                });
            }
        }
        // Load via the same proven path as desktop file-open. A load failure
        // is a client fault → 400, like the TS validation 400s.
        let loaded = op_pen_loader::load_canonical(&document_json).map_err(|e| e.to_string())?;
        for w in &loaded.warnings {
            eprintln!("openpencil-desktop --serve-web: schema warning: {w:?}");
        }
        let version = self.replace_document(loaded.value);
        Ok(PushOutcome {
            applied: true,
            current_version: version,
        })
    }

    #[cfg(test)]
    pub(crate) fn document_version_for_test(&self) -> u64 {
        self.version
    }
}

/// Outcome of [`WebCanvasState::reset_document_guarded`].
pub(crate) struct ResetOutcome {
    pub skipped: bool,
}

/// Outcome of [`WebCanvasState::apply_document_push`] — expresses only the
/// version verdict (parse/schema failures are `Err`, handled separately).
pub(crate) struct PushOutcome {
    pub applied: bool,
    pub current_version: u64,
}

/// Extract an optional top-level `baseVersion` from a document-sync POST
/// body via serde — never a hand-rolled scan. Malformed JSON here simply
/// yields `None`; `parse_document_sync_body` already owns rejecting a
/// malformed body with the real client-facing error.
fn extract_base_version(body: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("baseVersion").and_then(|v| v.as_u64()))
}

/// A handled reply: HTTP status line + JSON body, ready for
/// `write_mcp_http_response`.
pub struct WebReply {
    pub(crate) status: &'static str,
    pub(crate) body: String,
}

fn persist_api_settings<F>(
    method: &str,
    path: &str,
    state: &mut WebCanvasState,
    settings_before: crate::settings_io::Fingerprint,
    credential_settings_before: Option<AgentSettings>,
    reply: WebReply,
    save_credentials: F,
) -> WebReply
where
    F: FnOnce(&EditorState) -> Result<(), String>,
{
    if method == "POST" && path == "/api/settings/credentials" && reply.status == "200 OK" {
        if save_credentials(&state.editor).is_err() {
            if let Some(settings) = credential_settings_before {
                state.editor.editor_ui.agent_settings = settings;
                state.editor.rebuild_chat_models();
            }
            return WebReply {
                status: "500 Internal Server Error",
                body: crate::mcp_serve::rest_error_body("failed to persist server credentials"),
            };
        }
    } else {
        crate::settings_io::save_if_changed(&state.editor, settings_before);
    }
    reply
}

/// Handle one parsed web-canvas REST request against the in-memory state. Pure
/// w.r.t. IO — fully unit-testable without a socket. Mirrors the TS Nitro
/// routes:
/// - `GET  /api/mcp/server`   → health `{ok:true,…}` (like `server.get.ts`)
/// - `GET  /api/mcp/document` → `{document:<doc>,version}` (like `document.get.ts`)
/// - `POST /api/mcp/document` → whole-doc replace → `{ok:true,version}` (like
///   `document.post.ts`); an optional top-level `baseVersion` makes the write
///   conditional — a stale value 409s with `{ok:false,error:"version-conflict",
///   version}` and leaves the document untouched instead of clobbering it
/// - `GET  /api/mcp/version`  → `{version}` — Rust-only cheap change probe; the
///   TS stack pushes documents over SSE instead, so it never needs one. The
///   browser shell polls this and fetches the full document only on a bump.
/// - `GET  /api/mcp/selection` → `{selectedIds,activePageId}` (like `selection.get.ts`)
/// - `POST /api/mcp/selection` → renderer selection push (like `selection.post.ts`)
/// - `POST /api/file/save` → write the browser document to this daemon's
///   backing local path, including embedded active-page metadata
///   (and legacy `.opmeta` fallback compatibility)
/// - `POST /api/file/open-recent` → local-daemon recent-file open, used by the
///   browser shell because only the daemon can read local paths.
/// - anything else → 404 (the JSON-RPC `/mcp` path + SSE are handled by the
///   caller's connection loop, not here).
pub fn handle_web_canvas_request(
    method: &str,
    path: &str,
    body: &str,
    state: &mut WebCanvasState,
) -> WebReply {
    match (method, path) {
        ("GET", "/api/mcp/server") => WebReply {
            status: "200 OK",
            // `{running,port,localIp}` matches TS `server.get.ts`; the daemon
            // binds 127.0.0.1 (localhost-only) so localIp is loopback. Extra
            // `server`/`mode` fields are additive diagnostics.
            body: format!(
                r#"{{"running":true,"port":{},"localIp":"127.0.0.1","server":"openpencil-mcp","mode":"web-canvas"}}"#,
                state.port
            ),
        },
        ("POST", "/api/mcp/server") => update_mcp_server_settings(body, state),
        ("GET", "/api/mcp/document") => match serde_json::to_string(&state.editor.doc) {
            Ok(doc_json) => WebReply {
                status: "200 OK",
                body: format!(r#"{{"document":{doc_json},"version":{}}}"#, state.version),
            },
            Err(e) => WebReply {
                status: "500 Internal Server Error",
                body: crate::mcp_serve::rest_error_body(&e.to_string()),
            },
        },
        ("POST", "/api/mcp/document") => match state.apply_document_push(body, None) {
            Ok(outcome) if outcome.applied => WebReply {
                status: "200 OK",
                body: crate::mcp_serve::document_sync_ok(outcome.current_version),
            },
            Ok(outcome) => WebReply {
                // Stale baseVersion: reject without writing, TS-style error
                // envelope plus the current version so the caller can decide
                // whether to refetch and retry.
                status: "409 Conflict",
                body: serde_json::json!({
                    "ok": false,
                    "error": "version-conflict",
                    "version": outcome.current_version,
                })
                .to_string(),
            },
            Err(message) => WebReply {
                status: "400 Bad Request",
                body: crate::mcp_serve::rest_error_body(&message),
            },
        },
        ("GET", "/api/mcp/version") => WebReply {
            status: "200 OK",
            body: format!(r#"{{"version":{}}}"#, state.version),
        },
        ("GET", "/api/mcp/indicators") => WebReply {
            // Agent-indicator relay: design runs execute inside this
            // daemon, so the process-global registry the canvas paints
            // from lives HERE — the browser polls this and mirrors it
            // into its own registry (agent_indicators::apply_remote) so
            // agent borders / badges / reveal animations show on web.
            status: "200 OK",
            body: op_editor_core::agent_indicators::relay_json(),
        },
        ("POST", "/api/mcp/sync-reset") => match state.reset_document_guarded() {
            Ok(outcome) if outcome.skipped => WebReply {
                status: "200 OK",
                body: format!(r#"{{"ok":true,"skipped":true,"version":{}}}"#, state.version),
            },
            Ok(_) => WebReply {
                status: "200 OK",
                body: crate::mcp_serve::document_sync_ok(state.version),
            },
            Err(e) => WebReply {
                status: "400 Bad Request",
                body: crate::mcp_serve::rest_error_body(&format!("sync reset failed: {e}")),
            },
        },
        ("GET", "/api/mcp/selection") => {
            // TS `selection.get.ts` → `getSyncSelection()` shape:
            // `{selectedIds, activePageId}`. Read straight off the live
            // editor selection so MCP clients and the REST route agree.
            let ids: Vec<&str> = state
                .editor
                .selection
                .set
                .iter()
                .map(|id| id.as_str())
                .collect();
            let active_page_id = state
                .editor
                .doc
                .pages
                .as_ref()
                .and_then(|pages| pages.get(state.editor.ui.active_page_index))
                .map(|page| page.id.clone());
            let body = serde_json::json!({
                "selectedIds": ids,
                "activePageId": active_page_id,
            });
            WebReply {
                status: "200 OK",
                body: serde_json::to_string(&body)
                    .unwrap_or_else(|_| r#"{"selectedIds":[],"activePageId":null}"#.to_string()),
            }
        }
        ("POST", "/api/mcp/selection") => apply_selection_sync(body, state),
        ("POST", "/api/file/save") => save_current_file(body, state),
        ("POST", "/api/file/open-recent") => open_recent_file(body, state),
        ("POST", "/api/export/pdf") => export_pdf_download(body, state),
        ("POST", "/api/export/raster") => export_raster_download(body, state),
        ("GET", "/api/ai/models") => WebReply {
            // JSON array of model ids the AI proxy can serve (the
            // configured built-in agents). The web bundle queries this
            // to populate its model picker without bundling a static
            // list or holding API keys. `POST /api/ai/stream` is a
            // streaming route handled in the connection loop, not here.
            status: "200 OK",
            body: crate::ai_proxy::models_json(&state.editor),
        },
        ("GET", "/api/settings/credential-policy") => WebReply {
            status: "200 OK",
            body: serde_json::json!({
                "serverPersistence": state.credential_persistence.server_persistence(),
            })
            .to_string(),
        },
        ("POST", "/api/settings/credentials") => {
            if !state.credential_persistence.server_persistence() {
                WebReply {
                    status: "403 Forbidden",
                    body: crate::mcp_serve::rest_error_body(
                        "server credential persistence is disabled",
                    ),
                }
            } else {
                match crate::web_credentials::apply_json(&mut state.editor, body) {
                    Ok(()) => WebReply {
                        status: "200 OK",
                        body: r#"{"ok":true}"#.into(),
                    },
                    Err(message) => WebReply {
                        status: if body.len()
                            > crate::web_credentials::MAX_CREDENTIAL_BODY_BYTES
                        {
                            "413 Payload Too Large"
                        } else {
                            "400 Bad Request"
                        },
                        body: crate::mcp_serve::rest_error_body(&message),
                    },
                }
            }
        }
        _ => WebReply {
            status: "404 Not Found",
            body: r#"{"ok":false,"error":"Not found. Use /api/mcp/document, /api/mcp/sync-reset, /api/mcp/server, /api/file/save, /api/export/raster, /api/export/pdf, or /mcp."}"#
                .to_string(),
        },
    }
}

struct RasterDownload {
    file_name: &'static str,
    mime: &'static str,
    bytes: Vec<u8>,
}

fn export_raster_download(body: &str, state: &WebCanvasState) -> WebReply {
    match build_raster_download(body, &state.editor) {
        Ok(download) => WebReply {
            status: "200 OK",
            body: serde_json::json!({
                "ok": true,
                "fileName": download.file_name,
                "mime": download.mime,
                "dataBase64": base64::engine::general_purpose::STANDARD.encode(download.bytes),
            })
            .to_string(),
        },
        Err(e) => WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body(&format!("export raster failed: {e}")),
        },
    }
}

fn build_raster_download(body: &str, fallback: &EditorState) -> Result<RasterDownload, String> {
    let parsed = parse_export_body(body)?;
    let editor = export_editor_from_value(parsed.as_ref(), fallback)?;
    let (format, file_name, mime, ext) = raster_format_from_export_body(parsed.as_ref())?;
    let scale = parsed
        .as_ref()
        .and_then(|body| body.get("scale"))
        .and_then(|scale| scale.as_f64())
        .map(|scale| scale as f32)
        .unwrap_or(editor.editor_ui.export_scale);
    let selected_node_id = selected_node_id_from_export_body(parsed.as_ref(), &editor);
    let scene = op_pen_loader::editor_state_to_layout_scene(&editor);
    let tmp = tmp_export_path(ext);
    let result = match selected_node_id {
        Some(id) => crate::export::export_node_raster(&scene, &id, &tmp, format, scale),
        None => crate::export::export_raster(&scene, &tmp, format, scale),
    };
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    let bytes = std::fs::read(&tmp).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    Ok(RasterDownload {
        file_name,
        mime,
        bytes,
    })
}

fn raster_format_from_export_body(
    body: Option<&serde_json::Value>,
) -> Result<
    (
        crate::export::RasterFormat,
        &'static str,
        &'static str,
        &'static str,
    ),
    String,
> {
    let format = body
        .and_then(|body| body.get("format"))
        .and_then(|format| format.as_str())
        .unwrap_or("png");
    match format {
        "png" => Ok((
            crate::export::RasterFormat::Png,
            "openpencil-export.png",
            "image/png",
            "png",
        )),
        "jpeg" | "jpg" => Ok((
            crate::export::RasterFormat::Jpeg,
            "openpencil-export.jpg",
            "image/jpeg",
            "jpg",
        )),
        "webp" => Ok((
            crate::export::RasterFormat::Webp,
            "openpencil-export.webp",
            "image/webp",
            "webp",
        )),
        other => Err(format!("unsupported raster format: {other}")),
    }
}

fn selected_node_id_from_export_body(
    body: Option<&serde_json::Value>,
    editor: &EditorState,
) -> Option<String> {
    body.and_then(|body| body.get("selectedNodeId"))
        .and_then(|node_id| node_id.as_str())
        .filter(|node_id| !node_id.trim().is_empty())
        .map(|node_id| node_id.to_string())
        .or_else(|| {
            if editor.selection_count() == 1 && editor.selection.anchor.is_real() {
                Some(editor.selection.anchor.as_str().to_string())
            } else {
                None
            }
        })
}

fn export_pdf_download(body: &str, state: &WebCanvasState) -> WebReply {
    match build_pdf_download(body, &state.editor) {
        Ok(bytes) => WebReply {
            status: "200 OK",
            body: serde_json::json!({
                "ok": true,
                "fileName": "openpencil-export.pdf",
                "mime": "application/pdf",
                "dataBase64": base64::engine::general_purpose::STANDARD.encode(bytes),
            })
            .to_string(),
        },
        Err(e) => WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body(&format!("export PDF failed: {e}")),
        },
    }
}

fn build_pdf_download(body: &str, fallback: &EditorState) -> Result<Vec<u8>, String> {
    let editor = export_editor_from_body(body, fallback)?;
    let scene = op_pen_loader::editor_state_to_layout_scene(&editor);
    let tmp = tmp_export_path("pdf");
    crate::export_pdf::export_pdf(&scene, &tmp)?;
    let bytes = std::fs::read(&tmp).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&tmp);
    Ok(bytes)
}

fn export_editor_from_body(body: &str, fallback: &EditorState) -> Result<EditorState, String> {
    let parsed = parse_export_body(body)?;
    export_editor_from_value(parsed.as_ref(), fallback)
}

fn export_editor_from_value(
    body: Option<&serde_json::Value>,
    fallback: &EditorState,
) -> Result<EditorState, String> {
    let Some(doc) = body.and_then(|body| body.get("document")) else {
        return Ok(fallback.clone());
    };
    if !doc.is_object() {
        return Err("document must be an object".into());
    }
    let src = serde_json::to_string(doc).map_err(|e| e.to_string())?;
    let loaded = op_pen_loader::load_canonical(&src).map_err(|e| e.to_string())?;
    let mut editor = EditorState::from_document(loaded.value);
    if let Some(index) = body
        .and_then(|body| body.get("activePageIndex"))
        .and_then(|index| index.as_u64())
        .map(|index| index as usize)
    {
        let page_count = editor
            .doc
            .pages
            .as_ref()
            .map(|pages| pages.len())
            .unwrap_or(1)
            .max(1);
        editor.ui.active_page_index = index.min(page_count - 1);
    }
    Ok(editor)
}

fn parse_export_body(body: &str) -> Result<Option<serde_json::Value>, String> {
    if body.trim().is_empty() {
        return Ok(None);
    }
    serde_json::from_str(body)
        .map(Some)
        .map_err(|e| format!("parse request body: {e}"))
}

fn tmp_export_path(ext: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "openpencil-web-export-{}-{:?}.{ext}",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn save_current_file(body: &str, state: &mut WebCanvasState) -> WebReply {
    let Some(path) = state.current_path.clone() else {
        return WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body("No file path is bound to this web session"),
        };
    };
    match save_editor_from_body(body, &state.editor, &path) {
        Ok(next) => {
            state.editor = next;
            state.version += 1;
            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "untitled.op".to_string());
            WebReply {
                status: "200 OK",
                body: serde_json::json!({
                    "ok": true,
                    "version": state.version,
                    "fileName": file_name,
                })
                .to_string(),
            }
        }
        Err(e) => WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body(&format!("save failed: {e}")),
        },
    }
}

fn save_editor_from_body(
    body: &str,
    previous: &EditorState,
    path: &std::path::Path,
) -> Result<EditorState, String> {
    let (doc, active_page_index) = document_and_active_page_from_body(body)?;
    let mut next = previous.clone();
    next.replace_document(doc);
    if let Some(index) = active_page_index {
        let page_count = next
            .doc
            .pages
            .as_ref()
            .map(|pages| pages.len())
            .unwrap_or(1)
            .max(1);
        next.ui.active_page_index = index.min(page_count - 1);
    }
    set_file_name_display(&mut next, path);
    crate::doc_io::save_to_path(&next, path)?;
    Ok(next)
}

fn document_and_active_page_from_body(
    body: &str,
) -> Result<(jian_ops_schema::PenDocument, Option<usize>), String> {
    let parsed: serde_json::Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let Some(doc) = parsed.get("document") else {
        return Err("missing document".into());
    };
    if !doc.is_object() {
        return Err("document must be an object".into());
    }
    let doc_json = serde_json::to_string(doc).map_err(|e| e.to_string())?;
    let loaded = op_pen_loader::load_canonical(&doc_json).map_err(|e| e.to_string())?;
    for w in &loaded.warnings {
        eprintln!("openpencil-desktop --serve-web: schema warning: {w:?}");
    }
    let active_page_index = parsed
        .get("activePageIndex")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    Ok((loaded.value, active_page_index))
}

fn update_mcp_server_settings(body: &str, state: &mut WebCanvasState) -> WebReply {
    let parsed: serde_json::Value = match serde_json::from_str(body) {
        Ok(value) => value,
        Err(_) => {
            return WebReply {
                status: "400 Bad Request",
                body: crate::mcp_serve::rest_error_body("Invalid MCP server request body"),
            };
        }
    };
    let action = parsed
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    let port = match parsed.get("port").and_then(|v| v.as_u64()) {
        Some(raw) if raw <= u16::MAX as u64 => Some((raw as u16).max(1024)),
        Some(_) => {
            return WebReply {
                status: "400 Bad Request",
                body: crate::mcp_serve::rest_error_body("Invalid MCP server port"),
            };
        }
        None => None,
    };
    let server = &mut state.editor.editor_ui.agent_settings.mcp_server;
    match action {
        "start" => {
            if let Some(port) = port {
                server.port = port;
            }
            server.running = true;
        }
        "stop" => {
            if let Some(port) = port {
                server.port = port;
            }
            server.running = false;
        }
        _ => {
            return WebReply {
                status: "400 Bad Request",
                body: crate::mcp_serve::rest_error_body("Invalid MCP server action"),
            };
        }
    }
    WebReply {
        status: "200 OK",
        body: serde_json::json!({
            "ok": true,
            "running": server.running,
            "port": server.port,
        })
        .to_string(),
    }
}

pub fn handle_acp_agent_connect_request_with_probe<F>(
    body: &str,
    state: &mut WebCanvasState,
    probe: F,
) -> WebReply
where
    F: FnOnce(op_acp::AcpAgentConfig) -> crate::acp_agent_probe_host::AcpAgentProbeOutcome,
{
    let Some(id) = parse_acp_agent_connect_request(body) else {
        return WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body("Missing ACP agent id"),
        };
    };
    let Some(index) = state
        .editor
        .editor_ui
        .agent_settings
        .acp_agents
        .iter()
        .position(|agent| agent.id == id && agent.ready())
    else {
        return WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body("ACP agent is not configured"),
        };
    };
    let agent = state.editor.editor_ui.agent_settings.acp_agents[index].clone();
    state
        .editor
        .editor_ui
        .agent_settings
        .begin_acp_agent_connect(index);
    let outcome = probe(crate::acp_agent_probe_host::acp_config_for_probe(&agent));
    apply_acp_agent_probe_outcome(&id, outcome, state)
}

fn parse_acp_agent_connect_request(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    parsed
        .get("id")
        .or_else(|| parsed.get("agentId"))
        .and_then(|v| v.as_str())
        .filter(|id| !id.trim().is_empty())
        .map(str::to_string)
}

fn apply_acp_agent_probe_outcome(
    id: &str,
    outcome: crate::acp_agent_probe_host::AcpAgentProbeOutcome,
    state: &mut WebCanvasState,
) -> WebReply {
    state
        .editor
        .editor_ui
        .agent_settings
        .apply_acp_agent_connect_outcome(
            id,
            AcpAgentConnectOutcome {
                connected: outcome.connected,
                info: outcome.info.clone(),
                error: outcome.error.clone(),
            },
        );
    state.editor.rebuild_chat_models();
    WebReply {
        status: "200 OK",
        body: serde_json::json!({
            "ok": true,
            "id": id,
            "connected": outcome.connected,
            "connectionInfo": outcome.info,
            "error": outcome.error,
        })
        .to_string(),
    }
}

pub fn handle_provider_connect_request_with_probe<F>(
    body: &str,
    state: &mut WebCanvasState,
    probe: F,
) -> WebReply
where
    F: FnOnce(op_ai::agent_settings_state::AgentProvider) -> crate::provider_probe::ProbeOutcome,
{
    let Some(provider) = parse_provider_connect_request(body) else {
        return WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body("Missing provider"),
        };
    };
    state
        .editor
        .editor_ui
        .agent_settings
        .begin_provider_connect(provider);
    let outcome = probe(provider_to_probe(provider));
    apply_provider_probe_outcome(provider, outcome, state)
}

fn parse_provider_connect_request(body: &str) -> Option<op_editor_core::AgentProvider> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    parsed
        .get("provider")
        .and_then(|v| v.as_str())
        .and_then(parse_agent_provider)
}

fn parse_agent_provider(raw: &str) -> Option<op_editor_core::AgentProvider> {
    use op_editor_core::AgentProvider;
    let normalized = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect::<String>();
    match normalized.as_str() {
        "claude" | "claudecode" => Some(AgentProvider::ClaudeCode),
        "codex" | "codexcli" => Some(AgentProvider::CodexCli),
        "opencode" => Some(AgentProvider::OpenCode),
        "githubcopilot" | "copilot" => Some(AgentProvider::GithubCopilot),
        "gemini" | "geminicli" => Some(AgentProvider::GeminiCli),
        "antigravity" | "agy" => Some(AgentProvider::Antigravity),
        "grok" | "grokbuild" => Some(AgentProvider::GrokBuild),
        _ => None,
    }
}

fn provider_to_probe(
    provider: op_editor_core::AgentProvider,
) -> op_ai::agent_settings_state::AgentProvider {
    use op_ai::agent_settings_state::AgentProvider as ProbeProvider;
    use op_editor_core::AgentProvider;
    match provider {
        AgentProvider::ClaudeCode => ProbeProvider::ClaudeCode,
        AgentProvider::CodexCli => ProbeProvider::CodexCli,
        AgentProvider::OpenCode => ProbeProvider::OpenCode,
        AgentProvider::GithubCopilot => ProbeProvider::GithubCopilot,
        AgentProvider::GeminiCli => ProbeProvider::GeminiCli,
        AgentProvider::Antigravity => ProbeProvider::Antigravity,
        AgentProvider::GrokBuild => ProbeProvider::GrokBuild,
    }
}

fn apply_provider_probe_outcome(
    provider: op_editor_core::AgentProvider,
    outcome: crate::provider_probe::ProbeOutcome,
    state: &mut WebCanvasState,
) -> WebReply {
    let outcome = crate::provider_probe_host::normalize_provider_probe_outcome(provider, outcome);
    let crate::provider_probe::ProbeOutcome {
        connected,
        models,
        error,
        warning,
        not_installed,
        install_command,
        connection_info,
        hint_path,
        version,
    } = outcome;
    let response_models: Vec<serde_json::Value> = models
        .iter()
        .map(|m| {
            serde_json::json!({
                "provider": provider_key(provider),
                "value": m.value,
                "displayName": m.display_name,
            })
        })
        .collect();
    state
        .editor
        .editor_ui
        .agent_settings
        .apply_provider_connect_outcome(
            provider,
            ProviderConnectOutcome {
                connected,
                info: connection_info.clone(),
                warning: warning.clone(),
                error: error.clone(),
                not_installed,
                install_command: install_command.clone(),
                hint_path: hint_path.clone(),
                version: version.clone(),
            },
        );
    state
        .editor
        .editor_ui
        .agent_settings
        .pending_provider_connect = None;
    if connected && !models.is_empty() {
        state
            .editor
            .chat
            .discovered_models
            .retain(|m| m.provider != provider);
        state.editor.chat.discovered_models.extend(
            models
                .into_iter()
                .map(crate::model_discovery::model_entry_to_ec),
        );
        sort_discovered_models(&mut state.editor);
    }
    state.editor.rebuild_chat_models();
    WebReply {
        status: "200 OK",
        body: serde_json::json!({
            "ok": true,
            "provider": provider_key(provider),
            "connected": connected,
            "models": response_models,
            "error": error,
            "warning": warning,
            "notInstalled": not_installed,
            "installCommand": install_command,
            "connectionInfo": connection_info,
            "hintPath": hint_path,
            "version": version,
        })
        .to_string(),
    }
}

fn sort_discovered_models(editor: &mut EditorState) {
    editor.chat.discovered_models.sort_by_key(|m| {
        op_editor_core::AgentProvider::ALL
            .iter()
            .position(|p| *p == m.provider)
            .unwrap_or(usize::MAX)
    });
}

fn provider_key(provider: op_editor_core::AgentProvider) -> &'static str {
    use op_editor_core::AgentProvider;
    match provider {
        AgentProvider::ClaudeCode => "claude",
        AgentProvider::CodexCli => "codex",
        AgentProvider::OpenCode => "opencode",
        AgentProvider::GithubCopilot => "github-copilot",
        AgentProvider::GeminiCli => "gemini",
        AgentProvider::Antigravity => "antigravity",
        AgentProvider::GrokBuild => "grok-build",
    }
}

fn open_recent_file(body: &str, state: &mut WebCanvasState) -> WebReply {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let Some(path_s) = parsed
        .as_ref()
        .and_then(|v| v.get("path"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
    else {
        return WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body("Missing path string"),
        };
    };
    if !state
        .editor
        .editor_ui
        .recent_files
        .iter()
        .any(|recent| recent.path == path_s)
    {
        return WebReply {
            status: "404 Not Found",
            body: crate::mcp_serve::rest_error_body("Path is not in recent files"),
        };
    }
    let path = PathBuf::from(&path_s);
    match crate::mcp_serve::load_editor_state(&path) {
        Ok(mut next) => {
            preserve_web_canvas_preferences(&state.editor, &mut next);
            set_file_name_display(&mut next, &path);
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            next.editor_ui.touch_recent_file(path_s, now);
            state.editor = next;
            state.current_path = Some(path);
            state.version += 1;
            WebReply {
                status: "200 OK",
                body: crate::mcp_serve::document_sync_ok(state.version),
            }
        }
        Err(e) => {
            let pruned = state.editor.editor_ui.remove_recent_file(&path_s);
            WebReply {
                status: "400 Bad Request",
                body: serde_json::json!({
                    "ok": false,
                    "pruned": pruned,
                    "error": e,
                })
                .to_string(),
            }
        }
    }
}

fn preserve_web_canvas_preferences(previous: &EditorState, next: &mut EditorState) {
    let previous_selected_model = previous.chat.selected_model_entry().cloned();
    next.editor_ui.theme_mode = previous.editor_ui.theme_mode;
    next.editor_ui.locale = previous.editor_ui.locale;
    next.editor_ui.recent_files = previous.editor_ui.recent_files.clone();
    next.ui_kits = previous.ui_kits.clone();
    next.theme_presets = previous.theme_presets.clone();
    next.theme_presets_dirty = previous.theme_presets_dirty;
    next.editor_ui.agent_settings = previous.editor_ui.agent_settings.clone();
    next.editor_ui.chat_selected_agent = previous.editor_ui.chat_selected_agent;
    next.chat.discovered_models = previous.chat.discovered_models.clone();
    next.rebuild_chat_models();
    if let Some(prev) = previous_selected_model {
        if let Some(idx) = next.chat.available_models.iter().position(|m| {
            m.provider == prev.provider
                && m.value == prev.value
                && m.builtin_provider_id == prev.builtin_provider_id
        }) {
            next.select_chat_model(idx);
        }
    }
}

fn set_file_name_display(state: &mut EditorState, path: &std::path::Path) {
    state.editor_ui.file_name_display = path.file_name().map(|n| n.to_string_lossy().into_owned());
}

/// Apply a renderer selection push (`POST /api/mcp/selection`) to the live
/// editor state, mirroring TS `selection.post.ts` + `setSyncSelection`:
/// `selectedIds` must be an array (else 400 with the TS error text); the ids
/// are stored verbatim (TS does no validation — the browser's document is the
/// same synced document, so its ids are normally live here too); a present,
/// non-null `activePageId` switches the active page WHEN the id resolves
/// (documented divergence: TS stores the raw string, Rust keeps a page index
/// so an unknown id is ignored rather than stored). Selection is not part of
/// the document, so no version bump / SSE broadcast happens (TS parity).
fn apply_selection_sync(body: &str, state: &mut WebCanvasState) -> WebReply {
    let parsed: Option<serde_json::Value> = serde_json::from_str(body).ok();
    let Some(ids) = parsed
        .as_ref()
        .and_then(|v| v.get("selectedIds"))
        .and_then(|v| v.as_array())
    else {
        return WebReply {
            status: "400 Bad Request",
            body: crate::mcp_serve::rest_error_body("Missing selectedIds array"),
        };
    };
    let node_ids: Vec<op_editor_core::NodeId> = ids
        .iter()
        .filter_map(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(op_editor_core::NodeId::new)
        .collect();
    let editor = &mut state.editor;
    editor.selection.anchor = node_ids
        .last()
        .cloned()
        .unwrap_or(op_editor_core::NodeId::NONE);
    editor.selection.set = node_ids;
    if let Some(page_id) = parsed
        .as_ref()
        .and_then(|v| v.get("activePageId"))
        .and_then(|v| v.as_str())
    {
        let index = editor
            .doc
            .pages
            .as_ref()
            .and_then(|pages| pages.iter().position(|p| p.id == page_id));
        if let Some(index) = index {
            let _ = editor.set_active_page(index);
        }
    }
    WebReply {
        status: "200 OK",
        body: r#"{"ok":true}"#.to_string(),
    }
}

/// Fully parsed `--serve-web` invocation, covering both the legacy
/// positional syntax and the new `--managed` flag syntax (see
/// [`parse_serve_web_args`]).
pub struct ServeWebOptions {
    pub port: u16,
    pub path: Option<PathBuf>,
    pub host: String,
    /// `--managed`: the daemon was spawned by a supervising process (e.g.
    /// the VS Code extension) that expects the handshake-JSON + stdin-EOF
    /// lifecycle contract instead of the legacy fire-and-forget daemon.
    pub managed: bool,
    /// `--allow-origin <origin>` (repeatable), managed mode only. Enforced by
    /// `serve_one` via `cors_origin_for` to gate which `Origin` headers are
    /// echoed back in `Access-Control-Allow-Origin` responses.
    pub allow_origins: Vec<String>,
}

/// Parse the argv tail after `--serve-web` itself. Pure, so the flag shape is
/// unit-testable without spawning the binary. Supports two syntaxes:
///
/// - Legacy positional (unchanged): `<port> [doc] [--host <addr>]`.
/// - Managed flag form: `--managed --port <n|0> [--file <path>]
///   [--host <addr>] [--allow-origin <origin>]...` — used by supervising
///   processes that want the handshake-JSON / stdin-EOF lifecycle contract
///   (see [`run_web_canvas`]).
///
/// The host defaults to loopback; `--host 0.0.0.0` is the LAN/Docker opt-in
/// (no TLS — deploy behind a proxy for anything beyond a trusted network).
pub fn parse_serve_web_args<I: Iterator<Item = String>>(
    mut args: I,
) -> Result<ServeWebOptions, String> {
    let Some(first) = args.next() else {
        return Err("missing <port> arg".into());
    };
    if first.starts_with("--") {
        return parse_serve_web_args_managed(first, args);
    }
    let Ok(port) = first.parse::<u16>() else {
        return Err(format!("<port> must be a u16, got {first:?}"));
    };
    let mut path: Option<PathBuf> = None;
    let mut host = "127.0.0.1".to_string();
    while let Some(arg) = args.next() {
        if arg == "--host" {
            host = args.next().ok_or("--host needs a value (e.g. 0.0.0.0)")?;
        } else if let Some(value) = arg.strip_prefix("--host=") {
            host = value.to_string();
        } else if path.is_none() {
            // The document path is optional — without it the daemon starts
            // from the same starter document the web shell paints locally.
            path = Some(PathBuf::from(arg));
        } else {
            return Err(format!("unexpected arg {arg:?}"));
        }
    }
    if host.is_empty() {
        return Err("--host must not be empty".into());
    }
    Ok(ServeWebOptions {
        port,
        path,
        host,
        managed: false,
        allow_origins: Vec::new(),
    })
}

/// Parse the flag-style `--managed --port <n|0> [--file <path>]
/// [--host <addr>] [--allow-origin <origin>]...` form. `first_flag` is the
/// already-consumed first token (always `--managed` in practice, but any
/// leading `--`-prefixed token routes here so an unknown flag reports a
/// useful error instead of misparsing as a port).
fn parse_serve_web_args_managed<I: Iterator<Item = String>>(
    first_flag: String,
    mut args: I,
) -> Result<ServeWebOptions, String> {
    let mut managed = false;
    let mut port: Option<u16> = None;
    let mut path: Option<PathBuf> = None;
    let mut host = "127.0.0.1".to_string();
    let mut allow_origins: Vec<String> = Vec::new();
    let mut next_flag = Some(first_flag);
    while let Some(arg) = next_flag.take().or_else(|| args.next()) {
        match arg.as_str() {
            "--managed" => managed = true,
            "--port" => {
                let value = args.next().ok_or("--port needs a value")?;
                port = Some(
                    value
                        .parse::<u16>()
                        .map_err(|_| format!("--port must be a u16, got {value:?}"))?,
                );
            }
            "--file" => {
                path = Some(PathBuf::from(args.next().ok_or("--file needs a value")?));
            }
            "--host" => {
                host = args.next().ok_or("--host needs a value (e.g. 0.0.0.0)")?;
            }
            "--allow-origin" => {
                allow_origins.push(args.next().ok_or("--allow-origin needs a value")?);
            }
            other => return Err(format!("unexpected arg {other:?}")),
        }
    }
    let Some(port) = port else {
        return Err("missing --port <n>".into());
    };
    if host.is_empty() {
        return Err("--host must not be empty".into());
    }
    Ok(ServeWebOptions {
        port,
        path,
        host,
        managed,
        allow_origins,
    })
}

/// Build the single-line handshake JSON printed to stdout in managed mode
/// once the listener is bound: `{"ok":true,"port":<n>,"token":"<hex32>",
/// "version":"<crate version>"}`. The supervising process reads exactly one
/// line from the child's stdout to learn the actual bound port (relevant
/// when `--port 0` requested an OS-assigned port) and the per-instance auth
/// token.
pub(crate) fn handshake_json(port: u16, token: &str) -> String {
    format!(
        r#"{{"ok":true,"port":{port},"token":"{token}","version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    )
}

/// Generate a per-instance token for managed mode. Not a cryptographic PRNG —
/// `RandomState`'s per-process keying plus a nanosecond timestamp and the pid
/// give a token that's unguessable across separate daemon invocations. The real
/// access control is enforced by `serve_one` via `RequestAuth` gate on every
/// request and `cors_origin_for` on privileged endpoints like `/mcp`.
fn random_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h1 = s.build_hasher();
    h1.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let mut h2 = s.build_hasher();
    h2.write_u64(std::process::id() as u64);
    format!("{:016x}{:016x}", h1.finish(), h2.finish())
}

fn startup_editor_from_base_for_web_canvas(
    base: EditorState,
    path: Option<PathBuf>,
) -> Result<EditorState, String> {
    match path {
        Some(p) => {
            let mut next = crate::mcp_serve::load_editor_state(&p)?;
            preserve_web_canvas_preferences(&base, &mut next);
            set_file_name_display(&mut next, &p);
            next.editor_ui.touch_recent_file(
                p.to_string_lossy().into_owned(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
            );
            Ok(next)
        }
        None => Ok(base),
    }
}

fn startup_editor_for_web_canvas_with_loader<Checked>(
    path: Option<PathBuf>,
    _policy: WebCredentialPersistence,
    checked_load: Checked,
) -> Result<EditorState, String>
where
    Checked: FnOnce(&mut EditorState) -> Result<(), String>,
{
    let mut base = EditorState::starter();
    checked_load(&mut base)?;
    startup_editor_from_base_for_web_canvas(base, path)
}

fn startup_editor_for_web_canvas_with_policy(
    path: Option<PathBuf>,
    policy: WebCredentialPersistence,
) -> Result<EditorState, String> {
    startup_editor_for_web_canvas_with_loader(path, policy, crate::settings_io::load_checked)
}

pub fn startup_editor_for_web_canvas(path: Option<PathBuf>) -> Result<EditorState, String> {
    startup_editor_for_web_canvas_with_policy(path, crate::web_credential_policy::from_env())
}

/// Run the web-canvas daemon per `options` (host/port default `127.0.0.1`),
/// backed by the document at `options.path` (or the starter document when
/// `None`). Serves the static host page + bundle, the whole-document REST
/// sync + health routes, and falls through to the JSON-RPC `/mcp` tool
/// dispatch (applied against the in-memory document). Blocks until a
/// token-authed shutdown request (or, in managed mode, stdin EOF).
///
/// Managed mode (`options.managed`) layers on the parent-death lease
/// contract used by a supervising process (e.g. the VS Code extension):
/// once the listener is bound, a single-line handshake JSON
/// (`{"ok":true,"port":..,"token":..,"version":..}`) is printed to stdout so
/// the supervisor learns the actual port (relevant for `--port 0`) and a
/// per-instance token; a background thread then reads stdin to EOF/error and
/// raises the same `shutdown` flag the token-authed `openpencil/shutdown`
/// path uses, waking the accept loop by connecting back to the bound
/// address. Non-managed mode is untouched: no token, no handshake output, no
/// stdin thread.
pub fn run_web_canvas(options: ServeWebOptions) -> Result<(), String> {
    let ServeWebOptions {
        port,
        path,
        host,
        managed,
        allow_origins,
    } = options;
    let current_path = path.clone();
    let credential_persistence = crate::web_credential_policy::from_env();
    let mut editor = startup_editor_for_web_canvas_with_policy(path, credential_persistence)?;
    enforce_credential_persistence_policy(
        &mut editor,
        credential_persistence,
        crate::settings_io::save_checked,
    )?;
    let listener =
        TcpListener::bind((host.as_str(), port)).map_err(|e| format!("bind {host}:{port}: {e}"))?;
    let local_addr = listener.local_addr().map_err(|e| e.to_string())?;
    let bound = local_addr.port();
    eprintln!("openpencil-desktop --serve-web: listening on {host}:{bound}");
    match crate::web_static::resolve_bundle_dir() {
        Some(dir) => eprintln!(
            "openpencil-desktop --serve-web: serving web bundle from {}",
            dir.display()
        ),
        None => eprintln!(
            "openpencil-desktop --serve-web: no web bundle found — `/` serves build \
             instructions (tools/check-wasm-bundle.sh, or set OPENPENCIL_WEB_BUNDLE_DIR)"
        ),
    }
    // Shared across connection threads: the document authority (one writer at a
    // time via the Mutex) + the SSE broadcast hub. Thread-per-connection so a
    // long-lived SSE stream (or a slow client) never blocks other clients.
    let state = Arc::new(Mutex::new(WebCanvasState::new_with_path_and_policy(
        editor,
        bound,
        current_path,
        credential_persistence,
    )));
    let hub = Arc::new(SseHub::default());
    let conn_count = Arc::new(AtomicUsize::new(0));
    // Raised by a connection thread that accepted a token-authed
    // `openpencil/shutdown`; the accept loop checks it per iteration. The
    // raiser also pokes the listener with a throwaway connection so a blocked
    // `accept` wakes up and observes the flag.
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Managed mode only: per-instance token + handshake + parent-death
    // lease (stdin-EOF watcher). Non-managed mode never touches this branch
    // — it keeps the existing `OPENPENCIL_MCP_TOKEN` shutdown contract as
    // the only lifecycle signal, byte-for-byte as before.
    let managed_token = managed.then(random_token);
    if let Some(token) = &managed_token {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{}", handshake_json(bound, token));
        let _ = out.flush();
        drop(out);
        let shutdown_stdin = Arc::clone(&shutdown);
        std::thread::spawn(move || {
            let mut sink = [0u8; 64];
            let mut stdin = std::io::stdin();
            while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
            shutdown_stdin.store(true, Ordering::Release);
            // Wake the (possibly blocked) accept loop — reconnect to the
            // bound address exactly (works for IPv6 / custom --host, unlike
            // the loopback-only wake used by the token-authed shutdown
            // path below).
            let _ = std::net::TcpStream::connect(local_addr);
        });
    }
    // Stash the managed token + allow-origins on the shared state. `serve_one`
    // reads them via `RequestAuth` gate (token) and `cors_origin_for` (CORS
    // allowlist) to enforce auth and CORS policies on all incoming requests.
    if managed_token.is_some() || !allow_origins.is_empty() {
        let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
        guard.managed_token = managed_token;
        guard.allow_origins = allow_origins;
    }
    for stream in listener.incoming() {
        if shutdown.load(Ordering::Acquire) {
            break;
        }
        let mut s = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("openpencil-desktop --serve-web: accept: {e}");
                continue;
            }
        };
        if conn_count.load(Ordering::Acquire) >= MAX_CONNS {
            let _ = s.set_write_timeout(Some(IO_TIMEOUT));
            let _ = crate::mcp_serve::write_mcp_http_response(
                &mut s,
                "503 Service Unavailable",
                r#"{"ok":false,"error":"server busy"}"#,
            );
            continue;
        }
        conn_count.fetch_add(1, Ordering::AcqRel);
        let state = Arc::clone(&state);
        let hub = Arc::clone(&hub);
        let conns = Arc::clone(&conn_count);
        let shutdown_flag = Arc::clone(&shutdown);
        let spawned = thread::Builder::new()
            .name("op-serve-web-conn".into())
            .spawn(move || {
                let _conn_guard = ConnGuard(conns);
                let _ = s.set_read_timeout(Some(IO_TIMEOUT));
                let _ = s.set_write_timeout(Some(IO_TIMEOUT));
                match serve_one(&mut s, &state, &hub) {
                    Ok(true) => {
                        shutdown_flag.store(true, Ordering::Release);
                        // Wake the (possibly blocked) accept loop. Loopback
                        // reaches the listener for both the 127.0.0.1 and the
                        // 0.0.0.0 binds.
                        let _ = std::net::TcpStream::connect(("127.0.0.1", bound));
                    }
                    Ok(false) => {}
                    Err(e) => eprintln!("openpencil-desktop --serve-web: {e}"),
                }
            });
        if spawned.is_err() {
            conn_count.fetch_sub(1, Ordering::AcqRel);
        }
    }
    eprintln!("openpencil-desktop --serve-web: shutdown requested; exiting");
    Ok(())
}

fn enforce_credential_persistence_policy<F>(
    editor: &mut EditorState,
    policy: WebCredentialPersistence,
    save: F,
) -> Result<(), String>
where
    F: FnOnce(&EditorState) -> Result<(), String>,
{
    if !policy.server_persistence()
        && crate::web_credentials::remove_browser_owned_credentials(editor)
    {
        save(editor).map_err(|_| {
            "failed to remove browser-owned credentials while server persistence is disabled"
                .to_string()
        })?;
    }
    Ok(())
}

/// Managed-mode request gate (see `ServeWebOptions::managed` /
/// `WebCanvasState::managed_token`). Non-managed daemons construct
/// `RequestAuth { managed: false, .. }`, which `allows` always satisfies —
/// the legacy fire-and-forget daemon stays byte-for-byte tokenless.
pub(crate) struct RequestAuth {
    pub managed: bool,
    pub token: String,
}

impl RequestAuth {
    /// Whether `method path` may proceed without (or with a mismatched)
    /// `presented` token. Mirrors `web_static.rs`'s static route table
    /// (`web_static.rs:188`): everything the static layer serves stays
    /// tokenless — the page cannot know the token before the postMessage
    /// bootstrap hands it over — plus `OPTIONS` preflight. Every other
    /// request (the `POST /` JSON-RPC alias, `/mcp`, `/api/*` including the
    /// SSE `/api/mcp/events` and AI stream endpoints) requires the exact
    /// per-instance token.
    pub(crate) fn allows(&self, method: &str, path: &str, presented: Option<&str>) -> bool {
        if !self.managed {
            return true;
        }
        let static_get = method.eq_ignore_ascii_case("GET")
            && (path == "/"
                || path == "/index.html"
                || path.starts_with("/pkg/")
                || path.starts_with("/smoke/")
                || path.starts_with("/canvaskit/")
                || path.starts_with("/assets/"));
        let exempt = method.eq_ignore_ascii_case("OPTIONS") || static_get;
        exempt || presented == Some(self.token.as_str())
    }
}

/// Managed-mode CORS allowlist check: echoes `origin` back only when it
/// exactly matches an entry in `allow`, otherwise omits the header
/// (`None`). Unmanaged mode never calls this — it keeps the permissive
/// `*` inline at each call site instead.
pub(crate) fn cors_origin_for(allow: &[String], origin: Option<&str>) -> Option<String> {
    origin
        .filter(|o| allow.iter().any(|a| a == o))
        .map(str::to_string)
}

/// Handle one connection. Routes: static host page + wasm bundle (`GET /`,
/// `GET /pkg/*` via `crate::web_static`); SSE live-update stream (`GET
/// /api/mcp/events`); REST whole-doc sync / health (`/api/*` via
/// [`handle_web_canvas_request`]); else JSON-RPC `/mcp` tool dispatch. A
/// mutation (REST POST or a mutating tool call) bumps the version and is
/// broadcast to SSE subscribers. The state `Mutex` is held only across the
/// in-memory operation, never across the (long-lived) SSE wait.
///
/// Managed mode (`WebCanvasState::managed_token`) layers a token gate
/// (`RequestAuth::allows`) in front of every privileged branch below —
/// only the static GET routes and `OPTIONS` preflight stay tokenless — and
/// an origin allowlist (`cors_origin_for`) that replaces the permissive
/// `Access-Control-Allow-Origin: *` with an echo of the exact allowlisted
/// `Origin` (or no header at all). Non-managed mode is untouched: `auth`
/// always allows and `cors_origin` is always `Some("*")`.
///
/// Returns `Ok(true)` when the client requested a token-authed graceful
/// shutdown (same `openpencil/shutdown` contract as `--mcp-http`) — the
/// caller then stops the accept loop so `op stop` never signals a pid.
fn serve_one<S: Read + Write>(
    stream: &mut S,
    state: &Mutex<WebCanvasState>,
    hub: &SseHub,
) -> Result<bool, String> {
    let req = crate::mcp_serve::read_http_request(stream)?;
    let (auth, allow_origins) = {
        let guard = state.lock().unwrap_or_else(|p| p.into_inner());
        let auth = RequestAuth {
            managed: guard.managed_token.is_some(),
            token: guard.managed_token.clone().unwrap_or_default(),
        };
        (auth, guard.allow_origins.clone())
    };
    let cors_origin: Option<String> = if auth.managed {
        cors_origin_for(&allow_origins, req.origin.as_deref())
    } else {
        Some("*".to_string())
    };
    let cors_origin = cors_origin.as_deref();
    if req.method == "OPTIONS" {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "204 No Content",
            "",
            cors_origin,
        )
        .map(|()| false);
    }
    if is_sensitive_browser_post(&req) && !credential_request_origin_allowed(&req) {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "403 Forbidden",
            &crate::mcp_serve::rest_error_body("cross-origin sensitive request is forbidden"),
            cors_origin,
        )
        .map(|()| false);
    }
    // Sensitive JSON routes refuse CORS "simple request" content types
    // (text/plain, form-encoded, or none): a drive-by page can fire those
    // without a preflight, and unmanaged daemons have no token gate.
    if is_sensitive_browser_post(&req) && !content_type_is_json(req.content_type.as_deref()) {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "415 Unsupported Media Type",
            &crate::mcp_serve::rest_error_body(
                "this route requires Content-Type: application/json",
            ),
            cors_origin,
        )
        .map(|()| false);
    }
    // Static serving: the host page (`/`) and the wasm-bindgen bundle
    // (`/pkg/*`). Owns only those paths — everything else falls through.
    if req.method == "GET" {
        let bundle_dir = crate::web_static::resolve_bundle_dir();
        if let Some(reply) =
            crate::web_static::handle_static_request(&req.path, bundle_dir.as_deref())
        {
            return crate::web_static::write_static_response(stream, &reply, cors_origin)
                .map(|()| false);
        }
    }
    // Managed-mode token gate: everything below this point is a privileged
    // branch (SSE, AI streams, `/mcp`, `/api/*`, the `POST /` JSON-RPC
    // alias) — the static GET routes above and the `OPTIONS` preflight
    // already returned. Unmanaged mode's `allows` always returns true, so
    // this is a no-op there.
    if !auth.allows(&req.method, &req.path, req.token.as_deref()) {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "401 Unauthorized",
            r#"{"ok":false,"error":"unauthorized"}"#,
            cors_origin,
        )
        .map(|()| false);
    }
    // SSE live-update stream: the browser shell subscribes and re-syncs whenever
    // the document version advances. Subscribe BEFORE reading the current
    // version so no broadcast is missed (a duplicate is harmless — versions are
    // monotonic). The state lock is released before the long SSE wait.
    if req.method == "GET" && req.path == "/api/mcp/events" {
        let rx = hub.subscribe();
        let current = state.lock().unwrap_or_else(|p| p.into_inner()).version;
        return serve_sse(stream, rx, current, cors_origin).map(|()| false);
    }
    // AI proxy stream: the browser bundle POSTs a model request and we
    // stream the provider's `ChatDelta`s back as SSE. Streaming route
    // (long-lived socket write), so handled here rather than in the
    // whole-body REST handler. Parse the body + build the provider
    // under the state lock, then DROP the lock before the long stream
    // — `proxy_provider` returns an owned `Box<dyn ChatProvider>`, so
    // nothing borrows the editor across the stream.
    if req.method == "POST" && req.path == "/api/ai/stream" {
        let Some(ai_req) = crate::ai_proxy::parse_ai_stream_body(&req.body) else {
            return crate::ai_proxy::write_sse_error(stream, "invalid request body", cors_origin)
                .map_err(|e| format!("ai stream error: {e}"))
                .map(|()| false);
        };
        let provider = {
            let guard = state.lock().unwrap_or_else(|p| p.into_inner());
            crate::ai_proxy::proxy_provider_for_request(
                &guard.editor,
                &ai_req,
                guard.credential_persistence,
            )
        };
        let provider = match provider {
            Ok(Some(provider)) => provider,
            Ok(None) => {
                return crate::ai_proxy::write_sse_error(
                    stream,
                    "no model configured",
                    cors_origin,
                )
                .map_err(|e| format!("ai stream error: {e}"))
                .map(|()| false);
            }
            Err(message) => {
                return crate::ai_proxy::write_sse_error(stream, &message, cors_origin)
                    .map_err(|e| format!("ai stream error: {e}"))
                    .map(|()| false);
            }
        };
        return crate::ai_proxy::stream_ai_response(stream, ai_req, provider.as_ref(), cors_origin)
            .map_err(|e| format!("ai stream: {e}"))
            .map(|()| false);
    }
    // Standard web chat/design turn: same external-CLI routing shape as
    // desktop standard mode (classify → chat / modify / new design), but
    // applied against this web-canvas daemon's document authority.
    if req.method == "POST" && req.path == "/api/ai/standard" {
        let Some(standard_req) = crate::web_chat_standard::parse_standard_turn_body(&req.body)
        else {
            return crate::ai_proxy::write_sse_error(stream, "invalid request body", cors_origin)
                .map_err(|e| format!("ai standard error: {e}"))
                .map(|()| false);
        };
        return crate::web_chat_standard::stream_standard_turn(
            stream,
            standard_req,
            state,
            hub,
            cors_origin,
        )
        .map_err(|e| format!("ai standard: {e}"))
        .map(|()| false);
    }
    // Image panel Search popover (desktop `image_panel_host` parity). Long
    // blocking network (8 s timeout × ladder), so it runs on this
    // connection's own thread AFTER the brief parse-under-lock — the REST
    // handler below holds the state lock for its whole body and must not
    // host provider dials. Living under `/api/ai/` keeps it inside the
    // sensitive-POST origin gate and the managed-mode token gate.
    if req.method == "POST" && req.path == "/api/ai/image/search" {
        let parsed = {
            let guard = state.lock().unwrap_or_else(|p| p.into_inner());
            crate::web_image_search::parse_search_request(&req.body, &guard.editor)
        };
        let (status, body) = match parsed {
            Ok((query, credentials)) => {
                // One slot per running job — each holds this connection
                // thread for minutes of provider network, so unbounded
                // concurrency would exhaust the daemon's threads.
                match crate::web_image_search::ImageJobSlot::acquire() {
                    Some(_slot) => {
                        let outcome = crate::web_image_search::run_search_blocking(
                            &query,
                            credentials.as_ref(),
                        );
                        (
                            "200 OK",
                            crate::web_image_search::search_outcome_to_json(&outcome),
                        )
                    }
                    None => (
                        "429 Too Many Requests",
                        r#"{"ok":false,"error":"too many concurrent image requests"}"#.to_string(),
                    ),
                }
            }
            Err(message) => (
                "400 Bad Request",
                serde_json::json!({ "ok": false, "error": message }).to_string(),
            ),
        };
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            status,
            &body,
            cors_origin,
        )
        .map(|()| false);
    }
    // Image panel Generate popover (desktop `image_generate_host` parity).
    // Same threading rules as the search route; Replicate polling can run
    // for minutes.
    if req.method == "POST" && req.path == "/api/ai/image/generate" {
        let parsed = {
            let guard = state.lock().unwrap_or_else(|p| p.into_inner());
            crate::web_image_generate::parse_generate_request(&req.body, &guard.editor)
        };
        let (status, body) = match parsed {
            // Shares the search route's in-flight ceiling (see above).
            Ok(request) => match crate::web_image_search::ImageJobSlot::acquire() {
                Some(_slot) => match crate::web_image_generate::run_generate_blocking(&request) {
                    Ok(url) => ("200 OK", crate::web_image_generate::generate_ok_json(&url)),
                    Err(message) => (
                        "502 Bad Gateway",
                        crate::web_image_generate::generate_error_json(&message),
                    ),
                },
                None => (
                    "429 Too Many Requests",
                    r#"{"ok":false,"error":"too many concurrent image requests"}"#.to_string(),
                ),
            },
            Err(message) => (
                "400 Bad Request",
                crate::web_image_generate::generate_error_json(&message),
            ),
        };
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            status,
            &body,
            cors_origin,
        )
        .map(|()| false);
    }
    // Offline `.fig` -> `.op` convert for the VS Code plugin: it can't parse
    // fig-kiwi itself, so it POSTs the raw bytes here and boots the returned
    // document JSON through its normal open-document push. Conversion is
    // pure (no network, no state) so — unlike the image routes above — it
    // runs on this connection's own thread without ever touching the state
    // lock; only large/slow parsing needs to stay off it.
    if req.method == "POST" && req.path == "/api/figma/convert" {
        let (status, body) = match crate::figma_convert::convert_fig_json(&req.body) {
            Ok(body) => ("200 OK", body),
            Err(message) => (
                "400 Bad Request",
                serde_json::json!({ "ok": false, "error": message }).to_string(),
            ),
        };
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            status,
            &body,
            cors_origin,
        )
        .map(|()| false);
    }
    // All `/api/mcp/*` REST paths go to the REST handler — including ones this
    // daemon doesn't implement yet, which it answers with 404 rather than
    // mis-routing them into the JSON-RPC dispatch below.
    if req.path.starts_with("/api/") {
        let reply = {
            let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
            let before = guard.version;
            let settings_before = crate::settings_io::fingerprint(&guard.editor);
            let credential_settings_before = (req.method == "POST"
                && req.path == "/api/settings/credentials")
                .then(|| guard.editor.editor_ui.agent_settings.clone());
            let reply = handle_web_canvas_request(&req.method, &req.path, &req.body, &mut guard);
            let reply = persist_api_settings(
                &req.method,
                &req.path,
                &mut guard,
                settings_before,
                credential_settings_before,
                reply,
                crate::settings_io::save_checked,
            );
            // Broadcast INSIDE the state lock so the version bump and its
            // broadcast are atomic — otherwise two concurrent mutations could
            // broadcast their versions out of order (SSE clients seeing N then
            // N-1). `broadcast` only sends to unbounded channels (non-blocking),
            // so the lock is held briefly. Lock order is always state→hub.
            if guard.version != before {
                hub.broadcast(guard.version);
            }
            reply
        };
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            reply.status,
            &reply.body,
            cors_origin,
        )
        .map(|()| false);
    }
    // JSON-RPC tool dispatch is served ONLY as a POST to `/` or `/mcp`. An
    // unknown path is 404; a known path with the wrong method (e.g. `GET /mcp`)
    // is 405 — never silently dispatched as a tool call.
    let is_jsonrpc_path = req.path == "/" || req.path == "/mcp";
    if !is_jsonrpc_path {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "404 Not Found",
            r#"{"ok":false,"error":"Not found. Use /, /pkg/*, /api/mcp/document, /api/mcp/sync-reset, /api/mcp/server, /api/mcp/events, /api/file/save, /api/export/raster, /api/export/pdf, or /mcp."}"#,
            cors_origin,
        )
        .map(|()| false);
    }
    if req.method != "POST" {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "405 Method Not Allowed",
            r#"{"ok":false,"error":"Method not allowed. POST a JSON-RPC message to /mcp."}"#,
            cors_origin,
        )
        .map(|()| false);
    }
    // Token-authed graceful shutdown (`op stop`): same contract as the
    // `--mcp-http` server — only the exact per-instance token passed by the
    // spawning CLI (via OPENPENCIL_MCP_TOKEN) authenticates; a stale file, a
    // recycled pid, or a random client cannot shut the daemon down.
    if let Some(id) = crate::mcp_serve::shutdown_request_id(
        &req.body,
        &crate::mcp_serve::headless_token_from_env().unwrap_or_default(),
    ) {
        crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "200 OK",
            &crate::mcp_serve::shutdown_ok_response(&id),
            cors_origin,
        )?;
        return Ok(true);
    }
    // `debug_screenshot` for `--serve-web`: the browser shell mirrors this
    // daemon's document, so the daemon can satisfy the live screenshot tool from
    // the same raster export path desktop live MCP uses. Keep this ahead of the
    // generic dispatch, whose headless debug tool can only report no live
    // canvas.
    #[cfg(feature = "mcp-debug-tools")]
    if let Some(response) = {
        let guard = state.lock().unwrap_or_else(|p| p.into_inner());
        crate::mcp_live::screenshot::maybe_serve(
            &req.body,
            op_mcp::debug_tools_enabled(),
            |shot_req| {
                let spec = crate::mcp_live::screenshot::capture_spec(&shot_req);
                crate::export::screenshot::capture(&guard.editor, &spec)
            },
        )
    } {
        return crate::mcp_serve::write_mcp_http_response_with_origin(
            stream,
            "200 OK",
            &response,
            cors_origin,
        )
        .map(|()| false);
    }
    // JSON-RPC `/mcp` dispatch against the in-memory document. A mutating apply
    // bumps the sync version, broadcast to SSE subscribers so the browser shell
    // sees JSON-RPC-driven changes too.
    let response = {
        let mut guard = state.lock().unwrap_or_else(|p| p.into_inner());
        let before = guard.version;
        let mut applied_any = false;
        let response = crate::mcp_serve::process_message_with_applier(
            &mut guard.editor,
            &req.body,
            |editor, cmd| {
                let ok = editor.apply(cmd.clone());
                applied_any |= ok;
                ok
            },
        )?
        .unwrap_or_default();
        if applied_any {
            guard.version += 1;
        }
        // Atomic bump+broadcast under the state lock (see the REST path) so SSE
        // version events stay monotonic across concurrent mutations.
        if guard.version != before {
            hub.broadcast(guard.version);
        }
        response
    };
    let status = if response.is_empty() {
        "202 Accepted"
    } else {
        "200 OK"
    };
    crate::mcp_serve::write_mcp_http_response_with_origin(stream, status, &response, cors_origin)
        .map(|()| false)
}

const WEB_ALLOWED_ORIGINS_ENV: &str = "OPENPENCIL_WEB_ALLOWED_ORIGINS";

fn is_sensitive_browser_post(request: &crate::mcp_serve::HttpRequest) -> bool {
    request.method == "POST"
        && (request.path == "/api/settings/credentials"
            || request.path.starts_with("/api/ai/")
            || request.path.starts_with("/api/figma/"))
}

/// `application/json` (optionally with parameters, e.g. `; charset=utf-8`).
fn content_type_is_json(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case("application/json")
    })
}

fn credential_request_origin_allowed(request: &crate::mcp_serve::HttpRequest) -> bool {
    let allowed_origins = std::env::var(WEB_ALLOWED_ORIGINS_ENV).ok();
    credential_request_origin_allowed_with_config(request, allowed_origins.as_deref())
}

fn credential_request_origin_allowed_with_config(
    request: &crate::mcp_serve::HttpRequest,
    allowed_origins: Option<&str>,
) -> bool {
    let Some(origin) = request.origin.as_deref() else {
        // Non-browser clients do not normally send Origin. Server persistence
        // is an opt-in private-deployment feature, so those clients remain
        // usable while browser requests are constrained by the unforgeable
        // Origin header.
        return true;
    };
    let Some(host) = request.host.as_deref() else {
        return false;
    };
    let Some(origin) = parse_http_origin(origin) else {
        return false;
    };
    let Ok(host) = reqwest::Url::parse(&format!("http://{host}/")) else {
        return false;
    };
    let same_request_authority =
        origin
            .host_str()
            .zip(host.host_str())
            .is_some_and(|(origin_host, request_host)| {
                let request_port = host.port().or_else(|| match origin.scheme() {
                    "http" => Some(80),
                    "https" => Some(443),
                    _ => None,
                });
                origin_host.eq_ignore_ascii_case(request_host)
                    && origin.port_or_known_default() == request_port
            });
    if !same_request_authority {
        return false;
    }
    if origin.host_str().is_some_and(is_loopback_web_host) {
        return true;
    }
    allowed_origins
        .into_iter()
        .flat_map(|origins| origins.split(','))
        .filter_map(|configured| parse_http_origin(configured.trim()))
        .any(|configured| same_url_origin(&origin, &configured))
}

fn parse_http_origin(value: &str) -> Option<reqwest::Url> {
    let origin = reqwest::Url::parse(value).ok()?;
    (matches!(origin.scheme(), "http" | "https")
        && origin.username().is_empty()
        && origin.password().is_none()
        && origin.host_str().is_some()
        && origin.path() == "/"
        && origin.query().is_none()
        && origin.fragment().is_none())
    .then_some(origin)
}

fn same_url_origin(left: &reqwest::Url, right: &reqwest::Url) -> bool {
    left.scheme() == right.scheme()
        && left
            .host_str()
            .zip(right.host_str())
            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right))
        && left.port_or_known_default() == right.port_or_known_default()
}

fn is_loopback_web_host(host: &str) -> bool {
    let ip_literal = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.eq_ignore_ascii_case("localhost")
        || host
            .to_ascii_lowercase()
            .strip_suffix(".localhost")
            .is_some_and(|prefix| !prefix.is_empty())
        || ip_literal
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Stream Server-Sent Events to a subscribed client: write the SSE headers,
/// emit the current version immediately (initial sync), then forward each
/// version bump from `rx` as a `data: {"version":N}` event. A periodic
/// heartbeat comment keeps the connection alive AND detects a disconnected
/// client (the write fails once the socket is gone). Returns when the client
/// disconnects (write error) or the hub is dropped.
fn serve_sse<S: Write>(
    stream: &mut S,
    rx: Receiver<u64>,
    current_version: u64,
    cors_origin: Option<&str>,
) -> Result<(), String> {
    let cors_line = cors_origin
        .map(|origin| format!("Access-Control-Allow-Origin: {origin}\r\n"))
        .unwrap_or_default();
    let headers = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-cache\r\n\
         Connection: keep-alive\r\n\
         {cors_line}\r\n"
    );
    stream
        .write_all(headers.as_bytes())
        .map_err(|e| format!("sse headers: {e}"))?;
    write_sse_event(stream, current_version)?;
    loop {
        match rx.recv_timeout(SSE_HEARTBEAT) {
            Ok(mut version) => {
                // Coalesce any further queued bumps — only the latest version
                // matters (the client re-fetches the whole document on it), so
                // a burst of mutations collapses to a single event and the
                // channel can't accumulate unboundedly behind a slow client.
                while let Ok(next) = rx.try_recv() {
                    version = next;
                }
                write_sse_event(stream, version)?;
            }
            Err(RecvTimeoutError::Timeout) => {
                // SSE comment heartbeat — no-op for the client, but a failed
                // write here is how we notice it disconnected.
                stream
                    .write_all(b": ping\n\n")
                    .map_err(|e| format!("sse heartbeat: {e}"))?;
                stream.flush().map_err(|e| format!("sse flush: {e}"))?;
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Format + write one SSE `data:` event carrying the document version.
fn write_sse_event<S: Write>(stream: &mut S, version: u64) -> Result<(), String> {
    let event = format!("data: {{\"version\":{version}}}\n\n");
    stream
        .write_all(event.as_bytes())
        .map_err(|e| format!("sse write: {e}"))?;
    stream.flush().map_err(|e| format!("sse flush: {e}"))
}

#[cfg(test)]
#[path = "web_canvas_server_tests.rs"]
mod tests;
