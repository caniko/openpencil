use super::*;
use op_ai::chat_provider::{ChatDelta, ChatToolResult, StopReason, LOOP_FINALIZE_OP};
use op_editor_core::{ChatMessage, PenNodeExt};

#[test]
fn pump_defers_loop_finalize_until_registered_reveals_drain() {
    // Native scene construction can be slow on a loaded macOS runner and does
    // not use agent indicators. Finish it before publishing this test's epoch
    // so no stale background worker gets a long window to retire that epoch.
    let mut host = WidgetHostNative::new();
    let _guard = crate::agent_indicator_test_lock::LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    op_editor_core::agent_indicators::clear();
    let epoch = op_editor_core::agent_indicators::begin();

    host.editor_state_mut()
        .chat
        .messages
        .push(ChatMessage::assistant_streaming());
    let (delta_tx, delta_rx) = std::sync::mpsc::channel();
    let (tool_tx, tool_rx) = std::sync::mpsc::channel();
    let batch_ack = enqueue_tool_request(
        &tool_tx,
        "batch_design",
        r#"{"operations":"root=I(null,{type:'frame',name:'Header',width:1200,height:64})"}"#,
    );
    let mut current = Some(ChatSession::from_channels(delta_rx, Some(tool_rx)).into_design_loop());

    // Deliver the batch synchronously, then queue finalize only after its ack.
    // This matches UiChatToolExecutor's blocking request order without relying
    // on a worker being scheduled inside an arbitrary CI polling budget.
    pump(&mut host, &mut current, None, None, (1200.0, 800.0));
    let batch_result = batch_ack
        .try_recv()
        .expect("batch_design should be acknowledged by the first pump");
    assert!(!batch_result.is_error, "batch failed: {batch_result:?}");
    let reveal_end = op_editor_core::agent_indicators::latest_reveal_end_ms(epoch)
        .expect("batch_design should register reveals before finalize");

    let finalize_ack = enqueue_tool_request(&tool_tx, LOOP_FINALIZE_OP, r#"{"checkOnly":false}"#);
    pump(&mut host, &mut current, None, None, (1200.0, 800.0));
    assert!(
        matches!(
            finalize_ack.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ),
        "finalize must remain deferred while registered reveals are pending"
    );

    let header = header_node(&host).expect("Header frame inserted");
    assert_eq!(
        header.base().role.as_deref(),
        None,
        "finalize must not run while the reveal queue is still draining"
    );

    let now = reveal_now_millis();
    if reveal_end > now {
        std::thread::sleep(std::time::Duration::from_millis(reveal_end - now + 20));
    }
    pump(&mut host, &mut current, None, None, (1200.0, 800.0));
    let finalize_result = finalize_ack
        .try_recv()
        .expect("finalize should be acknowledged after the reveal drain");
    assert!(
        !finalize_result.is_error,
        "finalize failed: {finalize_result:?}"
    );

    delta_tx
        .send(ChatDelta::Done {
            stop_reason: StopReason::EndTurn,
        })
        .expect("live session receives terminal delta");
    pump(&mut host, &mut current, None, None, (1200.0, 800.0));
    assert!(current.is_none(), "turn must finish after reveal drain");
    let header = header_node(&host).expect("Header frame still present");
    assert_eq!(
        header.base().role.as_deref(),
        Some("navbar"),
        "loop-end finalize should run after the reveal queue drains"
    );

    op_editor_core::agent_indicators::end_if_epoch(epoch);
    op_editor_core::agent_indicators::clear();
}

fn enqueue_tool_request(
    tx: &std::sync::mpsc::Sender<op_editor_host_core::chat::ChatToolRequest>,
    name: &str,
    args_json: &str,
) -> std::sync::mpsc::Receiver<ChatToolResult> {
    let (ack_tx, ack_rx) = std::sync::mpsc::sync_channel(1);
    tx.send(op_editor_host_core::chat::ChatToolRequest {
        name: name.to_string(),
        args_json: args_json.to_string(),
        ack: ack_tx,
    })
    .expect("tool request should be queued");
    ack_rx
}

fn header_node(host: &WidgetHostNative) -> Option<&jian_ops_schema::node::PenNode> {
    host.editor_state()
        .active_children()
        .iter()
        .find(|node| node.base().name.as_deref() == Some("Header"))
}

fn reveal_now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}
