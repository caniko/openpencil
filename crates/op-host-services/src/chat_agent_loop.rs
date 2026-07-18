//! Tool-executing loops for builtin Anthropic and OpenAI-compatible providers.
//! Canvas tool calls stream from the model, execute through the injected
//! [`ChatToolExecutor`], and ride the next request as correlated results until
//! the model stops or the turn cap is reached. Production uses the UI-thread
//! bridge; loopback tests keep this transport layer deterministic.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use futures::StreamExt;
use op_ai::chat_provider::{
    ChatDelta, ChatHistoryRole, ChatToolDef, ChatToolExecutor, ChatToolResult, StopReason,
    UnfilledScreensReport,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[cfg(test)]
use crate::chat_agent_context::screenshot_image_base64;
use crate::chat_agent_context::{
    elide_inline_screenshots, prepare_screenshot_for_context, PreparedScreenshot,
    OMITTED_SCREENSHOT_TEXT, TEXT_ONLY_SCREENSHOT_TEXT,
};
use crate::chat_builtin_http::{map_anthropic_stop_reason, map_openai_stop_reason};

/// Everything one agent-loop run needs. `max_turns` is the TS `maxTurns`
/// cap — `MAX_TOOL_TURNS = 20` for plain chat, `DESIGN_LOOP_MAX_TURNS = 28`
/// for the gated design-generation loop (`chat_builtin_http.rs`; the two
/// callers pick per `finalize_on_exit`). This caps only ORDINARY turns —
/// a dedicated promise-delivery fill round (see `FILL_BUDGET_MAX_ROUNDS_PER_SCREEN`
/// below) is deliberately exempt from it. Tests shrink `max_turns` further.
pub struct AgentLoopConfig {
    pub url: String,
    pub api_key: String,
    pub model: String,
    pub system_prompt: String,
    pub history: Vec<(ChatHistoryRole, String)>,
    pub user_prompt: String,
    pub max_output_tokens: u32,
    pub tools: Vec<ChatToolDef>,
    pub executor: Arc<dyn ChatToolExecutor>,
    pub max_turns: usize,
    /// Opt-in structural backstop at design-loop exit. Plain chat must keep
    /// this false because finalization mutates the live document.
    pub finalize_on_exit: bool,
    /// Disable MiniMax / GLM hidden reasoning for structured design output;
    /// otherwise it can consume the whole output allowance before tool JSON.
    pub disable_thinking: bool,
    /// Dial policy inherited from the provider that spawned this loop —
    /// browser-originated endpoints resolve + pin per request.
    pub(crate) dial_policy: crate::provider_dial::EndpointDialPolicy,
}

impl AgentLoopConfig {
    fn level_for(&self, tool: &str) -> String {
        self.tools
            .iter()
            .find(|t| t.name == tool)
            .map(|t| t.level.clone())
            .unwrap_or_else(|| "read".to_string())
    }
}

/// One fully-accumulated model tool call.
#[derive(Debug, Clone)]
struct PendingToolCall {
    id: String,
    name: String,
    args_json: String,
}

/// Build the transcript tool-card payload for a `ChatDelta::ToolUse`.
/// The chat panel's tool card (`ai_chat_transcript_tools.rs`) parses
/// this envelope: `level` picks the expand default, `args` renders,
/// `status: "running"` animates until the host attaches the result.
pub fn tool_card_envelope(level: &str, args_json: &str) -> String {
    let args = serde_json::from_str::<Value>(args_json)
        .unwrap_or_else(|_| Value::String(args_json.to_string()));
    json!({ "level": level, "args": args, "status": "running" }).to_string()
}

/// Empty / whitespace accumulated tool arguments normalize to `{}`.
fn normalized_args(args_json: &str) -> String {
    let trimmed = args_json.trim();
    if trimmed.is_empty() {
        "{}".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Run one tool call through the executor on a blocking thread (the
/// executor blocks on the UI ack; never block the runtime directly).
async fn execute_tool(
    executor: &Arc<dyn ChatToolExecutor>,
    name: &str,
    args_json: &str,
) -> ChatToolResult {
    let executor = executor.clone();
    let name = name.to_string();
    let args = normalized_args(args_json);
    tokio::task::spawn_blocking(move || executor.execute(&name, &args))
        .await
        .unwrap_or_else(|e| ChatToolResult {
            content: json!({ "success": false, "error": format!("tool executor panicked: {e}") })
                .to_string(),
            is_error: true,
        })
}

/// Run the deterministic structural-quality backstop once, at loop end
/// (Track-1 Step 4). Forwards to the executor's `finalize` (which the desktop
/// host bridges to `op_orchestrator::apply_loop_finalize` against the live
/// `EditorState`). Runs on a blocking thread — the host round-trip blocks until
/// the UI thread acks — mirroring [`execute_tool`]. The default executor
/// `finalize` is a no-op, so this is inert for scripted / read-only executors.
///
/// `enabled` is `cfg.finalize_on_exit`: a no-op early-return when this loop run
/// is a regular chat turn (the shared agent loop also serves plain builtin
/// chat), so only the gated design-generation loop mutates the document here.
///
/// Returns the promise-delivery invariant's committed/unfilled report AFTER
/// finalize ran and any still-unfilled screens got canvas-marked — empty on
/// the default no-op and on the common path where nothing was left unfilled.
async fn run_loop_finalize(
    executor: &Arc<dyn ChatToolExecutor>,
    enabled: bool,
) -> UnfilledScreensReport {
    if !enabled {
        return UnfilledScreensReport::default();
    }
    let executor = executor.clone();
    tokio::task::spawn_blocking(move || executor.finalize())
        .await
        .unwrap_or_default()
}

/// Cheap, read-only promise-delivery probe — same detector [`run_loop_finalize`]
/// runs, but never mutates the document or marks the canvas. Called whenever
/// the loop is deciding whether a dedicated fill round is owed, so a
/// still-eligible screen gets one more shot instead of immediately being
/// branded "(unfilled)". Gated by the same `enabled` flag as
/// [`run_loop_finalize`] — a plain chat turn never touches the document.
async fn check_unfilled(
    executor: &Arc<dyn ChatToolExecutor>,
    enabled: bool,
) -> UnfilledScreensReport {
    if !enabled {
        return UnfilledScreensReport::default();
    }
    let executor = executor.clone();
    tokio::task::spawn_blocking(move || executor.check_unfilled_screens())
        .await
        .unwrap_or_default()
}

/// Per-screen dedicated fill-round budget. "预算只防失控，绝不截断已承诺的
/// 工作" (budget only guards against a runaway retry avalanche; it must
/// never itself be the reason a promised screen ships empty): a fill round
/// spent nudging the model about a still-unfilled COMMITTED screen is exempt
/// from the ordinary `max_turns` cap — see the two loops' `turn >= turn_cap`
/// gates below, which keep issuing dedicated rounds past the cap as long as
/// some committed screen is still eligible. This constant is the ONLY thing
/// that stops that from running forever: the SAME screen failing across
/// `FILL_BUDGET_MAX_ROUNDS_PER_SCREEN` dedicated attempts is accepted as a
/// real failure (reported honestly, tier 3) rather than retried indefinitely
/// — 2 rounds mirrors `geometry_echo`'s "detect, retry, then accept"
/// discipline stretched by one extra attempt, since a wholly blank screen is
/// a starker failure than a layout nit.
const FILL_BUDGET_MAX_ROUNDS_PER_SCREEN: usize = 2;

/// Which of `unfilled` still has dedicated fill-round budget left, per
/// `attempts` (screen name -> dedicated rounds already spent on it this run).
fn eligible_for_fill_round(unfilled: &[String], attempts: &HashMap<String, usize>) -> Vec<String> {
    unfilled
        .iter()
        .filter(|name| {
            attempts.get(name.as_str()).copied().unwrap_or(0) < FILL_BUDGET_MAX_ROUNDS_PER_SCREEN
        })
        .cloned()
        .collect()
}

/// Record that a dedicated fill round was just spent on each of `names`.
fn spend_fill_round(attempts: &mut HashMap<String, usize>, names: &[String]) {
    for name in names {
        *attempts.entry(name.clone()).or_insert(0) += 1;
    }
}

/// Post-exhaustion "salvage" budget — a SEPARATE pool from
/// [`FILL_BUDGET_MAX_ROUNDS_PER_SCREEN`]'s ("两个预算，职责分离": the
/// ordinary `max_turns` cap guards against the model rambling on forever;
/// this pool guards "承诺必达" specifically once that ordinary budget is
/// exhausted — running out of turns must never itself be why a committed
/// screen ships empty). Every committed-but-unfilled screen still gets AT
/// MOST one dedicated salvage round each (never retried a second time here —
/// unlike the richer 2-round budget under ordinary turns), and the whole run
/// never spends more than [`SALVAGE_MAX_ROUNDS`] dedicated rounds total —
/// belt-and-suspenders against an avalanche, even though bundling every
/// still-eligible screen into ONE contract message (see the `turn >=
/// turn_cap` gates below) means this converges in exactly one round for the
/// common case of "some screens got missed."
const SALVAGE_MAX_ROUNDS: usize = 3;

/// Which of `unfilled` has not yet been salvaged this run.
fn salvage_eligible(
    unfilled: &[String],
    salvaged: &std::collections::HashSet<String>,
) -> Vec<String> {
    unfilled
        .iter()
        .filter(|name| !salvaged.contains(name.as_str()))
        .cloned()
        .collect()
}

/// Tier-2 nudge text — states the FULL commitment, not just what's still
/// missing, so the model sees an explicit broken promise instead of a
/// generic "fill it now" ("把承诺变成模型可见的契约" — turn the promise into
/// a contract the model can see). `committed` is every screen the run
/// scaffolded (filled or not, from [`UnfilledScreensReport::committed`]);
/// `still_empty` is the fill-budget-eligible subset this round is asking the
/// model to act on.
fn contract_nudge_text(committed: &[String], still_empty: &[String]) -> String {
    let commit_clause = if committed.len() > 1 {
        format!(
            "You committed {} screens ({}); ",
            committed.len(),
            committed.join("/")
        )
    } else {
        String::new()
    };
    let (subject, pronoun) = if still_empty.len() == 1 {
        ("is", "it")
    } else {
        ("are", "them")
    };
    format!(
        "{commit_clause}{} {subject} still empty. Complete {pronoun} before finishing.",
        still_empty.join(", ")
    )
}

/// Tier-3 unconditional honest report — appended to the transcript right
/// before `Done` whenever [`run_loop_finalize`] still finds unfilled screens
/// (every dedicated fill round the screen was eligible for ran out, or the
/// executor never had a live document to check). Wording mirrors the classic
/// path's `Progress::UnfilledScreens` line (`op-host-desktop::design_session`'s
/// `apply_progress` / `op-host-services::web_chat_standard`'s
/// `progress_label`) so a user sees the same sentence shape on either path.
async fn report_unfilled_if_any(tx: &mpsc::Sender<ChatDelta>, names: &[String]) {
    if names.is_empty() {
        return;
    }
    let text = format!(
        "\n\n• {} screen(s) left unfilled: {}",
        names.len(),
        names.join(", ")
    );
    let _ = tx.send(ChatDelta::TextDelta(text)).await;
}

/// Self-diagnostic signal for the finalize-lifecycle invariant (0718-1-k3-1
/// postmortem) — a `run_anthropic_agent_loop` / `run_openai_agent_loop`
/// outer wrapper's backstop just ran [`run_loop_finalize`] on an `Err` exit
/// the inner loop's own paths never reached. Embedded directly in the
/// transcript (not a `tracing`/log call) on purpose: the ONLY forensic
/// trace available for the 0718-1-k3-1 incident afterward was the chat
/// transcript itself — no session log survived — so this is the greppable
/// answer to "did finalize actually run, and from where" the next time a
/// file turns up unfinalized. `enabled` mirrors [`run_loop_finalize`]'s own
/// gate — a plain (non-design) chat turn never touches the document, so it
/// never emits this either.
async fn emit_finalize_diagnostic(tx: &mpsc::Sender<ChatDelta>, enabled: bool, source: &str) {
    if !enabled {
        return;
    }
    let _ = tx
        .send(ChatDelta::TextDelta(format!(
            "\n\n• finalize ran (source={source})"
        )))
        .await;
}

// ---------------------------------------------------------------------------
// Shared SSE pump
// ---------------------------------------------------------------------------

/// Stateful per-event handler. `handle` returns the deltas to forward
/// to the chat channel for one SSE `data:` payload.
trait SseCollector {
    fn handle(&mut self, data: &str) -> Vec<ChatDelta>;
}

/// Drain `resp`'s SSE body through `collector`, forwarding returned
/// deltas to `tx`. Line/event framing mirrors
/// `chat_builtin_http::pump_sse_response` (multi-line `data:`
/// accumulation, trailing-buffer flush) but hands events to a
/// stateful collector instead of a pure parse fn — tool-call
/// accumulation spans many events.
async fn pump_sse<C: SseCollector>(
    resp: reqwest::Response,
    tx: &mpsc::Sender<ChatDelta>,
    collector: &mut C,
) -> Result<(), String> {
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut event_data = String::new();

    while let Some(chunk) = stream.next().await {
        if tx.is_closed() {
            return Ok(());
        }
        let bytes = chunk.map_err(|e| format!("sse stream: {e}"))?;
        buf.extend_from_slice(&bytes);
        while let Some(nl_pos) = buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = buf.drain(..=nl_pos).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches('\n').trim_end_matches('\r');
            if line.is_empty() {
                dispatch_event(&mut event_data, tx, collector).await;
                continue;
            }
            if let Some(data) = line.strip_prefix("data:") {
                if !event_data.is_empty() {
                    event_data.push('\n');
                }
                event_data.push_str(data.trim_start());
            }
        }
    }

    if !buf.is_empty() {
        let line = String::from_utf8_lossy(&buf);
        let line = line.trim_end_matches('\n').trim_end_matches('\r');
        if let Some(data) = line.strip_prefix("data:") {
            if !event_data.is_empty() {
                event_data.push('\n');
            }
            event_data.push_str(data.trim_start());
        }
    }
    dispatch_event(&mut event_data, tx, collector).await;
    Ok(())
}

async fn dispatch_event<C: SseCollector>(
    event_data: &mut String,
    tx: &mpsc::Sender<ChatDelta>,
    collector: &mut C,
) {
    let data = event_data.trim();
    if data.is_empty() {
        event_data.clear();
        return;
    }
    let deltas = collector.handle(data);
    event_data.clear();
    for delta in deltas {
        if tx.send(delta).await.is_err() {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Anthropic wire
// ---------------------------------------------------------------------------

/// Per-index content-block accumulation state for one Anthropic turn.
enum AnthropicBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        json: String,
    },
    Other,
}

#[derive(Default)]
struct AnthropicCollector {
    blocks: BTreeMap<u64, AnthropicBlock>,
    stop_reason: Option<String>,
    error: Option<String>,
}

impl AnthropicCollector {
    fn tool_calls(&self) -> Vec<PendingToolCall> {
        self.blocks
            .values()
            .filter_map(|b| match b {
                AnthropicBlock::ToolUse { id, name, json } => Some(PendingToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    args_json: normalized_args(json),
                }),
                _ => None,
            })
            .collect()
    }

    /// Assistant `content[]` for the follow-up request — text + the
    /// tool_use blocks in stream order. (Thinking blocks are not
    /// replayed: the builtin chat request never enables thinking, so
    /// none arrive with signatures worth preserving.)
    fn assistant_content(&self) -> Vec<Value> {
        self.blocks
            .values()
            .filter_map(|b| match b {
                AnthropicBlock::Text(text) if !text.is_empty() => {
                    Some(json!({ "type": "text", "text": text }))
                }
                AnthropicBlock::ToolUse {
                    id,
                    name,
                    json: args,
                } => {
                    let input = serde_json::from_str::<Value>(&normalized_args(args))
                        .unwrap_or_else(|_| json!({}));
                    Some(json!({ "type": "tool_use", "id": id, "name": name, "input": input }))
                }
                _ => None,
            })
            .collect()
    }
}

impl SseCollector for AnthropicCollector {
    fn handle(&mut self, data: &str) -> Vec<ChatDelta> {
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "content_block_start" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                let block = value.get("content_block");
                let kind = block
                    .and_then(|b| b.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let entry = match kind {
                    "text" => AnthropicBlock::Text(String::new()),
                    "tool_use" => AnthropicBlock::ToolUse {
                        id: block
                            .and_then(|b| b.get("id"))
                            .and_then(Value::as_str)
                            .unwrap_or("toolu_unknown")
                            .to_string(),
                        name: block
                            .and_then(|b| b.get("name"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        json: String::new(),
                    },
                    _ => AnthropicBlock::Other,
                };
                self.blocks.insert(index, entry);
                Vec::new()
            }
            "content_block_delta" => {
                let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                let Some(delta) = value.get("delta") else {
                    return Vec::new();
                };
                match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text_delta" => {
                        let Some(text) = delta.get("text").and_then(Value::as_str) else {
                            return Vec::new();
                        };
                        if let Some(AnthropicBlock::Text(acc)) = self.blocks.get_mut(&index) {
                            acc.push_str(text);
                        } else {
                            self.blocks
                                .insert(index, AnthropicBlock::Text(text.to_string()));
                        }
                        if text.is_empty() {
                            Vec::new()
                        } else {
                            vec![ChatDelta::TextDelta(text.to_string())]
                        }
                    }
                    "thinking_delta" => delta
                        .get("thinking")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(|s| vec![ChatDelta::Thinking(s.to_string())])
                        .unwrap_or_default(),
                    "input_json_delta" => {
                        if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                            if let Some(AnthropicBlock::ToolUse { json, .. }) =
                                self.blocks.get_mut(&index)
                            {
                                json.push_str(partial);
                            }
                        }
                        Vec::new()
                    }
                    _ => Vec::new(),
                }
            }
            "message_delta" => {
                if let Some(reason) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                    self.stop_reason = Some(reason.to_string());
                }
                Vec::new()
            }
            "error" => {
                let message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("anthropic stream error")
                    .to_string();
                self.error = Some(message);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }
}

/// Finalize-lifecycle invariant outer wrapper (0718-1-k3-1 postmortem — see
/// [`run_loop_finalize`]'s doc + `op-host-desktop::chat_session::
/// finalize_design_session_if_needed`'s doc for the desktop-side half of
/// this same invariant).
///
/// ## The actual break, confirmed by reading this file
///
/// [`run_anthropic_agent_loop_inner`] / [`run_openai_agent_loop_inner`] each
/// call [`run_loop_finalize`] on every NORMAL exit (turn-cap exhausted,
/// salvage exhausted, model voluntarily stopped) — but FOUR early-return
/// sites per loop bypass it entirely: `client_for(...).await?`,
/// `send_with_backoff(...).await?`, `pump_sse(...).await?` (each via `?`
/// propagation), and the explicit `if let Some(err) = collector.error {
/// return Err(err); }` right after — the last one is what the 0718-1-k3-1
/// transcript's mid-stream `openai-compatible http 400` line hit: `pump_sse`
/// itself returns `Ok(())` (it drained the stream fine), but the collected
/// SSE-level error makes the caller return `Err` one line later, never
/// reaching the loop's own finalize call below it. `chat_builtin_http.rs`'s
/// `Err(e) => { tx.send(Error); tx.send(Done{Aborted}); }` catch-all never
/// finalizes either — it only forwards the error.
///
/// This wrapper is the single place ALL of a loop run's exits funnel
/// through, so it is where the invariant is actually enforced: on `Err`,
/// run the SAME best-effort finalize the inner loop's own normal-exit paths
/// already do, tagged `loop-exit` so a future occurrence is locatable by
/// grepping for that tag. Idempotent — [`run_loop_finalize`] no-ops when
/// `!cfg.finalize_on_exit`, and `apply_loop_finalize`'s own passes are each
/// individually idempotent, so this never double-mutates a document that an
/// inner normal-exit path already finalized (those paths return `Ok`, so
/// this wrapper's backstop only fires on the exact paths that skipped it).
pub async fn run_anthropic_agent_loop(
    cfg: AgentLoopConfig,
    tx: &mpsc::Sender<ChatDelta>,
) -> Result<bool, String> {
    let executor = cfg.executor.clone();
    let enabled = cfg.finalize_on_exit;
    let result = run_anthropic_agent_loop_inner(cfg, tx).await;
    if result.is_err() {
        run_loop_finalize(&executor, enabled).await;
        emit_finalize_diagnostic(tx, enabled, "loop-exit").await;
    }
    result
}

/// Run the Anthropic agent loop to completion. Returns `Ok(true)` when
/// a terminal `Done` was emitted; `Err` for transport / in-stream
/// errors (caller surfaces them as `Error + Done{Aborted}`).
async fn run_anthropic_agent_loop_inner(
    cfg: AgentLoopConfig,
    tx: &mpsc::Sender<ChatDelta>,
) -> Result<bool, String> {
    let tools_json: Vec<Value> = cfg
        .tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": serde_json::from_str::<Value>(&t.input_schema_json)
                    .unwrap_or_else(|_| json!({ "type": "object" })),
            })
        })
        .collect();
    let mut messages: Vec<Value> = Vec::new();
    for (role, text) in &cfg.history {
        messages.push(json!({ "role": role.as_str(), "content": text }));
    }
    messages.push(json!({ "role": "user", "content": cfg.user_prompt }));

    // Tier 2 — under-budget per-screen fill-round budget (see
    // `FILL_BUDGET_MAX_ROUNDS_PER_SCREEN`'s doc comment for the "budget
    // never truncates committed work" rationale).
    let mut fill_attempts: HashMap<String, usize> = HashMap::new();
    // Tier 2b — post-exhaustion salvage budget. A SEPARATE pool from
    // `fill_attempts` above (see `SALVAGE_MAX_ROUNDS`'s doc comment).
    let mut salvaged_screens: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut salvage_rounds_used = 0usize;
    let turn_cap = cfg.max_turns.max(1);
    let mut turn = 0usize;
    loop {
        // Ordinary turn budget exhausted — the ONLY reason to send another
        // request is a committed screen that's still eligible for its own
        // dedicated salvage round. This request, if sent, does NOT count
        // against `turn`; it draws from `SALVAGE_MAX_ROUNDS` instead.
        if turn >= turn_cap {
            if salvage_rounds_used >= SALVAGE_MAX_ROUNDS {
                let still_unfilled = run_loop_finalize(&cfg.executor, cfg.finalize_on_exit).await;
                report_unfilled_if_any(tx, &still_unfilled.unfilled).await;
                let _ = tx
                    .send(ChatDelta::Done {
                        stop_reason: StopReason::MaxTokens,
                    })
                    .await;
                return Ok(true);
            }
            let report = check_unfilled(&cfg.executor, cfg.finalize_on_exit).await;
            let eligible = salvage_eligible(&report.unfilled, &salvaged_screens);
            if eligible.is_empty() {
                // Turn cap reached, and nothing committed is left worth
                // trying for — either nothing was ever unfilled, or every
                // unfilled screen already spent its one salvage round. Still
                // run the Step-4 structural backstop once over whatever the
                // run assembled, and report unconditionally.
                let still_unfilled = run_loop_finalize(&cfg.executor, cfg.finalize_on_exit).await;
                report_unfilled_if_any(tx, &still_unfilled.unfilled).await;
                let _ = tx
                    .send(ChatDelta::Done {
                        stop_reason: StopReason::MaxTokens,
                    })
                    .await;
                return Ok(true);
            }
            salvage_rounds_used += 1;
            for name in &eligible {
                salvaged_screens.insert(name.clone());
            }
            messages.push(
                json!({ "role": "user", "content": contract_nudge_text(&report.committed, &eligible) }),
            );
            // Falls through to send this dedicated salvage round below —
            // bundling every eligible screen into one message means this
            // branch converges (nothing left eligible) after exactly one
            // round for the common case.
        }

        let mut body = json!({
            "model": cfg.model,
            "max_tokens": cfg.max_output_tokens,
            "stream": true,
            "messages": messages,
            "tools": tools_json,
        });
        if !cfg.system_prompt.trim().is_empty() {
            body.as_object_mut()
                .expect("anthropic request body is object")
                .insert("system".into(), json!(cfg.system_prompt));
        }
        let client = crate::provider_dial::client_for(cfg.dial_policy, &cfg.url).await?;
        let (max_retries, min_gap) = crate::chat_builtin_http::default_backoff_knobs();
        let resp = crate::chat_builtin_http::send_with_backoff(
            "anthropic",
            &cfg.url,
            max_retries,
            min_gap,
            || {
                client
                    .post(&cfg.url)
                    .header("x-api-key", &cfg.api_key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&body)
            },
        )
        .await?;
        let mut collector = AnthropicCollector::default();
        pump_sse(resp, tx, &mut collector).await?;
        if tx.is_closed() {
            return Ok(true);
        }
        if let Some(err) = collector.error {
            return Err(err);
        }
        let calls = collector.tool_calls();
        if calls.is_empty() {
            let reason = collector
                .stop_reason
                .as_deref()
                .map(map_anthropic_stop_reason)
                .unwrap_or(StopReason::EndTurn);
            if turn >= turn_cap {
                // Already in salvage territory — the top-of-loop gate above
                // owns deciding whether another salvage round is owed (it
                // re-checks fresh on the next pass, against `salvaged_screens`
                // / `salvage_rounds_used`). Record this turn's reply in
                // history and defer, rather than deciding again here —
                // deciding in both places would double-nudge.
                messages
                    .push(json!({ "role": "assistant", "content": collector.assistant_content() }));
                continue;
            }
            // Model voluntarily stopped, still within ordinary budget. Try
            // one dedicated fill round for any committed screen that's still
            // eligible — NOT gated by remaining turn budget (only by the
            // per-screen cap above), so budget is never the reason a
            // promised screen ships empty.
            let report = check_unfilled(&cfg.executor, cfg.finalize_on_exit).await;
            let eligible = eligible_for_fill_round(&report.unfilled, &fill_attempts);
            if !eligible.is_empty() {
                spend_fill_round(&mut fill_attempts, &eligible);
                messages
                    .push(json!({ "role": "assistant", "content": collector.assistant_content() }));
                messages.push(
                    json!({ "role": "user", "content": contract_nudge_text(&report.committed, &eligible) }),
                );
                continue; // Does not count against `turn`.
            }
            // Nothing committed is left worth trying for: run the Step-4
            // structural backstop ONCE over the assembled doc BEFORE the
            // Done delta, so the finalized document is what the UI
            // persists/displays for this turn. Tier 3 — honest report —
            // fires unconditionally if anything is still unfilled (a screen
            // outside this run's committed set, or the executor being a
            // no-op).
            let still_unfilled = run_loop_finalize(&cfg.executor, cfg.finalize_on_exit).await;
            report_unfilled_if_any(tx, &still_unfilled.unfilled).await;
            let _ = tx
                .send(ChatDelta::Done {
                    stop_reason: reason,
                })
                .await;
            return Ok(true);
        }
        if turn < turn_cap {
            turn += 1;
        }

        messages.push(json!({ "role": "assistant", "content": collector.assistant_content() }));
        let mut results: Vec<Value> = Vec::new();
        for call in &calls {
            let level = cfg.level_for(&call.name);
            let _ = tx
                .send(ChatDelta::ToolUse {
                    name: call.name.clone(),
                    args: tool_card_envelope(&level, &call.args_json),
                })
                .await;
            let result = execute_tool(&cfg.executor, &call.name, &call.args_json).await;
            // Send screenshots as images only when the model supports them.
            let content: Value = match prepare_screenshot_for_context(
                &cfg.model,
                cfg.max_output_tokens,
                &call.name,
                &result,
            ) {
                Some(PreparedScreenshot::Inline(b64)) => {
                    // Keep only the newest visual observation.
                    for message in &mut messages {
                        elide_inline_screenshots(message);
                    }
                    for prior_result in &mut results {
                        elide_inline_screenshots(prior_result);
                    }
                    json!([
                    { "type": "text", "text": "Rendered screenshot of the current design:" },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": b64,
                        },
                    },
                    ])
                }
                Some(PreparedScreenshot::TextOnly) => {
                    Value::String(TEXT_ONLY_SCREENSHOT_TEXT.to_string())
                }
                Some(PreparedScreenshot::OverBudget) => {
                    Value::String(OMITTED_SCREENSHOT_TEXT.to_string())
                }
                None => Value::String(result.content),
            };
            results.push(json!({
                "type": "tool_result",
                "tool_use_id": call.id,
                "content": content,
                "is_error": result.is_error,
            }));
        }
        messages.push(json!({ "role": "user", "content": results }));
    }
}

