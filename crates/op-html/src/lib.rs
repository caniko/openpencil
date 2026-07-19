//! HTML → PenNode importer (structured path, CSS-subset cascade).

use jian_ops_schema::document::PenDocument;
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
pub mod resources;
pub mod snapshot;
pub mod special;
pub mod text;

pub use snapshot::{import_snapshot, import_snapshot_document};

#[cfg(test)]
mod e2e_tests;

pub struct HtmlImportOptions {
    pub viewport_width: f64,
    pub base_font_size: f64,
    pub document_name: Option<String>,
    pub base_url: Option<String>,
}

impl Default for HtmlImportOptions {
    fn default() -> Self {
        Self {
            viewport_width: 1440.0,
            base_font_size: 16.0,
            document_name: None,
            base_url: None,
        }
    }
}

pub struct HtmlImportResult {
    pub nodes: Vec<PenNode>,
    pub warnings: Vec<String>,
}

pub struct HtmlDocumentResult {
    pub document: PenDocument,
    pub warnings: Vec<String>,
}

pub fn import_html(source: &str, opts: &HtmlImportOptions) -> HtmlImportResult {
    import_html_with_resources(source, opts, None, None)
}

pub fn import_html_with_resources(
    source: &str,
    opts: &HtmlImportOptions,
    fetcher: Option<&resources::ResourceFetcher<'_>>,
    transform: Option<&resources::ImageTransform<'_>>,
) -> HtmlImportResult {
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
    let mut budget = resources::ResourceBudget::default();
    for (index, href) in resources::stylesheet_links(source).into_iter().enumerate() {
        let resolved = resources::resolve_url(opts.base_url.as_deref(), &href);
        let display_url = resolved.as_deref().unwrap_or(&href);
        if !budget.take(&mut warnings) {
            continue;
        }
        let Some(stylesheet) = resolved
            .as_deref()
            .and_then(|url| fetcher.and_then(|fetch| fetch(url)))
        else {
            warnings.push(format!("external stylesheet skipped: {display_url}"));
            continue;
        };
        let stylesheet = String::from_utf8_lossy(&stylesheet);
        let (author_rules, stylesheet_warnings) =
            css::cascade::parse_stylesheet(&stylesheet, 500 + index * 10_000);
        rules.extend(author_rules);
        warnings.extend(stylesheet_warnings);
    }
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
    let mut nodes = vec![root];
    if let Some(fetcher) = fetcher {
        resources::embed_images(
            &mut nodes,
            opts.base_url.as_deref(),
            fetcher,
            transform,
            &mut budget,
            &mut warnings,
        );
    }
    HtmlImportResult { nodes, warnings }
}

pub fn import_html_document(
    source: &str,
    opts: &HtmlImportOptions,
    fetcher: Option<&resources::ResourceFetcher<'_>>,
    transform: Option<&resources::ImageTransform<'_>>,
) -> HtmlDocumentResult {
    wrap_imported_document(import_html_with_resources(source, opts, fetcher, transform))
}

pub(crate) fn wrap_imported_document(imported: HtmlImportResult) -> HtmlDocumentResult {
    let name = imported.nodes.first().and_then(|node| match node {
        PenNode::Frame(frame) => frame.base.name.clone(),
        _ => None,
    });
    HtmlDocumentResult {
        document: PenDocument {
            version: "1.0".to_string(),
            name,
            themes: None,
            variables: None,
            pages: None,
            children: imported.nodes,
            format_version: None,
            responsive: None,
            id: None,
            app: None,
            routes: None,
            state: None,
            lifecycle: None,
            logic_modules: None,
            design_md: None,
            conversion: None,
        },
        warnings: imported.warnings,
    }
}

