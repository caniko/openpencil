//! In-process MCP HTTP server for the live desktop editor.
//!
//! The CLI-only `mcp_serve` path owns a file-backed `EditorState`.
//! This module keeps the GUI as the source of truth: the server thread
//! requests a fresh snapshot from the UI thread for each HTTP request,
//! then sends write commands back for the UI thread to apply.

use std::collections::HashSet;
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jian_ops_schema::node::PenNode;
use op_editor_core::pen_node_ext::PenNodeExt;
use op_editor_core::{EditorCommand, EditorState};

#[cfg(feature = "mcp-debug-tools")]
pub mod screenshot;

/// Per-request budget for a UI-thread snapshot/apply ack. Sized to cover a
/// large single editor operation without the connection giving up; the CLI
/// allows even longer per tool call (`POST_TIMEOUT`).
const UI_ACK_TIMEOUT: Duration = Duration::from_secs(15);
const ACCEPT_IDLE_SLEEP: Duration = Duration::from_millis(25);
/// Cap on concurrently-served connections, so a flood of localhost connects
/// can't exhaust threads/memory. Excess connections are shed with a 503.
const MAX_LIVE_CONNS: usize = 64;
/// Maximum UI-thread MCP requests to drain in one redraw. A client that
/// enqueues a large burst should not be able to monopolize the frame.
const UI_PUMP_REQUEST_BUDGET: usize = 8;
/// Per-request HTTP connection stack. Deep JSON deserialization is handled by
/// `serde_stacker`; reserving hundreds of MB per short-lived localhost request
/// makes large live-canvas syncs look like runaway desktop memory growth.
const LIVE_CONN_STACK_SIZE: usize = 16 * 1024 * 1024;
const _: () = assert!(LIVE_CONN_STACK_SIZE <= 16 * 1024 * 1024);
type UiWake = Arc<dyn Fn() + Send + Sync + 'static>;

/// Fallback client-facing label when `initialize` carried no
/// `clientInfo.name` (some minimal/older MCP clients omit it) — honest
/// about what it is ("some external MCP tool"), not a fabricated persona.
const MCP_CLIENT_FALLBACK_NAME: &str = "MCP Client";
/// Fixed neutral badge colour for every MCP-driven generation — a
/// deliberate contrast with the in-app agent persona pool
/// (`assign_agent_identities_seeded`'s vivid per-agent colours): an
/// external MCP client isn't one of the app's own named agents.
const MCP_CLIENT_COLOR: &str = "#8A8F98";

pub struct McpLiveServer {
    port: u16,
    /// Per-instance identity token, reported in the live `ping` reply and
    /// written into `~/.openpencil/.op-mcp-port`. Lets the `op` CLI confirm
    /// the server it pings is the exact instance that wrote the discovery
    /// file (defeats stale-file / port-reuse / pid-recycle confusion).
    token: String,
    /// Set by a connection thread when a token-authed `openpencil/shutdown`
    /// arrives; the UI thread polls it (`shutdown_requested`) and exits the
    /// event loop, so `op stop` quits the editor cleanly without a signal.
    quit_flag: Arc<AtomicBool>,
    req_rx: Receiver<UiRequest>,
    stop_tx: Sender<()>,
    /// The connected MCP client's declared identity — `(name, color)`,
    /// captured from `initialize`'s `params.clientInfo.name` the first
    /// time a connection thread sees it (`Arc<Mutex<_>>`: written from
    /// whichever connection thread handles that request, read from the
    /// UI thread in `pump` when tagging a `batch_design` root). A fixed
    /// neutral colour, not the in-app persona pool — an external MCP
    /// client is not one of the app's own named agents.
    client_identity: Arc<Mutex<Option<(String, String)>>>,
    /// The last epoch this server minted for a `batch_design` write, or
    /// `0` before the first one. `pump`'s indicator hook reuses it
    /// (rather than starting a fresh epoch) while its reveal queue is
    /// still draining, so a burst of back-to-back `batch_design` calls
    /// reads as one continuous generation instead of each call clipping
    /// the previous one's tail animation. UI-thread-only (`pump` takes
    /// `&mut self`), so a plain field — no interior mutability needed.
    last_mcp_epoch: u64,
}

struct ApplyAck {
    applied: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct McpPumpOutcome {
    pub repaint: bool,
    pub layout_dirty: bool,
    /// Set when this pump swapped in a whole new document
    /// (`UiRequest::ReplaceDocument` → `EditorState::replace_document`).
    /// `replace_document` resets `document_revision()` back to 0, which can
    /// alias a previously-scanned revision on an unrelated document — the
    /// caller MUST invalidate any revision-keyed cache (e.g. the desktop
    /// host's image-search scan gate) whenever this is `true`, the same way
    /// it already does after a Figma import installs a fresh `EditorState`.
    pub document_replaced: bool,
}

enum UiRequest {
    Snapshot {
        ack: SyncSender<EditorState>,
    },
    Apply {
        /// The MCP tool that produced `cmd` — `McpLiveServer::pump` reads
        /// this to gate canvas-generation indicators (frame glow + reveal
        /// sweep) on `"batch_design"` specifically, mirroring how the
        /// in-app design-agent loop only animates that one tool
        /// (`design_agent_tools.rs::should_register_batch_reveals`).
        tool_name: String,
        cmd: EditorCommand,
        ack: SyncSender<ApplyAck>,
    },
    /// Whole-document sync (TS REST `/api/mcp/document` parity). The document
    /// is parsed/loaded OFF the UI thread (so a large document never pins the
    /// UI thread, and the stateful lock is held only for the swap); the UI pump
    /// just swaps the already-loaded document into the live `EditorState`. The
    /// ack is `()` — the in-memory swap is infallible once the load succeeded.
    ReplaceDocument {
        doc: Box<jian_ops_schema::PenDocument>,
        ack: SyncSender<()>,
    },
    /// `debug_screenshot` against the LIVE canvas — the connection
    /// thread blocks on `ack` while the UI pump renders the active
    /// page / node through the raster export pipeline
    /// (`export::screenshot::capture`). Same pending-intent +
    /// bounded-wait discipline as the chat canvas tools.
    #[cfg(feature = "mcp-debug-tools")]
    Screenshot {
        spec: crate::export::screenshot::CaptureSpec,
        ack: SyncSender<Result<crate::export::screenshot::ScreenshotPng, String>>,
    },
}

impl McpLiveServer {
    #[doc(hidden)]
    pub fn start(port: u16) -> Result<Self, String> {
        Self::start_with_wake(port, || {})
    }

