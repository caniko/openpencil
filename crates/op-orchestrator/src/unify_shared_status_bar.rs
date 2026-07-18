//! Deterministic cross-screen status-bar chrome unification — [C from the
//! 0718-1-k3-1 postmortem] fixes screens missing the pinned status bar
//! entirely (measured: two of three screens had no status-bar subtree at
//! all while the third carried one) by cloning a reference screen's status
//! bar onto every OTHER mobile screen-shaped root that lacks one.
//!
//! Sibling pass to [`crate::unify_shared_nav`] — same "reuse, don't redraw"
//! shape — run immediately after it in `cleanup::run_cleanup_passes`, so
//! both the classic and loop-finalize paths pick it up through the one
//! shared choke point. Status bars and bottom-nav are both shared chrome
//! every screen should carry identically; splitting them into two passes
//! (rather than one pass unifying "all chrome") keeps each pass's
//! reference-detection + injection logic independently simple and
//! testable — status bars key off [`crate::cleanup::is_status_bar`]
//! (name/id substring), bottom-nav keys off `collect_nav_containers`
//! (role + tab-item structure), and the two shapes never overlap.
//!
//! ## Screen-shaped root gate
//!
//! Reuses [`crate::unfilled_screens::list_screen_candidates`]'s exact
//! mobile-artboard shape check (width/height band) rather than re-deriving
//! it — one definition of "this top-level root is a screen" shared across
//! both passes that need it.
//!
//! ## Never fabricates chrome from nothing
//!
//! If NO screen in the document carries a status bar, the whole pass is a
//! no-op — there's no reference to clone, and this pass never invents a
//! status bar that no screen in the document ever had.
//!
//! ## Injected clones must carry `role: "status-bar"` (0718-1-k3-1 review fix)
//!
//! Reference detection here is name/id-based ([`is_status_bar`]), but
//! `unfilled_screens.rs`'s chrome exclusion (`CHROME_ROLES`) is role-based —
//! an authored status bar frequently has no `role` at all. Left unstamped,
//! an injected clone's own text children (time/battery) would read as "real
//! content" to the promise-delivery check, silently flipping a genuinely
//! unfilled screen to "filled" the moment it gets a status bar — the
//! opposite of honest: the fix for one symptom (missing chrome) would have
//! quietly broken a DIFFERENT, more important invariant (never lie about
//! delivery). [`stamp_chrome_role`] force-sets the role on the CLONE only —
//! the authored reference node is never touched.
//!
//! ## Roadmap: geometry-based reference detection (recorded, not this batch)
//!
//! Reference-screen detection here is a structural/name check
//! ([`is_status_bar`]), not the resolved-layout geometric judgment
//! `op-host-native/src/preview/present.rs::pinned_status_bar_candidate`
//! uses (flush-to-top, ≥90% root width, ≤60px tall, against REAL post-
//! layout `SceneNode` bounds). The clone comes from an already-authored
//! sibling screen's real status bar, so it almost certainly satisfies that
//! same shape too — a mismatch would need the target root's own padding to
//! push the clone's resolved top offset outside the pin tolerance. Revisit
//! with a `op_pen_loader::editor_state_to_layout_scene`-based check (see
//! `wire_screen_navigation.rs`'s `resolved_y_offsets` for the same
//! infrastructure already proven inside this crate) if a real case turns
//! up where an injected status bar doesn't actually pin in preview.

use jian_ops_schema::node::PenNode;
use op_editor_core::{EditorCommand, NodeId, PenNodeExt};

use crate::cleanup::is_status_bar;
use crate::types::DocSink;
use crate::unfilled_screens::{list_screen_candidates, UnfilledScreen};

/// Entry point. No-ops when fewer than 2 mobile screen-shaped roots exist,
/// or when no screen carries a status bar at all.
pub fn unify_shared_status_bar(sink: &mut dyn DocSink) {
    let screens = list_screen_candidates(sink.state());
    if screens.len() < 2 {
        return;
    }

    let Some((reference_id, reference_bar)) = find_reference_status_bar(sink, &screens) else {
        return;
    };

    for screen in &screens {
        if screen.node_id == reference_id {
            continue; // the reference screen keeps its own status bar.
        }
        let Some(root) = op_editor_core::walkers::find_node(
            sink.state().active_children(),
            &NodeId::new(screen.node_id.clone()),
        ) else {
            continue;
        };
        if direct_status_bar_child(root).is_some() {
            continue; // idempotent: already carries its own status bar.
        }

        let mut clone = reference_bar.clone();
        stamp_chrome_role(&mut clone);

        // `InsertSubtree` APPENDS to the target parent's children, so the
        // clone lands as the LAST child first — reposition it to the FIRST
        // slot afterward so it pins to the top like every authored status
        // bar does. Mirrors `cleanup_mobile_chrome::anchor_bottom_nav_last`'s
        // own "move within the same parent" idiom for the opposite end.
        sink.apply(EditorCommand::InsertSubtree {
            nodes: vec![clone],
            parent_id: NodeId::new(screen.node_id.clone()),
            page_id: None,
        });
        let Some(root) = op_editor_core::walkers::find_node(
            sink.state().active_children(),
            &NodeId::new(screen.node_id.clone()),
        ) else {
            continue;
        };
        let Some(new_child) = root.children().and_then(|c| c.last()) else {
            continue;
        };
        let new_id = NodeId::new(new_child.id_str().to_string());
        sink.apply(EditorCommand::MoveNode {
            node_id: new_id,
            target_parent: NodeId::new(screen.node_id.clone()),
            page_id: None,
            index: Some(0),
        });
    }
}

/// Find the document-order first screen with a direct status-bar child,
/// returning its id + a CLONE of that status bar (owned, so the caller can
/// drop the `sink.state()` borrow before mutating).
fn find_reference_status_bar(
    sink: &dyn DocSink,
    screens: &[UnfilledScreen],
) -> Option<(String, PenNode)> {
    for screen in screens {
        let root = op_editor_core::walkers::find_node(
            sink.state().active_children(),
            &NodeId::new(screen.node_id.clone()),
        )?;
        if let Some(bar) = direct_status_bar_child(root) {
            return Some((screen.node_id.clone(), bar.clone()));
        }
    }
    None
}

/// The first DIRECT child of `root` that reads as a status bar (name/id
/// substring match — see [`is_status_bar`]). Direct-child only, matching
/// `cleanup::remove_duplicate_status_bars`'s own scan depth: a status bar
/// is authored as a top-level sibling of the screen's content, not nested.
fn direct_status_bar_child(root: &PenNode) -> Option<&PenNode> {
    root.children()?.iter().find(|c| is_status_bar(c))
}

/// Force `role: "status-bar"` onto `node` if it doesn't already carry a
/// role — see the module doc's "Injected clones must carry role" section.
/// Only ever called on an owned CLONE, never on the authored reference.
fn stamp_chrome_role(node: &mut PenNode) {
    if node.base().role.is_none() {
        node.base_mut().role = Some("status-bar".to_string());
    }
}

#[cfg(test)]
#[path = "unify_shared_status_bar_tests.rs"]
mod tests;
