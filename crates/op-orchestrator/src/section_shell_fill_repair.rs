//! Section shell/fill ownership repair — a horizontal "header row" whose
//! second child is actually the ENTIRE content body of the section (not a
//! chevron/badge), and that body redundantly repeats the header's own
//! title as its own first child.
//!
//! Measured (`0718-1-glm-1.op`, GLM-5.2 built-in loop provider): a
//! "Must-See" checklist section wanted the shape `[header-row(title),
//! content-section]` as two SIBLINGS, but the model instead nested the
//! whole `ChecklistSection` INSIDE the header row as its second
//! `layout:"horizontal"` flex child, alongside a "Must-See" title it
//! separately drew again as the checklist's own first child. With
//! `justifyContent:"space_between"` on the row, the short title pins to
//! the left while the much-taller checklist fills the remaining width to
//! its right — visually a giant title floating mid-list (confirmed by
//! render). Two symptoms, one root cause: the fill step didn't know a
//! title already existed for this section and didn't know the checklist
//! belonged as the header's sibling, not its child.
//!
//! This did NOT come from a Rust scaffold function seeding a pre-built
//! "header row with title" shell — `design-agent.md`'s skeleton-first
//! protocol only pre-authors a bare, empty, NAMED top-level section frame
//! (no title drawn yet); everything below that is freeform model output
//! across one or more `batch_design` calls. So there is no shell BUILDER
//! to restructure — the fix has to be a deterministic REPAIR of the
//! resulting shape, the same "shared throat-layer net, not model-specific
//! patch" pattern the rest of `cleanup.rs` already follows.
//!
//! Detection is deliberately narrow and evidence-driven: a horizontal row
//! with EXACTLY two children where one is a short title (a `text` node, or
//! a small wrapper whose own descendant text) and the other is a
//! container whose OWN FIRST CHILD's text is byte-for-byte IDENTICAL to
//! the title's. A design legitimately repeating the exact same string as
//! both a row's direct child AND its sibling's first child is not a
//! pattern real design ever produces on purpose — the duplicate-text
//! signal alone carries enough precision that no additional shape gating
//! (e.g. requiring `justifyContent:"space_between"`) is needed to avoid
//! false positives.
//!
//! Repair: delete the duplicate title inside the content container, then
//! move the (now title-free) container out of the header row to become
//! its SIBLING, inserted immediately after it — producing exactly
//! `[header-row(title), content-section]`.

use op_editor_core::{EditorCommand, NodeId};
use serde_json::Value;

use crate::types::DocSink;

fn children(v: &Value) -> &[Value] {
    v.get("children")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn layout_str(v: &Value) -> Option<&str> {
    v.get("layout").and_then(Value::as_str)
}

/// The text this node "is" or "starts with" — itself if it's a `text`
/// node, otherwise its first descendant `text` node (covers a title
/// wrapped in a small alignment/padding frame).
fn text_of(v: &Value) -> Option<&str> {
    if v.get("type").and_then(Value::as_str) == Some("text") {
        return v.get("content").and_then(Value::as_str);
    }
    children(v).iter().find_map(text_of)
}

/// Detect + queue fixes for the shell/fill duplicate-title shape, walking
/// `v`'s subtree. `parent_id` / `index_in_parent` describe `v` itself
/// (needed so a match can move the misnested container to be `v`'s own
/// sibling) — `None` at the tree root, where nothing can be promoted out
/// of (a root has no parent to become a sibling under).
fn collect_fixes(
    v: &Value,
    parent_id: Option<&str>,
    index_in_parent: Option<usize>,
    cmds: &mut Vec<EditorCommand>,
) {
    if layout_str(v) == Some("horizontal") {
        let kids = children(v);
        if kids.len() == 2 {
            if let Some(fix) = matched_duplicate_title(v, &kids[0], &kids[1])
                .or_else(|| matched_duplicate_title(v, &kids[1], &kids[0]))
            {
                if let (Some(parent_id), Some(row_index)) = (parent_id, index_in_parent) {
                    cmds.push(EditorCommand::DeleteNode {
                        node_id: NodeId::new(fix.duplicate_title_id),
                        page_id: None,
                    });
                    cmds.push(EditorCommand::MoveNode {
                        node_id: NodeId::new(fix.container_id),
                        target_parent: NodeId::new(parent_id.to_string()),
                        page_id: None,
                        index: Some(row_index + 1),
                    });
                }
                // Matched (or would have, but sits at the document root with
                // nothing to become a sibling under) — either way this row
                // is not a candidate for further recursion.
                return;
            }
        }
    }
    let this_id = v.get("id").and_then(Value::as_str);
    for (i, c) in children(v).iter().enumerate() {
        collect_fixes(c, this_id, Some(i), cmds);
    }
}

struct DuplicateTitleFix {
    /// The content container's own first child — the redundant title to
    /// delete.
    duplicate_title_id: String,
    /// The content container itself — to promote out as the row's sibling.
    container_id: String,
}

/// `title` and `container` are the row's two children, in either order.
/// Matches iff `container`'s OWN FIRST CHILD's text is identical to
/// `title`'s text.
fn matched_duplicate_title(
    _row: &Value,
    title: &Value,
    container: &Value,
) -> Option<DuplicateTitleFix> {
    let title_text = text_of(title)?;
    if title_text.is_empty() {
        return None;
    }
    let first_child = children(container).first()?;
    if text_of(first_child)? != title_text {
        return None;
    }
    Some(DuplicateTitleFix {
        duplicate_title_id: first_child.get("id")?.as_str()?.to_string(),
        container_id: container.get("id")?.as_str()?.to_string(),
    })
}

/// Repair every `[header-row(title, misnested-content-with-duplicate-title)]`
/// shape under `root_id`. Returns the number of sections repaired.
pub(crate) fn repair_section_shell_fill_ownership(sink: &mut dyn DocSink, root_id: &str) -> usize {
    let Some(root) = op_editor_core::walkers::find_node(
        sink.state().active_children(),
        &NodeId::new(root_id.to_string()),
    ) else {
        return 0;
    };
    let Ok(v) = serde_json::to_value(root) else {
        return 0;
    };
    let mut cmds = Vec::new();
    // The root itself has no parent to promote a sibling under, so start
    // the walk from its children (each already carries `root_id` as its
    // parent).
    let root_id_owned = v.get("id").and_then(Value::as_str).map(str::to_string);
    for (i, c) in children(&v).iter().enumerate() {
        collect_fixes(c, root_id_owned.as_deref(), Some(i), &mut cmds);
    }
    if cmds.is_empty() {
        return 0;
    }
    // Each match contributes exactly one DeleteNode + one MoveNode.
    let repaired = cmds.len() / 2;
    for cmd in cmds {
        sink.apply(cmd);
    }
    repaired
}

#[cfg(test)]
#[path = "section_shell_fill_repair_tests.rs"]
mod tests;
