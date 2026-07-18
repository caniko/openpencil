//! Deterministic cross-screen shared-chrome unification — fixes the "each
//! screen redraws its own bottom-nav → icons/labels drift between screens"
//! bug (measured: the Home screen's nav read Home/Search/Library/Premium;
//! the Library screen's OWN redraw of the "same" nav came out
//! Home/Search/**Your Library**/Premium — a different icon+label SET,
//! wasting generation budget on a component that should be one shared
//! object, not N independently re-authored ones).
//!
//! Runs in `cleanup::run_cleanup_passes`, **BEFORE**
//! `wire_screen_navigation` (Track A): Track A's nav-tab binding matches tab
//! LABELS against screen names, so it must see the post-unification tree
//! (every screen's nav sharing the SAME label set) — running it first would
//! bind against whatever pre-unification labels each screen happened to
//! have, some of which are about to be replaced anyway.
//!
//! ## Missing-nav screens: inject, don't just skip (2026-07 upgrade)
//!
//! The first cut of this pass only SWAPPED an already-present nav for the
//! reference's — a screen with NO nav at all (measured: a sub-agent's
//! "Bottom Navigation Bar" subtask outright failed and left the screen
//! nav-less) was left untouched under a "don't force chrome onto a screen
//! that may not want it" rule. That contradicted `decomposition.md`'s own
//! teaching (this pass's sibling change) — telling the model it's safe to
//! leave later screens' nav slot EMPTY because "the system unifies it" is a
//! promise this pass didn't keep for the empty case.
//!
//! The reconciled rule: a nav-less screen gets the reference nav INJECTED
//! when the reference nav's own tab set already declares a tab for that
//! screen (label/name normalize-match) — the nav's tab set is read as its
//! own contract for which screens it serves. A screen whose name matches
//! no tab (e.g. a standalone detail page with no nav on ANY screen) is
//! still never forced to grow one. See [`resolve_target`].
//!
//! ## Active-tab idempotency gap (0718-1-k3-1 postmortem, D)
//!
//! [`labels_already_unified`] only compares the tab LABEL set — it says
//! nothing about which tab is active. A model that draws every screen's nav
//! byte-for-byte identical (measured: three screens, one nav, one active
//! tab baked into all three) short-circuits `resolve_target` into `None`
//! before `retarget_active_tab` ever runs, so the wrong tab reads active on
//! every non-reference screen. [`active_tab_is_correct`] is the missing
//! second half of the idempotency check: labels matching is necessary but
//! not sufficient. When it disagrees, [`SyncTarget::RetargetActiveOnly`]
//! fixes just the active styling in place — no full nav replacement, so any
//! other authored drift on that screen's own nav (icons, ids) survives
//! untouched.
//!
//! ## Detail-page Inject exemption (0718-1-k3-1 postmortem, product decision)
//!
//! A push-in detail screen (opened by tapping a card/row on another screen,
//! with a header Back control, and no corresponding bottom-nav tab) should
//! never get a bottom-nav forced onto it — `decomposition.md` / `design-
//! agent.md` now teach the model this directly, but the deterministic
//! backstop lives here: [`resolve_target`]'s `Inject` branch additionally
//! checks [`wire_screen_navigation::screen_has_back_control_in_header`] so a
//! screen whose bare name happens to token-match a reference tab (see
//! `labels_match`'s token fallback) doesn't get chrome it structurally
//! signals it doesn't want. Deliberately Inject-only — an AUTHORED nav
//! (`Replace` / `RetargetActiveOnly`) is never removed; this pass does not
//! delete what the model deliberately drew. Back-header-only (not also
//! requiring a label mismatch): `resolve_target`'s own `eligible` check
//! already requires a label match before `Inject` is ever considered, so a
//! literal "no match AND back header" gate would be unreachable dead code
//! (reviewed + confirmed).
//!
//! ## Roadmap note
//!
//! This is the deterministic stand-in for shared chrome being a proper
//! COMPONENT/ref instance (multi-screen-consistency roadmap item C1). Once
//! bottom-nav / sidebar authoring goes through `RefNode` instancing, this
//! whole pass retires — shared chrome would just BE one component
//! referenced from every screen, with no post-hoc unification needed. Until
//! then, this pass is the deterministic backstop.

use jian_ops_schema::node::{PenNode, TextContent};
use op_editor_core::{EditorCommand, NodeId, PenNodeExt};

use crate::types::DocSink;
use crate::wire_screen_navigation::{
    collect_nav_containers, collect_screen_candidates, first_text_content, labels_match,
    normalize_label, screen_has_back_control_in_header, ScreenCandidate,
};

