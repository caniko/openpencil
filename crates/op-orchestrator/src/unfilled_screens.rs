//! "Promise-delivery" invariant — a shared, deterministic check for a
//! top-level screen the model (or a scaffold/seed pass) promised but
//! never actually filled.
//!
//! Root-cause postmortem (0718-1-glm-1): a skeleton-first design run
//! planned 3 screens, scaffolded all 3 top-level roots up front, then ran
//! out of turn budget before ever touching the 3rd ("Saved") — which
//! shipped as a bare shell, silently, with no signal anywhere that
//! anything was missing. [`detect_unfilled_screens`] is the shared
//! detector both the agentic loop (`op-host-services::chat_agent_loop`,
//! via the host bridge) and the classic orchestrator (`run.rs`) call at
//! their own completion points; [`mark_unfilled_screens`] is the honest-
//! delivery side effect (renames each hit so the canvas shows it, not
//! just a log line).
//!
//! ## Screen candidates
//!
//! A top-level node is a "screen" candidate when it's a `frame` sized like a
//! mobile screen artboard (`MIN_SCREEN_WIDTH..=MAX_SCREEN_WIDTH` wide, at
//! least `MIN_SCREEN_HEIGHT` tall). The width/height bounds mirror the
//! `MOBILE_ROOT_MAX_WIDTH` convention `geometry_validation.rs` already uses
//! for mobile-root detection. This intentionally excludes desktop multi-root
//! layouts (a dashboard's separate sidebar + content roots are not
//! mobile-shaped) — those were never "promised screens" in the
//! skeleton-first sense this check exists for.
//!
//! Deliberately NOT keyed off `FrameNode.screen.is_some()`, even though the
//! spec's own framing ("带 screen 标记或屏形") suggested it as an OR-branch:
//! `wire_screen_navigation` (`cleanup::finalize_design`, which
//! `apply_loop_finalize`/the classic path's cleanup stage both run before
//! this detector ever sees the document) auto-tags EVERY top-level frame
//! with a `screen` path the moment a document has 2+ top-level roots — it is
//! App-Mode routing plumbing, not a "this was promised/scaffolded as a
//! screen" signal. Trusting it flagged a 1200×64 navbar `Header` and a blank
//! 1200×800 desktop canvas as "unfilled screens" in testing (both carried a
//! `screen` tag, neither is mobile-shaped). Per the parallel-work agreement
//! for this task (skeleton-structure internals are still moving under
//! `wire_screen_navigation.rs`), this stays a pure shape check for now — a
//! follow-up can revisit once that module's contract for "this frame IS a
//! promised screen" (vs. "this frame got auto-tagged for nav routing")
//! stabilizes.
//!
//! ## "Unfilled" — the false-positive risk this is tuned against
//!
//! The obvious criterion — "fewer than N nodes" — was rejected: a
//! genuinely minimal but REAL screen (a background + one heading + one
//! button, 3-4 nodes) must never be flagged just for being small
//! ("稀疏但有真内容不误伤" — sparse-but-real must not be false-flagged).
//! Instead [`has_real_content`] asks a content QUESTION, not a size
//! question: does this subtree contain any non-empty text, any image, or
//! any icon with a real glyph, ANYWHERE beneath it — outside the
//! auto-provided chrome (`role: "status-bar"` / `"bottom-tab-bar"`, which
//! every screen carries regardless of how much the model has done, so
//! their own text/icons must not count as "the model did something
//! here"). A screen with even ONE real content node, however sparse,
//! reads as filled; a screen with only chrome (or literally nothing)
//! reads as unfilled — no node-count threshold needed at all.

use jian_ops_schema::node::PenNode;
use op_editor_core::{EditorState, PenNodeExt};

/// A top-level "screen" root that never received real content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnfilledScreen {
    pub node_id: String,
    pub name: String,
}

/// Roles that are auto-provided chrome, not model effort — present on
/// every screen regardless of how much of it the model actually built.
/// A screen carrying ONLY these subtrees is still, for the model's
/// purposes, unfilled.
const CHROME_ROLES: &[&str] = &["status-bar", "bottom-tab-bar"];

/// Mobile-artboard width band a screen-shaped root falls into, absent an
/// explicit `screen` tag — mirrors `geometry_validation.rs`'s
/// `MOBILE_ROOT_MAX_WIDTH` (480px) convention; `MIN_SCREEN_WIDTH` rules
/// out narrow desktop side-panels that are not full screens.
const MIN_SCREEN_WIDTH: f64 = 300.0;
const MAX_SCREEN_WIDTH: f64 = 480.0;
/// A screen-shaped root is also taller than it is a shallow strip — a
/// full mobile artboard, not a header band.
const MIN_SCREEN_HEIGHT: f64 = 480.0;

