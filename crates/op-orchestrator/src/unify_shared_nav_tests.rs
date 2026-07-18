//! Tests for the cross-screen shared-nav unification pass.

use super::*;
use crate::wire_screen_navigation::node_has_events;
use jian_ops_schema::PenDocument;
use op_editor_core::EditorState;

pub(super) const ACTIVE: &str = "#22C55E";
pub(super) const INACTIVE: &str = "#9CA3AF";

fn tab_json(id_prefix: &str, label: &str, icon: &str, active: bool) -> serde_json::Value {
    let color = if active { ACTIVE } else { INACTIVE };
    let mut children = vec![
        serde_json::json!({
            "type": "icon_font", "id": format!("{id_prefix}-icon"), "iconFontName": icon,
            "width": 24, "height": 24, "fill": [{"type":"solid","color": color}]
        }),
        serde_json::json!({
            "type": "text", "id": format!("{id_prefix}-lbl"), "content": label, "fontSize": 12,
            "fill": [{"type":"solid","color": color}]
        }),
    ];
    if active {
        children.push(serde_json::json!({
            "type": "rectangle", "id": format!("{id_prefix}-indicator"),
            "width": 20, "height": 2, "fill": [{"type":"solid","color": color}]
        }));
    }
    serde_json::json!({
        "type": "frame", "id": format!("{id_prefix}-item"), "width": 80, "height": 56,
        "layout": "vertical", "children": children
    })
}

/// A bottom-nav with 4 tabs (Home/Search/Library/Premium), whichever one's
/// label equals `active_label` styled active (accent fill + underline
/// indicator), everyone else the shared muted style.
pub(super) fn nav_json(
    nav_id: &str,
    id_prefix: &str,
    active_label: &str,
    library_label: &str,
) -> serde_json::Value {
    let tabs = [
        ("Home", "home"),
        ("Search", "search"),
        (library_label, "library"),
        ("Premium", "star"),
    ];
    let children: Vec<serde_json::Value> = tabs
        .iter()
        .map(|(label, icon)| {
            tab_json(
                &format!("{id_prefix}-{icon}"),
                label,
                icon,
                *label == active_label,
            )
        })
        .collect();
    serde_json::json!({
        "type": "frame", "id": nav_id, "name": "Bottom Nav", "role": "bottom-tab-bar",
        "width": "fill_container", "height": 72, "layout": "horizontal", "children": children
    })
}

/// Same 4-tab (Home/Search/Library/Premium) nav, but with EXACT CONTROL over
/// which tab indices carry the active style — for exercising
/// `find_active_by_fingerprint`'s degrade paths (zero outliers / 2+
/// outliers), which `nav_json`'s "exactly one active tab" shape can't reach.
pub(super) fn nav_with_active_indices(
    nav_id: &str,
    id_prefix: &str,
    active_indices: &[usize],
) -> serde_json::Value {
    let tabs = [
        ("Home", "home"),
        ("Search", "search"),
        ("Library", "library"),
        ("Premium", "star"),
    ];
    let children: Vec<serde_json::Value> = tabs
        .iter()
        .enumerate()
        .map(|(i, (label, icon))| {
            tab_json(
                &format!("{id_prefix}-{icon}"),
                label,
                icon,
                active_indices.contains(&i),
            )
        })
        .collect();
    serde_json::json!({
        "type": "frame", "id": nav_id, "name": "Bottom Nav", "role": "bottom-tab-bar",
        "width": "fill_container", "height": 72, "layout": "horizontal", "children": children
    })
}

pub(super) fn screen_json(id: &str, name: &str, nav: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "type": "frame", "id": id, "name": name, "width": 390, "height": 844,
        "layout": "vertical", "children": [nav]
    })
}

/// A screen-shaped top-level frame with NO nav container at all — the
/// "sub-agent's Bottom Navigation Bar subtask failed" shape the injection
/// upgrade targets. Carries one ordinary content child so it isn't an
/// empty frame.
pub(super) fn screen_json_no_nav(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "frame", "id": id, "name": name, "width": 390, "height": 844,
        "layout": "vertical", "children": [
            { "type": "text", "id": format!("{id}-body"), "content": "Body", "fontSize": 16 }
        ]
    })
}

