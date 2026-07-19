use jian_widgets::components::select::{SelectHit, SelectState};

use crate::theme::Theme;
use crate::widgets::property_panel::{PropertyPanel, PropertyPanelAction};
use crate::widgets::property_panel_fill::{
    fill_type_at, fill_type_picker_hit, fill_type_picker_rect, paint_fill_type_picker,
};
use crate::widgets::property_panel_fill_picker::{FILL_TYPE_COUNT, FILL_TYPE_ROW_HEIGHT};
use crate::widgets::property_panel_inputs::format_color_hex;
use crate::widgets::property_panel_sections as sections;
use crate::widgets::property_panel_test_support::{state_from, visible_for};
use crate::widgets::PaintCx;
use crate::{Color, Point2D, Rect, RenderBackend, TextLayout};
use op_editor_core::{FillType, NodeId, PropertyFocus};

#[derive(Default)]
struct PickerCaptureBackend {
    round_fills: Vec<Rect>,
}

impl RenderBackend for PickerCaptureBackend {
    fn begin_frame(&mut self) {}

    fn end_frame(&mut self) {}

    fn fill_rect(&mut self, _rect: Rect, _color: Color) {}

    fn stroke_rect(&mut self, _rect: Rect, _color: Color, _width: f32) {}

    fn draw_text(&mut self, _layout: &TextLayout, _origin: Point2D) {}

    fn clip_rect(&mut self, _rect: Rect) {}

    fn stroke_line(&mut self, _from: Point2D, _to: Point2D, _color: Color, _width: f32) {}

    fn fill_round_rect(&mut self, rect: Rect, _radius: f32, _color: Color) {
        self.round_fills.push(rect);
    }

    fn stroke_round_rect(&mut self, _rect: Rect, _radius: f32, _color: Color, _width: f32) {}

    fn stroke_svg_path(
        &mut self,
        _d: &str,
        _top_left: Point2D,
        _size: f32,
        _color: Color,
        _width: f32,
    ) {
    }

    fn save(&mut self) {}

    fn restore(&mut self) {}

    fn translate(&mut self, _offset: Point2D) {}

    fn resize(&mut self, _width: u32, _height: u32) {}

    fn dpi_scale(&self) -> f32 {
        1.0
    }
}

fn assert_rect_eq(actual: Rect, expected: Rect) {
    assert!(
        (actual.origin.x - expected.origin.x).abs() < 0.01
            && (actual.origin.y - expected.origin.y).abs() < 0.01
            && (actual.size.x - expected.size.x).abs() < 0.01
            && (actual.size.y - expected.size.y).abs() < 0.01,
        "expected {expected:?}, got {actual:?}"
    );
}

fn panel_rect() -> Rect {
    Rect {
        origin: Point2D::new(0.0, 0.0),
        size: Point2D::new(280.0, 1400.0),
    }
}

fn selected_rect_state() -> op_editor_core::EditorState {
    let mut state = state_from(
        r##"{ "version": "1.0.0", "children": [
              {"type":"rectangle","id":"rect","name":"Rect",
               "x":40,"y":40,"width":160,"height":100,
               "fill":[{"type":"solid","color":"#112233"}]}
        ]}"##,
    );
    state.set_single_selection(NodeId::new("rect"));
    state
}

fn selected_three_fill_state() -> op_editor_core::EditorState {
    let mut state = state_from(
        r##"{ "version": "1.0.0", "children": [
              {"type":"rectangle","id":"rect","name":"Rect",
               "x":40,"y":40,"width":160,"height":100,
               "fill":[
                 {"type":"solid","color":"#112233"},
                 {"type":"solid","color":"#445566"},
                 {"type":"solid","color":"#778899"}
               ]}
        ]}"##,
    );
    state.set_single_selection(NodeId::new("rect"));
    state
}

fn panel_for(state: &op_editor_core::EditorState) -> PropertyPanel {
    PropertyPanel::for_selection(state).expect("rectangle panel")
}

#[test]
fn fill_type_picker_hit_uses_shared_outside_protocol() {
    let panel_rect = Rect {
        origin: Point2D::new(600.0, 48.0),
        size: Point2D::new(256.0, 700.0),
    };
    let state = SelectState {
        open: true,
        ..Default::default()
    };
    let action_rect = Rect {
        origin: Point2D::new(panel_rect.origin.x + 32.0, panel_rect.origin.y + 120.0),
        size: Point2D::new(150.0, 30.0),
    };
    let picker = fill_type_picker_rect(action_rect, panel_rect);

    assert_eq!(
        fill_type_picker_hit(
            &state,
            action_rect,
            panel_rect,
            Point2D::new(picker.origin.x + 8.0, picker.origin.y + 10.0),
            &Theme::dark(),
        ),
        SelectHit::Row(0)
    );
    assert_eq!(fill_type_at(0), Some(op_editor_core::FillType::Solid));
    assert_eq!(
        fill_type_picker_hit(
            &state,
            action_rect,
            panel_rect,
            Point2D::new(picker.origin.x - 1.0, picker.origin.y),
            &Theme::dark(),
        ),
        SelectHit::Outside
    );
}

