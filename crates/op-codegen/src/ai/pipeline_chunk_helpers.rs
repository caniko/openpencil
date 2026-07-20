//! Dependency and node-slicing helpers for chunk generation.

use super::{ChunkStatus, CodegenPipeline};
use crate::ai::pipeline_preflight;
use crate::ai::types::ChunkContract;

impl CodegenPipeline {
    pub(super) fn collect_dep_contracts(&self, deps: &[String]) -> Vec<ChunkContract> {
        deps.iter()
            .filter_map(|dep_id| {
                self.chunks
                    .iter()
                    .find(|chunk| chunk.exec.plan.id == *dep_id)
                    .and_then(|chunk| chunk.result.as_ref())
                    .map(|result| result.contract.clone())
                    .filter(|contract| !contract.component_name.is_empty())
            })
            .collect()
    }

    pub(super) fn dep_status(&self, dep_id: &str) -> Option<ChunkStatus> {
        self.chunks
            .iter()
            .find(|chunk| chunk.exec.plan.id == dep_id)
            .map(|chunk| chunk.status)
    }

    /// Resolve each id recursively and fall back to the full tree only when
    /// none resolve.
    pub(super) fn chunk_nodes_json(&self, node_ids: &[String]) -> String {
        pipeline_preflight::chunk_nodes_json(
            &self.sanitized_nodes_value,
            &self.sanitized_nodes_json,
            node_ids,
        )
    }

    pub(super) fn all_chunks_terminal(&self) -> bool {
        self.chunks.iter().all(|chunk| chunk.is_terminal())
    }

    pub(super) fn lowest_incomplete_order(&self) -> Option<usize> {
        self.chunks
            .iter()
            .filter(|chunk| !chunk.is_terminal())
            .map(|chunk| chunk.order())
            .min()
    }
}
