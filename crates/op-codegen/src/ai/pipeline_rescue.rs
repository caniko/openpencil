//! Rescue and terminal fallback paths for [`CodegenPipeline`].

use super::{CodegenPipeline, InFlight, Phase};
use crate::ai::deterministic_fallback;
use crate::ai::parse::clean_code;
use crate::ai::pipeline_preflight::{model_output_failure, MAX_USER_PROMPT_BYTES};
use crate::ai::prompts::rescue_request;
use crate::ai::types::{PipelineStep, RequestKind};

impl CodegenPipeline {
    pub(super) fn step_rescue(&mut self) -> PipelineStep {
        if let Some((_id, flight)) = self.take_settled_inflight() {
            return self.resolve_rescue(flight);
        }
        if self.has_inflight() {
            return PipelineStep::Waiting;
        }
        let asset_paths = self
            .assets
            .iter()
            .map(|asset| asset.relative_path.clone())
            .collect::<Vec<_>>();
        let failure_summary = self.failure_summary();
        let id = self.register_inflight(RequestKind::Assembly);
        let req = rescue_request(
            id,
            &self.sanitized_nodes_json,
            &failure_summary,
            &self.input,
            &asset_paths,
        );
        if req.user_message.len() > MAX_USER_PROMPT_BYTES {
            self.in_flight.remove(&id);
            self.record_failure(
                "single-file rescue",
                format!(
                    "prompt is {} bytes (limit {MAX_USER_PROMPT_BYTES}); request was not dispatched",
                    req.user_message.len()
                ),
            );
            return self.finish_with_deterministic_fallback();
        }
        self.assembly_done = Some(false);
        PipelineStep::Dispatch(vec![req])
    }

    fn resolve_rescue(&mut self, flight: InFlight) -> PipelineStep {
        if let Some(message) = flight.error {
            self.record_failure("single-file rescue", message);
            return self.finish_with_deterministic_fallback();
        }
        let code = clean_code(&flight.buffer);
        if let Some(message) = model_output_failure(self.input.framework, &code) {
            self.record_failure("single-file rescue", message);
            return self.finish_with_deterministic_fallback();
        }
        self.assembly_done = Some(true);
        let step = PipelineStep::Done {
            code,
            degraded: true,
            assets: self.assets.clone(),
        };
        self.phase = Phase::Terminal(step.clone());
        step
    }

    fn finish_with_deterministic_fallback(&mut self) -> PipelineStep {
        match deterministic_fallback::generate(
            &self.sanitized_nodes_json,
            self.input.variables_json.as_deref(),
            self.input.framework,
        ) {
            Ok(code) => {
                self.assembly_done = Some(true);
                let step = PipelineStep::Done {
                    code,
                    degraded: true,
                    assets: self.assets.clone(),
                };
                self.phase = Phase::Terminal(step.clone());
                step
            }
            Err(message) => {
                self.record_failure("deterministic fallback", message);
                self.fail_with_history("Code generation failed after every fallback")
            }
        }
    }

    pub(super) fn record_failure(&mut self, stage: impl AsRef<str>, reason: impl AsRef<str>) {
        self.failures
            .push(format!("{}: {}", stage.as_ref(), reason.as_ref()));
    }

    fn failure_summary(&self) -> String {
        if self.failures.is_empty() {
            return "- no usable intermediate output".to_string();
        }
        self.failures
            .iter()
            .map(|failure| format!("- {failure}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn fail_with_history(&mut self, heading: &str) -> PipelineStep {
        self.assembly_done = Some(false);
        let step = PipelineStep::Failed {
            message: format!("{heading}. Failure history:\n{}", self.failure_summary()),
        };
        self.phase = Phase::Terminal(step.clone());
        step
    }
}
