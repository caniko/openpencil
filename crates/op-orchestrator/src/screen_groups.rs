//! Screen-grouping — Track A's fix for the classic Orchestrator's
//! single-root collapse (see the interactive-preview plan follow-up,
//! 2026-07-17): `plan_normalize::normalize` used to fold EVERY subtask onto
//! the plan's one `root_frame.id`, regardless of a subtask's own `screen`
//! label. A plan whose subtasks span ≥2 distinct screens (e.g. "generate the
//! remaining 3 pages" against an existing app) rendered as one flat
//! top-level frame instead of N separate screen roots.
//!
//! [`group_subtasks_by_screen`] is a straight revival of the identically
//! named function `aca0d3a0` deleted (2026-07-02, "remove … concurrent
//! multi-screen path") — it is PURE partitioning logic with no concurrency
//! attached, so reviving it does NOT revive the concurrent worker machinery
//! that commit also removed (deliberately out of scope here — see the
//! commit's own data verdict: concurrency bought no measured quality, only
//! wall-clock). `run.rs` now drives the resulting groups strictly
//! SEQUENTIALLY, one screen after another, through N scaffold roots instead
//! of the removed N concurrent workers.

use crate::plan::Subtask;

/// A group of subtask indices that share the same screen.
/// `screen` is the screen name; `indices` are into the plan's `subtasks` slice.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenGroup {
    pub screen: String,
    pub indices: Vec<usize>,
}

/// Groups subtasks by screen.
///
/// Rules:
/// - `first_screen` = the `screen` of the first subtask that has one, else
///   `"page"` (unreachable in practice — the `has_any_screen` gate below
///   only lets this branch run when at least one subtask DOES have a
///   `screen`, but the fallback stays for defensive completeness).
/// - A subtask with no `screen` falls back to `first_screen` — an untagged
///   subtask (e.g. a shared bottom-nav injected by
///   `plan_normalize::ensure_requested_bottom_nav_subtask`) joins whichever
///   group is "the main screen" rather than starting its own singleton group.
/// - Group order = first-seen order of distinct screen values.
/// - If NO subtask has a `screen`, returns an EMPTY `Vec` — the caller
///   (`plan_normalize::normalize`, `run.rs`) then takes the single-root path,
///   byte-identical to today's behavior. A single distinct screen value
///   across every tagged subtask also naturally collapses to exactly one
///   group, so callers gate multi-root specifically on `groups.len() > 1`.
pub fn group_subtasks_by_screen(subtasks: &[Subtask]) -> Vec<ScreenGroup> {
    let has_any_screen = subtasks.iter().any(|st| st.screen.is_some());
    if !has_any_screen {
        return vec![];
    }

    let first_screen: String = subtasks
        .iter()
        .find(|st| st.screen.is_some())
        .and_then(|st| st.screen.clone())
        .unwrap_or_else(|| "page".to_string());

    let mut groups: Vec<ScreenGroup> = Vec::new();
    let mut screen_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for (i, subtask) in subtasks.iter().enumerate() {
        let screen = subtask
            .screen
            .clone()
            .unwrap_or_else(|| first_screen.clone());

        if let Some(&group_idx) = screen_map.get(&screen) {
            groups[group_idx].indices.push(i);
        } else {
            screen_map.insert(screen.clone(), groups.len());
            groups.push(ScreenGroup {
                screen,
                indices: vec![i],
            });
        }
    }

    groups
}

#[cfg(test)]
#[path = "screen_groups_tests.rs"]
mod tests;