/// What a non-reference screen needs, resolved read-only before any
/// mutation (see [`resolve_target`]).
enum SyncTarget {
    /// This screen already has a (drifted) nav — replace it in place,
    /// keeping its node id / position (`ReplaceSubtree`).
    Replace(String),
    /// This screen has NO nav at all, but the reference nav declares a tab
    /// for it — append a fresh clone as its last child (`InsertSubtree`).
    Inject,
    /// Labels already match the reference, but the ACTIVE tab doesn't sit
    /// on this screen's own tab (the D bug — see the module doc). Carries
    /// the TARGET's own live nav id + an owned clone of it (not the
    /// reference's), so the in-place fix only ever moves active styling —
    /// any other authored drift on this screen's own nav survives.
    RetargetActiveOnly(String, Box<PenNode>),
}

/// Entry point. No-ops when fewer than 2 screen-shaped top-level frames
/// exist (single-screen docs — zero regression surface, mirrors
/// `wire_screen_navigation`'s own gate), or when NO screen carries a nav at
/// all (nothing to unify around).
pub fn unify_shared_nav(sink: &mut dyn DocSink) {
    let screens = collect_screen_candidates(sink.state());
    if screens.len() < 2 {
        return;
    }

    // Reference screen = document-order FIRST screen that already has a nav
    // — "reuse, don't redraw": whichever screen the user (or an earlier
    // turn) already generated wins, and its tab set becomes the shared
    // truth every other screen adopts.
    let Some((reference_screen_id, reference_nav)) = find_reference_nav(sink, &screens) else {
        return;
    };

    for screen in &screens {
        if screen.id == reference_screen_id {
            continue; // the reference screen keeps its own (authoritative) nav.
        }

        // Read-only decision first — collect an OWNED `SyncTarget` so the
        // borrow of `sink.state()` ends before `sink.apply()` below.
        let Some(target) = resolve_target(sink, screen, &reference_nav) else {
            continue; // already unified, or no eligible reason to touch it.
        };

        match target {
            SyncTarget::Replace(target_nav_id) => {
                let mut clone = reference_nav.clone();
                retarget_active_tab(&mut clone, &screen.name);
                stamp_chrome_role(&mut clone);
                // `ReplaceSubtree` remaps every id in `clone` (root AND
                // descendants) to fresh, non-colliding ids on apply
                // (`op_editor_core::command_node::cmd_replace_subtree` ->
                // `remap_subtree_ids`) — no hand-rolled id allocation
                // needed here.
                sink.apply(EditorCommand::ReplaceSubtree {
                    node_id: NodeId::new(target_nav_id),
                    node: Box::new(clone),
                    drop_children: true,
                    page_id: None,
                });
            }
            SyncTarget::Inject => {
                let mut clone = reference_nav.clone();
                retarget_active_tab(&mut clone, &screen.name);
                stamp_chrome_role(&mut clone);
                // `InsertSubtree` APPENDS to the target parent's children
                // (`cmd_insert_subtree`'s `slot.extend(nodes)`), so the
                // injected nav lands as the screen's LAST child — bottom
                // placement — by construction. `anchor_bottom_nav_last_
                // for_all_roots` (`cleanup.rs`) runs BEFORE this pass and
                // only repositions an EXISTING nav; it never re-runs after,
                // but a freshly appended child needs no repositioning (it
                // IS already tail-most) and nothing later in `run_cleanup_
                // passes` reorders children, so the injected nav's bottom
                // placement holds without re-invoking that pass.
                sink.apply(EditorCommand::InsertSubtree {
                    nodes: vec![clone],
                    parent_id: NodeId::new(screen.id.clone()),
                    page_id: None,
                });
            }
            SyncTarget::RetargetActiveOnly(target_nav_id, mut own_nav) => {
                // Mutate the TARGET's own live nav (captured read-only in
                // `resolve_target`), not a reference clone — labels already
                // matched, so a full replace would silently overwrite any
                // authored drift beyond the active indicator this screen's
                // own nav carries (icon glyphs, ids, extra chrome).
                retarget_active_tab(&mut own_nav, &screen.name);
                sink.apply(EditorCommand::ReplaceSubtree {
                    node_id: NodeId::new(target_nav_id),
                    node: own_nav,
                    drop_children: true,
                    page_id: None,
                });
            }
        }
    }
}

