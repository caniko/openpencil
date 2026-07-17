//! Concurrency primitives shared by the agent-team fan-out (`spawn_concurrent`)
//! and — since the multiscreen-fanout-break fix's item D-lite (2026-07-17) —
//! by the classic Orchestrator's INTER-screen-group executor.
//!
//! ## History
//! The orchestrator's former multi-screen concurrent path (screen grouping +
//! N-root scaffold + semaphore/`join_all` worker fan-in) was collapsed into
//! the sequential path by `aca0d3a0` (2026-07-02): its M3 quality gate only
//! ever measured concurrency WITHIN one screen (parallel sections of the
//! SAME root) against sequential — found byte-identical output, only
//! differing in wall-clock time, so the executor was deleted as unpaid-for
//! complexity. Item A (2026-07-17) revived the PURE partitioning half
//! (`screen_groups::group_subtasks_by_screen` + N-root scaffold,
//! `run_screen_groups.rs`) but kept every group's subtasks running
//! sequentially — the structural bug (`plan_normalize` folding every subtask
//! onto one root) was the actual break; concurrency was orthogonal to it.
//!
//! Item D-lite (this module's new half) revives GENUINE concurrency, but
//! scoped ONLY to DISTINCT screen groups — never same-screen section
//! parallelism, which is exactly what `aca0d3a0`'s data verdict evaluated
//! and found not worth the complexity. That verdict does not transfer here:
//! N independent screens (N independent root subtrees, N independent
//! design-agent turns with no shared mutable state) is a different question
//! from "does splitting ONE screen's sections across workers help" — the
//! former is the ⚡Nx "team size" UI setting's entire premise (`agent_team_size`
//! → `DesignRequest.concurrency`), which had been dead for the classic path
//! since the retirement; this module makes it live again.
//!
//! What's here:
//! - [`clamp_concurrency`] — the defensive `[1, 6]` permit cap.
//! - [`effective_concurrency`] — `min(clamp(request.concurrency), groups.len())`,
//!   forced to `1` when there's at most one group (no parallelism possible or
//!   meaningful) — `run.rs` takes the untouched sequential path whenever this
//!   returns `1`, so single-screen / single-group plans are a byte-identical
//!   regression lock.
//! - [`BufferDocSink`] — a per-worker buffering [`DocSink`] that collects
//!   applied [`EditorCommand`]s without touching the real document, so N
//!   worker futures can run concurrently on a single task with no shared
//!   `&mut` (each worker owns its buffer, moved in and out).
//! - [`run_subtask_retry_ladder`] — the 3-attempt tier-gated retry ladder,
//!   extracted from `run.rs`'s sequential loop so BOTH the sequential path
//!   and each screen-group worker share the IDENTICAL retry semantics —
//!   parallelizing groups must never let a group's retry behavior drift from
//!   what the M3 quality gate was measured against.
//! - [`run_screen_groups_concurrent`] — the executor: one worker future per
//!   screen group, driven together on a [`futures::stream::FuturesUnordered`]
//!   (no `tokio::spawn`, no `Send` bounds beyond what `LlmClient`/`DocSink`
//!   already require), gated by a shared `tokio::sync::Semaphore` sized to
//!   `effective_concurrency`. A `tokio::select!` loop polls the progress
//!   channel and the worker set TOGETHER (visibility fix, 2026-07-17): each
//!   `Progress` event reaches the caller the moment it's sent, and each
//!   group's buffered commands replay into the REAL sink the instant that
//!   group finishes — in COMPLETION order, not plan order (safe: every
//!   group's root is a disjoint pre-scaffolded subtree).

use crate::model_profile::ModelTier;
use crate::plan::{OrchestratorPlan, Subtask};
use crate::retry::is_non_retryable;
use crate::screen_groups::ScreenGroup;
use crate::subagent::{apply_command_with_reveal, reveal_now_millis, run_subtask_with_reveal_at};
use crate::types::{AbortFlag, DesignRequest, DocSink, LlmClient, Progress, SubtaskOutcome};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use op_editor_core::{EditorCommand, EditorState};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};

