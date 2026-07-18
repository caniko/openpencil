//! Multi-screen root position deconfliction — the deterministic net for a
//! failure mode that READS as "two screens stacked into one frame" but is
//! actually "two (or more) perfectly well-formed, single-nav screens with
//! no canvas position, silently overlapping at the same default origin".
//!
//! Measured (`0718-1-glm.op`, GLM-5.2 built-in loop provider, "Wander"
//! travel app, 3-screen prompt): "Trips Overview" / "Destination Detail" /
//! "Saved Places" are three CLEAN top-level frames, each with exactly one
//! bottom nav and correct internal structure — but all three carry
//! `x: None, y: None`. Every unpositioned root resolves to the SAME (0, 0)
//! origin (`op layout` on the raw file confirms all three land at
//! `x=0, y=0`), so on the infinite canvas they render on top of each
//! other. Because the three screens have different total heights
//! (864 / 844 / 1155), each one's OWN bottom nav lands at a different
//! absolute Y — from a canvas screenshot this reads exactly like "two
//! bottom navs stacked inside one giant frame", but there is no single
//! frame containing two screens' content to split apart; the document
//! already has the right SHAPE, it just needs the roots pulled apart.
//!
//! Root cause: the classic multi-screen path (`run_screen_groups.rs`)
//! deterministically places every new root via `next_root_insert_position`
//! plus a fixed gap (`run.rs`'s `FOLLOW_ON_ROOT_GAP` / `scaffold.rs`'s
//! `SCREEN_GROUP_GAP`, both 80px) — it can never produce this shape. The
//! agentic LOOP path instead relies on the model calling `find_empty_space`
//! and authoring `x`/`y` itself when it starts a new screen
//! (`design_agent_tools.rs`'s Screen-2+ protocol) — a model-behavior
//! contract, not a deterministic guarantee, and GLM-5.2 simply didn't
//! honor it here. This pass is the same deterministic net the classic
//! path has always had, applied post-hoc so it catches ANY producer (loop,
//! classic, or a future one) that leaves screen roots unpositioned —
//! reusing the same established gap so a loop-produced and a
//! classic-produced multi-screen document lay out identically either way.
//!
//! Deliberately conservative: a single screen-shaped root, or several
//! non-overlapping ones, are left completely alone — a long single-column
//! scrolling page is a legitimate shape, not a bug.

use jian_ops_schema::node::PenNode;
use op_editor_core::{EditorCommand, NodeId, PenNodeExt};

use crate::types::DocSink;

/// Mirrors `abandoned_duplicate_roots`'s `MIN_FULLSCREEN_WIDTH` /
/// `MIN_FULLSCREEN_HEIGHT` — the same "is this top-level frame screen
/// shaped" gate, so the two passes agree on what counts as a screen root.
const MIN_SCREEN_WIDTH: f64 = 320.0;
const MIN_SCREEN_HEIGHT: f64 = 500.0;

/// `run.rs`'s `FOLLOW_ON_ROOT_GAP` / `scaffold.rs`'s `SCREEN_GROUP_GAP` —
/// the established "next screen sits this far to the right" convention
/// (both currently 80px). Kept as an independent constant rather than
/// importing `run.rs`'s (module-private, and semantically a different
/// phase: that one picks a slot for a single NEW insertion mid-generation,
/// this one repairs N already-inserted roots after the fact) — value
/// parity with the existing convention is what matters here, not literal
/// code sharing.
const SCREEN_ROOT_GAP: f64 = 80.0;
/// `run.rs`'s `DEFAULT_ROOT_Y` — the shared baseline an unpositioned
/// screen lands on when it has to move.
const DEFAULT_Y: f64 = 40.0;

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn right(&self) -> f64 {
        self.x + self.w
    }

    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

struct ScreenRoot {
    id: String,
    rect: Rect,
    /// Whether `x`/`y` were actually authored vs. defaulted to 0 for this
    /// check — an authored root keeps its Y if it has to move; a
    /// never-positioned one falls onto the shared `DEFAULT_Y` baseline.
    positioned: bool,
}

