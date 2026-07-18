use super::*;
use serde_json::json;

/// A ring shaped like the wrapper this module targets: an unambiguous
/// track/progress pair plus one centre-content child, all direct children
/// of a `frame`. Children start in authoring order (track, progress,
/// centre) — canonical paint order is centre, progress, track — so tests
/// also exercise the reordering half of the fix.
fn ring(width: f64, height: f64, extra: Option<Value>) -> Value {
    let mut node = json!({
        "type": "frame",
        "id": "ring",
        "width": width,
        "height": height,
        "layout": "vertical",
        "children": [
            {"type": "ellipse", "id": "track", "width": 56, "height": 56, "innerRadius": 0.82},
            {"type": "ellipse", "id": "progress", "width": 56, "height": 56,
             "innerRadius": 0.82, "startAngle": -90, "sweepAngle": 230},
            {"type": "frame", "id": "centre", "width": 32, "height": 18,
             "children": [{"type": "text", "id": "pct", "content": "64%"}]}
        ]
    });
    if let Some(extra) = extra {
        for (key, value) in extra.as_object().expect("extra must be an object") {
            node[key] = value.clone();
        }
    }
    node
}

fn child<'a>(v: &'a Value, id: &str) -> &'a Value {
    v["children"]
        .as_array()
        .expect("children")
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("missing child {id}"))
}

#[test]
fn centres_a_non_square_wrapper_tier_one_declines_on_aspect_alone() {
    let mut node = ring(240.0, 56.0, None);
    assert!(is_still_off_center(&node), "precondition: not yet centred");

    assert!(force_concentric_radial_stack(&mut node));

    assert_eq!(node["layout"], json!("none"));
    let (track, progress) = (child(&node, "track"), child(&node, "progress"));
    assert_eq!(
        (track["x"].as_f64(), track["y"].as_f64()),
        (Some(92.0), Some(0.0)),
        "56-wide arc centred in a 240-wide box sits at (240-56)/2 = 92"
    );
    assert_eq!(
        (track["x"].as_f64(), track["y"].as_f64()),
        (progress["x"].as_f64(), progress["y"].as_f64()),
        "track and progress are the same size, so they share one point"
    );
    assert!(!is_still_off_center(&node), "must be safe after the fix");
}

#[test]
fn ignores_padding_because_jian_positions_absolute_children_from_the_border_box() {
    let mut padded = ring(56.0, 56.0, Some(json!({"padding": 8})));
    let mut bare = ring(56.0, 56.0, None);

    assert!(force_concentric_radial_stack(&mut padded));
    assert!(force_concentric_radial_stack(&mut bare));

    assert_eq!(
        child(&padded, "track")["x"],
        child(&bare, "track")["x"],
        "padding must not shift the computed centre — jian ignores it for \
         layout:none absolute children (only `border` is added to an \
         explicit inset, never `padding`)"
    );
    assert!(!is_still_off_center(&padded));
}

#[test]
fn reorders_children_to_centre_progress_track_paint_order() {
    let mut node = ring(56.0, 56.0, None);
    let before: Vec<&str> = node["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(before, ["track", "progress", "centre"], "precondition");

    assert!(force_concentric_radial_stack(&mut node));

    let after: Vec<&str> = node["children"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(after, ["centre", "progress", "track"]);
}

#[test]
fn declines_when_a_child_does_not_fit_in_the_wrapper_at_all() {
    let mut node = ring(56.0, 56.0, None);
    node["children"][2]["width"] = json!(160);

    assert!(!force_concentric_radial_stack(&mut node));
    assert!(
        is_still_off_center(&node),
        "an oversized child is an authoring bug, not a position bug"
    );
}

#[test]
fn declines_when_arc_diameters_are_too_mismatched_to_be_the_same_ring() {
    let mut node = ring(120.0, 120.0, None);
    node["children"][0]["width"] = json!(120);
    node["children"][0]["height"] = json!(120);
    node["children"][1]["width"] = json!(60);
    node["children"][1]["height"] = json!(60);

    assert!(!force_concentric_radial_stack(&mut node));
    assert!(is_still_off_center(&node));
}

#[test]
fn declines_when_more_than_one_direct_child_is_non_arc_centre_content() {
    let mut node = ring(56.0, 56.0, None);
    node["children"].as_array_mut().unwrap().push(
        json!({"type": "text", "id": "extra-label", "width": 20, "height": 10, "content": "x"}),
    );

    assert!(!force_concentric_radial_stack(&mut node));
    assert!(is_still_off_center(&node));
}

#[test]
fn is_a_no_op_on_a_node_that_is_not_a_radial_stack_at_all() {
    let mut plain = json!({
        "type": "frame",
        "id": "row",
        "width": 200,
        "height": 40,
        "layout": "horizontal",
        "children": [
            {"type": "ellipse", "id": "a", "width": 32, "height": 32},
            {"type": "ellipse", "id": "b", "width": 32, "height": 32}
        ]
    });

    assert!(!force_concentric_radial_stack(&mut plain));
    assert!(!is_still_off_center(&plain));
}