/// Every top-level "screen" root in `state`, filled or not — the full
/// "committed" set the promise-delivery contract is judged against. Read-only.
/// Order matches document order (`state.active_children()`).
pub fn list_screen_candidates(state: &EditorState) -> Vec<UnfilledScreen> {
    state
        .active_children()
        .iter()
        .filter(|node| looks_like_a_screen(node))
        .map(screen_hit)
        .collect()
}

/// Detect every top-level "screen" root in `state` that has no real
/// content yet — the subset of [`list_screen_candidates`] still unfilled.
/// Read-only — never mutates the document. Order matches document order
/// (`state.active_children()`).
pub fn detect_unfilled_screens(state: &EditorState) -> Vec<UnfilledScreen> {
    state
        .active_children()
        .iter()
        .filter(|node| looks_like_a_screen(node))
        .filter(|node| !has_real_content(node))
        .map(screen_hit)
        .collect()
}

fn screen_hit(node: &PenNode) -> UnfilledScreen {
    UnfilledScreen {
        node_id: node.id_str().to_string(),
        name: node
            .base()
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| node.id_str().to_string()),
    }
}

/// Idempotently append " (unfilled)" to each hit's node name so the
/// canvas itself carries the signal, not just a transcript line — a
/// user scrolling the layer panel or the canvas must not have to
/// discover a hidden empty screen by clicking into it. Callers apply
/// this ONLY as a FINAL step (after any in-session nudge attempt has
/// already run its course) — marking before a nudge would falsely
/// brand a screen the model was about to fill in the very next turn.
pub fn mark_unfilled_screens(sink: &mut dyn crate::types::DocSink, hits: &[UnfilledScreen]) {
    const SUFFIX: &str = " (unfilled)";
    for hit in hits {
        let Some(node) = op_editor_core::walkers::find_node(
            sink.state().active_children(),
            &op_editor_core::NodeId::new(hit.node_id.clone()),
        ) else {
            continue;
        };
        let current_name = node.base().name.clone().unwrap_or_default();
        if current_name.ends_with(SUFFIX) {
            continue;
        }
        sink.apply(op_editor_core::EditorCommand::UpdateNode {
            node_id: op_editor_core::NodeId::new(hit.node_id.clone()),
            x: None,
            y: None,
            width: None,
            height: None,
            name: Some(format!("{current_name}{SUFFIX}")),
            fill_hex: None,
            page_id: None,
        });
    }
}

/// Detect, then mark, over an already-mutated `EditorState` (the loop
/// host's live document, post `apply_loop_finalize`). Returns the
/// (pre-mark) names for the caller to report. A thin combinator so the
/// host bridge (`op-host-desktop::chat_session`) doesn't need to build
/// its own [`crate::loop_finalize::StateDocSink`] wrapper.
pub fn finalize_and_mark_unfilled_screens(state: &mut EditorState) -> Vec<String> {
    let hits = detect_unfilled_screens(state);
    if hits.is_empty() {
        return Vec::new();
    }
    let names = hits.iter().map(|h| h.name.clone()).collect();
    let mut sink = crate::loop_finalize::StateDocSink { state };
    mark_unfilled_screens(&mut sink, &hits);
    names
}

fn looks_like_a_screen(node: &PenNode) -> bool {
    if !matches!(node, PenNode::Frame(_)) {
        return false;
    }
    let (Some(w), Some(h)) = (node.width_px(), node.height_px()) else {
        return false;
    };
    (MIN_SCREEN_WIDTH..=MAX_SCREEN_WIDTH).contains(&w) && h >= MIN_SCREEN_HEIGHT
}

/// Whether `node`'s OWN role marks it as auto-provided chrome — its
/// subtree is excluded from the content search below.
fn is_chrome_role(node: &PenNode) -> bool {
    node.base()
        .role
        .as_deref()
        .is_some_and(|r| CHROME_ROLES.contains(&r))
}

/// Whether `node` itself (ignoring descendants) is a real content leaf:
/// non-empty text, any image, or an icon with a real glyph name.
fn is_real_leaf_content(node: &PenNode) -> bool {
    match node {
        PenNode::Text(t) => match &t.content {
            jian_ops_schema::node::text::TextContent::Plain(s) => !s.trim().is_empty(),
            jian_ops_schema::node::text::TextContent::Styled(segments) => !segments.is_empty(),
        },
        PenNode::Image(_) => true,
        PenNode::IconFont(icon) => !icon.icon_font_name.trim().is_empty(),
        _ => false,
    }
}

/// Whether `node`'s subtree (itself + descendants) contains any real
/// content, skipping auto-chrome subtrees entirely.
fn has_real_content(node: &PenNode) -> bool {
    if is_chrome_role(node) {
        return false;
    }
    if is_real_leaf_content(node) {
        return true;
    }
    node.children().into_iter().flatten().any(has_real_content)
}

#[cfg(test)]
#[path = "unfilled_screens_tests.rs"]
mod tests;
