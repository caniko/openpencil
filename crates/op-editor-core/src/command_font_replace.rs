//! Whole-document font-family replacement.
//!
//! Font family values imported from HTML may be CSS fallback stacks, while
//! Figma and native `.op` documents generally carry a single family. This
//! module handles both shapes through one source-independent editor command.

use crate::EditorState;
use jian_ops_schema::node::{PenNode, TextContent};

/// Replace one family token in a CSS font-family value.
///
/// Commas inside quotes or behind a backslash are not treated as separators.
/// Matching ignores ASCII case and surrounding whitespace. Non-matching
/// tokens, separators, and whitespace are retained byte-for-byte. The return
/// value is `None` when no token changed; otherwise it contains the updated
/// stack and the number of matching tokens replaced.
pub fn replace_font_family_tokens(
    stack: &str,
    from_family: &str,
    to_family: &str,
) -> Option<(String, usize)> {
    let from = decode_family_argument(from_family)?;
    let to = decode_family_argument(to_family)?;
    if from.is_empty() || to.is_empty() {
        return None;
    }

    replace_font_family_tokens_decoded(stack, &from, &to)
}

fn replace_font_family_tokens_decoded(
    stack: &str,
    from: &str,
    to: &str,
) -> Option<(String, usize)> {
    let mut output = String::with_capacity(stack.len().max(to.len()));
    let mut token_start = 0;
    let mut quote = None;
    let mut escaped = false;
    let mut replaced = 0;

    for (index, ch) in stack.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
            continue;
        }
        if ch == ',' {
            replaced += replace_stack_token(&stack[token_start..index], from, to, &mut output);
            output.push(',');
            token_start = index + ch.len_utf8();
        }
    }
    replaced += replace_stack_token(&stack[token_start..], from, to, &mut output);

    (replaced > 0).then_some((output, replaced))
}

impl EditorState {
    /// Replace `from_family` in every text node and explicit styled-text run.
    ///
    /// The mutation spans both legacy `doc.children` and all `pages`, recurses
    /// through every container kind, and lands as exactly one undo step. A
    /// missing match or invalid empty argument is a clean no-op with no history
    /// entry. Styled segments with an inherited (`None`) family stay inherited.
    pub fn replace_font_family_everywhere(&mut self, from_family: &str, to_family: &str) -> usize {
        let Some(from) = decode_family_argument(from_family).filter(|family| !family.is_empty())
        else {
            return 0;
        };
        let Some(to) = decode_family_argument(to_family).filter(|family| !family.is_empty()) else {
            return 0;
        };

        let snapshot = self.snapshot_for_history();
        let mut replaced = replace_in_forest(&mut self.doc.children, &from, &to);
        if let Some(pages) = self.doc.pages.as_mut() {
            for page in pages {
                replaced += replace_in_forest(&mut page.children, &from, &to);
            }
        }
        for component in &mut self.components.components {
            replaced += replace_in_node(&mut component.root, &from, &to);
        }
        if replaced > 0 {
            self.history_push_past(snapshot);
        }
        replaced
    }
}

fn replace_in_forest(roots: &mut [PenNode], from: &str, to: &str) -> usize {
    roots
        .iter_mut()
        .map(|node| replace_in_node(node, from, to))
        .sum()
}

fn replace_in_node(node: &mut PenNode, from: &str, to: &str) -> usize {
    let mut replaced = 0;
    if let PenNode::Text(text) = node {
        replaced += replace_slot(&mut text.font_family, from, to);
        if let TextContent::Styled(segments) = &mut text.content {
            for segment in segments {
                replaced += replace_slot(&mut segment.font_family, from, to);
            }
        }
    }
    if let PenNode::Ref(reference) = node {
        if let Some(descendants) = reference.descendants.as_mut() {
            for override_value in descendants.values_mut() {
                replaced += replace_in_override_json(override_value, from, to);
            }
        }
    }
    if let Some(children) = existing_children_mut(node) {
        replaced += replace_in_forest(children, from, to);
    }
    replaced
}

