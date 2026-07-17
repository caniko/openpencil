//! Manual per-subtask retry — the manual layer of the failed-subtask
//! remediation feature (phase 2 of the m1 investigation report). A user
//! clicks "Retry" on a failed row in the progress panel; this module
//! re-runs EXACTLY that one persisted [`Subtask`] against the LIVE
//! document, ONCE, at full complexity.
//!
//! Deliberately NOT a 3-attempt ladder and NOT wired into `run.rs`'s
//! automatic salvage pass: the user is in the loop here — they clicked,
//! they'll see the single result, and THEY decide whether to click again,
//! switch provider, or fall back to the chat modify flow. Stacking the
//! automatic retry ladder underneath a manual click would silently
//! multiply LLM calls the user never asked for.
//!
//! Reuses [`crate::subagent::run_subtask_with_reveal_at`] — the SAME
//! generation unit every subtask (orchestrator-planned or
//! `spawn_agents`-spawned) runs through — so a retried subtask's Class-A
//! passes (theme detection, canvas-width role resolution, self-check)
//! behave identically to its original attempt.

use crate::plan::{OrchestratorPlan, PlanFill, RootFrameSpec, Subtask};
use crate::subagent::{reveal_now_millis, run_subtask_with_reveal_at};
use crate::types::{AbortFlag, DesignRequest, DocSink, LlmClient, Progress, SubtaskOutcome};
use op_editor_core::PenNodeExt;

/// Build a minimal [`OrchestratorPlan`] context from the LIVE document's
/// current root frame — the SAME technique
/// `crate::spawn_concurrent::plan_from_state` uses for the model's own
/// `spawn_agents` tool, duplicated here rather than shared: that helper
/// flattens a `Subtask` down through a `SpawnAgentSpec` (dropping
/// `region`/`elements`/`screen`), which is exactly the fidelity a faithful
/// retry must NOT lose. Re-deriving `root_frame` from the CURRENT document
/// (rather than trusting whatever the ORIGINAL plan's root frame said)
/// correctly reflects any reshaping `finalize_design`'s cleanup passes did
/// after the original run.
fn plan_for_retry(sink: &dyn DocSink, subtask: &Subtask) -> OrchestratorPlan {
    let (width, height, fill) = sink
        .state()
        .active_children()
        .first()
        .map(|n| {
            (
                n.width_px().unwrap_or(1200.0),
                n.height_px().unwrap_or(800.0),
                op_editor_core::fills::first_solid_fill_hex(n).map(|hex| {
                    vec![PlanFill {
                        kind: "solid".into(),
                        color: hex.to_string(),
                    }]
                }),
            )
        })
        .unwrap_or((1200.0, 800.0, None));

    OrchestratorPlan {
        root_frame: RootFrameSpec {
            id: "retry-root".into(),
            name: "Page".into(),
            width,
            height,
            layout: None,
            gap: None,
            padding: None,
            fill,
        },
        // Exactly the ONE persisted subtask being retried, byte-for-byte —
        // no re-flattening through a spec type. Its region/elements/screen/
        // id_prefix ride through unchanged.
        subtasks: vec![subtask.clone()],
        style_guide_name: None,
    }
}

/// Re-run exactly one persisted, previously-failed [`Subtask`] against the
/// live document, ONCE, at full complexity. Returns a [`SubtaskOutcome`] —
/// the SAME shape every other subtask attempt returns — so callers (the
/// host's progress-panel folding logic) need no new result type.
///
/// When `subtask.parent_frame_id` no longer resolves in the live document —
/// most likely because `finalize_design`'s cleanup passes replaced the root
/// subtree after the original run (`ReplaceSubtree` allocates a FRESH root
/// id; an ordinary insert can't target the old one) — this fails FAST with
/// a `node_count: 0` outcome naming the stale id, instead of guessing an
/// insertion point. The approved v1 scope is "tell the user the truth"; see
/// the TODO below for the deferred structural fix.
///
/// TODO(v2): once the concurrent Track-A screen-navigation work reliably
/// marks every top-level frame with `FrameNode.screen`, resolve a stale
/// `parent_frame_id` by matching `subtask.screen` against the CURRENT
/// top-level frames' `screen` markers instead of failing — deferred so this
/// feature doesn't couple to that in-flight effort.
pub async fn retry_subtask(
    subtask: &Subtask,
    request: &DesignRequest,
    llm: &dyn LlmClient,
    sink: &mut dyn DocSink,
    abort: &AbortFlag,
    indicator_epoch: Option<u64>,
    on_progress: Option<&mut dyn FnMut(Progress)>,
) -> SubtaskOutcome {
    if let Some(parent_id) = &subtask.parent_frame_id {
        let resolves = op_editor_core::walkers::find_node(
            sink.state().active_children(),
            &op_editor_core::NodeId::new(parent_id.clone()),
        )
        .is_some();
        if !resolves {
            return SubtaskOutcome {
                id: subtask.id.clone(),
                node_count: 0,
                error: Some(format!(
                    "this section's original location (frame \"{parent_id}\") no longer exists \
                     in the document — describe where to add it instead"
                )),
                inserted_root_ids: Vec::new(),
                subtask: None,
            };
        }
    }
    let plan = plan_for_retry(sink, subtask);
    run_subtask_with_reveal_at(
        &plan.subtasks[0],
        &plan,
        request,
        llm,
        sink,
        abort,
        false,
        false,
        indicator_epoch,
        reveal_now_millis(),
        on_progress,
    )
    .await
}

#[cfg(test)]
#[path = "retry_subtask_tests.rs"]
mod tests;