/// Clamps a raw concurrency value to the valid range `[1, 6]`.
///
/// This mirrors the store-side clamp in TS (store clamps to [1,6] before
/// writing to `request.concurrency`). The Rust crate clamps defensively on
/// the way in so callers need not worry about out-of-range values.
pub(crate) fn clamp_concurrency(v: u32) -> u32 {
    v.clamp(1, 6)
}

/// The effective worker count for the screen-group executor:
/// `min(clamp_concurrency(concurrency), group_count)`, or `1` when there is
/// at most one group — a single group has nothing to run alongside, so
/// `run.rs` takes the untouched sequential path (byte-identical regression
/// lock for single-screen plans, append mode, and any plan whose subtasks
/// all share one `screen` tag).
pub(crate) fn effective_concurrency(concurrency: u32, group_count: usize) -> u32 {
    if group_count > 1 {
        clamp_concurrency(concurrency).min(group_count as u32)
    } else {
        1
    }
}

// ── BufferDocSink ─────────────────────────────────────────────────────────────

/// A per-worker buffering [`DocSink`] that collects every applied
/// [`EditorCommand`] into an in-memory `Vec` without touching the real document.
///
/// **Design choice (spec §5 implementer note):** the buffer is still a plain
/// command collector; it does not re-execute worker commands locally. The
/// pre-spawn snapshot is read-only context for generation post-processing
/// such as design-variable colour binding. Snapshotting the state ONCE
/// before the concurrent phase (rather than after each buffered command)
/// means a later subtask in the SAME group does not see an EARLIER
/// subtask's buffered output — an inherited limitation from the
/// pre-`aca0d3a0` design, not a new one: cross-subtask "does the canvas
/// already have X" context was never live within one buffered worker, only
/// across the (still fully sequential) plan-order loop in the single-root
/// path.
///
/// After all workers finish the caller replays `commands` into the real
/// `DocSink` in a deterministic order (serialized replay).
pub(crate) struct BufferDocSink {
    /// Snapshot of `EditorState` taken before the concurrent phase.
    /// Returned by `state()` unchanged for read-only generation context.
    snapshot: EditorState,
    /// All `EditorCommand`s collected via `apply()` calls.
    pub commands: Vec<EditorCommand>,
    /// Tracks undo-batch nesting depth (for parity with `DocSink` contract).
    pub batch_depth: i32,
}

impl BufferDocSink {
    /// Create a new buffer sink from a pre-concurrent state snapshot.
    pub(crate) fn new(snapshot: EditorState) -> Self {
        Self {
            snapshot,
            commands: Vec::new(),
            batch_depth: 0,
        }
    }
}

impl DocSink for BufferDocSink {
    fn state(&self) -> &EditorState {
        &self.snapshot
    }

    /// Buffer the command for later replay; always returns `true`.
    ///
    /// We return `true` unconditionally — the real document is not touched
    /// here; the per-worker result is validated after replay.
    fn apply(&mut self, cmd: EditorCommand) -> bool {
        self.commands.push(cmd);
        true
    }

    fn begin_undo_batch(&mut self) {
        self.batch_depth += 1;
    }

    fn end_undo_batch(&mut self) {
        self.batch_depth -= 1;
    }
}

// ── Shared retry ladder ─────────────────────────────────────────────────────

