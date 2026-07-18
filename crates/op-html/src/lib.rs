//! HTML → PenNode importer (structured path, CSS-subset cascade).

use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::container::LayoutMode;
use jian_ops_schema::node::{FrameNode, PenNode};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};
use jian_ops_schema::style::{PenFill, SolidFillBody};

pub mod color;
pub mod css;
pub mod dom;
pub mod length;
pub mod mapper;
pub mod special;
pub mod text;

#[cfg(test)]
mod e2e_tests;

pub struct HtmlImportOptions {
    pub viewport_width: f64,
    pub base_font_size: f64,
    pub document_name: Option<String>,
}

impl Default for HtmlImportOptions {
    fn default() -> Self {
        Self {
            viewport_width: 1440.0,
            base_font_size: 16.0,
            document_name: None,
        }
    }
}

pub struct HtmlImportResult {
    pub nodes: Vec<PenNode>,
    pub warnings: Vec<String>,
}

pub fn import_html(source: &str, opts: &HtmlImportOptions) -> HtmlImportResult {
    let mut warnings = Vec::new();
    if source.trim().is_empty() {
        warnings.push("no importable content: input HTML is empty".to_string());
        return HtmlImportResult {
            nodes: Vec::new(),
            warnings,
        };
    }
    let source = truncate_source(source, &mut warnings);
    let mut parsed = dom::parse_dom(source);
    if parsed.body.is_empty() {
        warnings.push("no importable content: input HTML produced an empty body".to_string());
        return HtmlImportResult {
            nodes: Vec::new(),
            warnings,
        };
    }
    let mut remaining = MAX_OUTPUT_NODES - 1;
    if truncate_dom_nodes(&mut parsed.body, &mut remaining) {
        warnings.push("node limit reached (20000), remaining content dropped".to_string());
    }

    let (mut rules, ua_warnings) = css::cascade::parse_stylesheet(css::cascade::UA_STYLESHEET, 0);
    warnings.extend(ua_warnings);
    for (index, stylesheet) in parsed.style_blocks.iter().enumerate() {
        let (author_rules, stylesheet_warnings) =
            css::cascade::parse_stylesheet(stylesheet, 1000 + index * 10_000);
        rules.extend(author_rules);
        warnings.extend(stylesheet_warnings);
    }

    let body = dom::DomElement {
        tag: "body".to_string(),
        attrs: Vec::new(),
        children: parsed.body,
    };
    let body_style = css::cascade::compute_style(&[&body], &rules, None, opts.base_font_size);
    let mut context = mapper::MapCtx {
        opts,
        rules: &rules,
        warnings: Vec::new(),
        next_id: 0,
        node_count: 1,
    };
    let root_id = context.generate_id();
    let mut container = mapper::container_props_from(&body_style, &mut context);
    container.width = Some(SizingBehavior::Number(opts.viewport_width));
    container.height = Some(SizingBehavior::Keyword(SizingKeyword::FitContent));
    container.layout = Some(LayoutMode::Vertical);
    if container.fill.is_none() {
        container.fill = Some(vec![solid_fill("#ffffff")]);
    }
    let children = text::map_children(&mut context, &[&body], &body_style, &body.children);
    let name = opts
        .document_name
        .clone()
        .or(parsed.title)
        .unwrap_or_else(|| "HTML Import".to_string());
    let root = PenNode::Frame(FrameNode {
        base: PenNodeBase {
            id: root_id,
            name: Some(name),
            ..Default::default()
        },
        container,
        children: Some(children),
        image_search_query: None,
        reusable: None,
        slot: None,
        state: None,
        bindings: None,
        events: None,
        lifecycle: None,
        semantics: None,
        gestures: None,
        route: None,
        screen: None,
        breakpoint: None,
    });
    warnings.extend(context.warnings);
    HtmlImportResult {
        nodes: vec![root],
        warnings,
    }
}

const MAX_INPUT_BYTES: usize = 10 * 1024 * 1024;
const MAX_OUTPUT_NODES: usize = 20_000;

fn truncate_source<'a>(source: &'a str, warnings: &mut Vec<String>) -> &'a str {
    if source.len() <= MAX_INPUT_BYTES {
        return source;
    }
    let mut end = MAX_INPUT_BYTES;
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    warnings.push("input HTML exceeded 10MB and was truncated".to_string());
    &source[..end]
}

fn truncate_dom_nodes(nodes: &mut Vec<dom::DomNode>, remaining: &mut usize) -> bool {
    let original_len = nodes.len();
    let mut keep = 0;
    let mut truncated = false;
    for node in nodes.iter_mut() {
        if *remaining == 0 {
            truncated = true;
            break;
        }
        *remaining -= 1;
        keep += 1;
        if let dom::DomNode::Element(element) = node {
            truncated |= truncate_dom_nodes(&mut element.children, remaining);
        }
    }
    if keep < original_len {
        nodes.truncate(keep);
        truncated = true;
    }
    truncated
}

fn solid_fill(color: &str) -> PenFill {
    PenFill::Solid(SolidFillBody {
        color: color.to_string(),
        explain: None,
        opacity: None,
        blend_mode: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_nodes_and_a_warning() {
        let r = import_html("", &HtmlImportOptions::default());
        assert!(r.nodes.is_empty());
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("no importable content"));
    }

    #[test]
    fn options_default_values() {
        let o = HtmlImportOptions::default();
        assert_eq!(o.viewport_width, 1440.0);
        assert_eq!(o.base_font_size, 16.0);
        assert!(o.document_name.is_none());
    }
}
