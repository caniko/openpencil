use super::property_panel::{PropertyPanel, PropertyPanelAction};
use super::property_panel_sections as sections;
use super::property_panel_test_support::{state_from, visible_for};
use crate::{Point2D, Rect};
use op_editor_core::{EditorState, NodeId};

#[test]
fn per_corner_expand_emits_grid_focuses_and_shifts_later_sections() {
    let mut state = state_from(
        r##"{"version":"1.0.0","children":[
          {"type":"rectangle","id":"r","x":0,"y":0,"width":100,"height":80,
           "cornerRadius":[8,4,2,0],"fill":[{"type":"solid","color":"#fff"}]}
        ]}"##,
    );
    state.set_single_selection(NodeId::new("r"));
    state.editor_ui.locale = op_editor_core::Locale::EnUs;
    let collapsed = PropertyPanel::for_selection(&state).unwrap();
    state.editor_ui.corner_expand_open = true;
    let panel = PropertyPanel::for_selection(&state).unwrap();
    assert!(panel.corner_expand_open);
    assert_eq!(panel.snapshot.corner_radii, [8.0, 4.0, 2.0, 0.0]);
    assert_eq!(panel.labels.mixed, "Mixed");

    let rect = Rect::xywh(0.0, 0.0, 280.0, 1400.0);
    let actions = sections::action_button_rects(
        rect,
        visible_for(&panel),
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
    );
    assert!(actions
        .iter()
        .any(|(action, _)| matches!(action, PropertyPanelAction::ToggleCornerExpand)));

    let inputs = sections::editable_input_rects(
        rect,
        visible_for(&panel),
        &panel.snapshot.fills,
        &panel.snapshot.effects,
    );
    for focus in [
        op_editor_core::PropertyFocus::CornerTL,
        op_editor_core::PropertyFocus::CornerTR,
        op_editor_core::PropertyFocus::CornerBL,
        op_editor_core::PropertyFocus::CornerBR,
    ] {
        let (_, target) = inputs
            .iter()
            .find(|(candidate, _)| *candidate == focus)
            .unwrap();
        assert_eq!(
            panel.hit_test(
                rect,
                Point2D::new(
                    target.origin.x + target.size.x / 2.0,
                    target.origin.y + target.size.y / 2.0,
                ),
            ),
            Some(focus)
        );
    }
    let size_y = |panel: &PropertyPanel| {
        sections::editable_input_rects(
            rect,
            visible_for(panel),
            &panel.snapshot.fills,
            &panel.snapshot.effects,
        )
        .into_iter()
        .find(|(focus, _)| *focus == op_editor_core::PropertyFocus::SizeW)
        .unwrap()
        .1
        .origin
        .y
    };
    assert_eq!(
        size_y(&panel) - size_y(&collapsed),
        super::property_panel_corner::CORNER_GRID_EXTRA_HEIGHT
    );
}

#[test]
fn path_fill_rule_segments_are_path_only_and_hittable() {
    let mut path_state = state_from(
        r##"{"version":"1.0.0","children":[
          {"type":"path","id":"p","d":"M0 0L20 0L20 20Z","width":20,"height":20,
           "fillRule":"evenodd","fill":[{"type":"solid","color":"#fff"}]}
        ]}"##,
    );
    path_state.set_single_selection(NodeId::new("p"));
    let panel = PropertyPanel::for_selection(&path_state).unwrap();
    let rect = Rect::xywh(0.0, 0.0, 280.0, 1200.0);
    let rule_actions: Vec<_> = sections::action_button_rects(
        rect,
        visible_for(&panel),
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
    )
    .into_iter()
    .filter(|(action, _)| matches!(action, PropertyPanelAction::SetFillRule(_)))
    .collect();
    assert_eq!(rule_actions.len(), 2);
    for (action, target) in rule_actions {
        assert_eq!(
            panel.hit_test_action(
                rect,
                Point2D::new(
                    target.origin.x + target.size.x / 2.0,
                    target.origin.y + target.size.y / 2.0,
                ),
            ),
            Some(action)
        );
    }

    let mut rect_state = EditorState::sample();
    rect_state.set_single_selection(NodeId::new("n13"));
    let rect_panel = PropertyPanel::for_selection(&rect_state).unwrap();
    assert!(!sections::action_button_rects(
        rect,
        visible_for(&rect_panel),
        &rect_panel.snapshot.effects,
        &rect_panel.snapshot.fills,
        &rect_panel.snapshot.interactions,
    )
    .iter()
    .any(|(action, _)| matches!(action, PropertyPanelAction::SetFillRule(_))));
}

#[test]
fn compact_effect_rows_expose_slider_input_eye_and_remove() {
    let mut state = state_from(
        r##"{"version":"1.0.0","children":[
          {"type":"rectangle","id":"r","width":100,"height":80,"effects":[
            {"type":"shadow","offsetX":0,"offsetY":4,"blur":8,"spread":0,"color":"#00000040"},
            {"type":"blur","radius":20,"visible":false},
            {"type":"background_blur","radius":30}
          ]}
        ]}"##,
    );
    state.set_single_selection(NodeId::new("r"));
    let panel = PropertyPanel::for_selection(&state).unwrap();
    let rect = Rect::xywh(0.0, 0.0, 280.0, 1400.0);
    let inputs = sections::editable_input_rects(
        rect,
        visible_for(&panel),
        &panel.snapshot.fills,
        &panel.snapshot.effects,
    );
    assert_eq!(
        inputs
            .iter()
            .filter(|(focus, _)| matches!(focus, op_editor_core::PropertyFocus::EffectRadius(_)))
            .count(),
        3
    );
    let actions = sections::action_button_rects(
        rect,
        visible_for(&panel),
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
    );
    assert_eq!(
        actions
            .iter()
            .filter(|(action, _)| matches!(action, PropertyPanelAction::RemoveEffect(_)))
            .count(),
        3
    );
    assert!(actions
        .iter()
        .any(|(action, _)| matches!(action, PropertyPanelAction::SetEffectVisible(1, true))));
    assert!(matches!(
        panel.effect_radius_drag_action(rect, 2, f32::MAX),
        Some(PropertyPanelAction::AdjustEffectParam {
            effect: 2,
            new_value: 100.0,
            ..
        })
    ));
}