fn replace_in_override_json(value: &mut serde_json::Value, from: &str, to: &str) -> usize {
    match value {
        serde_json::Value::Object(object) => object
            .iter_mut()
            .map(|(key, value)| {
                if key == "fontFamily" {
                    let Some(stack) = value.as_str() else {
                        return 0;
                    };
                    let Some((updated, count)) =
                        replace_font_family_tokens_decoded(stack, from, to)
                    else {
                        return 0;
                    };
                    *value = serde_json::Value::String(updated);
                    count
                } else {
                    replace_in_override_json(value, from, to)
                }
            })
            .sum(),
        serde_json::Value::Array(values) => values
            .iter_mut()
            .map(|value| replace_in_override_json(value, from, to))
            .sum(),
        _ => 0,
    }
}

fn replace_slot(slot: &mut Option<String>, from: &str, to: &str) -> usize {
    let Some(current) = slot.as_mut() else {
        return 0;
    };
    let Some((updated, count)) = replace_font_family_tokens_decoded(current, from, to) else {
        return 0;
    };
    *current = updated;
    count
}

/// Borrow authored children without materializing an absent child array.
fn existing_children_mut(node: &mut PenNode) -> Option<&mut Vec<PenNode>> {
    match node {
        PenNode::Frame(n) => n.children.as_mut(),
        PenNode::Group(n) => n.children.as_mut(),
        PenNode::Rectangle(n) => n.children.as_mut(),
        PenNode::Tabs(n) => n.children.as_mut(),
        PenNode::Ref(n) => n.children.as_mut(),
        _ => None,
    }
}

fn replace_stack_token(segment: &str, from: &str, to: &str, output: &mut String) -> usize {
    let start = segment
        .char_indices()
        .find_map(|(i, ch)| (!ch.is_whitespace()).then_some(i))
        .unwrap_or(segment.len());
    let end = segment
        .char_indices()
        .rev()
        .find_map(|(i, ch)| (!ch.is_whitespace()).then_some(i + ch.len_utf8()))
        .unwrap_or(start);
    let core = &segment[start..end];
    let matches = !core.is_empty()
        && decode_family_token(core).is_some_and(|candidate| candidate.eq_ignore_ascii_case(from));
    if !matches {
        output.push_str(segment);
        return 0;
    }

    output.push_str(&segment[..start]);
    output.push_str(&format_replacement_family(to, outer_quote(core)));
    output.push_str(&segment[end..]);
    1
}

fn decode_family_argument(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    decode_family_token(trimmed)
}

fn decode_family_token(raw: &str) -> Option<String> {
    let value = if let Some(quote) = outer_quote(raw) {
        let first = quote.len_utf8();
        &raw[first..raw.len() - quote.len_utf8()]
    } else {
        raw
    };
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let next = chars.next()?;
            decoded.push(next);
        } else {
            decoded.push(ch);
        }
    }
    Some(decoded)
}

fn outer_quote(raw: &str) -> Option<char> {
    let quote = raw.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let mut escaped = false;
    for (index, ch) in raw.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            return (index + ch.len_utf8() == raw.len()).then_some(quote);
        }
    }
    None
}

fn format_replacement_family(family: &str, preferred_quote: Option<char>) -> String {
    let quote = preferred_quote.or_else(|| family_needs_quotes(family).then_some('"'));
    let Some(quote) = quote else {
        return family.to_string();
    };
    let mut formatted = String::with_capacity(family.len() + 2);
    formatted.push(quote);
    for ch in family.chars() {
        if ch == '\\' || ch == quote {
            formatted.push('\\');
        }
        formatted.push(ch);
    }
    formatted.push(quote);
    formatted
}

