use html5ever::tendril::TendrilSink;
use html5ever::{parse_document, ParseOpts};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

const DROP_TAGS: &[&str] = &["script", "noscript", "template", "meta", "link", "head"];

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
    pub style_blocks: Vec<String>,
    pub title: Option<String>,
}

pub fn parse_dom(source: &str) -> ParsedDom {
    let mut bytes = source.as_bytes();
    let dom = parse_document(RcDom::default(), ParseOpts::default())
        .from_utf8()
        .read_from(&mut bytes)
        .expect("reading from &[u8] cannot fail");
    let mut out = ParsedDom::default();
    walk_document(&dom.document, &mut out);
    out
}

fn walk_document(handle: &Handle, out: &mut ParsedDom) {
    if let NodeData::Element { name, .. } = &handle.data {
        match name.local.as_ref() {
            "head" => {
                harvest_head(handle, out);
                return;
            }
            "body" => {
                for child in handle.children.borrow().iter() {
                    if let Some(node) = convert(child, out) {
                        out.body.push(node);
                    }
                }
                return;
            }
            _ => {}
        }
    }
    for child in handle.children.borrow().iter() {
        walk_document(child, out);
    }
}

fn harvest_head(handle: &Handle, out: &mut ParsedDom) {
    if let NodeData::Element { name, .. } = &handle.data {
        match name.local.as_ref() {
            "style" => {
                out.style_blocks.push(text_content(handle));
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
        harvest_head(child, out);
    }
}

fn text_content(handle: &Handle) -> String {
    match &handle.data {
        NodeData::Text { contents } => contents.borrow().to_string(),
        _ => handle.children.borrow().iter().map(text_content).collect(),
    }
}

fn convert(handle: &Handle, out: &mut ParsedDom) -> Option<DomNode> {
    match &handle.data {
        NodeData::Element { name, attrs, .. } => {
            let tag = name.local.to_string().to_lowercase();
            if tag == "style" {
                out.style_blocks.push(text_content(handle));
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
                .filter_map(|child| convert(child, out))
                .collect();
            Some(DomNode::Element(DomElement {
                tag,
                attrs,
                children,
            }))
        }
        NodeData::Text { contents } => {
            let text = contents.borrow().to_string();
            (!text.trim().is_empty()).then_some(DomNode::Text(text))
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
}
