use super::WidgetHostNative;
use op_editor_core::NodeId;

fn host_with_two_selected_nodes() -> WidgetHostNative {
    let doc = jian_ops_schema::load_str(
        r#"{"version":"1.0.0","children":[
            {"type":"rectangle","id":"n1","name":"one","x":0,"y":0,"width":10,"height":10},
            {"type":"rectangle","id":"n2","name":"two","x":20,"y":0,"width":10,"height":10}
        ]}"#,
    )
    .expect("fixture parses")
    .value;
    let mut host = WidgetHostNative::new();
    *host.editor_state_mut() = op_editor_core::EditorState::from_document(doc);
    host.editor_state_mut()
        .set_single_selection(NodeId::new("n1"));
    host.editor_state_mut().editor_ui.toggle_font_picker();
    host
}

#[test]
fn open_font_picker_consumes_delete_and_select_all_without_mutating_canvas() {
    let mut host = host_with_two_selected_nodes();
    let before_doc = host.editor_state().doc.clone();
    let before_selection = host.editor_state().selection.clone();
    let before_revision = host.editor_state().revision;

    assert!(host.apply_delete());
    assert!(host.apply_select_all());

    let state = host.editor_state();
    assert_eq!(state.doc, before_doc);
    assert_eq!(state.selection, before_selection);
    assert_eq!(state.revision, before_revision);
    assert!(!state.history.can_undo());
    assert!(state.editor_ui.font_picker.open);
}