#[test]
fn fill_type_picker_flips_above_when_below_space_is_insufficient() {
    let viewport = Rect::xywh(0.0, 36.0, 280.0, 264.0);
    let action_rect = Rect::xywh(40.0, 250.0, 110.0, 26.0);
    let popup_height = FILL_TYPE_ROW_HEIGHT * FILL_TYPE_COUNT as f32;

    let picker = fill_type_picker_rect(action_rect, viewport);

    assert_rect_eq(
        picker,
        Rect::xywh(
            action_rect.origin.x,
            action_rect.origin.y - 4.0 - popup_height,
            action_rect.size.x,
            popup_height,
        ),
    );
    assert!(picker.origin.y >= viewport.origin.y);
}

#[test]
fn fill_type_picker_clamps_inside_viewport_when_neither_side_fits() {
    let viewport = Rect::xywh(0.0, 36.0, 280.0, 180.0);
    let action_rect = Rect::xywh(40.0, 90.0, 110.0, 26.0);
    let popup_height = FILL_TYPE_ROW_HEIGHT * FILL_TYPE_COUNT as f32;

    let picker = fill_type_picker_rect(action_rect, viewport);

    assert_rect_eq(
        picker,
        Rect::xywh(
            action_rect.origin.x,
            viewport.origin.y + viewport.size.y - popup_height,
            action_rect.size.x,
            popup_height,
        ),
    );
}

#[test]
fn fill_type_picker_paint_and_hit_test_share_flipped_rect() {
    let theme = Theme::dark();
    let viewport = Rect::xywh(0.0, 36.0, 280.0, 264.0);
    let action_rect = Rect::xywh(40.0, 250.0, 110.0, 26.0);
    let popup_height = FILL_TYPE_ROW_HEIGHT * FILL_TYPE_COUNT as f32;
    let expected = Rect::xywh(
        action_rect.origin.x,
        action_rect.origin.y - 4.0 - popup_height,
        action_rect.size.x,
        popup_height,
    );
    let state = SelectState {
        open: true,
        ..Default::default()
    };
    let mut backend = PickerCaptureBackend::default();

    paint_fill_type_picker(
        &mut PaintCx {
            backend: &mut backend,
        },
        &theme,
        action_rect,
        viewport,
        &state,
        FillType::Solid,
        op_editor_core::Locale::EnUs,
    );

    assert!(
        backend.round_fills.contains(&expected),
        "picker must paint at the shared flipped rect; painted {:?}",
        backend.round_fills
    );
    assert_eq!(
        fill_type_picker_hit(
            &state,
            action_rect,
            viewport,
            Point2D::new(expected.origin.x + 8.0, expected.origin.y + 75.0),
            &theme,
        ),
        SelectHit::Row(2),
        "row hit-test must use the exact rect used by paint"
    );
    assert!(expected.origin.y >= viewport.origin.y);
}

#[test]
fn fill_type_picker_hit_anchors_to_instance_toggle_action_rect() {
    let mut state = state_from(
        r##"{
          "version":"1.0.0",
          "children":[
            {"type":"frame","id":"card","name":"Card","reusable":true,
             "x":0,"y":0,"width":"fill_container","height":"fill_container",
             "layout":"vertical",
             "fill":[{"type":"solid","color":"#222222"}],
             "children":[]},
            {"type":"ref","id":"inst1","ref":"card","x":300,"y":50,
             "descendants":{"card":{"fill":[{"type":"solid","color":"#ff8800"}]}}}
          ]
        }"##,
    );
    state.set_single_selection(NodeId::new("inst1"));
    let mut panel = panel_for(&state);
    panel.fill_type_picker.open = true;
    panel.fill_type_picker_index = 0;

    let rect = panel_rect();
    let visible = visible_for(&panel);
    assert!(
        visible.create_component && visible.size_fill_width && visible.size_fill_height,
        "fixture must exercise pre-Fill sections that can shift the fill row"
    );
    let toggle_rect = sections::action_button_rects(
        rect,
        visible,
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
    )
    .into_iter()
    .find_map(|(action, rect)| {
        matches!(action, PropertyPanelAction::ToggleFillTypePicker(0)).then_some(rect)
    })
    .expect("fill type toggle action rect");

    let expected_popup_y = toggle_rect.origin.y + toggle_rect.size.y + 4.0;
    let picker_rect = fill_type_picker_rect(toggle_rect, rect);
    assert!(
        (picker_rect.origin.y - expected_popup_y).abs() < 0.01,
        "open fill-type popup y must derive from ToggleFillTypePicker action rect: expected {expected_popup_y}, got {}",
        picker_rect.origin.y
    );

    assert_eq!(
        panel.fill_type_picker_hit(
            rect,
            Point2D::new(toggle_rect.origin.x + 8.0, expected_popup_y + 1.0)
        ),
        SelectHit::Row(0),
        "open fill-type popup must hit-test below the emitted ToggleFillTypePicker action rect"
    );
}

