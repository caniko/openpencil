//! `concurrent` unit tests — `BufferDocSink` (the buffering sink shared by
//! the `spawn_agents` fan-out and the screen-group executor) and
//! `effective_concurrency`'s gating logic. End-to-end coverage of the
//! screen-group executor itself (`run_screen_groups_concurrent`, the retry
//! ladder, failure isolation, progress completeness) lives in
//! `run_tests_screen_groups.rs` — it needs a full `Orchestrator::run()` to
//! exercise real per-root content routing.

use super::*;
use op_editor_core::EditorCommand;

// ── effective_concurrency ────────────────────────────────────────────────

#[test]
fn single_group_always_forces_sequential() {
    // At most one group — nothing to run alongside — regardless of how high
    // `request.concurrency` (⚡Nx) is set.
    assert_eq!(effective_concurrency(1, 0), 1);
    assert_eq!(effective_concurrency(1, 1), 1);
    assert_eq!(effective_concurrency(6, 1), 1);
}

#[test]
fn multi_group_is_capped_by_both_clamp_and_group_count() {
    // 3 groups, concurrency=6 (max) → capped to 3 (no point over-provisioning
    // permits beyond the number of groups that could ever use them).
    assert_eq!(effective_concurrency(6, 3), 3);
    // 5 groups, concurrency=2 → stays 2 (the [1,6] clamp is the binding
    // constraint here, not the group count).
    assert_eq!(effective_concurrency(2, 5), 2);
    // concurrency=0 clamps to 1 first, then is unaffected by group count.
    assert_eq!(effective_concurrency(0, 3), 1);
    // concurrency=99 clamps to 6, then capped to the group count.
    assert_eq!(effective_concurrency(99, 4), 4);
}

/// `BufferDocSink` collects commands without modifying a real doc.
#[test]
fn buffer_doc_sink_collects_commands() {
    let mut sink = BufferDocSink::new(EditorState::new());
    let applied = sink.apply(EditorCommand::InsertSubtree {
        nodes: vec![],
        parent_id: op_editor_core::NodeId::NONE,
        page_id: None,
    });
    assert!(applied, "BufferDocSink.apply must always return true");
    assert_eq!(sink.commands.len(), 1);
}

/// `state()` on `BufferDocSink` returns the snapshot passed at construction.
#[test]
fn buffer_doc_sink_state_returns_snapshot() {
    let state = EditorState::new();
    let sink = BufferDocSink::new(state.clone());
    let _ = sink.state();
}

/// `BufferDocSink` tracks undo-batch depth correctly.
#[test]
fn buffer_doc_sink_undo_batch_depth() {
    let mut sink = BufferDocSink::new(EditorState::new());
    assert_eq!(sink.batch_depth, 0);
    sink.begin_undo_batch();
    assert_eq!(sink.batch_depth, 1);
    sink.end_undo_batch();
    assert_eq!(sink.batch_depth, 0);
}