pub(super) fn two_screen_drifted_doc() -> serde_json::Value {
    serde_json::json!({
        "version": "1.0",
        "children": [
            screen_json("home", "Home", nav_json("nav-home", "h", "Home", "Library")),
            // The bug scenario: Library screen's OWN redraw uses "Your
            // Library" instead of "Library", and marks IT active.
            screen_json("library", "Library", nav_json("nav-library", "l", "Your Library", "Your Library")),
        ]
    })
}

pub(super) fn state_from(doc: serde_json::Value) -> EditorState {
    let doc: PenDocument = serde_json::from_value(doc).expect("valid doc");
    EditorState::from_document(doc)
}

pub(super) fn run_pass(state: &mut EditorState) {
    let mut sink = crate::loop_finalize::StateDocSink { state };
    unify_shared_nav(&mut sink);
}

pub(super) fn find_by_id<'a>(nodes: &'a [PenNode], id: &str) -> Option<&'a PenNode> {
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

pub(super) fn labels_of(nav: &PenNode) -> Vec<String> {
    nav.children()
        .into_iter()
        .flatten()
        .filter_map(first_text_content)
        .map(str::to_string)
        .collect()
}

/// The `fill` color string on the FIRST descendant that carries one — used
/// to read back whether a tab landed active (`#22C55E`) or inactive
/// (`#9CA3AF`) after unification.
pub(super) fn first_fill(node: &PenNode) -> Option<String> {
    let value = serde_json::to_value(node).ok()?;
    if let Some(fill) = value.get("fill").and_then(|f| f.as_array()) {
        if let Some(color) = fill
            .first()
            .and_then(|f| f.get("color"))
            .and_then(|c| c.as_str())
        {
            return Some(color.to_string());
        }
    }
    for child in node.children().into_iter().flatten() {
        if let Some(found) = first_fill(child) {
            return Some(found);
        }
    }
    None
}

fn collect_all_ids<'a>(nodes: &'a [PenNode], out: &mut Vec<&'a str>) {
    for node in nodes {
        out.push(node.id_str());
        if let Some(children) = node.children() {
            collect_all_ids(children, out);
        }
    }
}

#[test]
fn drifted_library_nav_adopts_reference_labels_and_correct_active_tab() {
    let mut state = state_from(two_screen_drifted_doc());
    run_pass(&mut state);

    let home_root = find_by_id(state.active_children(), "home").unwrap();
    let home_nav = find_by_id(home_root.children().unwrap(), "nav-home").unwrap();
    let library_root = find_by_id(state.active_children(), "library").unwrap();
    // The reference nav's id itself doesn't survive the swap on the target
    // screen — the whole subtree gets remapped ids on ReplaceSubtree — so
    // locate the target's nav by role instead of a fixed id.
    let mut library_navs = Vec::new();
    collect_nav_containers(library_root, &mut library_navs);
    let library_nav = library_navs
        .first()
        .expect("library screen still has a nav");

    // Labels are now IDENTICAL across both screens — the drift is gone.
    assert_eq!(
        labels_of(home_nav),
        vec!["Home", "Search", "Library", "Premium"]
    );
    assert_eq!(
        labels_of(library_nav),
        vec!["Home", "Search", "Library", "Premium"]
    );

    // Active tab correctness: Home screen's OWN nav is untouched (Home
    // active); Library screen's UNIFIED nav has "Library" active, "Home"
    // reverted to inactive.
    let home_tabs = library_nav.children().unwrap();
    let home_tab_fill = first_fill(&home_tabs[0]).unwrap();
    let library_tab_fill = first_fill(&home_tabs[2]).unwrap();
    assert_eq!(
        home_tab_fill, INACTIVE,
        "Home tab must be inactive on the Library screen"
    );
    assert_eq!(
        library_tab_fill, ACTIVE,
        "Library tab must be active on the Library screen"
    );
    // The indicator (extra 3rd child) followed the active styling.
    assert_eq!(
        home_tabs[0].children().unwrap().len(),
        2,
        "inactive tab has no indicator"
    );
    assert_eq!(
        home_tabs[2].children().unwrap().len(),
        3,
        "active tab keeps the indicator"
    );
}