// ---------------------------------------------------------------------------
// OpenAI-compatible wire
// ---------------------------------------------------------------------------

#[derive(Default)]
struct OpenAiToolCallAcc {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct OpenAiCollector {
    text: String,
    tool_calls: BTreeMap<u64, OpenAiToolCallAcc>,
    finish_reason: Option<String>,
    error: Option<String>,
}

impl SseCollector for OpenAiCollector {
    fn handle(&mut self, data: &str) -> Vec<ChatDelta> {
        if data == "[DONE]" {
            return Vec::new();
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            return Vec::new();
        };
        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            self.error = Some(message.to_string());
            return Vec::new();
        }
        let Some(choice) = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(delta) = choice.get("delta") {
            if let Some(reasoning) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                out.push(ChatDelta::Thinking(reasoning.to_string()));
            }
            if let Some(content) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                self.text.push_str(content);
                out.push(ChatDelta::TextDelta(content.to_string()));
            }
            if let Some(calls) = delta.get("tool_calls").and_then(Value::as_array) {
                for call in calls {
                    let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                    let acc = self.tool_calls.entry(index).or_default();
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        if !id.is_empty() {
                            acc.id = id.to_string();
                        }
                    }
                    if let Some(name) = call.pointer("/function/name").and_then(Value::as_str) {
                        if !name.is_empty() {
                            acc.name = name.to_string();
                        }
                    }
                    if let Some(args) = call.pointer("/function/arguments").and_then(Value::as_str)
                    {
                        acc.arguments.push_str(args);
                    }
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        out
    }
}