/// Run the 3-attempt tier-gated retry ladder for ONE subtask — attempt 1 full
/// complexity, attempt 2 `reduced_complexity` iff `tier == ModelTier::Basic`,
/// attempt 3 `reduced_complexity + minimal_skills` — emitting `SubtaskStarted`
/// / `SubtaskRetry` progress events via `on_progress`, exactly as `run.rs`'s
/// sequential loop always has.
///
/// Extracted so [`run_screen_group_worker`] and the sequential path in
/// `run.rs` share the IDENTICAL retry semantics: parallelizing groups must
/// never let a group's retry behavior drift from the ladder the M3 quality
/// gate was measured against.
///
/// Does not decide `SubtaskDone` / `SubtaskFailed` emission or salvage
/// eligibility — the caller does that from the returned [`SubtaskOutcome`],
/// since that bookkeeping differs slightly between the sequential loop's
/// flat plan-index accounting and a worker's group-relative accounting.
///
/// `agent_indicator_epoch` should be `None` when `sink` is a [`BufferDocSink`]
/// (buffered inserts have not actually landed in the document yet — wiring
/// the canvas reveal/indicator overlay to them here would race the replay
/// that makes them real) and `Some(epoch)` for the sequential path, which
/// writes directly to the real sink.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_subtask_retry_ladder(
    subtask: &Subtask,
    plan: &OrchestratorPlan,
    request: &DesignRequest,
    llm: &dyn LlmClient,
    sink: &mut dyn DocSink,
    abort: &AbortFlag,
    tier: ModelTier,
    agent_indicator_epoch: Option<u64>,
    on_progress: &mut dyn FnMut(Progress),
) -> SubtaskOutcome {
    on_progress(Progress::SubtaskStarted {
        id: subtask.id.clone(),
        label: subtask.label.clone(),
    });

    // Attempt 1 — full complexity. Forwards the progress sink so the
    // per-subtask SkillLoadReport reaches the chat UI.
    let outcome1 = run_subtask_with_reveal_at(
        subtask,
        plan,
        request,
        llm,
        sink,
        abort,
        false,
        false,
        agent_indicator_epoch,
        reveal_now_millis(),
        Some(&mut *on_progress),
    )
    .await;

    // Evaluate the non-retryable predicate once from attempt-1's error
    // (faithful to the sequential path: computed before the retry chain and
    // reused for both the attempt-2 and attempt-3 guards).
    let non_retryable = outcome1
        .error
        .as_deref()
        .map(is_non_retryable)
        .unwrap_or(false);

    let retryable = |o: &SubtaskOutcome| {
        o.error.is_some() && o.node_count == 0 && !abort.is_set() && !non_retryable
    };

    // Attempt 2 — reduced_complexity iff Basic tier.
    let outcome2 = if retryable(&outcome1) {
        tracing::warn!(
            subtask = %subtask.id,
            error = outcome1.error.as_deref().unwrap_or(""),
            "subtask failed, retrying (attempt 2)"
        );
        on_progress(Progress::SubtaskRetry {
            id: subtask.id.clone(),
            attempt: 2,
            reason: outcome1
                .error
                .clone()
                .unwrap_or_else(|| "zero nodes generated".into()),
        });
        Some(
            run_subtask_with_reveal_at(
                subtask,
                plan,
                request,
                llm,
                sink,
                abort,
                tier == ModelTier::Basic,
                false,
                agent_indicator_epoch,
                reveal_now_millis(),
                None,
            )
            .await,
        )
    } else {
        None
    };

    let outcome_after2 = outcome2.as_ref().unwrap_or(&outcome1);

    // Attempt 3 — minimal skills (last-ditch fallback).
    let outcome3 = if retryable(outcome_after2) {
        tracing::warn!(
            subtask = %subtask.id,
            error = outcome_after2.error.as_deref().unwrap_or(""),
            "subtask still empty after retry, falling back to minimal skills (attempt 3)"
        );
        on_progress(Progress::SubtaskRetry {
            id: subtask.id.clone(),
            attempt: 3,
            reason: outcome_after2
                .error
                .clone()
                .unwrap_or_else(|| "zero nodes generated".into()),
        });
        Some(
            run_subtask_with_reveal_at(
                subtask,
                plan,
                request,
                llm,
                sink,
                abort,
                true,
                true,
                agent_indicator_epoch,
                reveal_now_millis(),
                None,
            )
            .await,
        )
    } else {
        None
    };

    outcome3.unwrap_or_else(|| outcome2.unwrap_or(outcome1))
}

// ── Screen-group worker + concurrent executor ───────────────────────────────

/// One screen-group worker's collected results: its private command buffer
/// (replayed into the real sink after every worker finishes) plus
/// `(plan_index, outcome, is_zero)` for each subtask it ran, in `indices`
/// order.
struct WorkerResult {
    buffer: BufferDocSink,
    outcomes: Vec<(usize, SubtaskOutcome, bool)>,
    /// `true` when `abort` fired before this worker reached the end of its
    /// group (some of the group's subtasks never ran).
    aborted: bool,
}