#[test]
fn reference_screen_nav_is_left_untouched() {
    // NB: not a byte-identical JSON comparison — `ReplaceSubtree` on the
    // Library screen's nav walks the WHOLE `active_children()` tree
    // searching for its target id (`replace_node_in_children` recurses
    // through every sibling along the way), and `children_mut()` probing a
    // childless `Rectangle`/`Frame` materializes an empty `children: []`
    // where none was authored (`PenNodeExt::children_mut`'s `get_or_insert_
    // with`) as a side effect of merely being visited — a PRE-EXISTING,
    // pass-unrelated quirk of that shared walker, not something
    // `unify_shared_nav` does semantically. `None` vs `Some(vec![])`
    // behave identically everywhere `children()` is read, so this asserts
    // the SEMANTICS (labels + which tab is active) instead.
    let mut state = state_from(two_screen_drifted_doc());
    run_pass(&mut state);
    let home_nav = find_by_id(
        find_by_id(state.active_children(), "home")
            .unwrap()
            .children()
            .unwrap(),
        "nav-home",
    )
    .unwrap();

    assert_eq!(
        labels_of(home_nav),
        vec!["Home", "Search", "Library", "Premium"]
    );
    let tabs = home_nav.children().unwrap();
    assert_eq!(
        first_fill(&tabs[0]).unwrap(),
        ACTIVE,
        "Home stays active on its own (reference) screen"
    );
    assert_eq!(
        tabs[0].children().unwrap().len(),
        3,
        "Home keeps its indicator"
    );
    assert_eq!(
        first_fill(&tabs[2]).unwrap(),
        INACTIVE,
        "Library tab stays inactive on the Home screen"
    );
}

#[test]
fn second_run_is_idempotent() {
    let mut state = state_from(two_screen_drifted_doc());
    run_pass(&mut state);
    let once = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let twice = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(
        once, twice,
        "running the pass twice must be a no-op the second time"
    );
}

#[test]
fn single_screen_doc_is_untouched() {
    let doc = serde_json::json!({
        "version": "1.0",
        "children": [screen_json("home", "Home", nav_json("nav-home", "h", "Home", "Library"))]
    });
    let mut state = state_from(doc);
    let before = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let after = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(before, after, "single-screen doc must be a no-op");
}

#[test]
fn no_screen_has_a_nav_is_a_whole_pass_no_op() {
    let doc = serde_json::json!({
        "version": "1.0",
        "children": [
            { "type": "frame", "id": "home", "name": "Home", "width": 390, "height": 844,
              "layout": "vertical", "children": [] },
            { "type": "frame", "id": "library", "name": "Library", "width": 390, "height": 844,
              "layout": "vertical", "children": [] },
        ]
    });
    let mut state = state_from(doc);
    let before = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let after = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(before, after);
}

#[test]
fn cloned_nav_ids_never_collide_with_existing_document_ids() {
    let mut state = state_from(two_screen_drifted_doc());
    run_pass(&mut state);

    let mut ids = Vec::new();
    collect_all_ids(state.active_children(), &mut ids);
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        ids.len(),
        "every id in the document must be unique: {ids:?}"
    );
}