impl OpenAiCollector {
    fn pending_calls(&self) -> Vec<(u64, PendingToolCall)> {
        self.tool_calls
            .iter()
            .filter(|(_, acc)| !acc.name.is_empty())
            .map(|(index, acc)| {
                let id = if acc.id.is_empty() {
                    format!("call_{index}")
                } else {
                    acc.id.clone()
                };
                (
                    *index,
                    PendingToolCall {
                        id,
                        name: acc.name.clone(),
                        args_json: normalized_args(&acc.arguments),
                    },
                )
            })
            .collect()
    }
}

/// Finalize-lifecycle invariant outer wrapper — same contract, same
/// rationale, as [`run_anthropic_agent_loop`]'s own wrapper above (this
/// loop's early-return sites mirror the Anthropic loop's exactly).
pub async fn run_openai_agent_loop(
    cfg: AgentLoopConfig,
    tx: &mpsc::Sender<ChatDelta>,
) -> Result<bool, String> {
    let executor = cfg.executor.clone();
    let enabled = cfg.finalize_on_exit;
    let result = run_openai_agent_loop_inner(cfg, tx).await;
    if result.is_err() {
        run_loop_finalize(&executor, enabled).await;
        emit_finalize_diagnostic(tx, enabled, "loop-exit").await;
    }
    result
}

