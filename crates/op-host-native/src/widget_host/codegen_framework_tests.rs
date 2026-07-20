use super::{CodeSelectionDragState, WidgetHostNative};
use op_editor_core::codegen::{CodeSelection, CodegenPhase, Framework};
use op_editor_ui::widgets::property_panel_action::CodegenAction;
use op_editor_ui::widgets::PropertyPanelAction;

#[test]
fn framework_switch_clears_previous_native_output() {
    let mut host = WidgetHostNative::new();
    {
        let cg = &mut host.editor_state_mut().codegen;
        cg.phase = CodegenPhase::Complete;
        cg.code = "export const ReactView = () => null".into();
        cg.code_scroll.offset = 48.0;
        cg.code_selection = Some(CodeSelection {
            anchor: 0,
            focus: 6,
        });
        cg.degraded = true;
        cg.selection_snapshot = vec!["hero".into()];
    }
    host.code_selection_drag = Some(CodeSelectionDragState { anchor: 0 });

    host.apply_property_action(PropertyPanelAction::Codegen(
        CodegenAction::SelectFramework(Framework::Vue),
    ));

    let cg = &host.editor_state().codegen;
    assert_eq!(cg.framework, Framework::Vue);
    assert_eq!(cg.phase, CodegenPhase::Idle);
    assert!(cg.code.is_empty());
    assert_eq!(cg.code_scroll.offset, 0.0);
    assert!(cg.code_selection.is_none());
    assert!(!cg.degraded);
    assert!(cg.selection_snapshot.is_empty());
    assert!(host.code_selection_drag.is_none());
}
