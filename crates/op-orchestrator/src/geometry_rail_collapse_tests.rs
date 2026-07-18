//! Tests for [`collect_rail_width_collapse_fixes`] / [`fix_rail_width_collapse`] —
//! the geometry-driven repair for a card rail whose `fill_container` siblings
//! collapse beside a fixed-width reference card. Root-cause fixture is a
//! de-identified extraction of a real user report (a finance-dashboard
//! "Savings Goals" rail on a 375px mobile page, produced by the
//! `minimal_skills` last-ditch retry rung): card 1 declared `width: 200`,
//! cards 2 and 3 declared `width: "fill_container"` — on the 327px-inner
//! rail left after page padding, card 1 alone ate 200px + 24px of gaps,
//! leaving only ~103px to split between the two `fill_container` cards
//! (~51px each), well past the point their own icon tile + title + amount
//! content could fit — hence truncated titles ("New Car" → "Nev Car") and a
//! ballooned card height from all that forced text wrapping.

use super::*;
use serde_json::json;
use std::collections::HashMap;

fn rects(entries: &[(&str, f64, f64, f64, f64)]) -> HashMap<String, Rect> {
    entries
        .iter()
        .map(|(id, x, y, w, h)| {
            (
                (*id).to_string(),
                Rect {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                },
            )
        })
        .collect()
}

fn find_by_name<'a>(v: &'a serde_json::Value, name: &str) -> Option<&'a serde_json::Value> {
    if v.get("name").and_then(|x| x.as_str()) == Some(name) {
        return Some(v);
    }
    v.get("children")
        .and_then(|c| c.as_array())
        .into_iter()
        .flatten()
        .find_map(|c| find_by_name(c, name))
}

fn find_id_by_name(v: &serde_json::Value, name: &str) -> Option<String> {
    find_by_name(v, name)
        .and_then(|n| n.get("id"))
        .and_then(|x| x.as_str())
        .map(String::from)
}

fn resolved_widths_by_name(
    state: &op_editor_core::EditorState,
    rects: &HashMap<String, Rect>,
    names: &[&str],
) -> Vec<f64> {
    let v = serde_json::to_value(state.active_children()[0].clone()).unwrap();
    names
        .iter()
        .map(|name| {
            let id = find_id_by_name(&v, name).unwrap_or_else(|| panic!("{name} exists"));
            rects
                .get(&id)
                .unwrap_or_else(|| panic!("{name} resolved"))
                .w
        })
        .collect()
}

fn rail(cards: Vec<serde_json::Value>) -> serde_json::Value {
    json!({
        "type": "frame", "id": "rail", "name": "Rail", "layout": "horizontal",
        "width": "fill_container", "gap": 12, "children": cards
    })
}

fn card(id: &str, width: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "frame", "id": id, "name": id, "layout": "vertical",
        "width": width, "height": "fit_content", "cornerRadius": 12,
        "fill": [{"type": "solid", "color": "#FFFFFF"}], "children": []
    })
}

#[test]
fn collapsed_fill_container_sibling_is_widened_to_match_fixed_reference() {
    let row = rail(vec![
        card("c1", json!(200)),
        card("c2", json!("fill_container")),
        card("c3", json!("fill_container")),
    ]);
    // Matches the real fixture's arithmetic: c1 keeps its declared 200, c2/c3
    // squeeze to ~51px in a 327px-inner rail (200 + 2×12 gap + 2×51 ≈ 327).
    let rects = rects(&[
        ("c1", 0.0, 0.0, 200.0, 140.0),
        ("c2", 212.0, 0.0, 51.0, 640.0),
        ("c3", 275.0, 0.0, 51.0, 640.0),
    ]);
    let mut cmds = Vec::new();

    collect_rail_width_collapse_fixes(&row, &rects, &mut cmds);

    assert_eq!(
        cmds.len(),
        2,
        "both squeezed siblings must be widened: {cmds:?}"
    );
    for cmd in &cmds {
        match cmd {
            EditorCommand::UpdateNode { node_id, width, .. } => {
                assert!(
                    node_id.as_str() == "c2" || node_id.as_str() == "c3",
                    "only the collapsed siblings are touched, not the reference: {node_id:?}"
                );
                assert_eq!(
                    *width,
                    Some(200),
                    "widened to the reference card's declared width"
                );
            }
            other => panic!("expected UpdateNode (numeric width), got {other:?}"),
        }
    }
}

#[test]
fn evenly_split_rail_is_left_untouched() {
    // A healthy 3-up equal-share rail (no fixed reference at all): every
    // sibling gets a comfortable resolved width, nothing collapsed.
    let row = rail(vec![
        card("c1", json!("fill_container")),
        card("c2", json!("fill_container")),
        card("c3", json!("fill_container")),
    ]);
    let rects = rects(&[
        ("c1", 0.0, 0.0, 101.0, 140.0),
        ("c2", 113.0, 0.0, 101.0, 140.0),
        ("c3", 226.0, 0.0, 101.0, 140.0),
    ]);
    let mut cmds = Vec::new();

    collect_rail_width_collapse_fixes(&row, &rects, &mut cmds);

    assert!(
        cmds.is_empty(),
        "no fixed reference to normalize against — an equal-share rail is not this failure mode: {cmds:?}"
    );
}

