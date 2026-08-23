use jian_ops_schema::node::{
    AlignItems, ContainerProps, CornerRadius, JustifyContent, LayoutMode, PenNode, PenNodeBase,
};
use jian_ops_schema::sizing::SizingBehavior;
use jian_ops_schema::style::{PenEffect, PenFill, PenStroke};

pub(crate) fn justify(j: &JustifyContent) -> &'static str {
    match j {
        JustifyContent::Start => "start",
        JustifyContent::Center => "center",
        JustifyContent::End => "end",
        JustifyContent::SpaceBetween => "space_between",
        JustifyContent::SpaceAround => "space_around",
    }
}

pub(crate) fn align(a: &AlignItems) -> &'static str {
    match a {
        AlignItems::Start => "start",
        AlignItems::Center => "center",
        AlignItems::End => "end",
        AlignItems::Stretch => "stretch",
    }
}

pub(crate) fn is_flex(n: &PenNode) -> bool {
    matches!(
        container_of(n).and_then(|c| c.layout.as_ref()),
        Some(LayoutMode::Vertical | LayoutMode::Horizontal)
    )
}

pub(crate) fn node_id(n: &PenNode) -> &str {
    node_base(n).id.as_str()
}

pub(crate) fn node_kind(n: &PenNode) -> &'static str {
    match n {
        PenNode::Frame(_) => "frame",
        PenNode::Group(_) => "group",
        PenNode::Rectangle(_) => "rectangle",
        PenNode::Ellipse(_) => "ellipse",
        PenNode::Line(_) => "line",
        PenNode::Polygon(_) => "polygon",
        PenNode::Path(_) => "path",
        PenNode::Text(_) => "text",
        PenNode::TextInput(_) => "text_input",
        PenNode::Image(_) => "image",
        PenNode::IconFont(_) => "icon_font",
        PenNode::TextArea(_) => "text_area",
        PenNode::Select(_) => "select",
        PenNode::Switch(_) => "switch",
        PenNode::Checkbox(_) => "checkbox",
        PenNode::Slider(_) => "slider",
        PenNode::RadioGroup(_) => "radio_group",
        PenNode::NumberInput(_) => "number_input",
        PenNode::Progress(_) => "progress",
        PenNode::Tabs(_) => "tabs",
        PenNode::Ref(_) => "ref",
    }
}

pub(crate) fn node_base(n: &PenNode) -> &PenNodeBase {
    match n {
        PenNode::Frame(n) => &n.base,
        PenNode::Group(n) => &n.base,
        PenNode::Rectangle(n) => &n.base,
        PenNode::Ellipse(n) => &n.base,
        PenNode::Line(n) => &n.base,
        PenNode::Polygon(n) => &n.base,
        PenNode::Path(n) => &n.base,
        PenNode::Text(n) => &n.base,
        PenNode::TextInput(n) => &n.base,
        PenNode::Image(n) => &n.base,
        PenNode::IconFont(n) => &n.base,
        PenNode::TextArea(n) => &n.base,
        PenNode::Select(n) => &n.base,
        PenNode::Switch(n) => &n.base,
        PenNode::Checkbox(n) => &n.base,
        PenNode::Slider(n) => &n.base,
        PenNode::RadioGroup(n) => &n.base,
        PenNode::NumberInput(n) => &n.base,
        PenNode::Progress(n) => &n.base,
        PenNode::Tabs(n) => &n.base,
        PenNode::Ref(n) => &n.base,
    }
}

pub(crate) fn is_raster_leaf(n: &PenNode) -> bool {
    children_of(n).is_empty()
        && matches!(
            n,
            PenNode::Ellipse(_)
                | PenNode::Line(_)
                | PenNode::Polygon(_)
                | PenNode::Path(_)
                | PenNode::IconFont(_)
        )
}

pub(crate) fn page_index_of(
    doc: &jian_ops_schema::PenDocument,
    root_id: &str,
) -> Result<usize, crate::ExportError> {
    let Some(pages) = &doc.pages else {
        return Ok(0);
    };
    for (i, page) in pages.iter().enumerate() {
        if page.children.iter().any(|c| contains_id(c, root_id)) {
            return Ok(i);
        }
    }
    Err(crate::ExportError::msg(format!(
        "root {root_id} is not under pages"
    )))
}

fn contains_id(n: &PenNode, id: &str) -> bool {
    node_id(n) == id || children_of(n).iter().any(|c| contains_id(c, id))
}

pub(crate) fn select_root<'a>(
    doc: &'a jian_ops_schema::PenDocument,
    item: Option<&str>,
    index: &std::collections::HashMap<String, &'a PenNode>,
) -> Result<&'a PenNode, crate::ExportError> {
    if let Some(id) = item {
        return index
            .get(id)
            .copied()
            .ok_or_else(|| crate::ExportError::msg(format!("--item {id} not found")));
    }
    if let Some(pages) = &doc.pages {
        if let Some(page) = pages.first() {
            return one_root(&page.children, "first page");
        }
    }
    one_root(&doc.children, "document")
}