/// Runs one screen group's subtasks SEQUENTIALLY (via the SAME
/// [`run_subtask_retry_ladder`] the single-group path uses), writing into its
/// own private `buffer` — never the real document — so sibling workers
/// driven by [`join_all`] can run genuinely concurrently with no shared
/// `&mut` state: each worker future owns its buffer, moved in and returned in
/// [`WorkerResult`].
///
/// A shared `semaphore` caps the number of subtask LLM calls in flight
/// across ALL groups at once (RAII permit drop releases the slot when a
/// call completes).
#[allow(clippy::too_many_arguments)]
async fn run_screen_group_worker(
    group: &ScreenGroup,
    plan: &OrchestratorPlan,
    request: &DesignRequest,
    llm: &dyn LlmClient,
    abort: &AbortFlag,
    tier: ModelTier,
    mut buffer: BufferDocSink,
    semaphore: Arc<Semaphore>,
    progress_tx: mpsc::UnboundedSender<Progress>,
) -> WorkerResult {
    let mut outcomes = Vec::with_capacity(group.indices.len());
    let mut aborted = false;
    for &idx in &group.indices {
        if abort.is_set() {
            aborted = true;
            break;
        }
        let subtask = &plan.subtasks[idx];

        // Acquire a permit before the LLM call; RAII drop releases it when
        // this iteration's block ends (= the sequential ladder's own await
        // chain completing for this subtask).
        let _permit = semaphore
            .acquire()
            .await
            .expect("semaphore should not be closed");

        let ptx = progress_tx.clone();
        let mut emit = move |p: Progress| {
            let _ = ptx.send(p);
        };
        // `agent_indicator_epoch: None` — see `run_subtask_retry_ladder`'s
        // doc: this writes into `buffer`, not the real document yet.
        let outcome = run_subtask_retry_ladder(
            subtask,
            plan,
            request,
            llm,
            &mut buffer,
            abort,
            tier,
            None,
            &mut emit,
        )
        .await;

        let is_zero = outcome.node_count == 0;
        let _ = progress_tx.send(if is_zero {
            Progress::SubtaskFailed {
                id: subtask.id.clone(),
                error: outcome.error.clone().unwrap_or_default(),
            }
        } else {
            Progress::SubtaskDone {
                id: subtask.id.clone(),
                node_count: outcome.node_count,
            }
        });
        outcomes.push((idx, outcome, is_zero));
    }
    WorkerResult {
        buffer,
        outcomes,
        aborted,
    }
}

/// Result of the concurrent screen-group phase, in the same shape `run.rs`'s
/// sequential loop produces locally — so the downstream salvage /
/// zero-content / cleanup / `RunSummary` code is UNCHANGED regardless of
/// which phase ran.
pub(crate) struct ConcurrentPhaseResult {
    /// Every subtask's outcome that actually ran, in PLAN order — a subtask
    /// never reached because `abort` fired is simply absent, the same
    /// convention the sequential loop's early `break` already leaves.
    pub outcomes: Vec<SubtaskOutcome>,
    pub aborted_mid: bool,
    pub zero_node_failure: bool,
    /// `(plan_index, outcomes_index)` for every zero-node outcome — `run.rs`'s
    /// end-of-run salvage pass retries exactly these, unchanged.
    pub salvage: Vec<(usize, usize)>,
}

