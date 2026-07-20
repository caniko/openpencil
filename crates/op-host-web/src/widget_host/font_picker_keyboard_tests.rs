use super::WidgetHost;
use op_editor_core::chat::{AgentProvider, ModelEntry};
use op_editor_core::NodeId;

fn host_with_two_selected_nodes() -> WidgetHost {
    let doc = jian_ops_schema::load_str(
        r#"{"version":"1.0.0","children":[
            {"type":"rectangle","id":"n1","name":"one","x":0,"y":0,"width":10,"height":10},
            {"type":"rectangle","id":"n2","name":"two","x":20,"y":0,"width":10,"height":10}
        ]}"#,
    )
    .expect("fixture parses")
    .value;
    let mut host = WidgetHost::new();
    host.editor_state = op_editor_core::EditorState::from_document(doc);
    host.editor_state.set_single_selection(NodeId::new("n1"));
    host.editor_state.editor_ui.toggle_font_picker();
    host
}

#[test]
fn open_font_picker_consumes_delete_and_select_all_without_mutating_canvas() {
    let mut host = host_with_two_selected_nodes();
    let before_doc = host.editor_state.doc.clone();
    let before_selection = host.editor_state.selection.clone();
    let before_revision = host.editor_state.revision;

    assert!(host.apply_delete());
    assert!(host.apply_select_all());

    assert_eq!(host.editor_state.doc, before_doc);
    assert_eq!(host.editor_state.selection, before_selection);
    assert_eq!(host.editor_state.revision, before_revision);
    assert!(!host.editor_state.history.can_undo());
    assert!(host.editor_state.editor_ui.font_picker.open);
}

#[test]
fn open_font_picker_consumes_enter_without_sending_chat() {
    let mut host = host_with_two_selected_nodes();
    host.editor_state
        .chat
        .available_models
        .push(ModelEntry::new(AgentProvider::CodexCli, "gpt-5", "GPT-5"));
    host.editor_state.chat.focused = true;
    host.editor_state.chat.set_input_text("do not send");
    let before_message_count = host.editor_state.chat.messages.len();

    assert!(host.apply_send());

    assert_eq!(host.editor_state.chat.messages.len(), before_message_count);
    assert_eq!(host.editor_state.chat.input.text(), "do not send");
    assert!(host.editor_state.editor_ui.font_picker.open);
}
