//! Dependency-health helpers for the in-memory codegen plan store.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value;

use super::{chunk_dependencies, chunk_id, topo_sort};

/// Effective blockers derived from local submission statuses.
///
/// `blocked` is derived rather than persisted over local submission state.
/// Retrying an ancestor therefore unblocks pending descendants; successful
/// descendant results are separately invalidated when an ancestor changes.
pub(super) struct DependencyState {
    blockers: HashMap<String, Vec<String>>,
}

impl DependencyState {
    pub(super) fn resolve(chunks: &[Value], statuses: &HashMap<String, String>) -> Self {
        let mut blockers: HashMap<String, Vec<String>> = HashMap::new();

        for chunk in topo_sort(chunks) {
            let id = chunk_id(chunk).unwrap_or("?");
            let mut roots = Vec::new();
            let mut seen = HashSet::new();
            for dependency in chunk_dependencies(chunk).map(Vec::as_slice).unwrap_or(&[]) {
                let dependency_id = dependency.as_str().unwrap_or("");
                match statuses.get(dependency_id).map(String::as_str) {
                    Some("failed") | Some("skipped") => {
                        if seen.insert(dependency_id.to_string()) {
                            roots.push(dependency_id.to_string());
                        }
                    }
                    _ => {
                        for root in blockers
                            .get(dependency_id)
                            .map(Vec::as_slice)
                            .unwrap_or(&[])
                        {
                            if seen.insert(root.clone()) {
                                roots.push(root.clone());
                            }
                        }
                    }
                }
            }
            if !roots.is_empty() {
                blockers.insert(id.to_string(), roots);
            }
        }

        Self { blockers }
    }

    pub(super) fn blockers(&self, chunk_id: &str) -> &[String] {
        self.blockers
            .get(chunk_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return every transitive dependent in breadth-first dependency order.
    pub(super) fn dependents(chunks: &[Value], root_id: &str) -> Vec<String> {
        let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
        for chunk in chunks {
            let id = chunk_id(chunk).unwrap_or("?");
            for dependency in chunk_dependencies(chunk).map(Vec::as_slice).unwrap_or(&[]) {
                adjacency
                    .entry(dependency.as_str().unwrap_or(""))
                    .or_default()
                    .push(id);
            }
        }

        let mut queue = VecDeque::from([root_id]);
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        while let Some(id) = queue.pop_front() {
            for dependent in adjacency.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                if seen.insert((*dependent).to_string()) {
                    result.push((*dependent).to_string());
                    queue.push_back(dependent);
                }
            }
        }
        result
    }

    pub(super) fn effective_status<'a>(&self, chunk_id: &str, local_status: &'a str) -> &'a str {
        if !matches!(local_status, "failed" | "skipped") && !self.blockers(chunk_id).is_empty() {
            "blocked"
        } else {
            local_status
        }
    }

    pub(super) fn is_retryable(
        &self,
        chunk_id: &str,
        local_status: &str,
        statuses: &HashMap<String, String>,
    ) -> bool {
        match self.effective_status(chunk_id, local_status) {
            "failed" | "degraded" => true,
            "blocked" => self
                .blockers(chunk_id)
                .iter()
                .any(|root| statuses.get(root).map(String::as_str) == Some("failed")),
            _ => false,
        }
    }
}
