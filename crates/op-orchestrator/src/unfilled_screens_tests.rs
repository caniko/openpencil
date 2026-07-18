use super::*;
use crate::test_support::VecDocSink;
use crate::types::DocSink;
use serde_json::json;

fn state_from(children: serde_json::Value) -> EditorState {
    let doc: jian_ops_schema::PenDocument = serde_json::from_value(json!({
        "version": "1.0",
        "children": children,
    }))
    .expect("doc");
    EditorState::from_document(doc)
}

/// The exact auto-provided status-bar chrome shape — a status-bar-role
/// frame carrying real text (the clock) that must NOT, on its own, count
/// as "the model filled this screen".
fn status_bar_chrome(id_suffix: &str) -> serde_json::Value {
    json!({
        "type": "frame", "id": format!("status-{id_suffix}"), "name": "Status Bar",
        "role": "status-bar", "width": "fill_container", "height": 44,
        "children": [
            { "type": "text", "id": format!("clock-{id_suffix}"), "name": "Time", "content": "9:41" }
        ]
    })
}

fn mobile_screen(
    id: &str,
    name: &str,
    extra_children: Vec<serde_json::Value>,
) -> serde_json::Value {
    let mut children = vec![status_bar_chrome(id)];
    children.extend(extra_children);
    json!({
        "type": "frame", "id": id, "name": name, "layout": "vertical",
        "width": 390, "height": 844, "children": children
    })
}

#[test]
fn a_screen_with_only_the_status_bar_is_unfilled() {
    let state = state_from(json!([mobile_screen("root-a", "Saved", vec![])]));
    let hits = detect_unfilled_screens(&state);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].node_id, "root-a");
    assert_eq!(hits[0].name, "Saved");
}

#[test]
fn a_screen_with_zero_children_at_all_is_unfilled() {
    let state = state_from(json!([{
        "type": "frame", "id": "root-a", "name": "Blank",
        "width": 390, "height": 844, "children": []
    }]));
    let hits = detect_unfilled_screens(&state);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "Blank");
}

#[test]
fn a_screen_with_real_content_is_not_flagged() {
    let state = state_from(json!([mobile_screen(
        "root-a",
        "Home",
        vec![json!({
            "type": "frame", "id": "body", "name": "Body", "layout": "vertical",
            "width": "fill_container", "height": "fit_content", "children": [
                { "type": "text", "id": "greeting", "content": "Welcome back" }
            ]
        })]
    )]));
    assert!(detect_unfilled_screens(&state).is_empty());
}

/// Sparse but REAL content (a background + one heading, nothing else)
/// must never be false-flagged just for being small — the whole reason
/// a node-count threshold was rejected in favor of a content check.
#[test]
fn sparse_but_real_content_is_not_falsely_flagged() {
    let state = state_from(json!([mobile_screen(
        "root-a",
        "Saved",
        vec![json!({ "type": "text", "id": "only-label", "content": "No saved items yet" })]
    )]));
    assert!(
        detect_unfilled_screens(&state).is_empty(),
        "one real text node is enough to count as filled, however sparse the screen otherwise is"
    );
}

/// An icon with a real glyph counts as real content too (an icon-led
/// empty state is still authored content, not a shell).
#[test]
fn an_icon_with_a_real_glyph_counts_as_real_content() {
    let state = state_from(json!([mobile_screen(
        "root-a",
        "Saved",
        vec![json!({ "type": "icon_font", "id": "empty-icon", "iconFontName": "bookmark" })]
    )]));
    assert!(detect_unfilled_screens(&state).is_empty());
}

/// A screen-shaped root with no chrome at all and only decorative,
/// content-free shapes (a background rectangle) is still unfilled.
#[test]
fn decorative_only_content_does_not_count_as_filled() {
    let state = state_from(json!([mobile_screen(
        "root-a",
        "Saved",
        vec![json!({ "type": "rectangle", "id": "bg", "width": 390, "height": 400 })]
    )]));
    assert_eq!(
        detect_unfilled_screens(&state).len(),
        1,
        "a bare decorative rectangle is not evidence the model did anything here"
    );
}

/// A desktop-shaped root (not mobile-sized, no explicit `screen` tag) is
/// never a "promised screen" candidate, even if it's empty — dashboards
/// legitimately split sidebar + content into separate top-level roots.
#[test]
fn a_desktop_shaped_empty_root_is_not_a_screen_candidate() {
    let state = state_from(json!([{
        "type": "frame", "id": "root-a", "name": "Sidebar",
        "width": 240, "height": 900, "children": []
    }]));
    assert!(detect_unfilled_screens(&state).is_empty());
}

/// A `screen`-tagged root that is mobile-shaped AND empty is still a
/// candidate — the tag doesn't disqualify a genuine mobile screen, it's just
/// no longer the thing that QUALIFIES one (see the module doc's "screen
/// candidates" section for why the tag alone is untrustworthy).
#[test]
fn a_screen_tagged_mobile_shaped_root_is_still_a_candidate_via_shape() {
    let state = state_from(json!([{
        "type": "frame", "id": "root-a", "name": "Checkout",
        "screen": "/checkout", "width": 390, "height": 844, "children": []
    }]));
    let hits = detect_unfilled_screens(&state);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "Checkout");
}

/// A screen-shaped root that is BOTH `screen`-tagged AND has real
/// content is not flagged.
#[test]
fn a_filled_explicitly_tagged_screen_is_not_flagged() {
    let state = state_from(json!([{
        "type": "frame", "id": "root-a", "name": "Checkout", "screen": "/checkout",
        "width": 390, "height": 844, "children": [
            { "type": "text", "id": "t", "content": "Order summary" }
        ]
    }]));
    assert!(detect_unfilled_screens(&state).is_empty());
}