/// Resolve what (if anything) `screen` needs, from a read-only scan of
/// `sink.state()` — an OWNED decision so the caller can drop the borrow
/// before mutating. `None` covers BOTH idempotency (already unified) and
/// the "don't force chrome" case (no nav, and no tab in the reference nav
/// names this screen).
fn resolve_target(
    sink: &dyn DocSink,
    screen: &ScreenCandidate,
    reference_nav: &PenNode,
) -> Option<SyncTarget> {
    let root = op_editor_core::walkers::find_node(
        sink.state().active_children(),
        &NodeId::new(screen.id.clone()),
    )?;
    let mut navs = Vec::new();
    collect_nav_containers(root, &mut navs);
    match navs.into_iter().next() {
        Some(target_nav) => {
            if !labels_already_unified(target_nav, reference_nav) {
                Some(SyncTarget::Replace(target_nav.id_str().to_string()))
            } else if active_tab_is_correct(target_nav, &screen.name) {
                None // truly idempotent: labels match AND active tab is right.
            } else {
                // Labels match but the active tab is wrong (the D bug) —
                // fix in place rather than a full replace.
                Some(SyncTarget::RetargetActiveOnly(
                    target_nav.id_str().to_string(),
                    Box::new(target_nav.clone()),
                ))
            }
        }
        None => {
            // Missing nav: inject ONLY when the reference nav's own tab set
            // declares a tab for THIS screen — the tab set is the nav's own
            // contract for which screens it serves. A screen with no
            // matching tab (a standalone detail page, say) is never forced
            // to grow chrome it never asked for.
            let eligible = reference_nav
                .children()
                .is_some_and(|tabs| find_tab_index_for_screen(tabs, &screen.name).is_some());
            if !eligible {
                return None;
            }
            // Detail-page exemption (see the module doc): even when the
            // screen's bare name happens to token-match a reference tab, a
            // back-shaped header control marks it as a push-in detail
            // screen, not a tab destination — don't force chrome onto it.
            // Inject-only: an authored nav (Replace / RetargetActiveOnly)
            // is never touched by this check.
            if screen_has_back_control_in_header(sink, screen) {
                return None;
            }
            Some(SyncTarget::Inject)
        }
    }
}

/// Find the document-order first screen with a nav container, returning its
/// id + a CLONE of that nav (owned, so the caller can drop the `sink.state()`
/// borrow before mutating).
fn find_reference_nav(
    sink: &dyn DocSink,
    screens: &[ScreenCandidate],
) -> Option<(String, PenNode)> {
    for screen in screens {
        let root = op_editor_core::walkers::find_node(
            sink.state().active_children(),
            &NodeId::new(screen.id.clone()),
        )?;
        let mut navs = Vec::new();
        collect_nav_containers(root, &mut navs);
        if let Some(nav) = navs.into_iter().next() {
            return Some((screen.id.clone(), nav.clone()));
        }
    }
    None
}

/// Whether `target`'s nav already carries the SAME tab-label set (in order)
/// as `reference` — the idempotency gate. Active-tab correctness is
/// deliberately NOT part of this check (see the module doc): once the label
/// set matches, the screen is considered "already unified" even if a prior
/// run's active-tab detection degraded (see [`retarget_active_tab`]).
fn labels_already_unified(target: &PenNode, reference: &PenNode) -> bool {
    nav_label_set(target) == nav_label_set(reference)
}

/// Whether `nav`'s currently-active tab already sits on the tab matching
/// `screen_name` — the other half of the idempotency check
/// [`labels_already_unified`] deliberately leaves out (see the module doc's
/// "Active-tab idempotency gap" section). Mirrors [`retarget_active_tab`]'s
/// own degrade conditions exactly (same helpers, same order) so a `false`
/// here reliably means `retarget_active_tab` will find a confident swap to
/// make, not disagree and no-op again.
fn active_tab_is_correct(nav: &PenNode, screen_name: &str) -> bool {
    let Some(children) = nav.children() else {
        return true; // no children to be wrong about.
    };
    if children.len() < 2 {
        return true;
    }
    let fingerprints: Vec<String> = children.iter().map(style_fingerprint).collect();
    let Some(active_idx) = find_active_by_fingerprint(&fingerprints) else {
        return true; // no active styling detected — nothing to fix.
    };
    let Some(target_idx) = find_tab_index_for_screen(children, screen_name) else {
        return true; // no tab matches this screen — nothing to fix.
    };
    active_idx == target_idx
}

fn nav_label_set(nav: &PenNode) -> Vec<String> {
    nav.children()
        .into_iter()
        .flatten()
        .filter_map(first_text_content)
        .map(normalize_label)
        .collect()
}

