//! Unit tests for `chat_session_launch` — sibling file per the
//! 800-line-per-file ceiling (mirrors `chat_session_launch_selection_tests.rs`).

use super::*;
use op_editor_core::pen_node_ext::PenNodeExt;

fn frame(
    id: &str,
    name: &str,
    children: Vec<jian_ops_schema::node::PenNode>,
) -> jian_ops_schema::node::PenNode {
    let mut node: jian_ops_schema::node::PenNode = serde_json::from_value(serde_json::json!({
        "type": "frame",
        "id": id,
        "name": name,
        "width": 390,
        "height": 120,
        "children": []
    }))
    .expect("frame fixture");
    if let Some(kids) = node.children_mut() {
        *kids = children;
    }
    node
}

#[test]
fn clear_fresh_starter_frame_bumps_document_revision() {
    let mut state = EditorState::new();
    // Install the exact blank starter frame the design classifier
    // recognizes (id "n10", name "Frame", 1200x800, white fill).
    let starter: jian_ops_schema::node::PenNode = serde_json::from_value(serde_json::json!({
        "type": "frame",
        "id": "n10",
        "name": "Frame",
        "x": 0,
        "y": 0,
        "width": 1200,
        "height": 800,
        "fill": [{ "type": "solid", "color": "#ffffff" }],
        "children": []
    }))
    .expect("starter frame fixture");
    state.active_children_mut().clear();
    state.active_children_mut().push(starter);
    let revision_before = state.document_revision();

    assert!(
        clear_fresh_starter_frame_for_design(&mut state),
        "the blank starter frame must be recognized and cleared"
    );
    assert!(
        state.active_children().is_empty(),
        "the starter Frame row must be gone after the clear"
    );
    // Regression: the raw `active_children_mut().clear()` must bump the
    // revision, or the layer-panel row cache (keyed on
    // `document_revision()`) keeps painting the deleted "Frame" row.
    assert_ne!(
        state.document_revision(),
        revision_before,
        "clearing the starter frame must advance document_revision"
    );
}

#[test]
fn stash_design_request_for_retry_writes_json_onto_the_last_message() {
    let mut host = WidgetHostNative::new();
    host.editor_state_mut()
        .chat
        .messages
        .push(op_editor_core::ChatMessage::assistant_streaming());
    let request = op_orchestrator::DesignRequest {
        prompt: "design a login page".into(),
        model: None,
        provider: None,
        design_md: None,
        concurrency: 1,
        append_context: None,
        validation_enabled: true,
        visual_ref_enabled: false,
    };

    stash_design_request_for_retry(&mut host, &request);

    let json = host
        .editor_state()
        .chat
        .messages
        .last()
        .unwrap()
        .design_request_json_for_retry
        .clone()
        .expect("request must be persisted");
    let restored: op_orchestrator::DesignRequest =
        serde_json::from_str(&json).expect("must round-trip");
    assert_eq!(restored.prompt, "design a login page");
}

/// Regression lock for a real bug a user hit: the CLI-standard route
/// (`launch_cli_standard_turn`, reached whenever no builtin/ACP model is
/// selected — the common case) never stashed `design_request_json_for_retry`
/// onto the turn's bubble, so the manual "Retry" button always failed with
/// "nothing to retry" even though the row's icon painted correctly. Only
/// the synchronous portion of the launch is observable here without either
/// mocking provider construction or spawning a real CLI subprocess — that's
/// exactly where the stash must happen (before the worker thread moves the
/// request away), so it's exactly what this test needs to prove.
#[test]
fn launch_if_pending_stashes_the_design_request_on_the_cli_standard_route() {
    // `launch_cli_standard_turn` (reached below) unconditionally calls
    // `agent_indicators::begin()` on THIS thread before it ever spawns its
    // worker (chat_session_launch.rs's indicator_epoch setup) — that's a
    // write to the same process-global registry every other design-turn
    // test in this binary guards with this lock. Without it, this test's
    // `begin()` can land between another (locked) test's own `begin()` and
    // its first `add_frame`, silently bumping `active_epoch()` out from
    // under it and starving that test's frame-populated assertion — see
    // `chat_intent_host_tests::cli_new_design_populates_frame_indicators_the_canvas_scan_gates_on`.
    let _guard = crate::agent_indicator_test_lock::LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    op_editor_core::agent_indicators::clear();

    let mut host = WidgetHostNative::new();
    // No builtin/ACP model selected (the default) — `is_builtin_or_acp` is
    // false, so `launch_if_pending` routes into `launch_cli_standard_turn`
    // regardless of what the prompt says (that route does its OWN async
    // classification on the worker thread).
    host.editor_state_mut()
        .chat
        .set_input_text("design a login page");
    assert!(host.editor_state_mut().chat.begin_send());
    let mut current_chat = None;
    let mut current_design = None;

    let launched = launch_if_pending(&mut host, &mut current_chat, &mut current_design);

    assert!(
        launched,
        "the default CLI-backed provider must launch a turn"
    );
    let msg = host
        .editor_state()
        .chat
        .messages
        .last()
        .expect("begin_send pushed the assistant bubble");
    let json = msg
        .design_request_json_for_retry
        .as_deref()
        .expect("the CLI-standard route must stash the request too");
    let restored: op_orchestrator::DesignRequest =
        serde_json::from_str(json).expect("must round-trip");
    assert_eq!(restored.prompt, "design a login page");

    op_editor_core::agent_indicators::clear();
}

#[test]
fn builtin_design_keyword_with_existing_target_prefers_modify_route() {
    let mut state = EditorState::new();
    state.active_children_mut().clear();
    state.active_children_mut().push(frame(
        "screen",
        "Food App Home",
        vec![frame("popular-card", "Bella Napoli Pizzeria", Vec::new())],
    ));
    state.set_single_selection(op_editor_core::NodeId::new("popular-card"));

    assert!(
            should_launch_direct_modify(&state, "修改成饺子"),
            "selected existing design + modify wording should update in place, not start a new orchestrator design"
        );
}
