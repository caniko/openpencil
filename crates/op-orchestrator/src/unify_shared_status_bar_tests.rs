//! Tests for the cross-screen shared-status-bar unification pass.

use super::*;
use jian_ops_schema::PenDocument;
use op_editor_core::EditorState;

fn status_bar_json(id: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "frame", "id": id, "name": "Status Bar",
        "width": "fill_container", "height": 44, "layout": "horizontal",
        "children": [
            { "type": "text", "id": format!("{id}-time"), "content": "9:41", "fontSize": 14 }
        ]
    })
}

fn body_json(id: &str) -> serde_json::Value {
    serde_json::json!({ "type": "text", "id": format!("{id}-body"), "content": "Body", "fontSize": 16 })
}

/// A screen-shaped (390×844 — within `unfilled_screens`'s mobile band)
/// top-level frame with the given children in order.
fn screen_json(id: &str, name: &str, children: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "type": "frame", "id": id, "name": name, "width": 390, "height": 844,
        "layout": "vertical", "children": children
    })
}

fn state_from(doc: serde_json::Value) -> EditorState {
    let doc: PenDocument = serde_json::from_value(doc).expect("valid doc");
    EditorState::from_document(doc)
}

fn run_pass(state: &mut EditorState) {
    let mut sink = crate::loop_finalize::StateDocSink { state };
    unify_shared_status_bar(&mut sink);
}

fn find_by_id<'a>(nodes: &'a [PenNode], id: &str) -> Option<&'a PenNode> {
    for node in nodes {
        if node.id_str() == id {
            return Some(node);
        }
        if let Some(children) = node.children() {
            if let Some(found) = find_by_id(children, id) {
                return Some(found);
            }
        }
    }
    None
}

fn two_screen_doc(home_has_status_bar: bool, library_has_status_bar: bool) -> serde_json::Value {
    let mut home_children = Vec::new();
    if home_has_status_bar {
        home_children.push(status_bar_json("home-status"));
    }
    home_children.push(body_json("home"));

    let mut library_children = Vec::new();
    if library_has_status_bar {
        library_children.push(status_bar_json("library-status"));
    }
    library_children.push(body_json("library"));

    serde_json::json!({
        "version": "1.0",
        "children": [
            screen_json("home", "Home", home_children),
            screen_json("library", "Library", library_children),
        ]
    })
}

#[test]
fn missing_status_bar_screen_gets_reference_injected_as_first_child() {
    let mut state = state_from(two_screen_doc(true, false));
    run_pass(&mut state);

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    let children = library_root.children().unwrap();
    assert!(
        is_status_bar(&children[0]),
        "cloned status bar must land as the FIRST child: {children:?}"
    );
    assert_eq!(children.len(), 2, "body child is still present");
}

#[test]
fn screen_with_own_status_bar_is_left_untouched() {
    let mut state = state_from(two_screen_doc(true, true));
    let before = serde_json::to_string(
        find_by_id(state.active_children(), "library")
            .unwrap()
            .children()
            .unwrap(),
    )
    .unwrap();
    run_pass(&mut state);
    let after = serde_json::to_string(
        find_by_id(state.active_children(), "library")
            .unwrap()
            .children()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        before, after,
        "a screen that already carries its own status bar must be untouched"
    );
}

#[test]
fn no_reference_status_bar_is_a_whole_pass_no_op() {
    let mut state = state_from(two_screen_doc(false, false));
    let before = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let after = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(
        before, after,
        "no screen has a status bar to clone from — must be a total no-op"
    );
}

#[test]
fn single_screen_doc_is_untouched() {
    let doc = serde_json::json!({
        "version": "1.0",
        "children": [screen_json("home", "Home", vec![status_bar_json("home-status"), body_json("home")])]
    });
    let mut state = state_from(doc);
    let before = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let after = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(before, after, "single-screen doc must be a no-op");
}

#[test]
fn second_run_is_idempotent() {
    let mut state = state_from(two_screen_doc(true, false));
    run_pass(&mut state);
    let once = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let twice = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(
        once, twice,
        "running the pass twice must be a no-op the second time"
    );
}

