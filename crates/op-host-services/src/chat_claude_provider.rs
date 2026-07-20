//! `ChatProvider` adapter and transport-level cancellation for Claude Code.

use super::*;

pub(super) fn unexpected_stream_end_deltas() -> [ChatDelta; 2] {
    [
        ChatDelta::Error("claude stream ended before a terminal result".into()),
        ChatDelta::Done {
            stop_reason: StopReason::Aborted,
        },
    ]
}

impl ChatProvider for ClaudeCodeProvider {
    fn provider_label(&self) -> &str {
        &self.label
    }

    fn send(&self, request: ChatRequest) -> Box<dyn Iterator<Item = ChatDelta> + Send> {
        self.send_inner(request, None)
    }

    fn send_cancellable(
        &self,
        request: ChatRequest,
        cancel: Arc<AtomicBool>,
    ) -> Box<dyn Iterator<Item = ChatDelta> + Send> {
        self.send_inner(request, Some(cancel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use op_ai::chat_provider::{EffortLevel, ThinkingMode};

    #[test]
    fn options_env_never_carries_path() {
        // The SDK rejects PATH in options.env as dangerous; GUI PATH repair
        // happens process-wide before the provider is constructed.
        let request = ChatRequest {
            system_prompt: "design something".into(),
            user_message: "hi".into(),
            history: vec![],
            max_output_tokens: 1024,
            thinking: ThinkingMode::Adaptive,
            effort: EffortLevel::Low,
            attachments: vec![],
            model: None,
        };
        let options = effective_options(None, &request, None);
        assert!(
            !options.env.contains_key("PATH"),
            "PATH must never ride options.env: {:?}",
            options.env.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn unexpected_stream_end_aborts_instead_of_ending_turn() {
        let deltas = unexpected_stream_end_deltas();
        assert!(matches!(
            deltas.first(),
            Some(ChatDelta::Error(message)) if message.contains("terminal result")
        ));
        assert!(matches!(
            deltas.get(1),
            Some(ChatDelta::Done {
                stop_reason: StopReason::Aborted
            })
        ));
        assert!(!deltas.iter().any(|delta| matches!(
            delta,
            ChatDelta::Done {
                stop_reason: StopReason::EndTurn
            }
        )));
    }
}
