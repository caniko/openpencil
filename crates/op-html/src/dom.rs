use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

const DROP_TAGS: &[&str] = &["script", "noscript", "template", "meta", "link", "head"];
pub(crate) const MAX_DOM_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub enum DomNode {
    Element(DomElement),
    Text(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DomElement {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<DomNode>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StylesheetSource {
    Inline(String),
    Link(String),
}

impl DomElement {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(attr_name, _)| attr_name == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn classes(&self) -> Vec<&str> {
        self.attr("class")
            .map(|value| value.split_whitespace().collect())
            .unwrap_or_default()
    }

    pub fn id(&self) -> Option<&str> {
        self.attr("id")
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedDom {
    pub body: Vec<DomNode>,
    pub html_attrs: Vec<(String, String)>,
    pub head_attrs: Vec<(String, String)>,
    pub body_attrs: Vec<(String, String)>,
    pub style_blocks: Vec<String>,
    pub stylesheet_sources: Vec<StylesheetSource>,
    pub title: Option<String>,
    pub depth_truncated: bool,
}

pub fn parse_dom(source: &str) -> ParsedDom {
    let mut bytes = source.as_bytes();
    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut bytes)
        .expect("reading from &[u8] cannot fail");
    let mut out = ParsedDom::default();
    walk_document(&dom.document, &mut out, 0);
    out
}

fn walk_document(handle: &Handle, out: &mut ParsedDom, depth: usize) {
    if depth >= MAX_DOM_DEPTH {
        out.depth_truncated = true;
        return;
    }
    if let NodeData::Element { name, attrs, .. } = &handle.data {
        match name.local.as_ref() {
            "html" => {
                out.html_attrs = collect_attrs(&attrs.borrow());
            }
            "head" => {
                out.head_attrs = collect_attrs(&attrs.borrow());
                harvest_head(handle, out, depth);
                return;
            }
            "body" => {
                out.body_attrs = collect_attrs(&attrs.borrow());
                for child in handle.children.borrow().iter() {
                    if let Some(node) = convert(child, out, depth + 1) {
                        out.body.push(node);
                    }
                }
                return;
            }
            _ => {}
        }
    }
    for child in handle.children.borrow().iter() {
        walk_document(child, out, depth + 1);
    }
}

fn collect_attrs(attrs: &[html5ever::Attribute]) -> Vec<(String, String)> {
    attrs
        .iter()
        .map(|attr| {
            (
                attr.name.local.to_string().to_lowercase(),
                attr.value.to_string(),
            )
        })
        .collect()
}

fn harvest_head(handle: &Handle, out: &mut ParsedDom, depth: usize) {
    if depth >= MAX_DOM_DEPTH {
        out.depth_truncated = true;
        return;
    }
    if let NodeData::Element { name, attrs, .. } = &handle.data {
        match name.local.as_ref() {
            "style" => {
                push_inline_stylesheet(out, text_content(handle));
                return;
            }
            "link" => {
                if let Some(href) = stylesheet_href(&attrs.borrow()) {
                    out.stylesheet_sources.push(StylesheetSource::Link(href));
                }
                return;
            }
            "title" => {
                out.title = Some(text_content(handle));
                return;
            }
            _ => {}
        }
    }
    for child in handle.children.borrow().iter() {
        harvest_head(child, out, depth + 1);
    }
}

fn push_inline_stylesheet(out: &mut ParsedDom, stylesheet: String) {
    out.style_blocks.push(stylesheet.clone());
    out.stylesheet_sources
        .push(StylesheetSource::Inline(stylesheet));
}

fn stylesheet_href(attrs: &[html5ever::Attribute]) -> Option<String> {
    let rel = attrs
        .iter()
        .find(|attr| attr.name.local.as_ref() == "rel")?
        .value
        .to_string();
    let is_stylesheet = rel
        .split_whitespace()
        .any(|token| token.eq_ignore_ascii_case("stylesheet"));
    is_stylesheet.then(|| {
        attrs
            .iter()
            .find(|attr| attr.name.local.as_ref() == "href")
            .map(|attr| attr.value.to_string())
    })?
}

fn text_content(handle: &Handle) -> String {
    let mut output = String::new();
    let mut pending = vec![handle.clone()];
    while let Some(node) = pending.pop() {
        if let NodeData::Text { contents } = &node.data {
            output.push_str(&contents.borrow());
        } else {
            pending.extend(node.children.borrow().iter().rev().cloned());
        }
    }
    output
}

fn convert(handle: &Handle, out: &mut ParsedDom, depth: usize) -> Option<DomNode> {
    if depth >= MAX_DOM_DEPTH {
        out.depth_truncated = true;
        return None;
    }
    match &handle.data {
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.to_string().to_lowercase();
            if tag == "style" {
                push_inline_stylesheet(out, text_content(handle));
                return None;
            }
            if tag == "link" {
                if let Some(href) = stylesheet_href(&attrs.borrow()) {
                    out.stylesheet_sources.push(StylesheetSource::Link(href));
                }
                return None;
            }
            if tag == "title" {
                out.title = Some(text_content(handle));
                return None;
            }
            if DROP_TAGS.contains(&tag.as_str()) {
                return None;
            }
            let attrs = attrs
                .borrow()
                .iter()
                .map(|attr| {
                    (
                        attr.name.local.to_string().to_lowercase(),
                        attr.value.to_string(),
                    )
                })
                .collect();
            let children = handle
                .children
                .borrow()
                .iter()
                .filter_map(|child| convert(child, out, depth + 1))
                .collect();
            Some(DomNode::Element(DomElement {
                tag,
                attrs,
                children,
            }))
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            // Whitespace-only nodes can be significant in an inline formatting
            // context.  In particular, dropping the node between two inline
            // elements turns `<span>a</span> <span>b</span>` into `ab` before
            // the text mapper has a chance to apply CSS whitespace collapsing.
            (!text.is_empty()).then_some(DomNode::Text(text))
        }
        NodeData::Document => None,
        NodeData::Doctype { .. }
        | NodeData::Comment { .. }
        | NodeData::ProcessingInstruction { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fragment_without_wrapper() {
        let d = parse_dom("<div class=\"a b\" id=\"x\"><p>hi</p></div>");
        assert_eq!(d.body.len(), 1);
        let DomNode::Element(div) = &d.body[0] else {
            panic!("expected element")
        };
        assert_eq!(div.tag, "div");
        assert_eq!(div.classes(), vec!["a", "b"]);
        assert_eq!(div.id(), Some("x"));
        let DomNode::Element(p) = &div.children[0] else {
            panic!("expected p")
        };
        assert_eq!(p.tag, "p");
        assert!(matches!(&p.children[0], DomNode::Text(t) if t == "hi"));
    }

    #[test]
    fn collects_style_blocks_and_title_drops_script() {
        let d = parse_dom(
            "<html><head><title>Page</title><style>.a{color:red}</style></head>\
             <body><script>evil()</script><span>ok</span></body></html>",
        );
        assert_eq!(d.title.as_deref(), Some("Page"));
        assert_eq!(d.style_blocks, vec![".a{color:red}".to_string()]);
        assert_eq!(
            d.stylesheet_sources,
            vec![StylesheetSource::Inline(".a{color:red}".to_string())]
        );
        assert_eq!(d.body.len(), 1);
    }

    #[test]
    fn tolerates_dirty_html() {
        let d = parse_dom("<div><b>unclosed<div>next</div>");
        assert!(!d.body.is_empty());
    }

    #[test]
    fn style_inside_body_is_still_collected() {
        let d = parse_dom("<div><style>p{margin:0}</style><p>x</p></div>");
        assert_eq!(d.style_blocks, vec!["p{margin:0}".to_string()]);
    }

    #[test]
    fn preserves_whitespace_between_inline_elements() {
        let d = parse_dom("<p><span>a</span> <span>b</span></p>");
        let DomNode::Element(paragraph) = &d.body[0] else {
            panic!("expected paragraph")
        };
        assert!(matches!(
            &paragraph.children[1],
            DomNode::Text(text) if text == " "
        ));
    }

    #[test]
    fn preserves_body_attributes_separately_from_body_children() {
        let d = parse_dom("<body class=page lang=zh-CN style='color:red'><main>x</main></body>");
        assert_eq!(
            d.body_attrs,
            vec![
                ("class".to_string(), "page".to_string()),
                ("lang".to_string(), "zh-CN".to_string()),
                ("style".to_string(), "color:red".to_string()),
            ]
        );
        assert_eq!(d.body.len(), 1);
    }

    #[test]
    fn preserves_document_element_and_head_attributes() {
        let d = parse_dom(
            "<html class=theme lang=en><head id=metadata></head><body><p>x</p></body></html>",
        );
        assert_eq!(
            d.html_attrs,
            vec![
                ("class".to_string(), "theme".to_string()),
                ("lang".to_string(), "en".to_string()),
            ]
        );
        assert_eq!(
            d.head_attrs,
            vec![("id".to_string(), "metadata".to_string())]
        );
    }

    #[test]
    fn fragment_keeps_the_parser_generated_document_chain() {
        let d = parse_dom("<main>x</main>");
        assert!(d.html_attrs.is_empty());
        assert!(d.head_attrs.is_empty());
        assert!(matches!(d.body.as_slice(), [DomNode::Element(main)] if main.tag == "main"));
    }

    #[test]
    fn preserves_link_and_inline_stylesheet_source_order() {
        let d = parse_dom(
            "<head><link rel='preload stylesheet' href='a.css'><style>.a{color:red}</style>\
             <link rel='stylesheet' href='b.css'></head><body>x</body>",
        );
        assert_eq!(
            d.stylesheet_sources,
            vec![
                StylesheetSource::Link("a.css".into()),
                StylesheetSource::Inline(".a{color:red}".into()),
                StylesheetSource::Link("b.css".into()),
            ]
        );
    }

    #[test]
    fn truncates_pathological_dom_depth_before_recursive_mapping() {
        let mut html = String::new();
        for _ in 0..(MAX_DOM_DEPTH + 100) {
            html.push_str("<div>");
        }
        html.push('x');
        for _ in 0..(MAX_DOM_DEPTH + 100) {
            html.push_str("</div>");
        }
        let parsed = parse_dom(&html);
        assert!(parsed.depth_truncated);
    }
}