/// 0718-1-k3-1 real-structure regression: three screens (Trips/Destination/
/// Saved), only Trips authored a status bar — after the pass all three do.
#[test]
fn three_screen_k3_shape_regression_all_screens_end_up_with_a_status_bar() {
    let doc = serde_json::json!({
        "version": "1.0",
        "children": [
            screen_json("trips", "Trips", vec![status_bar_json("trips-status"), body_json("trips")]),
            screen_json("destination", "Destination", vec![body_json("destination")]),
            screen_json("saved", "Saved", vec![body_json("saved")]),
        ]
    });
    let mut state = state_from(doc);
    run_pass(&mut state);

    for id in ["trips", "destination", "saved"] {
        let root = find_by_id(state.active_children(), id).unwrap();
        let children = root.children().unwrap();
        assert!(
            children.iter().any(is_status_bar),
            "screen `{id}` must carry a status bar after unification"
        );
    }
    // Destination/Saved got theirs as the FIRST child specifically.
    for id in ["destination", "saved"] {
        let root = find_by_id(state.active_children(), id).unwrap();
        assert!(
            is_status_bar(&root.children().unwrap()[0]),
            "screen `{id}`'s injected status bar must be the first child"
        );
    }
}

// ---------------------------------------------------------------------
// Role-stamping (0718-1-k3-1 review fix). `status_bar_json` above is
// deliberately roleless (matches the common authored shape) — reference
// detection here is name-based, but `unfilled_screens.rs`'s chrome
// exclusion is role-based. An unstamped clone's own text child ("9:41")
// would read as real content, silently flipping a genuinely unfilled
// screen to "filled" the moment it gains a status bar.
// ---------------------------------------------------------------------

#[test]
fn injected_status_bar_clone_gets_the_chrome_role_stamped() {
    let mut state = state_from(two_screen_doc(true, false));
    run_pass(&mut state);

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    let injected = library_root
        .children()
        .unwrap()
        .iter()
        .find(|c| is_status_bar(c))
        .expect("status bar injected");
    assert_eq!(
        injected.base().role.as_deref(),
        Some("status-bar"),
        "an injected clone must carry role:\"status-bar\" even though the \
         authored reference has none, so unfilled_screens' CHROME_ROLES \
         check recognizes it as chrome, not model-authored content"
    );
}

#[test]
fn authored_reference_status_bar_role_is_never_touched() {
    let mut state = state_from(two_screen_doc(true, false));
    run_pass(&mut state);

    let home_root = find_by_id(state.active_children(), "home").unwrap();
    let reference = find_by_id(home_root.children().unwrap(), "home-status").unwrap();
    assert_eq!(
        reference.base().role,
        None,
        "role-stamping must only ever touch the CLONE — the authored \
         reference screen's own status bar must be left exactly as authored"
    );
}

/// The regression this fix exists for: a genuinely unfilled screen (zero
/// real content) that gains ONLY a status bar must still read as unfilled
/// afterward — the injected chrome's own text ("9:41") must not count as
/// "the model did something here".
#[test]
fn screen_that_only_gained_a_status_bar_still_reads_as_unfilled() {
    let doc = serde_json::json!({
        "version": "1.0",
        "children": [
            screen_json("home", "Home", vec![status_bar_json("home-status"), body_json("home")]),
            screen_json("library", "Library", Vec::<serde_json::Value>::new()),
        ]
    });
    let mut state = state_from(doc);
    run_pass(&mut state);

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    assert!(
        library_root.children().unwrap().iter().any(is_status_bar),
        "setup: Library must have gained the injected status bar"
    );

    let unfilled = crate::unfilled_screens::detect_unfilled_screens(&state);
    assert!(
        unfilled.iter().any(|hit| hit.name == "Library"),
        "Library gained ONLY chrome (a status bar) — it must still be \
         reported unfilled, not silently flipped to filled: {unfilled:?}"
    );
}