fn one_root<'a>(nodes: &'a [PenNode], where_: &str) -> Result<&'a PenNode, crate::ExportError> {
    match nodes {
        [] => Err(crate::ExportError::msg(format!("{where_} has no children"))),
        [n] => Ok(n),
        many => Err(crate::ExportError::msg(format!(
            "{where_} has {} roots; pass --item",
            many.len()
        ))),
    }
}

pub(crate) fn index_nodes(
    doc: &jian_ops_schema::PenDocument,
) -> std::collections::HashMap<String, &PenNode> {
    let mut m = std::collections::HashMap::new();
    fn walk<'a>(n: &'a PenNode, m: &mut std::collections::HashMap<String, &'a PenNode>) {
        m.insert(node_id(n).to_string(), n);
        for c in children_of(n) {
            walk(c, m);
        }
    }
    if let Some(pages) = &doc.pages {
        for p in pages {
            for c in &p.children {
                walk(c, &mut m);
            }
        }
    }
    for c in &doc.children {
        walk(c, &mut m);
    }
    m
}

pub(crate) fn children_of(n: &PenNode) -> &[PenNode] {
    match n {
        PenNode::Frame(n) => n.children.as_deref().unwrap_or(&[]),
        PenNode::Group(n) => n.children.as_deref().unwrap_or(&[]),
        PenNode::Rectangle(n) => n.children.as_deref().unwrap_or(&[]),
        PenNode::Tabs(n) => n.children.as_deref().unwrap_or(&[]),
        PenNode::Ref(n) => n.children.as_deref().unwrap_or(&[]),
        _ => &[],
    }
}

pub(crate) fn container_of(n: &PenNode) -> Option<&ContainerProps> {
    match n {
        PenNode::Frame(n) => Some(&n.container),
        PenNode::Group(n) => Some(&n.container),
        PenNode::Rectangle(n) => Some(&n.container),
        _ => None,
    }
}

pub(crate) fn node_size(n: &PenNode) -> (Option<&SizingBehavior>, Option<&SizingBehavior>) {
    match n {
        PenNode::Frame(n) => (n.container.width.as_ref(), n.container.height.as_ref()),
        PenNode::Group(n) => (n.container.width.as_ref(), n.container.height.as_ref()),
        PenNode::Rectangle(n) => (n.container.width.as_ref(), n.container.height.as_ref()),
        PenNode::Ellipse(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Line(_) => (None, None),
        PenNode::Polygon(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Path(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Text(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::TextInput(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Image(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::IconFont(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::TextArea(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Select(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Switch(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Checkbox(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Slider(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::RadioGroup(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::NumberInput(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Progress(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Tabs(n) => (n.width.as_ref(), n.height.as_ref()),
        PenNode::Ref(_) => (None, None),
    }
}

pub(crate) fn fills_of(n: &PenNode) -> Option<&[PenFill]> {
    match n {
        PenNode::Frame(n) => n.container.fill.as_deref(),
        PenNode::Group(n) => n.container.fill.as_deref(),
        PenNode::Rectangle(n) => n.container.fill.as_deref(),
        PenNode::Text(n) => n.fill.as_deref(),
        PenNode::Tabs(n) => n.fill.as_deref(),
        PenNode::Ellipse(n) => n.fill.as_deref(),
        PenNode::Polygon(n) => n.fill.as_deref(),
        PenNode::Path(n) => n.fill.as_deref(),
        _ => None,
    }
}

pub(crate) fn stroke_of(n: &PenNode) -> Option<&PenStroke> {
    match n {
        PenNode::Frame(n) => n.container.stroke.as_ref(),
        PenNode::Group(n) => n.container.stroke.as_ref(),
        PenNode::Rectangle(n) => n.container.stroke.as_ref(),
        PenNode::Tabs(n) => n.stroke.as_ref(),
        PenNode::Ellipse(n) => n.stroke.as_ref(),
        PenNode::Polygon(n) => n.stroke.as_ref(),
        PenNode::Path(n) => n.stroke.as_ref(),
        PenNode::Line(n) => n.stroke.as_ref(),
        _ => None,
    }
}

pub(crate) fn effects_of(n: &PenNode) -> Option<&[PenEffect]> {
    match n {
        PenNode::Frame(n) => n.container.effects.as_deref(),
        PenNode::Group(n) => n.container.effects.as_deref(),
        PenNode::Rectangle(n) => n.container.effects.as_deref(),
        PenNode::Text(n) => n.effects.as_deref(),
        PenNode::Image(n) => n.effects.as_deref(),
        PenNode::Tabs(n) => n.effects.as_deref(),
        PenNode::Ellipse(n) => n.effects.as_deref(),
        PenNode::Polygon(n) => n.effects.as_deref(),
        PenNode::Path(n) => n.effects.as_deref(),
        _ => None,
    }
}

pub(crate) fn corner_of(n: &PenNode) -> Option<&CornerRadius> {
    match n {
        PenNode::Frame(n) => n.container.corner_radius.as_ref(),
        PenNode::Group(n) => n.container.corner_radius.as_ref(),
        PenNode::Rectangle(n) => n.container.corner_radius.as_ref(),
        PenNode::Image(n) => n.corner_radius.as_ref(),
        PenNode::Tabs(n) => n.corner_radius.as_ref(),
        _ => None,
    }
}