/// Regression (found live in `pump_runs_loop_finalize_backstop_against_live_state`,
/// op-host-desktop): `wire_screen_navigation` (run inside
/// `cleanup::finalize_design`, which both `apply_loop_finalize` and the
/// classic path's cleanup stage run BEFORE this detector ever sees the
/// document) auto-tags EVERY top-level frame with a `screen` path once a
/// document has 2+ top-level roots — including a 1200×64 navbar `Header`
/// fragment and a blank 1200×800 desktop canvas, neither of which is a
/// "promised screen" in any meaningful sense. A `screen` tag alone must
/// never make a non-mobile-shaped frame a candidate.
#[test]
fn a_screen_tagged_but_non_mobile_shaped_root_is_never_a_candidate() {
    let state = state_from(json!([
        { "type": "frame", "id": "root-a", "name": "Frame", "screen": "/", "width": 1200, "height": 800, "children": [] },
        { "type": "frame", "id": "root-b", "name": "Header", "role": "navbar", "screen": "/header", "width": 1200, "height": 64, "children": [] },
    ]));
    assert!(
        detect_unfilled_screens(&state).is_empty(),
        "a nav-routing `screen` tag on a desktop-shaped frame must not, by itself, flag it as an unfilled screen"
    );
}

/// `list_screen_candidates` returns every committed screen — filled or not
/// — used to build the "you committed N screens (A/B/C)" contract line the
/// in-session fill-round nudge states.
#[test]
fn list_screen_candidates_includes_both_filled_and_unfilled_screens() {
    let state = state_from(json!([
        mobile_screen(
            "root-a",
            "Trips",
            vec![json!({"type":"text","id":"t1","content":"hi"})]
        ),
        mobile_screen("root-b", "Destination", vec![]),
        mobile_screen("root-c", "Saved", vec![]),
    ]));
    let candidates = list_screen_candidates(&state);
    let names: Vec<&str> = candidates.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["Trips", "Destination", "Saved"]);
}

/// A desktop-shaped / non-screen-shaped root is never a candidate here
/// either — `list_screen_candidates` shares `looks_like_a_screen` with
/// `detect_unfilled_screens`, so the same shape rule applies to both.
#[test]
fn list_screen_candidates_excludes_non_mobile_shaped_roots() {
    let state = state_from(json!([{
        "type": "frame", "id": "root-a", "name": "Sidebar",
        "width": 240, "height": 900, "children": []
    }]));
    assert!(list_screen_candidates(&state).is_empty());
}

/// Multiple unfilled screens are all reported, in document order.
#[test]
fn multiple_unfilled_screens_are_all_reported() {
    let state = state_from(json!([
        mobile_screen(
            "root-a",
            "Home",
            vec![json!({"type":"text","id":"t1","content":"hi"})]
        ),
        mobile_screen("root-b", "Saved", vec![]),
        mobile_screen("root-c", "Profile", vec![]),
    ]));
    let hits = detect_unfilled_screens(&state);
    let names: Vec<&str> = hits.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["Saved", "Profile"]);
}

#[test]
fn mark_unfilled_screens_appends_the_suffix_once() {
    let mut sink = VecDocSink {
        state: state_from(json!([mobile_screen("root-a", "Saved", vec![])])),
        applied: Vec::new(),
        batch_depth: 0,
    };
    let hits = detect_unfilled_screens(sink.state());
    mark_unfilled_screens(&mut sink, &hits);
    let node = op_editor_core::walkers::find_node(
        sink.state().active_children(),
        &op_editor_core::NodeId::new("root-a"),
    )
    .expect("root still present");
    assert_eq!(node.base().name.as_deref(), Some("Saved (unfilled)"));

    // Idempotent: marking an already-marked screen again must not
    // double the suffix.
    let hits_again = detect_unfilled_screens(sink.state());
    mark_unfilled_screens(&mut sink, &hits_again);
    let node = op_editor_core::walkers::find_node(
        sink.state().active_children(),
        &op_editor_core::NodeId::new("root-a"),
    )
    .expect("root still present");
    assert_eq!(node.base().name.as_deref(), Some("Saved (unfilled)"));
}

#[test]
fn finalize_and_mark_unfilled_screens_returns_names_and_marks_the_canvas() {
    let mut state = state_from(json!([mobile_screen("root-a", "Saved", vec![])]));
    let names = finalize_and_mark_unfilled_screens(&mut state);
    assert_eq!(names, vec!["Saved".to_string()]);
    let node = op_editor_core::walkers::find_node(
        state.active_children(),
        &op_editor_core::NodeId::new("root-a"),
    )
    .expect("root still present");
    assert_eq!(node.base().name.as_deref(), Some("Saved (unfilled)"));
}

#[test]
fn finalize_and_mark_is_a_true_no_op_when_nothing_is_unfilled() {
    let mut state = state_from(json!([mobile_screen(
        "root-a",
        "Home",
        vec![json!({"type":"text","id":"t","content":"hi"})]
    )]));
    let before = serde_json::to_value(&state.doc).expect("serialize before");
    let names = finalize_and_mark_unfilled_screens(&mut state);
    assert!(names.is_empty());
    let after = serde_json::to_value(&state.doc).expect("serialize after");
    assert_eq!(
        after, before,
        "must not touch the document when nothing is unfilled"
    );
}