#[test]
fn mild_width_variance_below_ratio_and_floor_is_not_flagged() {
    // A fixed reference IS present, but the fill_container sibling still
    // resolves to a comfortable width — normal design variety, not collapse.
    let row = rail(vec![
        card("c1", json!(160)),
        card("c2", json!("fill_container")),
    ]);
    let rects = rects(&[
        ("c1", 0.0, 0.0, 160.0, 140.0),
        ("c2", 172.0, 0.0, 140.0, 140.0),
    ]);
    let mut cmds = Vec::new();

    collect_rail_width_collapse_fixes(&row, &rects, &mut cmds);

    assert!(
        cmds.is_empty(),
        "140px is a perfectly readable card width, not a collapse: {cmds:?}"
    );
}

#[test]
fn all_fixed_width_row_is_left_untouched() {
    // Every card in the row already carries its own explicit fixed width —
    // deliberately different sizes (e.g. a wide "featured" card beside
    // smaller ones), not a fill_container squeeze.
    let row = rail(vec![
        card("c1", json!(200)),
        card("c2", json!(80)),
        card("c3", json!(80)),
    ]);
    let rects = rects(&[
        ("c1", 0.0, 0.0, 200.0, 140.0),
        ("c2", 212.0, 0.0, 80.0, 140.0),
        ("c3", 304.0, 0.0, 80.0, 140.0),
    ]);
    let mut cmds = Vec::new();

    collect_rail_width_collapse_fixes(&row, &rects, &mut cmds);

    assert!(
        cmds.is_empty(),
        "a deliberately smaller FIXED card is left alone, only fill_container is a candidate: {cmds:?}"
    );
}

#[test]
fn real_layout_widens_collapsed_savings_goals_rail_end_to_end() {
    use crate::test_support::VecDocSink;
    use crate::types::DocSink;
    use jian_ops_schema::node::PenNode;
    use op_editor_core::PenNodeExt;

    // De-identified extraction of the real user .op's "Savings Goals Rail"
    // subtree: a 375px mobile page, [0,24] section padding (327px inner
    // width), a horizontal rail of 3 cards where only the first carries a
    // fixed width. Enough child structure (icon tile + title + amount row)
    // to reproduce genuine overshoot on the squeezed cards, matching what
    // drove `collect_card_overflow_clips` to clip them in production.
    let goal_card = |id: &str, name: &str, width: serde_json::Value| {
        json!({
            "type": "frame", "id": id, "name": name, "layout": "vertical",
            "width": width, "height": "fit_content", "gap": 16, "padding": 20,
            "cornerRadius": 12, "fill": [{"type": "solid", "color": "#FFFFFF"}],
            "children": [
                {"type": "frame", "id": format!("{id}-top"), "name": "Top", "layout": "horizontal",
                 "width": "fill_container", "gap": 10, "alignItems": "center", "children": [
                    {"type": "frame", "id": format!("{id}-icon"), "name": "IconTile",
                     "width": 36, "height": 36, "children": []},
                    {"type": "text", "id": format!("{id}-title"), "name": "Title",
                     "content": name, "fontSize": 14, "width": "fill_container",
                     "textGrowth": "fixed-width", "children": []}
                 ]},
                {"type": "frame", "id": format!("{id}-amounts"), "name": "Amounts", "layout": "horizontal",
                 "width": "fill_container", "justifyContent": "space_between", "children": [
                    {"type": "text", "id": format!("{id}-saved"), "name": "Saved",
                     "content": "$3,150", "fontSize": 16, "width": "fit_content", "children": []},
                    {"type": "text", "id": format!("{id}-pct"), "name": "Pct",
                     "content": "17%", "fontSize": 12, "width": "fit_content", "children": []}
                 ]}
            ]
        })
    };

    let root: PenNode = serde_json::from_value(json!({
        "type": "frame", "id": "page", "name": "Finance Overview", "width": 375,
        "height": "fit_content", "layout": "vertical", "children": [
            {"type": "frame", "id": "section", "name": "Savings Goals Rail", "layout": "vertical",
             "width": "fill_container", "height": "fit_content", "gap": 12, "padding": [0, 24],
             "children": [
                {"type": "frame", "id": "rail", "name": "Rail", "layout": "horizontal",
                 "width": "fill_container", "gap": 12, "children": [
                    goal_card("c1", "Emergency Fund", json!(200)),
                    goal_card("c2", "New Car", json!("fill_container")),
                    goal_card("c3", "Vacation", json!("fill_container"))
                 ]}
             ]}
        ]
    }))
    .expect("valid root");

    let mut sink = VecDocSink::new();
    sink.apply(EditorCommand::InsertSubtree {
        nodes: vec![root],
        parent_id: NodeId::NONE,
        page_id: None,
    });
    let root_id = sink.state().active_children()[0].id_str().to_string();

    let before_rects = resolved_rects(sink.state());
    let before = resolved_widths_by_name(
        sink.state(),
        &before_rects,
        &["Emergency Fund", "New Car", "Vacation"],
    );
    assert!(
        before[0] - before[1] > 100.0,
        "fixture must reproduce the real collapse before repair, got {before:?}"
    );

    assert!(
        fix_rail_width_collapse(&mut sink, &root_id),
        "the collapsed siblings must be widened"
    );

    let v = serde_json::to_value(sink.state().active_children()[0].clone()).unwrap();
    for name in ["New Car", "Vacation"] {
        let node = find_by_name(&v, name).unwrap_or_else(|| panic!("{name} exists"));
        assert_eq!(
            node.get("width").and_then(|w| w.as_f64()),
            Some(200.0),
            "{name} must now declare the reference card's fixed width"
        );
    }

    let after_rects = resolved_rects(sink.state());
    let after = resolved_widths_by_name(
        sink.state(),
        &after_rects,
        &["Emergency Fund", "New Car", "Vacation"],
    );
    let max = after.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min = after.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        max - min <= 1.0,
        "all three cards resolve to the same width after repair, got {after:?}"
    );
}
