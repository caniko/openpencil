use super::image_panel_selection::ImageInputSelectionDragState;
use super::WidgetHostNative;
use op_editor_ui::widgets::property_panel_image_assets::ImagePopoverInputKind;

fn search_host(text: &str, caret: usize) -> WidgetHostNative {
    let mut host = WidgetHostNative::new();
    let panel = &mut host.editor_state_mut().editor_ui.image_panel;
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
        host.editor_state()
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
        host.editor_state()
            .editor_ui
            .image_panel
            .search_query
            .text(),
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
        host.editor_state()
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
        .editor_state()
        .editor_ui
        .image_panel
        .search_query
        .selection();
    assert!(selection.anchor > selection.focus);
    assert_eq!(selection.ordered(), (1, "ab你c".len()));
}

#[test]
fn generate_prompt_drag_only_starts_when_the_editor_is_visible() {
    let mut host = WidgetHostNative::new();
    host.editor_state_mut().editor_ui.image_panel.generate_open = true;
    assert!(!host.begin_image_input_selection_drag(ImagePopoverInputKind::Generate, 0));

    let id = host
        .editor_state_mut()
        .editor_ui
        .agent_settings
        .add_image_gen_profile();
    host.editor_state_mut()
        .editor_ui
        .agent_settings
        .image_gen_profiles
        .iter_mut()
        .find(|profile| profile.id == id)
        .expect("profile")
        .api_key = "sk-test".into();
    host.editor_state_mut()
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