/// End-to-end: unify runs before `wire_screen_navigation` in the real
/// cleanup pipeline — after BOTH passes, every screen's unified nav tabs
/// still bind `events.onTap`, proving the clone's tabs are exactly what
/// Track A expects to find (same shape it binds on any other screen).
#[test]
fn unified_navs_still_get_wired_by_wire_screen_navigation() {
    let mut state = state_from(two_screen_drifted_doc());
    {
        let mut sink = crate::loop_finalize::StateDocSink { state: &mut state };
        unify_shared_nav(&mut sink);
        crate::wire_screen_navigation::wire_screen_navigation(&mut sink);
    }

    // Only "Home" and "Library" tabs name-match an actual screen in this
    // 2-screen fixture — "Search"/"Premium" have no corresponding screen to
    // navigate to, so wire_screen_navigation correctly leaves THOSE unbound
    // (this fixture's minimalism, not a defect). The point under test is
    // that the CLONED tabs are shaped exactly like any other screen's own
    // tabs, so Track A binds them the same way on every screen — not just
    // the reference one.
    let home_root = find_by_id(state.active_children(), "home").unwrap();
    let library_root = find_by_id(state.active_children(), "library").unwrap();
    for root in [home_root, library_root] {
        let mut navs = Vec::new();
        collect_nav_containers(root, &mut navs);
        let nav = navs.first().expect("nav present");
        for tab in nav.children().unwrap() {
            let label = first_text_content(tab).unwrap_or("");
            if label != "Home" && label != "Library" {
                continue;
            }
            assert!(
                node_has_events(tab),
                "tab `{label}` must get bound by wire_screen_navigation on every screen: {tab:?}"
            );
        }
    }
}

/// Reference (Home) nav built with exact control over which tabs carry the
/// active style; Library's OWN (pre-replacement) nav is deliberately
/// drifted (`"Your Library"`) so the label-set idempotency gate doesn't
/// short-circuit the replace before the degrade path can be exercised.
fn two_screen_doc_with_reference_active(active_indices: &[usize]) -> serde_json::Value {
    serde_json::json!({
        "version": "1.0",
        "children": [
            screen_json("home", "Home", nav_with_active_indices("nav-home", "h", active_indices)),
            screen_json(
                "library",
                "Library",
                nav_json("nav-library", "l", "Your Library", "Your Library"),
            ),
        ]
    })
}

/// Degrade path 1: every tab shares the SAME fingerprint (no active styling
/// anywhere in the reference nav to detect or move) — the clone still
/// unifies labels, but retargeting is skipped entirely rather than guessing.
#[test]
fn no_active_signal_degrades_to_clone_without_retargeting() {
    let mut state = state_from(two_screen_doc_with_reference_active(&[]));
    run_pass(&mut state);

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    let mut navs = Vec::new();
    collect_nav_containers(library_root, &mut navs);
    let library_nav = navs.first().expect("library screen still has a nav");

    // Labels still unified — the primary goal survives the degrade.
    assert_eq!(
        labels_of(library_nav),
        vec!["Home", "Search", "Library", "Premium"]
    );

    // No tab was retargeted: every tab keeps the reference's own (uniformly
    // inactive) style, none gained an indicator, nothing panicked.
    for tab in library_nav.children().unwrap() {
        assert_eq!(first_fill(tab).unwrap(), INACTIVE);
        assert_eq!(
            tab.children().unwrap().len(),
            2,
            "no tab should have gained an indicator"
        );
    }
}

/// Degrade path 2: TWO tabs (Home, Library) diverge from the other two — no
/// single outlier, so which one is "really" active is genuinely ambiguous.
/// Retargeting is skipped rather than guessing wrong.
#[test]
fn ambiguous_active_signal_degrades_to_clone_without_retargeting() {
    let mut state = state_from(two_screen_doc_with_reference_active(&[0, 2]));
    run_pass(&mut state);

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    let mut navs = Vec::new();
    collect_nav_containers(library_root, &mut navs);
    let library_nav = navs.first().expect("library screen still has a nav");

    assert_eq!(
        labels_of(library_nav),
        vec!["Home", "Search", "Library", "Premium"]
    );

    // Positions untouched: Home (0) and Library (2) keep the reference's own
    // active styling verbatim; Search/Premium stay inactive. No swap ran.
    let tabs = library_nav.children().unwrap();
    assert_eq!(
        first_fill(&tabs[0]).unwrap(),
        ACTIVE,
        "no retargeting attempted"
    );
    assert_eq!(first_fill(&tabs[1]).unwrap(), INACTIVE);
    assert_eq!(
        first_fill(&tabs[2]).unwrap(),
        ACTIVE,
        "no retargeting attempted"
    );
    assert_eq!(first_fill(&tabs[3]).unwrap(), INACTIVE);
}