const MAX_INPUT_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_OUTPUT_NODES: usize = 20_000;

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
        assert!(o.base_url.is_none());
    }

    #[test]
    fn external_stylesheet_participates_in_cascade() {
        let html = r#"<html><head><link rel="stylesheet" href="site.css"></head>
            <body><p class="hot">x</p></body></html>"#;
        let fetcher = |url: &str| -> Option<Vec<u8>> {
            (url == "https://a.dev/site.css").then(|| b".hot { color: #ff0000 }".to_vec())
        };
        let opts = HtmlImportOptions {
            base_url: Some("https://a.dev/page.html".into()),
            ..Default::default()
        };
        let r = import_html_with_resources(html, &opts, Some(&fetcher), None);
        let PenNode::Frame(root) = &r.nodes[0] else {
            panic!()
        };
        let PenNode::Frame(p) = &root.children.as_ref().unwrap()[0] else {
            panic!()
        };
        let PenNode::Text(t) = &p.children.as_ref().unwrap()[0] else {
            panic!()
        };
        let Some(fills) = &t.fill else {
            panic!("text should carry color fill")
        };
        assert!(matches!(&fills[0], PenFill::Solid(s) if s.color == "#ff0000"));
    }

    #[test]
    fn missing_fetcher_degrades_with_warning() {
        let html = r#"<link rel="stylesheet" href="https://a.dev/s.css"><p>x</p>"#;
        let r = import_html(html, &HtmlImportOptions::default());
        assert!(r
            .warnings
            .iter()
            .any(|warning| warning.contains("external stylesheet skipped")));
    }

    #[test]
    fn e2e_images_embed_via_fetcher_with_dedup_and_placeholder() {
        let html = r#"<div><img src="a.png"><img src="a.png"><img src="missing.png"></div>"#;
        let png: Vec<u8> = vec![0x89, 0x50, 0x4e, 0x47, 1, 2, 3];
        let fetched = std::cell::RefCell::new(0usize);
        let fetcher = |url: &str| -> Option<Vec<u8>> {
            *fetched.borrow_mut() += 1;
            (url == "https://a.dev/a.png").then(|| png.clone())
        };
        let opts = HtmlImportOptions {
            base_url: Some("https://a.dev/p.html".into()),
            ..Default::default()
        };
        let r = import_html_with_resources(html, &opts, Some(&fetcher), None);
        let PenNode::Frame(root) = &r.nodes[0] else {
            panic!()
        };
        let PenNode::Frame(div) = &root.children.as_ref().unwrap()[0] else {
            panic!()
        };
        let kids = div.children.as_ref().unwrap();
        let PenNode::Image(i1) = &kids[0] else {
            panic!()
        };
        let PenNode::Image(i2) = &kids[1] else {
            panic!()
        };
        let PenNode::Image(i3) = &kids[2] else {
            panic!()
        };
        assert!(i1.src.as_str().starts_with("data:image/png;base64,"));
        assert_eq!(i1.src.as_str(), i2.src.as_str());
        assert_eq!(*fetched.borrow(), 2);
        assert!(i3.src.as_str().starts_with("data:image/png;base64,"));
        assert!(r
            .warnings
            .iter()
            .any(|warning| warning.contains("missing.png")));
    }

    #[test]
    fn e2e_transform_callback_rewrites_bytes() {
        let html = r#"<img src="https://a.dev/big.jpg">"#;
        let fetcher = |_: &str| Some(vec![0xffu8, 0xd8, 9, 9, 9, 9]);
        let transform = |_: &[u8]| Some(vec![0xffu8, 0xd8, 1]);
        let r = import_html_with_resources(
            html,
            &HtmlImportOptions::default(),
            Some(&fetcher),
            Some(&transform),
        );
        let PenNode::Frame(root) = &r.nodes[0] else {
            panic!()
        };
        let PenNode::Image(img) = &root.children.as_ref().unwrap()[0] else {
            panic!()
        };
        use base64::Engine as _;
        let b64 = img
            .src
            .as_str()
            .strip_prefix("data:image/jpeg;base64,")
            .unwrap();
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(b64)
                .unwrap(),
            vec![0xff, 0xd8, 1]
        );
    }

    #[test]
    fn e2e_document_wrapper_produces_pendocument() {
        let r = import_html_document(
            "<html><head><title>T</title></head><body><p>x</p></body></html>",
            &HtmlImportOptions::default(),
            None,
            None,
        );
        assert_eq!(r.document.children.len(), 1);
        assert_eq!(r.document.name.as_deref(), Some("T"));
    }
}