/// Move the "active" tab styling from wherever the reference nav had it onto
/// the tab whose label matches `screen_name` — so a clone of the Home
/// screen's nav, dropped onto the Library screen, shows Library (not Home)
/// as active.
///
/// ## Active-tab detection (and its degrade path)
///
/// The schema has no explicit "this tab is active" flag, so this detects it
/// structurally: among the nav's direct tab-item children, compute a STYLE
/// FINGERPRINT per tab (its own JSON with content fields — text/icon-glyph/
/// id/name — blanked out, so two tabs with different labels but identical
/// styling fingerprint the same). In the common authored pattern (measured:
/// accent fill + underline indicator on exactly one tab, every other tab
/// sharing one plain/muted style), N-1 tabs share a MAJORITY fingerprint and
/// exactly one is the odd one out — that one is "active".
///
/// This is intentionally whole-node, not property-specific: it makes no
/// assumption about WHAT constitutes "active" styling (a fill color, an
/// extra underline-indicator child, both, or something else entirely) —
/// whatever differs, differs, and the whole node (with everything that
/// makes it look active) is what moves.
///
/// **Degrade path**: if there's no clear single outlier (all tabs identical
/// — no active styling exists in this nav at all — or 2+ tabs equally
/// differ from the rest — genuinely ambiguous), OR no tab's label matches
/// `screen_name`, this is a no-op: the clone keeps whatever the reference
/// screen's own active tab was. Consistency of labels/icons across screens
/// (this pass's primary goal) is preserved either way; only the active-state
/// indicator is potentially "wrong" (still pointing at the reference
/// screen's own tab) in that narrow, hard-to-detect case — a strictly
/// better outcome than the pre-fix icon/label drift, so this is the
/// documented degrade rather than something worth making the detection more
/// fragile to chase.
fn retarget_active_tab(nav: &mut PenNode, screen_name: &str) {
    let Some(children) = nav.children_mut() else {
        return;
    };
    if children.len() < 2 {
        return;
    }

    let fingerprints: Vec<String> = children.iter().map(style_fingerprint).collect();
    let Some(active_idx) = find_active_by_fingerprint(&fingerprints) else {
        return; // no active styling detected, or ambiguous — degrade.
    };

    let Some(target_idx) = find_tab_index_for_screen(children, screen_name) else {
        return; // no tab matches this screen's name — degrade.
    };

    if active_idx != target_idx {
        swap_tab_style(children, active_idx, target_idx);
    }
}

/// Index of the direct tab-item child whose label normalize-matches
/// `screen_name`, if any. Shared by [`retarget_active_tab`] (find which tab
/// should become active) and [`resolve_target`] (decide whether a nav-less
/// screen is eligible for injection) — one matching rule, one place.
fn find_tab_index_for_screen(children: &[PenNode], screen_name: &str) -> Option<usize> {
    children.iter().position(|tab| {
        first_text_content(tab).is_some_and(|label| labels_match(label, screen_name))
    })
}

/// Force `role: "bottom-tab-bar"` onto `node` if it doesn't already carry a
/// role (0718-1-k3-1 review fix). [`find_reference_nav`] / [`is_nav_container`]
/// match a reference nav by role OR name/id substring — a purely
/// name-matched reference (no role at all) is a real, reachable case, and
/// `unfilled_screens.rs`'s chrome exclusion (`CHROME_ROLES`) is role-only:
/// an unstamped clone's own tab-label text would read as "real content" to
/// the promise-delivery check, silently flipping a genuinely unfilled
/// screen to "filled" the moment it gets a nav — the same class of bug
/// `unify_shared_status_bar`'s `stamp_chrome_role` closes for status bars.
/// Only ever called on an owned CLONE (`Replace` / `Inject`), never on the
/// authored reference or a `RetargetActiveOnly` target (that branch mutates
/// the screen's OWN pre-existing nav, whose role — or lack of one — is
/// already whatever it authentically was).
fn stamp_chrome_role(node: &mut PenNode) {
    if node.base().role.is_none() {
        node.base_mut().role = Some("bottom-tab-bar".to_string());
    }
}

/// JSON fingerprint of `node` with every content-identifying field blanked
/// (`content` / `iconFontName` / `id` / `name`), recursively. Two tabs with
/// the SAME styling but different labels/icons fingerprint identically.
fn style_fingerprint(node: &PenNode) -> String {
    let Ok(mut value) = serde_json::to_value(node) else {
        return String::new();
    };
    blank_content_fields(&mut value);
    value.to_string()
}

