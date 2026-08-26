use super::property_panel::{PropertyPanel, PropertyPanelAction};
use super::property_panel_runtime::{enabled_action_rect, input_rects};
use super::property_panel_test_support::state_from;
use crate::{Point2D, Rect};
use op_editor_core::{NodeId, PropertyTab};

#[test]
fn runtime_tab_snapshots_and_hits_every_authored_field() {
    let mut state = state_from(
        r#"{"version":"1.0.0","children":[{"type":"rectangle","id":"button","runtimeId":"menu.play","role":"button","accessibilityLabel":"Play","tabIndex":2,"enabled":false,"visualStates":{"default":"menu.play.default","hover":"menu.play.hover","pressed":"menu.play.pressed","disabled":"menu.play.disabled","focused":"menu.play.focused"},"width":10,"height":10}]}"#,
    );
    state.set_single_selection(NodeId::new("button"));
    state.editor_ui.property_tab = PropertyTab::Interact;
    let panel = PropertyPanel::for_selection(&state).unwrap();
    let runtime = &panel.snapshot.runtime_ui;
    assert_eq!(runtime.runtime_id, "menu.play");
    assert_eq!(runtime.accessibility_label, "Play");
    assert_eq!(runtime.tab_index, "2");
    assert!(!runtime.enabled);

    let panel_rect = Rect::xywh(100.0, 20.0, 280.0, 600.0);
    for (focus, rect) in input_rects(panel_rect) {
        let center = Point2D::new(
            rect.origin.x + rect.size.x / 2.0,
            rect.origin.y + rect.size.y / 2.0,
        );
        assert_eq!(panel.hit_test(panel_rect, center), Some(focus));
    }
    let (_, enabled_rect) = enabled_action_rect(panel_rect, false);
    let center = Point2D::new(
        enabled_rect.origin.x + enabled_rect.size.x / 2.0,
        enabled_rect.origin.y + enabled_rect.size.y / 2.0,
    );
    assert_eq!(
        panel.hit_test_action(panel_rect, center),
        Some(PropertyPanelAction::ToggleRuntimeEnabled(true))
    );
}
