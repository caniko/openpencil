use super::*;
use serde_json::json;

/// A narrow sidebar row whose `fill_container` name block can shrink below
/// its `fit_content` text child (jian taffy `min-size: 0` default) — the
/// SAME overflow shape `real_layout_wraps_text_overflowing_a_constrained_block`
/// (`geometry_validation_tests.rs`) already proves the real jian layout
/// produces. `name_content` is the knob: a long name overflows into the
/// sibling time column, a short one doesn't.
fn sidebar_row(root_id: &str, name_content: &str) -> serde_json::Value {
    json!({
        "type": "frame", "id": root_id, "name": "Sidebar", "layout": "vertical",
        "width": 220, "height": "fit_content", "children": [
            { "type": "frame", "id": format!("{root_id}-row"), "name": "Row",
              "layout": "horizontal", "gap": 8, "width": "fill_container",
              "height": "fit_content", "children": [
                { "type": "frame", "id": format!("{root_id}-nameblock"), "name": "NameBlock",
                  "layout": "vertical", "width": "fill_container", "height": "fit_content",
                  "children": [
                    { "type": "text", "id": format!("{root_id}-name"), "name": "Name",
                      "content": name_content, "fontSize": 15 }
                  ] },
                { "type": "frame", "id": format!("{root_id}-timeblock"), "name": "TimeBlock",
                  "layout": "vertical", "width": "fit_content", "height": "fit_content",
                  "children": [
                    { "type": "text", "id": format!("{root_id}-time"), "name": "Time",
                      "content": "9:00 AM", "fontSize": 13 }
                  ] }
              ] }
        ]
    })
}

/// `root-a`'s long name overflows into its sibling time column; `root-b`'s
/// short name fits — simulating two DIFFERENT subtasks' own inserted
/// subtrees sharing one document, only one of which is actually broken.
fn two_root_doc() -> op_editor_core::EditorState {
    let doc: jian_ops_schema::PenDocument = serde_json::from_value(json!({
        "version": "1.0",
        "children": [
            sidebar_row("root-a", "Alexander Wellington Montgomery"),
            sidebar_row("root-b", "Bob"),
        ]
    }))
    .expect("doc");
    op_editor_core::EditorState::from_document(doc)
}

#[test]
fn reports_only_the_given_root_ids_violation() {
    let state = two_root_doc();
    let issues = geometry_diagnostics_for_roots(&state, &["root-a".to_string()]);
    assert!(
        issues
            .iter()
            .any(|i| i.contains("Name (") && i.contains("overflows into siblings")),
        "root-a's own overflow must be reported: {issues:?}"
    );
}

#[test]
fn a_clean_root_reports_nothing_even_though_a_sibling_is_broken() {
    let state = two_root_doc();
    let issues = geometry_diagnostics_for_roots(&state, &["root-b".to_string()]);
    assert!(
        issues.is_empty(),
        "root-b is geometrically fine on its own — a sibling's violation is not this \
         subtask's fault: {issues:?}"
    );
}

#[test]
fn multiple_root_ids_are_all_scanned() {
    let doc: jian_ops_schema::PenDocument = serde_json::from_value(json!({
        "version": "1.0",
        "children": [
            sidebar_row("root-a", "Alexander Wellington Montgomery"),
            sidebar_row("root-c", "Christopher Fitzgerald Huntington"),
        ]
    }))
    .expect("doc");
    let state = op_editor_core::EditorState::from_document(doc);
    // A subtask that produced MULTIPLE top-level roots must have every one
    // of them scanned, not just the first.
    let issues =
        geometry_diagnostics_for_roots(&state, &["root-a".to_string(), "root-c".to_string()]);
    let overflow_roots: std::collections::HashSet<&str> = issues
        .iter()
        .filter_map(|i| {
            if i.starts_with("Name (root-a") {
                Some("root-a")
            } else if i.starts_with("Name (root-c") {
                Some("root-c")
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        overflow_roots,
        std::collections::HashSet::from(["root-a", "root-c"]),
        "both roots' overflow must surface: {issues:?}"
    );
}

#[test]
fn a_root_id_that_does_not_resolve_is_skipped_without_panicking() {
    let state = two_root_doc();
    let issues = geometry_diagnostics_for_roots(&state, &["not-a-real-id".to_string()]);
    assert!(issues.is_empty());
}

#[test]
fn empty_root_ids_reports_nothing() {
    let state = two_root_doc();
    let issues = geometry_diagnostics_for_roots(&state, &[]);
    assert!(issues.is_empty());
}