#[test]
fn stacked_fills_emit_add_and_per_index_remove_actions() {
    let mut state = selected_rect_state();
    assert!(state.add_selected_fill());
    let panel = panel_for(&state);
    assert_eq!(panel.snapshot.fills.len(), 2);

    let actions: Vec<_> = sections::action_button_rects_with_fill_picker(
        panel_rect(),
        visible_for(&panel),
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
        false,
        0,
        false,
        false,
        false,
        false,
        false,
    )
    .into_iter()
    .map(|(action, _)| action)
    .collect();

    assert!(actions.contains(&PropertyPanelAction::AddFill));
    assert!(actions.contains(&PropertyPanelAction::RemoveFill(0)));
    assert!(actions.contains(&PropertyPanelAction::RemoveFill(1)));
}

#[test]
fn three_fill_rows_emit_exact_boundary_move_actions_in_row_order() {
    let state = selected_three_fill_state();
    let panel = panel_for(&state);

    let moves: Vec<_> = sections::action_button_rects(
        panel_rect(),
        visible_for(&panel),
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
    )
    .into_iter()
    .filter_map(|(action, _)| {
        matches!(action, PropertyPanelAction::MoveFill { .. }).then_some(action)
    })
    .collect();

    assert_eq!(
        moves,
        vec![
            PropertyPanelAction::MoveFill { from: 0, to: 1 },
            PropertyPanelAction::MoveFill { from: 1, to: 0 },
            PropertyPanelAction::MoveFill { from: 1, to: 2 },
            PropertyPanelAction::MoveFill { from: 2, to: 1 },
        ],
        "first has down only, middle has up then down, last has up only"
    );
}

#[test]
fn fill_move_action_rect_centres_hit_the_exact_emitted_actions() {
    let state = selected_three_fill_state();
    let panel = panel_for(&state);
    let move_rects: Vec<_> = sections::action_button_rects(
        panel_rect(),
        visible_for(&panel),
        &panel.snapshot.effects,
        &panel.snapshot.fills,
        &panel.snapshot.interactions,
    )
    .into_iter()
    .filter(|(action, _)| matches!(action, PropertyPanelAction::MoveFill { .. }))
    .collect();

    assert_eq!(move_rects.len(), 4);
    for (expected, rect) in move_rects {
        let centre = Point2D::new(
            rect.origin.x + rect.size.x / 2.0,
            rect.origin.y + rect.size.y / 2.0,
        );
        assert_eq!(
            panel.hit_test_action(panel_rect(), centre),
            Some(expected),
            "the painted move control's action rect must be its hit region"
        );
    }
}

#[test]
fn remove_selected_fill_removes_requested_index() {
    let mut state = selected_rect_state();
    assert!(state.add_selected_fill());
    assert!(state.set_selected_fill_hex_at(1, "#445566"));

    assert!(state.remove_selected_fill(0));

    let panel = panel_for(&state);
    assert_eq!(panel.snapshot.fills.len(), 1);
    assert_eq!(
        format_color_hex(panel.snapshot.fills[0].color),
        "#445566",
        "removing fill 0 should leave the former second fill"
    );
}

#[test]
fn set_selected_fill_type_at_updates_only_requested_index() {
    let mut state = selected_rect_state();
    assert!(state.add_selected_fill());

    assert!(state.set_selected_fill_type_at(1, FillType::LinearGradient));

    let panel = panel_for(&state);
    assert_eq!(panel.snapshot.fills[0].fill_type, FillType::Solid);
    assert_eq!(panel.snapshot.fills[1].fill_type, FillType::LinearGradient);
}

#[test]
fn set_selected_fill_hex_at_updates_only_requested_index() {
    let mut state = selected_rect_state();
    assert!(state.add_selected_fill());

    assert!(state.set_selected_fill_hex_at(1, "#123456"));

    let panel = panel_for(&state);
    assert_eq!(format_color_hex(panel.snapshot.fills[0].color), "#112233");
    assert_eq!(format_color_hex(panel.snapshot.fills[1].color), "#123456");
}

#[test]
fn set_selected_fill_opacity_at_updates_only_requested_index() {
    let mut state = selected_rect_state();
    assert!(state.add_selected_fill());

    assert!(state.set_selected_fill_opacity_at(1, 0.5));

    let panel = panel_for(&state);
    assert!((panel.snapshot.fills[0].opacity - 1.0).abs() < f32::EPSILON);
    assert!((panel.snapshot.fills[1].opacity - 0.5).abs() < f32::EPSILON);
}

#[test]
fn single_fill_node_emits_indexed_hex_and_opacity_inputs() {
    let state = selected_rect_state();
    let panel = panel_for(&state);
    assert_eq!(panel.snapshot.fills.len(), 1);

    let focuses: Vec<_> = sections::editable_input_rects(
        panel_rect(),
        visible_for(&panel),
        &panel.snapshot.fills,
        &panel.snapshot.effects,
    )
    .into_iter()
    .map(|(focus, _)| focus)
    .collect();

    assert!(focuses.contains(&PropertyFocus::FillHex(0)));
    assert!(focuses.contains(&PropertyFocus::FillOpacity(0)));
}
