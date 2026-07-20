//! Safe assembly of terminal codegen plan results.

use serde_json::{json, Map, Value};

use super::{chunk_id, lock_plans, now_ms, topo_sort, DependencyState};

/// Assemble results whose complete dependency chain remains usable.
///
/// A fully successful plan is consumed. Retryable partial/degraded results are
/// returned without consuming the plan, so callers may repair failed chunks
/// and assemble again. Explicitly skipped-only partials are terminal.
pub(crate) fn assemble_plan(plan_id: &str, _framework: &str) -> Result<Value, String> {
    let mut plans = lock_plans();
    let (out, retain_plan) = {
        let state = plans
            .get_mut(plan_id)
            .ok_or_else(|| format!("Plan {plan_id} not found"))?;
        state.last_activity_ms = now_ms();
        let sorted = topo_sort(&state.chunks);
        let dependencies = DependencyState::resolve(&state.chunks, &state.statuses);
        let pending: Vec<&str> = sorted
            .iter()
            .filter_map(|chunk| chunk_id(chunk).ok())
            .filter(|id| {
                let local = state
                    .statuses
                    .get(*id)
                    .map(String::as_str)
                    .unwrap_or("pending");
                dependencies.effective_status(id, local) == "pending"
            })
            .collect();
        if !pending.is_empty() {
            return Err(format!(
                "Plan {plan_id} is incomplete; pending chunks: {}. Submit each ready chunk before assembling. The plan remains available.",
                pending.join(", ")
            ));
        }

        let mut chunks_out = Vec::new();
        let mut contracts = Vec::new();
        let mut dependency_graph = Map::new();
        let mut chunk_statuses = Vec::new();
        let mut omitted_chunks = Vec::new();
        let mut retryable = false;

        for chunk in sorted {
            let id = chunk_id(chunk).unwrap_or("?");
            let local_status = state
                .statuses
                .get(id)
                .map(String::as_str)
                .unwrap_or("pending");
            let status = dependencies.effective_status(id, local_status);
            let chunk_retryable = dependencies.is_retryable(id, local_status, &state.statuses);
            retryable |= chunk_retryable;
            let result = state.results.get(id);
            let has_code = result
                .and_then(|value| value.get("code"))
                .and_then(Value::as_str)
                .is_some_and(|code| !code.trim().is_empty());
            let usable = matches!(status, "done" | "degraded") && has_code;

            if usable {
                let result = result.expect("usable result exists");
                chunks_out.push(result.clone());
                contracts.push(result.get("contract").cloned().unwrap_or(Value::Null));
            } else {
                let reason = omission_reason(status, result, dependencies.blockers(id));
                let mut omission = Map::new();
                omission.insert("chunkId".into(), json!(id));
                omission.insert("status".into(), json!(status));
                omission.insert("reason".into(), json!(reason));
                omission.insert("retryable".into(), json!(chunk_retryable));
                if status == "blocked" {
                    omission.insert("blockedBy".into(), json!(dependencies.blockers(id)));
                    if local_status != "pending" {
                        omission.insert("submittedStatus".into(), json!(local_status));
                    }
                }
                omitted_chunks.push(Value::Object(omission));
            }

            let mut status_entry = Map::new();
            status_entry.insert("chunkId".into(), json!(id));
            status_entry.insert("status".into(), json!(status));
            status_entry.insert(
                "attempts".into(),
                json!(state.attempts.get(id).copied().unwrap_or(0)),
            );
            status_entry.insert("retryable".into(), json!(chunk_retryable));
            if status == "blocked" {
                status_entry.insert("blockedBy".into(), json!(dependencies.blockers(id)));
                if local_status != "pending" {
                    status_entry.insert("submittedStatus".into(), json!(local_status));
                }
            }
            chunk_statuses.push(Value::Object(status_entry));
            dependency_graph.insert(
                id.to_string(),
                chunk.get("dependencies").cloned().unwrap_or(json!([])),
            );
        }

        if chunks_out.is_empty() {
            let omitted = omitted_chunks
                .iter()
                .filter_map(|entry| entry.get("chunkId").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Plan {plan_id} has no usable chunk code (failed/skipped/blocked: {omitted}). Resubmit failed dependencies before assembling. The plan remains available."
            ));
        }

        let partial = !omitted_chunks.is_empty();
        let degraded = chunk_statuses.iter().any(|entry| entry["status"] != "done");
        (
            json!({
                "chunks": chunks_out,
                "contracts": contracts,
                "dependencyGraph": dependency_graph,
                "chunkStatuses": chunk_statuses,
                "omittedChunks": omitted_chunks,
                "partial": partial,
                "degraded": degraded,
                "retryable": retryable,
                "planRetained": retryable,
            }),
            retryable,
        )
    };

    if !retain_plan {
        plans.remove(plan_id);
    }
    Ok(out)
}

fn omission_reason(status: &str, result: Option<&Value>, blockers: &[String]) -> String {
    if status == "blocked" {
        return format!(
            "blocked by failed/skipped dependencies: {}",
            blockers.join(", ")
        );
    }
    result
        .and_then(|value| value.get("error"))
        .and_then(Value::as_str)
        .filter(|message| !message.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| match status {
            "failed" => "chunk generation failed".into(),
            "skipped" => "chunk was skipped".into(),
            _ => "chunk produced no usable code".into(),
        })
}
