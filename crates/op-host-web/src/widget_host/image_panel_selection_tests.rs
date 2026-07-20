use super::image_panel_selection::ImageInputSelectionDragState;
use super::WidgetHost;
use op_editor_ui::widgets::property_panel_image_assets::ImagePopoverInputKind;
use op_editor_ui::{Color, Point2D, Rect, RenderBackend, TextLayout};

struct ProportionalBackend;

impl RenderBackend for ProportionalBackend {
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
    fn fill_rect(&mut self, _: Rect, _: Color) {}
    fn stroke_rect(&mut self, _: Rect, _: Color, _: f32) {}
    fn draw_text(&mut self, _: &TextLayout, _: Point2D) {}
    fn clip_rect(&mut self, _: Rect) {}
    fn stroke_line(&mut self, _: Point2D, _: Point2D, _: Color, _: f32) {}
    fn fill_round_rect(&mut self, _: Rect, _: f32, _: Color) {}
    fn stroke_round_rect(&mut self, _: Rect, _: f32, _: Color, _: f32) {}
    fn stroke_svg_path(&mut self, _: &str, _: Point2D, _: f32, _: Color, _: f32) {}
    fn save(&mut self) {}
    fn restore(&mut self) {}
    fn translate(&mut self, _: Point2D) {}
    fn resize(&mut self, _: u32, _: u32) {}
    fn dpi_scale(&self) -> f32 {
        1.0
    }
    fn measure_text_family(&mut self, text: &str, _: f32, _: &str) -> f32 {
        text.chars()
            .map(|ch| match ch {
                'i' => 2.0,
                'W' => 10.0,
                ch if !ch.is_ascii() => 11.0,
                _ => 6.0,
            })
            .sum()
    }
}

fn search_host(text: &str, caret: usize) -> WidgetHost {
    let mut host = WidgetHost::new();
    let panel = &mut host.editor_state.editor_ui.image_panel;
    panel.search_open = true;
    panel.search_query.set_text(text);
    panel.search_query.set_caret(caret, 0);
    host
}

#[test]
fn image_input_drag_selects_utf8_range_and_release_keeps_it_editable() {
    let mut host = search_host("ab你cd", 1);
    assert!(host.begin_image_input_selection_drag(ImagePopoverInputKind::Search, 1));
    let drag = host.image_input_selection_drag.expect("selection drag");
    assert!(host.drag_image_input_selection_to(drag, "ab你".len()));
    assert_eq!(
        host.editor_state
            .editor_ui
            .image_panel
            .search_query
            .highlight_range(),
        Some((1, "ab你".len()))
    );
    assert!(host.apply_release());
    assert!(host.image_input_selection_drag.is_none());
    assert!(host.apply_image_panel_text('X'));
    assert_eq!(
        host.editor_state.editor_ui.image_panel.search_query.text(),
        "aXcd"
    );
}

#[test]
fn image_input_shift_click_extends_from_existing_anchor() {
    let mut host = search_host("ab你cd", 1);
    host.set_modifier_shift(true);
    assert!(host.begin_image_input_selection_drag(ImagePopoverInputKind::Search, "ab你c".len()));
    assert_eq!(
        host.image_input_selection_drag.map(|drag| drag.anchor),
        Some(1)
    );
    assert_eq!(
        host.editor_state
            .editor_ui
            .image_panel
            .search_query
            .highlight_range(),
        Some((1, "ab你c".len()))
    );
}

#[test]
fn reverse_image_input_drag_keeps_direction_and_ordered_range() {
    let mut host = search_host("ab你cd", "ab你c".len());
    assert!(host.begin_image_input_selection_drag(ImagePopoverInputKind::Search, "ab你c".len()));
    let drag = ImageInputSelectionDragState {
        kind: ImagePopoverInputKind::Search,
        anchor: "ab你c".len(),
    };
    assert!(host.drag_image_input_selection_to(drag, 1));
    let selection = host
        .editor_state
        .editor_ui
        .image_panel
        .search_query
        .selection();
    assert!(selection.anchor > selection.focus);
    assert_eq!(selection.ordered(), (1, "ab你c".len()));
}

#[test]
fn generate_prompt_drag_only_starts_when_the_editor_is_visible() {
    let mut host = WidgetHost::new();
    host.editor_state.editor_ui.image_panel.generate_open = true;
    assert!(!host.begin_image_input_selection_drag(ImagePopoverInputKind::Generate, 0));

    let id = host
        .editor_state
        .editor_ui
        .agent_settings
        .add_image_gen_profile();
    host.editor_state
        .editor_ui
        .agent_settings
        .image_gen_profiles
        .iter_mut()
        .find(|profile| profile.id == id)
        .expect("profile")
        .api_key = "sk-test".into();
    host.editor_state
        .editor_ui
        .image_panel
        .generate_prompt
        .set_text("dream cover");
    assert!(host.begin_image_input_selection_drag(ImagePopoverInputKind::Generate, 5));
    assert_eq!(
        host.image_input_selection_drag.map(|drag| drag.kind),
        Some(ImagePopoverInputKind::Generate)
    );
}

#[test]
fn latest_real_paint_geometry_drives_image_click_and_ime_anchor() {
    let mut host = WidgetHost::new();
    let _ = host
        .editor_state
        .insert_image_node_at_viewport("Hero", "https://x/y.png");
    let panel = &mut host.editor_state.editor_ui.image_panel;
    panel.search_open = true;
    panel.search_query.set_text("iiiiWWWW中文");
    panel.search_query.set_caret(0, 0);

    let mut backend = ProportionalBackend;
    host.paint_editor(&mut backend, 1200.0, 800.0);
    let geometry = host
        .image_input_geometry
        .clone()
        .expect("painted input geometry");
    let line = geometry.line.rect();
    let point = Point2D::new(line.origin.x + 16.0, line.origin.y + line.size.y / 2.0);
    assert_eq!(
        geometry.byte_offset_at(&host.editor_state.editor_ui.image_panel, point, false),
        Some(4)
    );
    assert_eq!(
        host.ime_anchor_rect(),
        geometry.caret_rect(&host.editor_state.editor_ui.image_panel)
    );

    assert!(host.dismiss_image_popovers_on_press(point.x, point.y, 1200.0, 800.0));
    assert_eq!(
        host.editor_state.editor_ui.image_panel.search_query.caret(),
        4
    );
}