fn family_needs_quotes(family: &str) -> bool {
    family
        .chars()
        .any(|ch| ch == ',' || ch == '\'' || ch == '"' || ch == '\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::EditorCommand;
    use crate::pen_node_ext::PenNodeExt;
    use jian_ops_schema::PenDocument;
    use serde_json::json;

    fn state_from(value: serde_json::Value) -> EditorState {
        let doc: PenDocument = serde_json::from_value(value).expect("valid document");
        EditorState::from_document(doc)
    }

    fn text_families<'a>(
        state: &'a EditorState,
        id: &str,
    ) -> (Option<&'a str>, Vec<Option<&'a str>>) {
        fn find<'a>(roots: &'a [PenNode], id: &str) -> Option<&'a PenNode> {
            for node in roots {
                if node.id_str() == id {
                    return Some(node);
                }
                if let Some(children) = node.children() {
                    if let Some(found) = find(children, id) {
                        return Some(found);
                    }
                }
            }
            None
        }
        let node = find(&state.doc.children, id).or_else(|| {
            state
                .doc
                .pages
                .as_deref()
                .and_then(|pages| pages.iter().find_map(|page| find(&page.children, id)))
        });
        let PenNode::Text(text) = node.expect("text node") else {
            panic!("expected text")
        };
        let segments = match &text.content {
            TextContent::Styled(segments) => segments
                .iter()
                .map(|segment| segment.font_family.as_deref())
                .collect(),
            TextContent::Plain(_) => Vec::new(),
        };
        (text.font_family.as_deref(), segments)
    }

    #[test]
    fn stack_replacement_is_case_insensitive_quote_aware_and_lossless() {
        let stack = r#"  Inter , 'ACME, Display', "PingFang SC" , sans-serif  "#;
        let (updated, count) =
            replace_font_family_tokens(stack, "acme, display", "Noto Sans").unwrap();
        assert_eq!(count, 1);
        assert_eq!(
            updated,
            r#"  Inter , 'Noto Sans', "PingFang SC" , sans-serif  "#
        );

        let (updated, count) =
            replace_font_family_tokens(r#"Acme\,Display, serif"#, "ACME,DISPLAY", "A, B").unwrap();
        assert_eq!(count, 1);
        assert_eq!(updated, r#""A, B", serif"#);
    }

    #[test]
    fn command_replaces_top_level_pages_nested_nodes_and_explicit_segments() {
        let mut state = state_from(json!({
            "version": "1.0.0",
            "children": [{
                "type": "frame", "id": "root", "children": [{
                    "type": "text", "id": "legacy", "content": "A",
                    "fontFamily": "INTER, system-ui"
                }]
            }],
            "pages": [{
                "id": "p1", "name": "Page 1", "children": [{
                    "type": "group", "id": "page-root", "children": [{
                        "type": "text", "id": "styled",
                        "fontFamily": "Other",
                        "content": [
                            {"text": "A", "fontFamily": "'inter'"},
                            {"text": "B"},
                            {"text": "C", "fontFamily": "Roboto"}
                        ]
                    }]
                }]
            }]
        }));

        assert!(state.apply(EditorCommand::ReplaceFontFamily {
            from: "inter".into(),
            to: "Source Sans 3".into(),
        }));
        assert_eq!(
            text_families(&state, "legacy"),
            (Some("Source Sans 3, system-ui"), vec![])
        );
        assert_eq!(
            text_families(&state, "styled"),
            (
                Some("Other"),
                vec![Some("'Source Sans 3'"), None, Some("Roboto")]
            )
        );
    }

    #[test]
    fn replacement_is_one_undo_and_redo_step() {
        let mut state = state_from(json!({
            "version": "1.0.0",
            "children": [
                {"type": "text", "id": "a", "content": "A", "fontFamily": "Inter"},
                {"type": "text", "id": "b", "content": "B", "fontFamily": "INTER"}
            ]
        }));

        assert!(state.apply(EditorCommand::ReplaceFontFamily {
            from: "inter".into(),
            to: "Geist".into(),
        }));
        assert_eq!(text_families(&state, "a").0, Some("Geist"));
        assert_eq!(text_families(&state, "b").0, Some("Geist"));
        assert!(state.undo());
        assert_eq!(text_families(&state, "a").0, Some("Inter"));
        assert_eq!(text_families(&state, "b").0, Some("INTER"));
        assert!(!state.undo(), "one replace must push one snapshot");
        assert!(state.redo());
        assert_eq!(text_families(&state, "a").0, Some("Geist"));
        assert_eq!(text_families(&state, "b").0, Some("Geist"));
        assert!(!state.redo());
    }

    #[test]
    fn no_match_empty_arguments_and_inherited_segments_do_not_push_history() {
        let mut state = state_from(json!({
            "version": "1.0.0",
            "children": [{
                "type": "text", "id": "styled", "fontFamily": "Inter",
                "content": [{"text": "inherits"}]
            }]
        }));

        assert!(!state.apply(EditorCommand::ReplaceFontFamily {
            from: "Roboto".into(),
            to: "Geist".into(),
        }));
        assert!(!state.apply(EditorCommand::ReplaceFontFamily {
            from: "Inter".into(),
            to: "  ".into(),
        }));
        assert!(!state.history.can_undo());
        assert_eq!(text_families(&state, "styled").1, vec![None]);
    }

    #[test]
    fn replacement_updates_component_prototypes_and_ref_descendant_overrides() {
        let mut state = state_from(json!({
            "version": "1.0.0",
            "children": [
                {
                    "type": "frame", "id": "component", "name": "Card",
                    "reusable": true,
                    "children": [{
                        "type": "text", "id": "label", "content": "Base",
                        "fontFamily": "Inter"
                    }]
                },
                {
                    "type": "ref", "id": "instance", "ref": "component",
                    "descendants": {
                        "label": {
                            "content": [{"text": "Override", "fontFamily": "INTER"}],
                            "fontFamily": "inter, sans-serif"
                        }
                    }
                }
            ]
        }));

        assert!(state.apply(EditorCommand::ReplaceFontFamily {
            from: "iNtEr".into(),
            to: "Geist".into(),
        }));
        let component = state
            .components
            .find_by_id(&crate::NodeId::new("component"))
            .expect("component prototype");
        let PenNode::Frame(frame) = &component.root else {
            panic!("component frame")
        };
        let PenNode::Text(label) = &frame.children.as_ref().expect("children")[0] else {
            panic!("component label")
        };
        assert_eq!(label.font_family.as_deref(), Some("Geist"));

        let PenNode::Ref(reference) =
            crate::walkers::find_node(state.active_children(), &crate::NodeId::new("instance"))
                .expect("ref")
        else {
            panic!("instance ref")
        };
        let override_value = &reference.descendants.as_ref().expect("overrides")["label"];
        assert_eq!(override_value["fontFamily"], "Geist, sans-serif");
        assert_eq!(override_value["content"][0]["fontFamily"], "Geist");

        assert!(state.undo());
        let component = state
            .components
            .find_by_id(&crate::NodeId::new("component"))
            .expect("restored prototype");
        let PenNode::Frame(frame) = &component.root else {
            panic!("component frame")
        };
        let PenNode::Text(label) = &frame.children.as_ref().expect("children")[0] else {
            panic!("component label")
        };
        assert_eq!(label.font_family.as_deref(), Some("Inter"));
        assert!(state.redo());
        let new_instance = state
            .instantiate_component(&crate::NodeId::new("component"))
            .expect("instantiate replaced prototype");
        let root = crate::walkers::find_node(state.active_children(), &new_instance)
            .expect("new instance root");
        let PenNode::Frame(frame) = root else {
            panic!("instantiated frame")
        };
        let PenNode::Text(label) = &frame.children.as_ref().expect("children")[0] else {
            panic!("instantiated label")
        };
        assert_eq!(label.font_family.as_deref(), Some("Geist"));
    }
}