/// Home (reference, full nav, Home active), Library (NO nav, name matches
/// a reference tab), Detail (NO nav, name matches NO tab) — the exact shape
/// the injection upgrade targets: a sub-agent's bottom-nav subtask failed
/// on some screens while the reference screen's nav still declares tabs
/// for them.
pub(super) fn three_screen_missing_nav_doc() -> serde_json::Value {
    serde_json::json!({
        "version": "1.0",
        "children": [
            screen_json("home", "Home", nav_json("nav-home", "h", "Home", "Library")),
            screen_json_no_nav("library", "Library"),
            screen_json_no_nav("detail", "Detail"),
        ]
    })
}

#[test]
fn missing_nav_screen_with_matching_tab_gets_nav_injected_and_activated() {
    let mut state = state_from(three_screen_missing_nav_doc());
    run_pass(&mut state);

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    let mut navs = Vec::new();
    collect_nav_containers(library_root, &mut navs);
    let nav = navs.first().expect("nav injected onto the Library screen");

    assert_eq!(labels_of(nav), vec!["Home", "Search", "Library", "Premium"]);

    // Injected as the LAST child of the screen frame (bottom placement).
    let children = library_root.children().unwrap();
    assert_eq!(
        children.last().unwrap().id_str(),
        nav.id_str(),
        "injected nav must land as the screen's last child"
    );

    // Active tab correctness: Library tab active, Home reverted to inactive.
    let tabs = nav.children().unwrap();
    assert_eq!(
        first_fill(&tabs[0]).unwrap(),
        INACTIVE,
        "Home is inactive on the Library screen"
    );
    assert_eq!(
        first_fill(&tabs[2]).unwrap(),
        ACTIVE,
        "Library is active on its own screen"
    );
}

#[test]
fn missing_nav_screen_without_matching_tab_stays_untouched() {
    let mut state = state_from(three_screen_missing_nav_doc());
    run_pass(&mut state);

    let detail_root = find_by_id(state.active_children(), "detail").unwrap();
    let mut navs = Vec::new();
    collect_nav_containers(detail_root, &mut navs);
    assert!(
        navs.is_empty(),
        "Detail matches no reference tab, so no nav should be forced onto it"
    );
}

#[test]
fn injected_nav_is_idempotent_on_second_run() {
    let mut state = state_from(three_screen_missing_nav_doc());
    run_pass(&mut state);
    let once = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let twice = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(
        once, twice,
        "running the pass twice after injection must be a no-op the second time"
    );
}

/// End-to-end: an INJECTED nav (not just a replaced one) is shaped exactly
/// like any other screen's nav, so Track A binds its matching tabs the
/// same way.
#[test]
fn injected_nav_still_gets_wired_by_wire_screen_navigation() {
    let mut state = state_from(three_screen_missing_nav_doc());
    {
        let mut sink = crate::loop_finalize::StateDocSink { state: &mut state };
        unify_shared_nav(&mut sink);
        crate::wire_screen_navigation::wire_screen_navigation(&mut sink);
    }

    let library_root = find_by_id(state.active_children(), "library").unwrap();
    let mut navs = Vec::new();
    collect_nav_containers(library_root, &mut navs);
    let nav = navs.first().expect("nav present after injection");
    for tab in nav.children().unwrap() {
        let label = first_text_content(tab).unwrap_or("");
        if label != "Home" && label != "Library" {
            continue;
        }
        assert!(
            node_has_events(tab),
            "tab `{label}` must get bound after injection: {tab:?}"
        );
    }
}
