//! Host-side dispatch tests for the Interactions section's write
//! path — `apply_property_action` + `EditorCommand::PatchNodeData` +
//! undo. Hit-test / popover-row / row-parsing coverage lives in
//! `op-editor-ui`'s `property_panel_interactions_tests.rs`; these
//! tests cover what only the host can exercise (real `EditorState`
//! mutation + history).

use super::WidgetHostNative;
use op_editor_core::NodeId;
use op_editor_ui::widgets::{PropertyPanel, PropertyPanelAction};
use op_editor_ui::Point2D;

const VIEWPORT_W: f32 = 1200.0;
const VIEWPORT_H: f32 = 800.0;

fn seed(host: &mut WidgetHostNative, json: &str) {
    let doc = jian_ops_schema::load_str(json)
        .expect("fixture JSON parses")
        .value;
    *host.editor_state_mut() = op_editor_core::EditorState::from_document(doc);
    host.mark_paint_dirty_for_test();
}

fn node_events_json(host: &WidgetHostNative, id: &str) -> serde_json::Value {
    let node =
        op_editor_core::walkers::find_node(host.editor_state().active_children(), &NodeId::new(id))
            .expect("node present");
    serde_json::to_value(node).expect("node serializes")
}

/// Brute-force scan the property rail for a point the open Navigate/
/// Back/Remove popover maps to row `want_row` — mirrors the
/// `point_for_action` scan in `property_panel_press_tests.rs`.
fn point_for_menu_row(host: &WidgetHostNative, want_row: usize) -> Point2D {
    let panel = PropertyPanel::for_selection(host.editor_state()).expect("property panel");
    let rect = host.property_rect(VIEWPORT_W, VIEWPORT_H);
    let mut y = rect.origin.y;
    while y < rect.origin.y + rect.size.y {
        let mut x = rect.origin.x;
        while x < rect.origin.x + rect.size.x {
            let point = Point2D::new(x, y);
            if panel.interaction_menu_row_at(rect, point) == Some(want_row) {
                return point;
            }
            x += 2.0;
        }
        y += 2.0;
    }
    panic!("no point maps to interaction-menu row {want_row}");
}

#[test]
fn toggle_interaction_menu_opens_and_closes() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame"}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));
    assert!(!host.editor_state().editor_ui.interaction_menu_open);

    host.apply_property_action(PropertyPanelAction::ToggleInteractionMenu);
    assert!(host.editor_state().editor_ui.interaction_menu_open);

    host.apply_property_action(PropertyPanelAction::ToggleInteractionMenu);
    assert!(!host.editor_state().editor_ui.interaction_menu_open);
}

#[test]
fn set_interaction_navigate_writes_the_double_encoded_patch() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame"}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));

    host.apply_property_action(PropertyPanelAction::SetInteractionNavigate {
        path: "/profile".to_string(),
    });

    let value = node_events_json(&host, "f1");
    assert_eq!(
        value["events"]["onTap"][0]["replace"],
        serde_json::Value::String("\"/profile\"".to_string())
    );
}

#[test]
fn set_interaction_navigate_then_undo_restores_no_events() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame"}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));

    host.apply_property_action(PropertyPanelAction::SetInteractionNavigate {
        path: "/profile".to_string(),
    });
    assert!(node_events_json(&host, "f1").get("events").is_some());

    assert!(host.editor_state_mut().undo(), "one undo step available");
    let restored = node_events_json(&host, "f1");
    assert!(
        restored.get("events").is_none(),
        "undo should restore the pre-edit (no events) node"
    );
}

#[test]
fn remove_interaction_clears_on_tap_without_leaving_an_empty_events_shell() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame",
             "events":{"onTap":[{"pop":null}]}}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));

    host.apply_property_action(PropertyPanelAction::RemoveInteraction);

    let value = node_events_json(&host, "f1");
    assert!(
        value.get("events").is_none(),
        "onTap was the only handler — events must serialize away entirely, not as `events:{{}}`"
    );
}

#[test]
fn remove_interaction_preserves_a_sibling_event_handler() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"text_input","id":"in1","name":"Input",
             "events":{"onTap":[{"pop":null}],"onChange":[{"set":{"$state.x":"1"}}]}}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("in1"));

    host.apply_property_action(PropertyPanelAction::RemoveInteraction);

    let value = node_events_json(&host, "in1");
    assert!(
        value["events"]["onTap"].is_null() || value["events"].get("onTap").is_none(),
        "onTap should be cleared"
    );
    assert!(
        value["events"]["onChange"].is_array(),
        "a sibling handler must survive the Remove"
    );
}

#[test]
fn set_interaction_pop_writes_the_pop_action() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame"}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));

    host.apply_property_action(PropertyPanelAction::SetInteractionPop);

    let value = node_events_json(&host, "f1");
    assert!(value["events"]["onTap"][0]["pop"].is_null());
}

#[test]
fn interaction_menu_hover_tracks_the_row_under_the_cursor() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame"}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));
    host.apply_property_action(PropertyPanelAction::ToggleInteractionMenu);
    assert!(host.editor_state().editor_ui.interaction_menu_open);
    assert_eq!(host.editor_state().editor_ui.interaction_menu_hover, None);

    // No authored screens on this doc — row 0 is "Back (pop)".
    let point = point_for_menu_row(&host, 0);
    assert!(host.update_interaction_menu_hover(point.x, point.y, VIEWPORT_W, VIEWPORT_H));
    assert_eq!(
        host.editor_state().editor_ui.interaction_menu_hover,
        Some(0)
    );

    // A second update at the same point is a no-op (hover unchanged).
    assert!(!host.update_interaction_menu_hover(point.x, point.y, VIEWPORT_W, VIEWPORT_H));

    // Moving off every row clears the hover.
    assert!(host.update_interaction_menu_hover(-500.0, -500.0, VIEWPORT_W, VIEWPORT_H));
    assert_eq!(host.editor_state().editor_ui.interaction_menu_hover, None);
}

#[test]
fn interaction_menu_hover_is_a_no_op_while_closed() {
    let mut host = WidgetHostNative::new();
    seed(
        &mut host,
        r#"{ "version": "1.0.0", "children": [
            {"type":"frame","id":"f1","name":"Frame"}
        ]}"#,
    );
    host.editor_state_mut()
        .set_single_selection(NodeId::new("f1"));
    assert!(!host.editor_state().editor_ui.interaction_menu_open);

    assert!(!host.update_interaction_menu_hover(0.0, 0.0, VIEWPORT_W, VIEWPORT_H));
}