fn blank_content_fields(value: &mut serde_json::Value) {
    let serde_json::Value::Object(map) = value else {
        return;
    };
    map.remove("id");
    map.remove("name");
    if map.get("type").and_then(serde_json::Value::as_str) == Some("text") {
        map.insert(
            "content".to_string(),
            serde_json::Value::String(String::new()),
        );
    }
    if map.get("type").and_then(serde_json::Value::as_str) == Some("icon_font") {
        map.insert(
            "iconFontName".to_string(),
            serde_json::Value::String(String::new()),
        );
    }
    if let Some(serde_json::Value::Array(children)) = map.get_mut("children") {
        for child in children.iter_mut() {
            blank_content_fields(child);
        }
    }
}

/// The tab whose fingerprint DIFFERS from the majority — `None` when every
/// tab shares one fingerprint (no active styling exists) or when 2+ tabs
/// equally diverge from the majority (ambiguous — see the module doc).
fn find_active_by_fingerprint(fingerprints: &[String]) -> Option<usize> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for f in fingerprints {
        *counts.entry(f.as_str()).or_insert(0) += 1;
    }
    let (majority_fp, majority_count) = counts.iter().max_by_key(|(_, count)| **count)?;
    let majority_fp = *majority_fp;
    if *majority_count == fingerprints.len() {
        return None; // every tab looks the same — no active signal.
    }
    let outliers: Vec<usize> = fingerprints
        .iter()
        .enumerate()
        .filter(|(_, f)| f.as_str() != majority_fp)
        .map(|(i, _)| i)
        .collect();
    (outliers.len() == 1).then_some(outliers[0])
}

/// Swap the WHOLE nodes at `a`/`b` (carrying styling — fills, extra
/// indicator children, everything), then restore each POSITION's own
/// original content (text / icon glyph, in document order) so the tab at
/// position `a` still reads as whatever it always represented, just with
/// the OTHER position's styling now attached.
fn swap_tab_style(children: &mut [PenNode], a: usize, b: usize) {
    let content_a = snapshot_content(&children[a]);
    let content_b = snapshot_content(&children[b]);
    children.swap(a, b);
    restore_content(&mut children[a], &content_a);
    restore_content(&mut children[b], &content_b);
}

/// A tab's content identity: every `Text` node's plain string and every
/// `IconFont` node's glyph name, in document (depth-first) order.
struct ContentSnapshot {
    texts: Vec<String>,
    icon_names: Vec<String>,
}

fn snapshot_content(node: &PenNode) -> ContentSnapshot {
    let mut snapshot = ContentSnapshot {
        texts: Vec::new(),
        icon_names: Vec::new(),
    };
    collect_content(node, &mut snapshot);
    snapshot
}

fn collect_content(node: &PenNode, out: &mut ContentSnapshot) {
    match node {
        PenNode::Text(t) => {
            if let TextContent::Plain(s) = &t.content {
                out.texts.push(s.clone());
            }
        }
        PenNode::IconFont(icon) => out.icon_names.push(icon.icon_font_name.clone()),
        _ => {}
    }
    for child in node.children().into_iter().flatten() {
        collect_content(child, out);
    }
}

/// Write `snapshot`'s content back into `node`'s Text/IconFont descendants,
/// positionally, in the SAME document-order traversal `snapshot_content`
/// used. Gracefully leaves whatever content the swap brought along when a
/// count mismatches (never panics, never mismatches by more than "one leaf
/// keeps the swapped-in content instead of the restored one").
fn restore_content(node: &mut PenNode, snapshot: &ContentSnapshot) {
    let mut texts = snapshot.texts.iter();
    let mut icons = snapshot.icon_names.iter();
    restore_content_walk(node, &mut texts, &mut icons);
}

fn restore_content_walk<'a>(
    node: &mut PenNode,
    texts: &mut std::slice::Iter<'a, String>,
    icons: &mut std::slice::Iter<'a, String>,
) {
    match node {
        PenNode::Text(t) => {
            if let TextContent::Plain(s) = &mut t.content {
                if let Some(orig) = texts.next() {
                    *s = orig.clone();
                }
            }
        }
        PenNode::IconFont(icon) => {
            if let Some(orig) = icons.next() {
                icon.icon_font_name = orig.clone();
            }
        }
        _ => {}
    }
    for child in node.children_mut().into_iter().flatten() {
        restore_content_walk(child, texts, icons);
    }
}

#[cfg(test)]
#[path = "unify_shared_nav_tests.rs"]
mod tests;

// Split out from `tests` above to stay under the 800-line cap — see that
// file's own split-out sibling for details.
#[cfg(test)]
#[path = "unify_shared_nav_active_tab_tests.rs"]
mod active_tab_tests;