    pub fn start_with_wake<F>(port: u16, wake_ui: F) -> Result<Self, String>
    where
        F: Fn() + Send + Sync + 'static,
    {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
        let bound_port = listener
            .local_addr()
            .map_err(|e| format!("read bound MCP port: {e}"))?
            .port();
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set nonblocking: {e}"))?;
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel();
        let token = make_live_token();
        let server_token = token.clone();
        let quit_flag = Arc::new(AtomicBool::new(false));
        let server_quit = Arc::clone(&quit_flag);
        let wake_ui: UiWake = Arc::new(wake_ui);
        let client_identity = Arc::new(Mutex::new(None));
        let server_identity = Arc::clone(&client_identity);
        thread::Builder::new()
            .name("op-mcp-live-http".into())
            .spawn(move || {
                server_loop(
                    listener,
                    req_tx,
                    stop_rx,
                    server_token,
                    server_quit,
                    wake_ui,
                    server_identity,
                )
            })
            .map_err(|e| format!("spawn MCP live server: {e}"))?;
        eprintln!("openpencil-desktop mcp: listening on 127.0.0.1:{bound_port}/mcp");
        Ok(Self {
            port: bound_port,
            token,
            quit_flag,
            req_rx,
            stop_tx,
            client_identity,
            last_mcp_epoch: 0,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Identity token to publish in the discovery file.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Whether a token-authed `openpencil/shutdown` was received — the UI
    /// thread should exit the event loop.
    pub fn shutdown_requested(&self) -> bool {
        self.quit_flag.load(Ordering::Acquire)
    }

    pub fn pump(&mut self, state: &mut EditorState) -> McpPumpOutcome {
        let mut outcome = McpPumpOutcome::default();
        for _ in 0..UI_PUMP_REQUEST_BUDGET {
            match self.req_rx.try_recv() {
                Ok(UiRequest::Snapshot { ack }) => {
                    let _ = ack.send(state.clone());
                }
                Ok(UiRequest::Apply {
                    tool_name,
                    cmd,
                    ack,
                }) => {
                    let layout_dirty = command_invalidates_layout(&cmd);
                    // Snapshot BEFORE apply, only for the one tool the canvas
                    // radar-scan cares about — `design_agent_tools.rs`'s
                    // `should_register_batch_reveals` gates the in-app
                    // design-agent loop's reveal registration the exact same
                    // way, for the exact same reason (every other write tool
                    // is a targeted property tweak, not a "generation").
                    let ids_before = (tool_name == "batch_design")
                        .then(|| crate::design_agent_tools::collect_active_node_ids(state));
                    let applied = state.apply(cmd);
                    if applied {
                        if let Some(ids_before) = ids_before {
                            self.register_mcp_generation(&ids_before, state);
                        }
                    }
                    let _ = ack.send(ApplyAck { applied });
                    if applied {
                        outcome.repaint = true;
                        outcome.layout_dirty |= layout_dirty;
                    }
                }
                Ok(UiRequest::ReplaceDocument { doc, ack }) => {
                    // The document was already loaded off the UI thread; just
                    // swap it in (preserving editor chrome). Layout is computed
                    // by the renderer at paint, so the swap + dirty flag
                    // repaints the new document with no extra layout pass here.
                    state.replace_document(*doc);
                    let _ = ack.send(());
                    outcome.repaint = true;
                    outcome.layout_dirty = true;
                    outcome.document_replaced = true;
                }
                #[cfg(feature = "mcp-debug-tools")]
                Ok(UiRequest::Screenshot { spec, ack }) => {
                    // Read-only render of the live state — no repaint needed.
                    let _ = ack.send(crate::export::screenshot::capture(state, &spec));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        outcome
    }

    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(());
    }

    /// Tag a JUST-APPLIED `batch_design` write's genuinely new content
    /// with canvas-generation indicators, so the radar-scan (root frame
    /// glow — `op_editor_ui`'s `canvas_generation_scan` gates purely on
    /// `AgentIndicators.frames` being non-empty) and the per-node reveal
    /// entrance animation activate for MCP-driven generation the same
    /// way they already do for the in-app design-agent loop. A no-op
    /// when nothing new landed (a `batch_design` Direct-op call that
    /// only tweaked an existing node's properties, say).
    fn register_mcp_generation(&mut self, ids_before: &HashSet<String>, state: &EditorState) {
        let now_ms = crate::design_agent_tools::reveal_now_millis();
        let mut saw_new_content = false;
        for node in state.active_children() {
            if !ids_before.contains(node.id_str()) {
                saw_new_content = true;
                break;
            }
        }
        if !saw_new_content {
            // Reveals can still land nested under an EXISTING root (an
            // "add a section to this screen" follow-up) even when no
            // top-level id is new — `register_new_node_reveals` below
            // walks the whole tree, so check that shape too before
            // giving up entirely.
            saw_new_content = state
                .active_children()
                .iter()
                .any(|node| subtree_has_new_id(node, ids_before));
        }
        if !saw_new_content {
            return;
        }
        let epoch = self.resolve_mcp_epoch(now_ms);
        let (name, color) = self
            .client_identity
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_else(|| {
                (
                    MCP_CLIENT_FALLBACK_NAME.to_string(),
                    MCP_CLIENT_COLOR.to_string(),
                )
            });
        // Only a genuinely NEW top-level Frame is a fresh generation
        // root — mirrors `design_loop_indicator.rs::register_new_frames`
        // (Frame-only) and `run_screen_groups.rs::insert_screen_group_roots`
        // (new-vs-existing top-level diff), the two established places
        // this codebase already tags a root frame.
        for node in state.active_children() {
            if matches!(node, PenNode::Frame(_)) && !ids_before.contains(node.id_str()) {
                op_editor_core::agent_indicators::add_frame(epoch, node.id_str(), &color, &name);
            }
        }
        crate::design_agent_tools::register_new_node_reveals(
            ids_before,
            state,
            Some(epoch),
            now_ms,
        );
        self.last_mcp_epoch = epoch;
        // Each `batch_design` call is its own self-contained turn — no
        // idle-timeout bookkeeping needed. `finish_if_epoch` is the
        // established graceful-drain: `run_active` drops immediately but
        // the already-queued reveals keep animating at their own pace
        // (`agent_indicators.rs`'s paint-side maintenance retires the
        // epoch once the queue empties), so this is safe to call even
        // though the call that just landed is still visually settling.
        op_editor_core::agent_indicators::finish_if_epoch(epoch);
    }

    /// Reuse the last MCP epoch while its reveal queue is still
    /// draining (`latest_reveal_end_ms` is the same "is anything still
    /// animating" check the orchestrator's own tree-restructure gate
    /// polls), so a burst of back-to-back `batch_design` calls (a
    /// multi-screen design, one call per screen) reads as one
    /// continuous generation instead of each call clipping the
    /// previous one's tail animation via `begin()`'s "bump epoch +
    /// clear every map" semantics. Starts fresh once the queue empties.
    fn resolve_mcp_epoch(&self, now_ms: u64) -> u64 {
        if self.last_mcp_epoch != 0 {
            if let Some(end_ms) =
                op_editor_core::agent_indicators::latest_reveal_end_ms(self.last_mcp_epoch)
            {
                if now_ms < end_ms {
                    return self.last_mcp_epoch;
                }
            }
        }
        op_editor_core::agent_indicators::begin()
    }
}

/// Whether `node` or any descendant carries an id not in `ids_before` —
/// the "did ANY new content land, even nested under an existing root"
/// check `register_mcp_generation` needs before deciding whether a
/// `batch_design` call is worth animating at all.
fn subtree_has_new_id(node: &PenNode, ids_before: &HashSet<String>) -> bool {
    if !ids_before.contains(node.id_str()) {
        return true;
    }
    if let Some(children) = node.children() {
        return children
            .iter()
            .any(|child| subtree_has_new_id(child, ids_before));
    }
    false
}

impl Drop for McpLiveServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn command_invalidates_layout(cmd: &EditorCommand) -> bool {
    match cmd {
        EditorCommand::ClearSelection
        | EditorCommand::SetSelection { .. }
        | EditorCommand::SetSelectionSet { .. }
        | EditorCommand::ToggleNodeSelection { .. }
        | EditorCommand::SetViewport { .. }
        | EditorCommand::SetActiveTool { .. }
        | EditorCommand::CopySelected => false,
        EditorCommand::SetNodeFlag { flag, .. } => {
            !matches!(flag, op_editor_core::NodeFlag::Collapsed)
        }
        _ => true,
    }
}

/// RAII decrement for the live-connection counter — see its use in the
/// per-connection thread. `Drop` runs during normal exit AND panic unwind, so a
/// panicking connection can't leak its reserved slot.
struct ConnGuard(Arc<AtomicUsize>);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn server_loop(
    listener: TcpListener,
    req_tx: Sender<UiRequest>,
    stop_rx: Receiver<()>,
    token: String,
    quit_flag: Arc<AtomicBool>,
    wake_ui: UiWake,
    client_identity: Arc<Mutex<Option<(String, String)>>>,
) {
    // Serializes only *stateful* requests so a concurrent multi-apply batch
    // can't interleave. Stateless probes (`ping`/`initialize`) bypass it, so
    // `op`'s liveness probe is never blocked behind an in-flight write.
    let stateful_lock = Arc::new(Mutex::new(()));
    // Bounds the number of in-flight connection threads.
    let conn_count = Arc::new(AtomicUsize::new(0));
    loop {
        match stop_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }
        match listener.accept() {
            Ok((mut stream, _addr)) => {
                if let Err(e) = stream.set_nonblocking(false) {
                    eprintln!("openpencil-desktop mcp: accepted stream blocking mode: {e}");
                    continue;
                }
                if conn_count.load(Ordering::Acquire) >= MAX_LIVE_CONNS {
                    // Shed load rather than spawn unbounded threads.
                    let _ = stream.set_write_timeout(Some(ACCEPT_IDLE_SLEEP));
                    let _ = crate::mcp_serve::write_mcp_http_response(
                        &mut stream,
                        "503 Service Unavailable",
                        r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"server busy"},"id":null}"#,
                    );
                    continue;
                }
                // Handle each connection on its own thread so a long write
                // (which waits on the UI thread) doesn't stall concurrent
                // pings. Threads are short-lived and detached.
                conn_count.fetch_add(1, Ordering::AcqRel);
                let req_tx = req_tx.clone();
                let token = token.clone();
                let lock = Arc::clone(&stateful_lock);
                let quit = Arc::clone(&quit_flag);
                let conns = Arc::clone(&conn_count);
                let wake = Arc::clone(&wake_ui);
                let identity = Arc::clone(&client_identity);
                let spawned = thread::Builder::new()
                    .name("op-mcp-live-conn".into())
                    .stack_size(LIVE_CONN_STACK_SIZE)
                    .spawn(move || {
                        // RAII: decrement the live-connection count even if
                        // `serve_connection` panics. Without this, a panic would
                        // leak the slot, and after MAX_LIVE_CONNS panics the
                        // server would wedge into permanent load-shedding.
                        let _conn_guard = ConnGuard(conns);
                        let mut stream = stream;
                        let _ = stream.set_read_timeout(Some(UI_ACK_TIMEOUT));
                        let _ = stream.set_write_timeout(Some(UI_ACK_TIMEOUT));
                        if let Err(e) = serve_connection(
                            &mut stream,
                            &req_tx,
                            &token,
                            &lock,
                            &quit,
                            &wake,
                            &identity,
                        ) {
                            eprintln!("openpencil-desktop mcp: {e}");
                            let _ = crate::mcp_serve::write_mcp_http_response(
                                &mut stream,
                                "500 Internal Server Error",
                                &error_json(&e),
                            );
                        }
                    });
                if spawned.is_err() {
                    // The closure (and its `conns` clone) was dropped without
                    // running — undo the count we reserved above.
                    conn_count.fetch_sub(1, Ordering::AcqRel);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_IDLE_SLEEP);
            }
            Err(e) => {
                eprintln!("openpencil-desktop mcp: accept: {e}");
                thread::sleep(ACCEPT_IDLE_SLEEP);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn serve_connection<S: std::io::Read + std::io::Write>(
    stream: &mut S,
    req_tx: &Sender<UiRequest>,
    token: &str,
    stateful_lock: &Mutex<()>,
    quit_flag: &AtomicBool,
    wake_ui: &UiWake,
    client_identity: &Mutex<Option<(String, String)>>,
) -> Result<(), String> {
    let req = crate::mcp_serve::read_http_request(stream)?;
    if req.method == "OPTIONS" {
        return crate::mcp_serve::write_mcp_http_response(stream, "204 No Content", "");
    }
    // TS live-canvas whole-document sync (REST `POST /api/mcp/document`),
    // distinct from the JSON-RPC `/mcp` path below. Lets a TS whole-doc-sync
    // client (`setSyncDocument` → POST `{document}`) drive THIS editor's
    // on-screen canvas, mirroring `apps/web/server/api/mcp/document.post.ts`.
    if crate::mcp_serve::is_document_sync_route(&req.method, &req.path) {
        return serve_document_sync(stream, req_tx, wake_ui, stateful_lock, &req.body);
    }
    if req.path != "/mcp" && req.path != "/" {
        return crate::mcp_serve::write_mcp_http_response(
            stream,
            "404 Not Found",
            r#"{"error":"Not found"}"#,
        );
    }
    if req.method != "POST" {
        return crate::mcp_serve::write_mcp_http_response(
            stream,
            "400 Bad Request",
            r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Invalid or missing session ID"},"id":null}"#,
        );
    }
    // Token-authed graceful shutdown: ack, then flag the UI thread to exit
    // the event loop. No pid-kill ⇒ no signal-the-wrong-process race.
    if let Some(id) = crate::mcp_serve::shutdown_request_id(&req.body, token) {
        write_json_rpc_response(stream, &crate::mcp_serve::shutdown_ok_response(&id))?;
        quit_flag.store(true, Ordering::Release);
        wake_ui();
        return Ok(());
    }
    // Stateless handshake/liveness methods must NOT block on the UI thread or
    // take the stateful lock, so `op`'s `ping` probe stays fast and never
    // false-negatives a busy editor. The live `ping` reply carries our
    // identity token so the CLI can confirm THIS server published the file.
    match crate::mcp_serve::classify_stateless(&req.body) {
        crate::mcp_serve::Stateless::Respond(resp) => {
            // Capture the client's declared identity off the SAME
            // `initialize` request `classify_stateless` just answered —
            // the ONLY message this wire protocol carries a name in.
            // Always overwrite (not "first wins"): a later `initialize`
            // means a different tool connected, and the badge should
            // say who is ACTUALLY driving now.
            if let Some(name) = crate::mcp_serve::parse_client_info_name(&req.body) {
                if let Ok(mut identity) = client_identity.lock() {
                    *identity = Some((name, MCP_CLIENT_COLOR.to_string()));
                }
            }
            return write_json_rpc_response(stream, &resp);
        }
        crate::mcp_serve::Stateless::Swallow => {
            return write_json_rpc_response(stream, "");
        }
        crate::mcp_serve::Stateless::Ping(id) => {
            return write_json_rpc_response(stream, &live_ping_response(&id, token));
        }
        crate::mcp_serve::Stateless::NeedsState => {}
    }
    // Everything below mutates shared state — the live `EditorState` OR a
    // `--file` document on disk (a read-modify-write). Serialize ALL of it
    // under one lock so concurrent connection threads can't interleave live
    // applies OR race two file-backed writes to the same document. Tolerate
    // a poisoned lock (a panicked sibling connection) rather than cascade.
    let _guard = stateful_lock
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    // File-backed path (`--file` arg): handle the whole read-modify-write
    // while holding the lock.
    if let Some(response) =
        crate::mcp_serve::file_path::process_message_for_file_path_arg(None, &req.body)?
    {
        return write_json_rpc_response(stream, &response);
    }
    // `debug_screenshot` against the LIVE canvas: route through the
    // raster export pipeline on the UI thread instead of the generic
    // dispatch (whose headless tool can only report no-live-canvas).
    // Gate-closed (`OPENPENCIL_DEBUG_TOOLS` unset) falls through to the
    // generic dispatch, which reports UnknownTool exactly like the
    // headless registry that never registered the tool.
    #[cfg(feature = "mcp-debug-tools")]
    if let Some(response) =
        screenshot::maybe_serve(&req.body, op_mcp::debug_tools_enabled(), |shot_req| {
            request_screenshot(req_tx, wake_ui, shot_req)
        })
    {
        return write_json_rpc_response(stream, &response);
    }
    let mut state = request_snapshot(req_tx, wake_ui)?;
    let response = crate::mcp_serve::process_message_with_applier(
        &mut state,
        &req.body,
        |tool_name, local_state, cmd| match request_apply(
            req_tx,
            wake_ui,
            tool_name.to_string(),
            cmd.clone(),
        ) {
            Ok(ack) => {
                if ack.applied {
                    let _ = local_state.apply(cmd.clone());
                }
                ack.applied
            }
            Err(e) => {
                eprintln!("openpencil-desktop mcp: apply failed: {e}");
                false
            }
        },
    )?
    .unwrap_or_default();
    write_json_rpc_response(stream, &response)
}

/// Monotonic counter for the whole-document sync `version` reported back to a
/// TS whole-doc-sync client (parity with `setSyncDocument`'s monotonic
/// version). Process-global so concurrent connections still get distinct,
/// increasing versions; the exact starting value is not part of the contract.
static LIVE_SYNC_VERSION: AtomicU64 = AtomicU64::new(0);

/// Handle a TS-style whole-document sync (`POST /api/mcp/document`): validate
/// the body, swap the document into the live `EditorState` on the UI thread,
/// and reply `{ok:true,version}` (or HTTP 400 with `{ok:false,error}`).
fn serve_document_sync<S: std::io::Read + std::io::Write>(
    stream: &mut S,
    req_tx: &Sender<UiRequest>,
    wake_ui: &UiWake,
    stateful_lock: &Mutex<()>,
    body: &str,
) -> Result<(), String> {
    let document_json = match crate::mcp_serve::parse_document_sync_body(body) {
        Ok(json) => json,
        Err(message) => {
            return crate::mcp_serve::write_mcp_http_response(
                stream,
                "400 Bad Request",
                &crate::mcp_serve::rest_error_body(&message),
            );
        }
    };
    // Load OFF the UI thread (parse only — no skia; layout runs at paint) so a
    // large document never pins the UI thread or holds the lock during the
    // parse. A load failure is a client fault → 400, like the TS validation
    // 400s in `document.post.ts`.
    let loaded = match op_pen_loader::load_canonical(&document_json) {
        Ok(loaded) => loaded,
        Err(e) => {
            return crate::mcp_serve::write_mcp_http_response(
                stream,
                "400 Bad Request",
                &crate::mcp_serve::rest_error_body(&e.to_string()),
            );
        }
    };
    for w in &loaded.warnings {
        eprintln!("openpencil-desktop mcp: document-sync schema warning: {w:?}");
    }
    // Serialize the swap against concurrent live writes (same lock the JSON-RPC
    // apply path takes). The lock is now held only for the fast in-memory
    // replace, not the parse — so a huge document can't time out mid-load while
    // still eventually mutating the doc.
    let _guard = stateful_lock
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    match request_replace(req_tx, wake_ui, loaded.value) {
        Ok(()) => {
            let version = LIVE_SYNC_VERSION.fetch_add(1, Ordering::Relaxed) + 1;
            crate::mcp_serve::write_mcp_http_response(
                stream,
                "200 OK",
                &crate::mcp_serve::document_sync_ok(version),
            )
        }
        // The UI thread is gone or didn't ack in time — a server fault, mapped
        // to 500 (TS throws → 500 for server-side failures).
        Err(transport_err) => crate::mcp_serve::write_mcp_http_response(
            stream,
            "500 Internal Server Error",
            &crate::mcp_serve::rest_error_body(&transport_err),
        ),
    }
}

fn write_json_rpc_response<S: std::io::Write>(
    stream: &mut S,
    response: &str,
) -> Result<(), String> {
    if response.is_empty() {
        crate::mcp_serve::write_mcp_http_response(stream, "202 Accepted", "")
    } else {
        crate::mcp_serve::write_mcp_http_response(stream, "200 OK", response)
    }
}

fn request_snapshot(req_tx: &Sender<UiRequest>, wake_ui: &UiWake) -> Result<EditorState, String> {
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    req_tx
        .send(UiRequest::Snapshot { ack: ack_tx })
        .map_err(|_| "UI thread is not accepting MCP snapshot requests".to_string())?;
    wake_ui();
    recv_with_timeout(ack_rx.recv_timeout(UI_ACK_TIMEOUT), "snapshot")
}

fn request_apply(
    req_tx: &Sender<UiRequest>,
    wake_ui: &UiWake,
    tool_name: String,
    cmd: EditorCommand,
) -> Result<ApplyAck, String> {
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    req_tx
        .send(UiRequest::Apply {
            tool_name,
            cmd,
            ack: ack_tx,
        })
        .map_err(|_| "UI thread is not accepting MCP apply requests".to_string())?;
    wake_ui();
    recv_with_timeout(ack_rx.recv_timeout(UI_ACK_TIMEOUT), "apply")
}

/// Ask the UI thread to render a `debug_screenshot` capture. Bounded
/// wait per the request's own budget (TS `min(timeoutMs ?? 15000,
/// 60000)`) — the same recv-timeout discipline the chat canvas tools
/// use, so a stalled UI pump can never pin the connection thread.
#[cfg(feature = "mcp-debug-tools")]
fn request_screenshot(
    req_tx: &Sender<UiRequest>,
    wake_ui: &UiWake,
    req: op_mcp::ScreenshotRequest,
) -> Result<crate::export::screenshot::ScreenshotPng, String> {
    let timeout = Duration::from_millis(req.timeout_ms.max(1));
    let spec = screenshot::capture_spec(&req);
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    req_tx
        .send(UiRequest::Screenshot { spec, ack: ack_tx })
        .map_err(|_| "UI thread is not accepting MCP screenshot requests".to_string())?;
    wake_ui();
    recv_with_timeout(ack_rx.recv_timeout(timeout), "screenshot")?
}

/// Ask the UI thread to swap in an already-loaded document. `Err` means the
/// request channel closed or the UI ack timed out — a server fault → HTTP 500.
/// (Load/validation failures are handled on the caller's thread BEFORE this,
/// and surface as HTTP 400.)
fn request_replace(
    req_tx: &Sender<UiRequest>,
    wake_ui: &UiWake,
    doc: jian_ops_schema::PenDocument,
) -> Result<(), String> {
    let (ack_tx, ack_rx) = mpsc::sync_channel(1);
    req_tx
        .send(UiRequest::ReplaceDocument {
            doc: Box::new(doc),
            ack: ack_tx,
        })
        .map_err(|_| "UI thread is not accepting MCP document-sync requests".to_string())?;
    wake_ui();
    recv_with_timeout(ack_rx.recv_timeout(UI_ACK_TIMEOUT), "document-sync")
}

fn recv_with_timeout<T>(result: Result<T, RecvTimeoutError>, label: &str) -> Result<T, String> {
    match result {
        Ok(v) => Ok(v),
        Err(RecvTimeoutError::Timeout) => Err(format!("timed out waiting for UI {label} ack")),
        Err(RecvTimeoutError::Disconnected) => Err(format!("UI {label} ack channel closed")),
    }
}

/// Build a per-instance identity token. Not security-sensitive — it only
/// needs to be distinct across editor instances so the CLI can tell a
/// live server apart from a stale discovery file or a recycled port. pid
/// + start nanos is unique enough (two instances never start at the same
///   nanosecond under the same pid).
fn make_live_token() -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{pid:x}-{nanos:x}")
}

/// Live `ping` reply: OpenPencil identity + `mode:"live"` + the instance
/// token, so `op` can distinguish this live canvas server from the
/// headless file server and confirm it owns the discovery file.
fn live_ping_response(id_raw: &str, token: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":{id_raw},"result":{{"server":"{}","mode":"live","token":"{}"}}}}"#,
        crate::mcp_serve::MCP_SERVER_NAME,
        crate::mcp_serve::json_escape(token)
    )
}

fn error_json(message: &str) -> String {
    format!(
        r#"{{"error":"{}"}}"#,
        crate::mcp_serve::json_escape(message)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_ping_response_carries_identity_mode_and_token() {
        let resp = live_ping_response("3", "abc-123");
        assert!(resp.contains(r#""server":"openpencil-mcp""#), "{resp}");
        assert!(resp.contains(r#""mode":"live""#), "{resp}");
        assert!(resp.contains(r#""token":"abc-123""#), "{resp}");
        assert!(resp.contains(r#""id":3"#), "{resp}");
    }

    #[test]
    fn make_live_token_is_nonempty_and_structured() {
        let token = make_live_token();
        assert!(token.contains('-'), "{token}");
        assert!(!token.starts_with('-') && !token.ends_with('-'), "{token}");
    }

    #[test]
    fn snapshot_request_wakes_ui_event_loop_after_queueing() {
        let (req_tx, req_rx) = mpsc::channel();
        let wake_count = Arc::new(AtomicUsize::new(0));
        let wake_ui: UiWake = {
            let wake_count = Arc::clone(&wake_count);
            Arc::new(move || {
                wake_count.fetch_add(1, Ordering::AcqRel);
            })
        };

        let requester = thread::spawn(move || request_snapshot(&req_tx, &wake_ui));
        let request = req_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("snapshot request should be queued");
        let wake_deadline = std::time::Instant::now() + Duration::from_secs(1);
        while wake_count.load(Ordering::Acquire) == 0 && std::time::Instant::now() < wake_deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            wake_count.load(Ordering::Acquire),
            1,
            "queueing a UI request must wake winit instead of relying on idle polling"
        );
        let UiRequest::Snapshot { ack } = request else {
            panic!("expected snapshot request");
        };
        ack.send(EditorState::new()).expect("ack snapshot");
        requester
            .join()
            .expect("requester thread should not panic")
            .expect("snapshot request should complete");
    }

    #[test]
    fn pump_yields_after_ui_request_budget() {
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = McpLiveServer {
            port: 0,
            token: "test-token".to_string(),
            quit_flag: Arc::new(AtomicBool::new(false)),
            req_rx,
            stop_tx,
            client_identity: Arc::new(Mutex::new(None)),
            last_mcp_epoch: 0,
        };
        let mut state = EditorState::new();
        let mut acks = Vec::new();

        for index in 0..(UI_PUMP_REQUEST_BUDGET + 3) {
            let (ack_tx, ack_rx) = mpsc::sync_channel(1);
            req_tx
                .send(UiRequest::Apply {
                    tool_name: "set_viewport".to_string(),
                    cmd: EditorCommand::SetViewport {
                        pan_x: Some(index as i32),
                        pan_y: None,
                        zoom_percent: None,
                    },
                    ack: ack_tx,
                })
                .expect("queue apply request");
            acks.push(ack_rx);
        }

        let outcome = server.pump(&mut state);
        assert!(outcome.repaint);
        assert!(!outcome.layout_dirty);
        let acked = acks.iter().filter(|ack| ack.try_recv().is_ok()).count();
        assert_eq!(acked, UI_PUMP_REQUEST_BUDGET);
    }

    /// The UI pump renders queued screenshot requests off the live
    /// state through the raster export pipeline — the scripted-channel
    /// half of the live `debug_screenshot` roundtrip.
    #[cfg(feature = "mcp-debug-tools")]
    #[test]
    fn pump_fulfills_screenshot_requests_off_the_live_state() {
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = McpLiveServer {
            port: 0,
            token: "test-token".to_string(),
            quit_flag: Arc::new(AtomicBool::new(false)),
            req_rx,
            stop_tx,
            client_identity: Arc::new(Mutex::new(None)),
            last_mcp_epoch: 0,
        };
        let mut state = EditorState::new();
        assert!(state.apply(EditorCommand::InsertNode {
            kind: "rect".into(),
            name: "Shot".into(),
            x: 5,
            y: 5,
            width: 20,
            height: 10,
            fill_hex: Some("#ff0000".into()),
            target_parent: op_editor_core::NodeId::NONE,
            page_id: None,
        }));

        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        req_tx
            .send(UiRequest::Screenshot {
                spec: crate::export::screenshot::CaptureSpec {
                    node_id: None,
                    padding: 0.0,
                    scale: 1.0,
                },
                ack: ack_tx,
            })
            .expect("queue screenshot request");
        let _ = server.pump(&mut state);
        let shot = ack_rx
            .try_recv()
            .expect("pump must ack the screenshot")
            .expect("capture must succeed for a filled rect");
        assert!(!shot.png_base64.is_empty());
        assert!(shot.actual_bounds.is_none(), "root capture has no bounds");
    }

    #[test]
    fn pump_marks_layout_dirty_only_for_layout_affecting_commands() {
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = McpLiveServer {
            port: 0,
            token: "test-token".to_string(),
            quit_flag: Arc::new(AtomicBool::new(false)),
            req_rx,
            stop_tx,
            client_identity: Arc::new(Mutex::new(None)),
            last_mcp_epoch: 0,
        };
        let mut state = EditorState::new();
        state.selection.anchor = op_editor_core::NodeId::new("n1");
        state.selection.set = vec![op_editor_core::NodeId::new("n1")];

        let (selection_ack_tx, _selection_ack_rx) = mpsc::sync_channel(1);
        req_tx
            .send(UiRequest::Apply {
                tool_name: "clear_selection".to_string(),
                cmd: EditorCommand::ClearSelection,
                ack: selection_ack_tx,
            })
            .expect("queue selection request");
        let selection = server.pump(&mut state);
        assert!(selection.repaint);
        assert!(
            !selection.layout_dirty,
            "selection-only MCP commands must not rebuild layout"
        );

        let (insert_ack_tx, _insert_ack_rx) = mpsc::sync_channel(1);
        req_tx
            .send(UiRequest::Apply {
                tool_name: "insert_node".to_string(),
                cmd: EditorCommand::InsertNode {
                    kind: "rect".to_string(),
                    name: "Box".to_string(),
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                    fill_hex: None,
                    target_parent: op_editor_core::NodeId::NONE,
                    page_id: None,
                },
                ack: insert_ack_tx,
            })
            .expect("queue insert request");
        let insert = server.pump(&mut state);
        assert!(insert.repaint);
        assert!(
            insert.layout_dirty,
            "document-mutating MCP commands must rebuild layout"
        );
        assert!(
            !insert.document_replaced,
            "a plain Apply command is not a whole-document replace"
        );
    }

    /// `ReplaceDocument` swaps in a whole new `EditorState` and resets
    /// `document_revision()` back to 0 (see `EditorState::replace_document`).
    /// Callers that key a cache off `document_revision()` (the desktop
    /// host's image-search scan gate) need an explicit signal to
    /// distinguish this from an ordinary revision-bumping `Apply`, since a
    /// fresh document's revision can alias a previously-scanned one.
    #[test]
    fn pump_flags_document_replaced_only_for_replace_document_requests() {
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = McpLiveServer {
            port: 0,
            token: "test-token".to_string(),
            quit_flag: Arc::new(AtomicBool::new(false)),
            req_rx,
            stop_tx,
            client_identity: Arc::new(Mutex::new(None)),
            last_mcp_epoch: 0,
        };
        let mut state = EditorState::new();

        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        req_tx
            .send(UiRequest::ReplaceDocument {
                doc: Box::new(EditorState::new().doc),
                ack: ack_tx,
            })
            .expect("queue replace-document request");
        let outcome = server.pump(&mut state);
        assert!(ack_rx.try_recv().is_ok(), "replace must ack");
        assert!(outcome.repaint);
        assert!(outcome.layout_dirty);
        assert!(
            outcome.document_replaced,
            "ReplaceDocument must flag document_replaced"
        );
    }

    // ── MCP-driven canvas generation indicators ─────────────────────────
    //
    // No dedicated lock around the shared `agent_indicators` registry —
    // matches the existing convention in this crate
    // (`design_agent_tools.rs`'s
    // `execute_design_batch_design_registers_reveals_when_epoch_is_set`
    // also touches it lock-free). Each test clears on entry AND exit so
    // it starts and leaves the registry empty regardless of ordering.

    fn fresh_mcp_server(req_rx: Receiver<UiRequest>, stop_tx: Sender<()>) -> McpLiveServer {
        McpLiveServer {
            port: 0,
            token: "test-token".to_string(),
            quit_flag: Arc::new(AtomicBool::new(false)),
            req_rx,
            stop_tx,
            client_identity: Arc::new(Mutex::new(None)),
            last_mcp_epoch: 0,
        }
    }

    /// A one-frame-one-child subtree, JSON-built (same shape as
    /// `design_agent_tools.rs`'s own `batch_design` reveal test) so the
    /// insert produces both a fresh top-level root AND a fresh
    /// descendant — the frame tag and the reveal path are two distinct
    /// code branches in `register_mcp_generation` and this exercises both.
    fn root_frame_with_child(root_id: &str, child_id: &str) -> PenNode {
        serde_json::from_str(&format!(
            r#"{{"type":"frame","id":"{root_id}","name":"Root","width":200,"height":200,
                "children":[{{"type":"rectangle","id":"{child_id}","name":"Box","width":50,"height":50}}]}}"#
        ))
        .expect("valid frame json")
    }

    fn send_batch_design_insert(
        req_tx: &Sender<UiRequest>,
        node: PenNode,
    ) -> mpsc::Receiver<ApplyAck> {
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        req_tx
            .send(UiRequest::Apply {
                tool_name: "batch_design".to_string(),
                // `InsertAuthoredSubtree`, not `InsertSubtree` — the
                // latter remaps every incoming id to a fresh editor id
                // (`command.rs`'s own doc: "the caller's ids are
                // structural placeholders only"), which would make this
                // test's assertions target whatever id landed rather
                // than the one it authored. Doesn't change what's under
                // test: `register_mcp_generation` diffs ids generically
                // and doesn't care which `EditorCommand` variant ran.
                cmd: EditorCommand::InsertAuthoredSubtree {
                    nodes: vec![node],
                    parent_id: op_editor_core::NodeId::NONE,
                    page_id: None,
                },
                ack: ack_tx,
            })
            .expect("queue batch_design apply");
        ack_rx
    }

    #[test]
    fn batch_design_apply_populates_the_scan_gate_frames_and_reveals() {
        op_editor_core::agent_indicators::clear();
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = fresh_mcp_server(req_rx, stop_tx);
        let mut state = EditorState::new();

        let ack_rx = send_batch_design_insert(&req_tx, root_frame_with_child("root-a", "box-a"));
        let outcome = server.pump(&mut state);
        assert!(ack_rx.try_recv().is_ok_and(|ack| ack.applied));
        assert!(outcome.repaint);

        // The canvas radar-scan (`canvas_generation_scan::generating_paint_sets`)
        // gates purely on `AgentIndicators.frames` being non-empty — this is
        // the exact fact the bug report needed and the CLI-provider path
        // already has covered (`chat_intent_host_tests.rs`'s
        // `cli_new_design_populates_frame_indicators_the_canvas_scan_gates_on`);
        // this is the MCP-tool-driven counterpart.
        let snapshot = op_editor_core::agent_indicators::snapshot();
        assert!(
            snapshot.frames.contains_key("root-a"),
            "the new top-level root must be tagged: {:?}",
            snapshot.frames
        );
        assert!(
            snapshot.reveals.contains_key("root-a") && snapshot.reveals.contains_key("box-a"),
            "both the new root and its new child must have entrance reveals: {:?}",
            snapshot.reveals
        );

        op_editor_core::agent_indicators::clear();
    }

    #[test]
    fn batch_design_apply_is_a_noop_when_nothing_new_landed() {
        op_editor_core::agent_indicators::clear();
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = fresh_mcp_server(req_rx, stop_tx);
        let mut state = EditorState::new();

        // A `batch_design` call whose command carries an EMPTY subtree —
        // shaped like a Direct-op property tweak that inserts nothing new.
        let (ack_tx, ack_rx) = mpsc::sync_channel(1);
        req_tx
            .send(UiRequest::Apply {
                tool_name: "batch_design".to_string(),
                cmd: EditorCommand::InsertSubtree {
                    nodes: vec![],
                    parent_id: op_editor_core::NodeId::NONE,
                    page_id: None,
                },
                ack: ack_tx,
            })
            .expect("queue batch_design apply");
        let _ = server.pump(&mut state);
        let _ = ack_rx.try_recv();

        assert_eq!(
            op_editor_core::agent_indicators::snapshot().frames.len(),
            0,
            "an apply that produced no new content must not begin an epoch at all"
        );
        assert_eq!(server.last_mcp_epoch, 0, "no epoch was ever minted");

        op_editor_core::agent_indicators::clear();
    }

    /// Locks in Track "batch_design 单批自成一轮 + 漂移窗口合并" — the whole
    /// value of that design over the simpler "every call gets its own
    /// fresh epoch" alternative: a second `batch_design` apply that lands
    /// while the FIRST one's reveal queue is still draining must extend
    /// the same epoch, not `begin()` a fresh one — `begin()` unconditionally
    /// clears every map (`AgentIndicators::clear_maps`), so if this
    /// coalescing didn't happen the first root's frame tag would be wiped
    /// the instant the second call landed.
    #[test]
    fn back_to_back_batch_design_calls_coalesce_into_one_epoch_while_reveals_drain() {
        op_editor_core::agent_indicators::clear();
        let (req_tx, req_rx) = mpsc::channel();
        let (stop_tx, _stop_rx) = mpsc::channel();
        let mut server = fresh_mcp_server(req_rx, stop_tx);
        let mut state = EditorState::new();

        let first_ack = send_batch_design_insert(&req_tx, root_frame_with_child("root-a", "box-a"));
        server.pump(&mut state);
        assert!(first_ack.try_recv().is_ok_and(|ack| ack.applied));
        let epoch_after_first = server.last_mcp_epoch;
        assert_ne!(epoch_after_first, 0, "first call must have minted an epoch");
        // The first call's own reveal queue must still be mid-flight for
        // this test to prove anything — it always is here (`REVEAL_DURATION_MS`
        // is a full second; this whole test runs in microseconds).
        assert!(
            op_editor_core::agent_indicators::latest_reveal_end_ms(epoch_after_first)
                .is_some_and(|end_ms| reveal_now_millis_for_test() < end_ms),
            "test precondition: the first batch's reveal queue must still be draining"
        );

        // Second call, different root, arriving inside that drain window.
        let second_ack =
            send_batch_design_insert(&req_tx, root_frame_with_child("root-b", "box-b"));
        server.pump(&mut state);
        assert!(second_ack.try_recv().is_ok_and(|ack| ack.applied));

        assert_eq!(
            server.last_mcp_epoch, epoch_after_first,
            "the second call must REUSE the first call's still-draining epoch, not begin() a fresh one"
        );
        let snapshot = op_editor_core::agent_indicators::snapshot();
        assert!(
            snapshot.frames.contains_key("root-a") && snapshot.frames.contains_key("root-b"),
            "both roots must coexist in the SAME registry snapshot — if the second \
             call had begun a fresh epoch instead, begin()'s clear_maps() would have \
             wiped root-a's tag the instant root-b's call landed: {:?}",
            snapshot.frames
        );
        assert!(
            snapshot.reveals.contains_key("box-a") && snapshot.reveals.contains_key("box-b"),
            "both batches' reveals must coexist too: {:?}",
            snapshot.reveals
        );

        op_editor_core::agent_indicators::clear();
    }

    fn reveal_now_millis_for_test() -> u64 {
        crate::design_agent_tools::reveal_now_millis()
    }
}
