use std::collections::{BTreeMap, BTreeSet};

use crate::css::declarations::{parse_declarations, Declaration};
use crate::css::selectors::{matches, parse_selector_list, specificity, Selector};
use crate::dom::DomElement;
use crate::length::{parse_length, CssLength, LengthCtx};

pub const UA_STYLESHEET: &str = "\
body{font-size:16px;color:#111111}\
h1{font-size:32px;font-weight:700;margin:21px 0}\
h2{font-size:24px;font-weight:700;margin:20px 0}\
h3{font-size:19px;font-weight:700;margin:18px 0}\
h4{font-size:16px;font-weight:700;margin:21px 0}\
h5{font-size:13px;font-weight:700;margin:22px 0}\
h6{font-size:11px;font-weight:700;margin:24px 0}\
p{margin:16px 0}\
ul,ol{margin:16px 0;padding:0 0 0 40px}\
b,strong{font-weight:700}\
i,em{font-style:italic}\
u{text-decoration:underline}\
s,del,strike{text-decoration:line-through}\
a{color:#0066cc;text-decoration:underline}\
code,pre{font-family:monospace}\
hr{margin:8px 0}";

#[derive(Clone, Debug)]
pub struct StyleRule {
    pub selector: Selector,
    pub declarations: Vec<Declaration>,
    pub order: usize,
}

#[derive(Clone, Debug)]
pub struct ComputedStyle {
    pub props: BTreeMap<String, String>,
    pub font_size: f64,
}

impl ComputedStyle {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.props.get(name).map(String::as_str)
    }
}

pub fn parse_stylesheet(css: &str, first_order: usize) -> (Vec<StyleRule>, Vec<String>) {
    let css = strip_comments(css);
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let mut warned_at_rules = BTreeSet::new();
    let mut index = 0;
    let mut order = first_order;
    while index < css.len() {
        index = skip_whitespace(&css, index);
        if index >= css.len() {
            break;
        }
        if css.as_bytes()[index] == b'@' {
            let name_end = css[index + 1..]
                .find(|ch: char| ch.is_whitespace() || ch == '{' || ch == ';')
                .map_or(css.len(), |offset| index + 1 + offset);
            let name = css[index + 1..name_end].to_ascii_lowercase();
            if warned_at_rules.insert(name.clone()) {
                warnings.push(at_rule_warning(&name));
            }
            let next_brace = css[index..].find('{').map(|offset| index + offset);
            let next_semicolon = css[index..].find(';').map(|offset| index + offset);
            index = match (next_brace, next_semicolon) {
                (Some(brace), Some(semicolon)) if semicolon < brace => semicolon + 1,
                (Some(brace), _) => matching_brace(&css, brace).map_or(css.len(), |end| end + 1),
                (_, Some(semicolon)) => semicolon + 1,
                _ => css.len(),
            };
            continue;
        }
        let Some(open_offset) = css[index..].find('{') else {
            break;
        };
        let open = index + open_offset;
        let Some(close) = matching_brace(&css, open) else {
            break;
        };
        let selectors = parse_selector_list(css[index..open].trim());
        let declarations = parse_declarations(&css[open + 1..close]);
        for selector in selectors {
            rules.push(StyleRule {
                selector,
                declarations: declarations.clone(),
                order,
            });
            order = order.saturating_add(1);
        }
        index = close + 1;
    }
    (rules, warnings)
}

fn strip_comments(css: &str) -> String {
    let mut stripped = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        stripped.push_str(&rest[..start]);
        let after_start = &rest[start + 2..];
        if let Some(end) = after_start.find("*/") {
            rest = &after_start[end + 2..];
        } else {
            rest = "";
        }
    }
    stripped.push_str(rest);
    stripped
}

fn skip_whitespace(css: &str, mut index: usize) -> usize {
    while index < css.len() && css.as_bytes()[index].is_ascii_whitespace() {
        index += 1;
    }
    index
}

fn matching_brace(css: &str, open: usize) -> Option<usize> {
    let mut depth = 0u32;
    let mut quote = None;
    for (offset, ch) in css[open..].char_indices() {
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(open + offset);
                }
            }
            _ => {}
        }
    }
    None
}

fn at_rule_warning(name: &str) -> String {
    if name == "media" {
        "@media rules ignored (import viewport applies)".to_string()
    } else {
        format!("@{name} rules ignored")
    }
}

