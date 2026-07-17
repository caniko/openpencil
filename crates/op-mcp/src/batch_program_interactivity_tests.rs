//! Track B of the interactive-preview plan: `screen` (on `FrameNode`) and
//! `events` (on Frame/Group/Rectangle) are ordinary flattened `PenNode`
//! fields — `parse_node_json`'s `serde_json::from_value::<PenNode>` has no
//! field whitelist, so anything script-gen's compiled `I(parent, json)`
//! line hands it lands on the node unfiltered. This exercises the exact
//! `execute_insert` path a script-gen program compiles down to (see
//! `op_mcp::script_runner::eval_to_program`'s `__record` -> `I(parent,
//! json)` line format) and confirms both fields survive to the inserted
//! node with the model-authored contract's exact wire shape — the
//! quote-literal navigate body (`"\"/detail\""`) included.

use jian_ops_schema::node::PenNode;

use super::*;

#[test]
fn i_call_passes_screen_and_events_through_unfiltered() {
    let mut state = sample();
    let program = r##"home=I(null, {"type":"frame","name":"Home","width":390,"height":844,"screen":"/","events":{"onTap":[{"push":"\"/detail\""}]}})"##;

    let (envelope, cmd) = call_operations(&state, program);
    assert!(envelope.get("errors").is_none(), "{envelope}");
    let home_id = binding_id(&envelope, "home");

    assert!(state.apply(cmd.expect("insert must emit a command")));
    let children = state.active_children();
    let home =
        op_editor_core::walkers::find_node(children, &NodeId::new(&home_id)).expect("home node");
    let PenNode::Frame(home_frame) = home else {
        panic!("expected a frame node, got {home:?}");
    };
    assert_eq!(
        home_frame.screen.as_deref(),
        Some("/"),
        "screen marker must pass through I() unfiltered"
    );
    let events_json = serde_json::to_value(home).unwrap()["events"].clone();
    assert_eq!(
        events_json,
        serde_json::json!({ "onTap": [ { "push": "\"/detail\"" } ] }),
        "events.onTap must pass through I() with the exact quote-literal navigate body: {events_json}"
    );
}
