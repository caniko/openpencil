use op_editor_core::EditorState;

use crate::design_agent_tools::execute_design_tool;

#[test]
fn rolled_back_batch_is_an_error_and_preserves_structured_feedback() {
    let mut state = EditorState::new();
    let before = serde_json::to_value(state.active_children()).expect("initial document snapshot");
    let args = serde_json::json!({
        "operations": concat!(
            "root=I(null,{type:'frame',name:'Never Applied',width:120,height:80})\n",
            "U(\"missing-node\",{x:5})"
        )
    })
    .to_string();

    let (result, mutated) = execute_design_tool(&mut state, "batch_design", &args);

    assert!(result.is_error, "rolled-back batch must be a tool error");
    assert!(!mutated, "rolled-back batch must not report a mutation");
    assert_eq!(
        serde_json::to_value(state.active_children()).expect("final document snapshot"),
        before,
        "the transaction must leave the document unchanged"
    );

    let envelope: serde_json::Value =
        serde_json::from_str(&result.content).expect("chat tool result envelope");
    assert_eq!(envelope["success"], false, "{envelope}");
    assert_eq!(envelope["data"]["applied"], false, "{envelope}");
    assert_eq!(
        envelope["data"]["errors"]
            .as_array()
            .expect("structured errors")
            .len(),
        1,
        "{envelope}"
    );
    let hint = envelope["data"]["hint"]
        .as_str()
        .expect("structured resend hint");
    assert!(hint.contains("rolled back"), "{hint}");
    assert_eq!(envelope["error"], hint, "{envelope}");
}