/// Run the OpenAI-compatible agent loop to completion. Same contract
/// as [`run_anthropic_agent_loop_inner`].
async fn run_openai_agent_loop_inner(
    cfg: AgentLoopConfig,
    tx: &mpsc::Sender<ChatDelta>,
) -> Result<bool, String> {
    let tools_json: Vec<Value> = cfg
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": serde_json::from_str::<Value>(&t.input_schema_json)
                        .unwrap_or_else(|_| json!({ "type": "object" })),
                },
            })
        })
        .collect();
    let mut messages: Vec<Value> = Vec::new();
    if !cfg.system_prompt.trim().is_empty() {
        messages.push(json!({ "role": "system", "content": cfg.system_prompt }));
    }
    for (role, text) in &cfg.history {
        messages.push(json!({ "role": role.as_str(), "content": text }));
    }
    messages.push(json!({ "role": "user", "content": cfg.user_prompt }));

    // Tier 2 — under-budget per-screen fill-round budget, and Tier 2b —
    // post-exhaustion salvage budget — see the Anthropic loop above
    // (`FILL_BUDGET_MAX_ROUNDS_PER_SCREEN` / `SALVAGE_MAX_ROUNDS`'s doc
    // comments) for the full "budget never truncates committed work"
    // rationale and why these are two separate pools.
    let mut fill_attempts: HashMap<String, usize> = HashMap::new();
    let mut salvaged_screens: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut salvage_rounds_used = 0usize;
    let turn_cap = cfg.max_turns.max(1);
    let mut turn = 0usize;
    loop {
        if turn >= turn_cap {
            if salvage_rounds_used >= SALVAGE_MAX_ROUNDS {
                let still_unfilled = run_loop_finalize(&cfg.executor, cfg.finalize_on_exit).await;
                report_unfilled_if_any(tx, &still_unfilled.unfilled).await;
                let _ = tx
                    .send(ChatDelta::Done {
                        stop_reason: StopReason::MaxTokens,
                    })
                    .await;
                return Ok(true);
            }
            let report = check_unfilled(&cfg.executor, cfg.finalize_on_exit).await;
            let eligible = salvage_eligible(&report.unfilled, &salvaged_screens);
            if eligible.is_empty() {
                let still_unfilled = run_loop_finalize(&cfg.executor, cfg.finalize_on_exit).await;
                report_unfilled_if_any(tx, &still_unfilled.unfilled).await;
                let _ = tx
                    .send(ChatDelta::Done {
                        stop_reason: StopReason::MaxTokens,
                    })
                    .await;
                return Ok(true);
            }
            salvage_rounds_used += 1;
            for name in &eligible {
                salvaged_screens.insert(name.clone());
            }
            messages.push(
                json!({ "role": "user", "content": contract_nudge_text(&report.committed, &eligible) }),
            );
            // Falls through to send this dedicated salvage round below.
        }

        let mut body = json!({
            "model": cfg.model,
            "stream": true,
            "max_tokens": cfg.max_output_tokens,
            "messages": messages,
            "tools": tools_json,
        });
        // Turn OFF hidden reasoning for MiniMax / GLM. Without this a glm-5.2
        // design turn spends its whole `max_tokens` on `reasoning_content` and
        // truncates the `batch_design` mid-JSON — the single-shot builtin body
        // gates the same field on the same flag (`chat_builtin_http`), but the
        // loop body was missing it, so every loop turn leaked thinking.
        if cfg.disable_thinking
            && (crate::chat_builtin_http::is_minimax_model(&cfg.model)
                || crate::chat_builtin_http::is_glm_model(&cfg.model))
        {
            if let Some(obj) = body.as_object_mut() {
                obj.insert("thinking".into(), json!({ "type": "disabled" }));
            }
        }
        // Through the shared throttle/backoff: this tool-loop path used
        // to post raw, so a provider rate limit killed the design run with
        // no retries and a raw JSON error (measured: glm-5.2, 429
        // AccountRateLimitExceeded, 2026-07-12).
        let client = crate::provider_dial::client_for(cfg.dial_policy, &cfg.url).await?;
        let (max_retries, min_gap) = crate::chat_builtin_http::default_backoff_knobs();
        let resp = crate::chat_builtin_http::send_with_backoff(
            "openai-compatible",
            &cfg.url,
            max_retries,
            min_gap,
            || client.post(&cfg.url).bearer_auth(&cfg.api_key).json(&body),
        )
        .await?;
        let mut collector = OpenAiCollector::default();
        pump_sse(resp, tx, &mut collector).await?;
        if tx.is_closed() {
            return Ok(true);
        }
        if let Some(err) = collector.error {
            return Err(err);
        }
        let calls = collector.pending_calls();
        if calls.is_empty() {
            let reason = collector
                .finish_reason
                .as_deref()
                .map(map_openai_stop_reason)
                .unwrap_or(StopReason::EndTurn);
            let stop_content = || {
                if collector.text.is_empty() {
                    Value::Null
                } else {
                    Value::String(collector.text.clone())
                }
            };
            if turn >= turn_cap {
                // Already in salvage territory — the top-of-loop gate above
                // owns deciding whether another salvage round is owed.
                // Record this turn's reply and defer.
                messages.push(json!({ "role": "assistant", "content": stop_content() }));
                continue;
            }
            // Model voluntarily stopped, still within ordinary budget. Try
            // one dedicated fill round for any committed screen that's
            // still eligible — NOT gated by remaining turn budget (only by
            // the per-screen cap above).
            let report = check_unfilled(&cfg.executor, cfg.finalize_on_exit).await;
            let eligible = eligible_for_fill_round(&report.unfilled, &fill_attempts);
            if !eligible.is_empty() {
                spend_fill_round(&mut fill_attempts, &eligible);
                messages.push(json!({ "role": "assistant", "content": stop_content() }));
                messages.push(
                    json!({ "role": "user", "content": contract_nudge_text(&report.committed, &eligible) }),
                );
                continue; // Does not count against `turn`.
            }
            // Normal model-stop exit: run the Step-4 structural backstop ONCE
            // over the assembled doc BEFORE the Done delta. Tier 3 — honest
            // report — fires unconditionally if anything is still unfilled.
            let still_unfilled = run_loop_finalize(&cfg.executor, cfg.finalize_on_exit).await;
            report_unfilled_if_any(tx, &still_unfilled.unfilled).await;
            let _ = tx
                .send(ChatDelta::Done {
                    stop_reason: reason,
                })
                .await;
            return Ok(true);
        }
        if turn < turn_cap {
            turn += 1;
        }

        let tool_calls_json: Vec<Value> = calls
            .iter()
            .map(|(_, call)| {
                json!({
                    "id": call.id,
                    "type": "function",
                    "function": { "name": call.name, "arguments": call.args_json },
                })
            })
            .collect();
        let content = if collector.text.is_empty() {
            Value::Null
        } else {
            Value::String(collector.text.clone())
        };
        messages.push(json!({
            "role": "assistant",
            "content": content,
            "tool_calls": tool_calls_json,
        }));
        for (_, call) in &calls {
            let level = cfg.level_for(&call.name);
            let _ = tx
                .send(ChatDelta::ToolUse {
                    name: call.name.clone(),
                    args: tool_card_envelope(&level, &call.args_json),
                })
                .await;
            let result = execute_tool(&cfg.executor, &call.name, &call.args_json).await;
            // OpenAI images follow the short role:tool acknowledgement.
            match prepare_screenshot_for_context(
                &cfg.model,
                cfg.max_output_tokens,
                &call.name,
                &result,
            ) {
                Some(PreparedScreenshot::Inline(b64)) => {
                    // Elide older images; keep intent, tool ids, and text.
                    for message in &mut messages {
                        elide_inline_screenshots(message);
                    }
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": "Rendered screenshot attached as an image in the following message.",
                    }));
                    messages.push(json!({
                        "role": "user",
                        "content": [
                            { "type": "text", "text": "Rendered screenshot of the current design:" },
                            {
                                "type": "image_url",
                                "image_url": { "url": format!("data:image/png;base64,{b64}") },
                            },
                        ],
                    }));
                }
                Some(PreparedScreenshot::TextOnly) => messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": TEXT_ONLY_SCREENSHOT_TEXT,
                })),
                Some(PreparedScreenshot::OverBudget) => messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": OMITTED_SCREENSHOT_TEXT,
                })),
                None => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call.id,
                        "content": result.content,
                    }));
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "chat_agent_loop_tests.rs"]
mod tests;

// Finalize-lifecycle invariant regression tests (0718-1-k3-1) — split out
// as a sibling rather than growing `chat_agent_loop_tests.rs` past its
// existing 800-line-cap debt further; reuses that file's scripted-executor
// + loopback-SSE test infra via `pub(super)`.
#[cfg(test)]
#[path = "chat_agent_loop_finalize_tests.rs"]
mod finalize_tests;