fn is_screen_shaped(width: Option<f64>, height: Option<f64>) -> bool {
    width.is_some_and(|w| w >= MIN_SCREEN_WIDTH) && height.is_some_and(|h| h >= MIN_SCREEN_HEIGHT)
}

fn screen_root_candidate(node: &PenNode) -> Option<ScreenRoot> {
    if !matches!(node, PenNode::Frame(_)) {
        return None;
    }
    let width = node.width_px();
    let height = node.height_px();
    if !is_screen_shaped(width, height) {
        return None;
    }
    let x = node.base().x;
    let y = node.base().y;
    Some(ScreenRoot {
        id: node.id_str().to_string(),
        rect: Rect {
            x: x.unwrap_or(0.0),
            y: y.unwrap_or(0.0),
            w: width?,
            h: height?,
        },
        positioned: x.is_some() || y.is_some(),
    })
}

/// Detect screen-shaped top-level roots that overlap — most commonly
/// because one or more was never given a canvas position — and spread
/// them into a left-to-right row using the established screen-gap
/// convention. Returns the number of roots moved (0 in the common case:
/// a single screen, or several already-distinct ones, both left
/// untouched).
pub(crate) fn spread_overlapping_screen_roots(sink: &mut dyn DocSink) -> usize {
    let candidates: Vec<ScreenRoot> = sink
        .state()
        .active_children()
        .iter()
        .filter_map(screen_root_candidate)
        .collect();
    if candidates.len() < 2 {
        return 0;
    }

    // Nothing to do unless at least one pair actually overlaps — a
    // healthy multi-screen document (classic path, or a loop run that DID
    // position its screens) must be left byte-for-byte alone.
    let any_overlap = candidates.iter().enumerate().any(|(i, a)| {
        candidates[i + 1..]
            .iter()
            .any(|b| a.rect.intersects(&b.rect))
    });
    if !any_overlap {
        return 0;
    }

    // Sweep in document order: the first root stays wherever it is (moving
    // it too would be arbitrary — there is no "more correct" anchor to
    // prefer), then every SUBSEQUENT root that overlaps anything placed so
    // far goes immediately right of the rightmost edge seen. A root that
    // already sits clear of everything placed before it is left alone even
    // if it happens to be unpositioned — absent x/y isn't itself the bug,
    // only an actual collision is.
    let mut placed: Vec<Rect> = Vec::new();
    let mut rightmost = f64::MIN;
    let mut ops: Vec<EditorCommand> = Vec::new();
    let mut moved_ids: Vec<String> = Vec::new();

    for candidate in &candidates {
        let overlaps_placed = placed.iter().any(|p| p.intersects(&candidate.rect));
        let rect = if overlaps_placed {
            let new_x = rightmost + SCREEN_ROOT_GAP;
            let new_y = if candidate.positioned {
                candidate.rect.y
            } else {
                DEFAULT_Y
            };
            ops.push(EditorCommand::UpdateNode {
                node_id: NodeId::new(candidate.id.clone()),
                x: Some(new_x.round() as i32),
                y: Some(new_y.round() as i32),
                width: None,
                height: None,
                name: None,
                fill_hex: None,
                page_id: None,
            });
            moved_ids.push(candidate.id.clone());
            Rect {
                x: new_x,
                y: new_y,
                w: candidate.rect.w,
                h: candidate.rect.h,
            }
        } else {
            candidate.rect
        };
        rightmost = rightmost.max(rect.right());
        placed.push(rect);
    }

    if ops.is_empty() {
        return 0;
    }
    let moved = ops.len();
    tracing::info!(
        moved,
        ids = ?moved_ids,
        "spread_overlapping_screen_roots: {moved} screen root(s) overlapped at an unauthored canvas position, spread left-to-right"
    );
    for op in ops {
        sink.apply(op);
    }
    moved
}

#[cfg(test)]
#[path = "spread_screen_roots_tests.rs"]
mod tests;
