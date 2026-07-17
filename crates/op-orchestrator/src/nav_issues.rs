//! Track B of the interactive-preview plan — the `navIssues` per-batch echo
//! (see
//! `openpencil-docs/openpencil/generation/preview-interactive-app-mode-0712.md`,
//! Track B item 2). `wire_screen_navigation` (Track A) is the deterministic
//! backstop that fills unbound nav tabs once the loop finalizes; this scan
//! runs IN-LOOP, right after a `batch_design` write, so the model can see and
//! fix its own unbound navigation while the design is still open instead of
//! only being caught by Track A's end-of-run repair.
//!
//! Detection: once a document has **≥ 2 top-level frames the model itself
//! already marked with `screen`**, any bottom-tab-bar / sidebar-nav item
//! whose label name-matches one of those screens but carries no `events` yet
//! is flagged, naming the tab's node id and the exact `events.onTap` patch it
//! should bind. This is an ECHO ONLY — it never mutates the document. Which
//! tab a model intends to route where is INTENT, not a structural defect
//! (same "回波不硬修" discipline as the other intent-shaped echoes in
//! `design_agent_tools.rs`), and Track A repairs it anyway if the model never
//! gets to it.
//!
//! Reuses `wire_screen_navigation`'s own nav-container / label-matching /
//! events-presence helpers (`pub(crate)`) so the echo and the write pass can
//! never disagree about what counts as "an unbound matching tab".

use jian_ops_schema::node::PenNode;
use op_editor_core::{EditorState, PenNodeExt};

use crate::wire_screen_navigation::{
    collect_nav_containers, first_text_content, labels_match, node_has_events, normalize_label,
};

/// A top-level `Frame` already carrying an authored `screen` route.
struct MarkedScreen<'a> {
    node: &'a PenNode,
    name: String,
    path: String,
}

/// Top-level `Frame` children that already carry an authored `screen` marker
/// — i.e. screens the model itself has already committed to routing, as
/// opposed to Track A's own width/height-shape heuristic (this echo takes no
/// position on frames the model hasn't marked yet).
fn marked_screens(nodes: &[PenNode]) -> Vec<MarkedScreen<'_>> {
    nodes
        .iter()
        .filter_map(|node| {
            let PenNode::Frame(frame) = node else {
                return None;
            };
            let path = frame.screen.clone()?;
            let name = frame
                .base
                .name
                .clone()
                .unwrap_or_else(|| frame.base.id.clone());
            Some(MarkedScreen { node, name, path })
        })
        .collect()
}

/// Scan the active page for nav-tab items that name-match an already
/// screen-marked frame but carry no `events` yet. Returns one human-readable
/// line per unbound-but-matched tab, naming the node id, the screen it sits
/// on, and the exact `events.onTap` patch to bind. No-ops (returns empty)
/// when fewer than two top-level frames are screen-marked — mirrors
/// `wire_screen_navigation`'s own multi-screen gate: a single marked screen
/// has no navigation target to check against yet.
pub fn scan_nav_issues(state: &EditorState) -> Vec<String> {
    let screens = marked_screens(state.active_children());
    if screens.len() < 2 {
        return Vec::new();
    }
    let targets: Vec<(String, String)> = screens
        .iter()
        .map(|s| (normalize_label(&s.name), s.path.clone()))
        .collect();

    let mut issues = Vec::new();
    for screen in &screens {
        let mut nav_containers = Vec::new();
        collect_nav_containers(screen.node, &mut nav_containers);
        for nav in nav_containers {
            let Some(items) = nav.children() else {
                continue;
            };
            for item in items {
                if node_has_events(item) {
                    continue;
                }
                let Some(label) = first_text_content(item) else {
                    continue;
                };
                let tab_key = normalize_label(label);
                let Some((_, target_path)) = targets
                    .iter()
                    .find(|(screen_key, _)| labels_match(&tab_key, screen_key))
                else {
                    continue;
                };
                let item_id = item.id_str();
                let screen_path = &screen.path;
                issues.push(format!(
                    "{item_id} (\"{label}\") on screen \"{screen_path}\" is not bound to \
                     events.onTap yet - bind events:{{\"onTap\":[{{\"replace\":\"\\\"{target_path}\\\"\"}}]}} \
                     so tapping it navigates to that screen"
                ));
            }
        }
    }
    issues.sort();
    issues.truncate(8);
    issues
}

#[cfg(test)]
#[path = "nav_issues_tests.rs"]
mod tests;