pub fn compute_style(
    path: &[&DomElement],
    rules: &[StyleRule],
    parent: Option<&ComputedStyle>,
    root_font_size: f64,
) -> ComputedStyle {
    let mut matching_rules: Vec<_> = rules
        .iter()
        .filter(|rule| matches(&rule.selector, path))
        .map(|rule| (specificity(&rule.selector), rule.order, rule))
        .collect();
    matching_rules.sort_by_key(|(specificity, order, _)| (*specificity, *order));

    let inline = path
        .last()
        .and_then(|element| element.attr("style"))
        .map(parse_declarations)
        .unwrap_or_default();
    let mut props = BTreeMap::new();
    apply_rule_declarations(&mut props, &matching_rules, false);
    apply_declarations(&mut props, &inline, false);
    apply_rule_declarations(&mut props, &matching_rules, true);
    apply_declarations(&mut props, &inline, true);

    let declared_font_size = props.get("font-size").cloned();
    if let Some(parent) = parent {
        for &property in INHERITED_PROPERTIES {
            if !props.contains_key(property) {
                if let Some(value) = parent.props.get(property) {
                    props.insert(property.to_string(), value.clone());
                }
            }
        }
    }
    let inherited_size = parent.map_or(root_font_size, |style| style.font_size);
    let font_size = declared_font_size
        .as_deref()
        .and_then(|value| resolve_font_size(value, inherited_size, root_font_size))
        .unwrap_or(inherited_size);
    ComputedStyle { props, font_size }
}

const INHERITED_PROPERTIES: &[&str] = &[
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "line-height",
    "letter-spacing",
    "text-align",
];

fn apply_rule_declarations(
    props: &mut BTreeMap<String, String>,
    rules: &[((u32, u32, u32), usize, &StyleRule)],
    important: bool,
) {
    for (_, _, rule) in rules {
        apply_declarations(props, &rule.declarations, important);
    }
}

fn apply_declarations(
    props: &mut BTreeMap<String, String>,
    declarations: &[Declaration],
    important: bool,
) {
    for declaration in declarations
        .iter()
        .filter(|declaration| declaration.important == important)
    {
        props.insert(declaration.name.clone(), declaration.value.clone());
    }
}

fn resolve_font_size(value: &str, parent_size: f64, root_size: f64) -> Option<f64> {
    let context = LengthCtx {
        font_size: parent_size,
        root_font_size: root_size,
        viewport_w: 0.0,
        viewport_h: 0.0,
    };
    match parse_length(value, &context)? {
        CssLength::Px(value) => Some(value),
        CssLength::Percent(percent) => Some(parent_size * percent / 100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dom::DomElement;

    fn el(tag: &str, class: &str, style: &str) -> DomElement {
        let mut attrs = Vec::new();
        if !class.is_empty() {
            attrs.push(("class".into(), class.into()));
        }
        if !style.is_empty() {
            attrs.push(("style".into(), style.into()));
        }
        DomElement {
            tag: tag.into(),
            attrs,
            children: Vec::new(),
        }
    }

    #[test]
    fn specificity_and_order_win() {
        let (rules, _) = parse_stylesheet(
            "p { color: #111111 } .hot { color: #ff0000 } p { margin-top: 4px }",
            100,
        );
        let paragraph = el("p", "hot", "");
        let computed = compute_style(&[&paragraph], &rules, None, 16.0);
        assert_eq!(computed.get("color"), Some("#ff0000"));
        assert_eq!(computed.get("margin-top"), Some("4px"));
    }

    #[test]
    fn inline_beats_rules_but_important_beats_inline() {
        let (rules, _) = parse_stylesheet(".a { color: #00ff00 !important }", 100);
        let div = el("div", "a", "color: #0000ff");
        let computed = compute_style(&[&div], &rules, None, 16.0);
        assert_eq!(computed.get("color"), Some("#00ff00"));
    }

    #[test]
    fn inheritance_and_font_size_units() {
        let (rules, _) = parse_stylesheet("div { color: #333333; font-size: 20px }", 100);
        let parent_element = el("div", "", "");
        let parent = compute_style(&[&parent_element], &rules, None, 16.0);
        assert_eq!(parent.font_size, 20.0);
        let child_element = el("span", "", "font-size: 1.5em");
        let child = compute_style(
            &[&parent_element, &child_element],
            &rules,
            Some(&parent),
            16.0,
        );
        assert_eq!(child.get("color"), Some("#333333"));
        assert_eq!(child.font_size, 30.0);
    }

    #[test]
    fn at_rules_skipped_with_warning() {
        let (rules, warnings) = parse_stylesheet(
            "@media (max-width:600px){ p{color:red} } p{color:#222222}",
            0,
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn ua_defaults_apply() {
        let (rules, _) = parse_stylesheet(UA_STYLESHEET, 0);
        let heading = el("h1", "", "");
        let computed = compute_style(&[&heading], &rules, None, 16.0);
        assert_eq!(computed.font_size, 32.0);
        assert_eq!(computed.get("font-weight"), Some("700"));
    }
}