/// Drives one worker future per screen group on a [`FuturesUnordered`] —
/// genuine concurrency (LLM calls from different groups overlap in
/// wall-clock time) with no `tokio::spawn`/extra `Send` bounds and no shared
/// `&mut` (each worker owns its own [`BufferDocSink`]). A shared
/// [`Semaphore`] caps in-flight subtask LLM calls at `effective_concurrency`.
///
/// **Visibility (three-piece fix, 2026-07-17)** — the earlier `join_all`
/// version drove every worker to completion, drained `on_progress` in one
/// batch AFTER all of them finished, then replayed every buffer in
/// ascending-plan-index order. That made a concurrent run LOOK sequential:
/// the progress checklist sat frozen until the very end, then every root
/// popped in at once. This version instead runs a `tokio::select!` loop
/// polling the progress channel and the `FuturesUnordered` TOGETHER:
/// - Every `Progress` event reaches `on_progress` the moment its worker
///   sends it — mid-run, not after `join_all` resolves.
/// - Each group's buffered commands replay into the REAL `sink` the INSTANT
///   that group's worker finishes — whichever group wins the LLM race first,
///   not plan order. This is safe because every group's root was already
///   scaffolded (fixed id + position) before this phase started (see
///   `run_screen_groups::insert_screen_group_roots`), so replaying group B's
///   content before group A's never collides — they're disjoint subtrees.
///
/// The REAL indicator epoch is applied only at replay time — see
/// `run_subtask_retry_ladder`'s doc for why buffered/worker-phase inserts
/// never touch `agent_indicators`. Per-group visual identity (distinct
/// colour/name per root) is NOT decided here — the caller already tagged
/// each group's scaffold root via `agent_indicators::add_frame` before this
/// phase runs, and the canvas paint's ownership walk inherits that tag down
/// into whatever this function replays, however it interleaves.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_screen_groups_concurrent(
    groups: &[ScreenGroup],
    plan: &OrchestratorPlan,
    request: &DesignRequest,
    llm: &dyn LlmClient,
    sink: &mut dyn DocSink,
    abort: &AbortFlag,
    tier: ModelTier,
    effective_concurrency: u32,
    agent_indicator_epoch: Option<u64>,
    on_progress: &mut dyn FnMut(Progress),
) -> ConcurrentPhaseResult {
    // Item 4: a fact-line up front makes ⚡Nx's effect legible instead of
    // silent — before this, nothing distinguished a concurrent run from a
    // sequential one in the progress panel until content started landing.
    on_progress(Progress::ConcurrentGroupsStarted {
        group_count: groups.len(),
        workers: effective_concurrency,
    });

    let snapshot = sink.state().clone();
    let semaphore = Arc::new(Semaphore::new(effective_concurrency as usize));
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<Progress>();

    // Build one worker future per screen group, each future OWNS its own
    // `BufferDocSink` (moved in) — no shared `&mut`. Tagged with its plan
    // group index so the caller knows which group finished.
    let mut worker_futures: FuturesUnordered<_> = groups
        .iter()
        .enumerate()
        .map(|(g_idx, group)| {
            let fut = run_screen_group_worker(
                group,
                plan,
                request,
                llm,
                abort,
                tier,
                BufferDocSink::new(snapshot.clone()),
                Arc::clone(&semaphore),
                progress_tx.clone(),
            );
            async move { (g_idx, fut.await) }
        })
        .collect();
    // Drop the master sender so the channel closes once every worker (and
    // thus every clone) has finished — `progress_rx.recv()` then observes
    // `None` instead of hanging forever.
    drop(progress_tx);

    let mut per_subtask: Vec<Option<(SubtaskOutcome, bool)>> = vec![None; plan.subtasks.len()];
    let mut aborted_mid = abort.is_set();
    let mut remaining = groups.len();
    // Poll the progress channel and the worker set TOGETHER so a progress
    // event reaches `on_progress` as soon as it's sent, and a finished
    // group replays into `sink` as soon as it completes — never held back
    // waiting for its siblings.
    while remaining > 0 {
        tokio::select! {
            Some(event) = progress_rx.recv() => {
                on_progress(event);
            }
            Some((_g_idx, result)) = worker_futures.next() => {
                remaining -= 1;
                aborted_mid |= result.aborted;
                for cmd in result.buffer.commands {
                    apply_command_with_reveal(sink, cmd, agent_indicator_epoch, reveal_now_millis());
                }
                for (plan_idx, outcome, is_zero) in result.outcomes {
                    per_subtask[plan_idx] = Some((outcome, is_zero));
                }
            }
        }
    }
    // A progress event and the LAST worker's completion can both be ready
    // in the same poll — `select!` only picks one branch per iteration, so
    // drain whatever's left in the (now-closing) channel after the loop.
    while let Ok(event) = progress_rx.try_recv() {
        on_progress(event);
    }

    let mut outcomes = Vec::new();
    let mut salvage = Vec::new();
    let mut zero_node_failure = false;
    for (plan_idx, slot) in per_subtask.into_iter().enumerate() {
        if let Some((outcome, is_zero)) = slot {
            outcomes.push(outcome);
            if is_zero {
                zero_node_failure = true;
                salvage.push((plan_idx, outcomes.len() - 1));
            }
        }
    }

    ConcurrentPhaseResult {
        outcomes,
        aborted_mid,
        zero_node_failure,
        salvage,
    }
}

#[cfg(test)]
#[path = "concurrent_tests.rs"]
mod tests;
