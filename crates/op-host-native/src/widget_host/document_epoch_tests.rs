//! Document-epoch guard: bumps on whole-document replacement, never
//! on an in-place edit. Async work (e.g. clipboard paste decode)
//! captures this and re-checks it before applying so a result decoded
//! for a since-replaced document is dropped.

use crate::widget_host::WidgetHostNative;

#[test]
fn epoch_bumps_on_replace_not_on_edit() {
    let mut host = WidgetHostNative::new();
    let start = host.document_epoch();

    // An in-place mutation through the generic accessor must NOT bump.
    host.editor_state_mut().editor_ui.sidebar_open ^= true;
    host.mark_editor_state_dirty();
    assert_eq!(
        host.document_epoch(),
        start,
        "in-place edit must not bump the document epoch"
    );

    // A whole-document replacement (Open / New) bumps once.
    host.replace_editor_state(op_editor_core::EditorState::starter());
    assert_eq!(
        host.document_epoch(),
        start + 1,
        "replace_editor_state must bump the epoch"
    );

    host.replace_editor_state(op_editor_core::EditorState::starter());
    assert_eq!(host.document_epoch(), start + 2, "each replace bumps once");
}
