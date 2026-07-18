use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::text::{
    FontStyleKind as TextFontStyle, FontWeight, TextAlign, TextContent, TextNode,
};
use jian_ops_schema::node::PenNode;
use jian_ops_schema::style::{
    FontStyleKind as SegmentFontStyle, PenFill, SolidFillBody, StyledTextSegment,
};

use crate::color::parse_css_color;
use crate::css::cascade::{compute_style, ComputedStyle};
use crate::dom::{DomElement, DomNode};
use crate::length::{parse_length, CssLength, LengthCtx};
use crate::mapper::{map_element, MapCtx};

pub fn is_inline_tag(tag: &str) -> bool {
    matches!(
        tag,
        "a" | "b"
            | "strong"
            | "i"
            | "em"
            | "u"
            | "s"
            | "del"
            | "strike"
            | "span"
            | "code"
            | "small"
            | "sub"
            | "sup"
            | "label"
            | "br"
            | "mark"
    )
}

#[derive(Clone, Debug, PartialEq)]
struct SegStyle {
    weight: Option<u32>,
    style: Option<SegmentFontStyle>,
    underline: Option<bool>,
    strike: Option<bool>,
    fill: Option<String>,
    href: Option<String>,
    font_size: Option<f32>,
    font_family: Option<String>,
}

#[derive(Clone, Debug)]
struct Segment {
    text: String,
    style: SegStyle,
}

pub fn build_text_node(
    context: &mut MapCtx<'_>,
    run: &[&DomNode],
    block_style: &ComputedStyle,
) -> Option<PenNode> {
    let base_style = segment_style(block_style, None);
    let mut segments = Vec::new();
    for node in run {
        collect_segments(node, context, block_style, &base_style, &mut segments);
    }
    segments.retain(|segment| !segment.text.is_empty());
    if segments
        .iter()
        .all(|segment| segment.text.trim().is_empty())
    {
        return None;
    }
    merge_adjacent_segments(&mut segments);
    let plain = segments.iter().all(|segment| segment.style == base_style);
    let content = if plain {
        TextContent::Plain(collapse_complete(
            &segments
                .iter()
                .map(|segment| segment.text.as_str())
                .collect::<String>(),
        ))
    } else {
        TextContent::Styled(
            segments
                .into_iter()
                .map(|segment| StyledTextSegment {
                    text: segment.text,
                    font_family: segment.style.font_family,
                    font_size: segment.style.font_size,
                    font_weight: segment.style.weight,
                    font_style: segment.style.style,
                    fill: segment.style.fill,
                    underline: segment.style.underline,
                    strikethrough: segment.style.strike,
                    href: segment.style.href,
                })
                .collect(),
        )
    };
    let node = PenNode::Text(TextNode {
        base: PenNodeBase {
            id: context.generate_id(),
            name: Some("Text".to_string()),
            ..Default::default()
        },
        width: None,
        height: None,
        content,
        font_family: block_style.get("font-family").map(str::to_string),
        font_size: Some(block_style.font_size),
        font_weight: parse_weight(block_style.get("font-weight")).map(FontWeight::Number),
        font_style: parse_text_font_style(block_style.get("font-style")),
        letter_spacing: block_style
            .get("letter-spacing")
            .and_then(|value| text_length_px(value, block_style, context)),
        line_height: block_style
            .get("line-height")
            .and_then(|value| parse_line_height(value, block_style, context)),
        text_align: block_style.get("text-align").and_then(parse_text_align),
        text_align_vertical: None,
        text_growth: None,
        underline: decoration(block_style, "underline").then_some(true),
        strikethrough: decoration(block_style, "line-through").then_some(true),
        fill: block_style
            .get("color")
            .and_then(parse_css_color)
            .map(|color| vec![solid_fill(color)]),
        effects: None,
        state: None,
        bindings: None,
        events: None,
        lifecycle: None,
        semantics: None,
        gestures: None,
        route: None,
        limits: Default::default(),
    });
    context.node_count += 1;
    Some(node)
}

pub(crate) fn map_children(
    context: &mut MapCtx<'_>,
    path: &[&DomElement],
    block_style: &ComputedStyle,
    children: &[DomNode],
) -> Vec<PenNode> {
    let mut nodes = Vec::new();
    let mut run = Vec::new();
    for child in children {
        let inline = matches!(child, DomNode::Text(_))
            || matches!(child, DomNode::Element(element) if is_inline_tag(&element.tag));
        if inline {
            run.push(child);
            continue;
        }
        flush_run(context, &mut nodes, &mut run, block_style);
        if let DomNode::Element(element) = child {
            let mut child_path = path.to_vec();
            child_path.push(element);
            if let Some(node) = map_element(context, &child_path, Some(block_style)) {
                nodes.push(node);
            }
        }
    }
    flush_run(context, &mut nodes, &mut run, block_style);
    nodes
}

fn flush_run(
    context: &mut MapCtx<'_>,
    nodes: &mut Vec<PenNode>,
    run: &mut Vec<&DomNode>,
    block_style: &ComputedStyle,
) {
    if let Some(node) = build_text_node(context, run, block_style) {
        nodes.push(node);
    }
    run.clear();
}

fn collect_segments(
    node: &DomNode,
    context: &MapCtx<'_>,
    parent_style: &ComputedStyle,
    inherited: &SegStyle,
    segments: &mut Vec<Segment>,
) {
    match node {
        DomNode::Text(text) => push_segment(segments, collapse_segment(text), inherited.clone()),
        DomNode::Element(element) if element.tag == "br" => {
            push_segment(segments, "\n".to_string(), inherited.clone());
        }
        DomNode::Element(element) => {
            let computed = compute_style(
                &[element],
                context.rules,
                Some(parent_style),
                context.opts.base_font_size,
            );
            let mut style = segment_style(&computed, inherited.href.clone());
            apply_semantic_style(element, &mut style);
            for child in &element.children {
                collect_segments(child, context, &computed, &style, segments);
            }
        }
    }
}

fn segment_style(style: &ComputedStyle, href: Option<String>) -> SegStyle {
    SegStyle {
        weight: parse_weight(style.get("font-weight")),
        style: match style.get("font-style") {
            Some("italic" | "oblique") => Some(SegmentFontStyle::Italic),
            Some("normal") => Some(SegmentFontStyle::Normal),
            _ => None,
        },
        underline: decoration(style, "underline").then_some(true),
        strike: decoration(style, "line-through").then_some(true),
        fill: style.get("color").and_then(parse_css_color),
        href,
        font_size: Some(style.font_size as f32),
        font_family: style.get("font-family").map(str::to_string),
    }
}

fn apply_semantic_style(element: &DomElement, style: &mut SegStyle) {
    match element.tag.as_str() {
        "b" | "strong" => style.weight = Some(700),
        "i" | "em" => style.style = Some(SegmentFontStyle::Italic),
        "u" => style.underline = Some(true),
        "s" | "del" | "strike" => style.strike = Some(true),
        "a" => style.href = element.attr("href").map(str::to_string),
        _ => {}
    }
}

fn push_segment(segments: &mut Vec<Segment>, text: String, style: SegStyle) {
    if !text.is_empty() {
        segments.push(Segment { text, style });
    }
}

fn merge_adjacent_segments(segments: &mut Vec<Segment>) {
    let mut merged: Vec<Segment> = Vec::new();
    for segment in segments.drain(..) {
        if let Some(previous) = merged.last_mut() {
            if previous.style == segment.style {
                previous.text.push_str(&segment.text);
                continue;
            }
        }
        merged.push(segment);
    }
    *segments = merged;
}

fn collapse_segment(text: &str) -> String {
    if text.contains('\n') {
        return collapse_complete(text);
    }
    let leading = text.chars().next().is_some_and(char::is_whitespace);
    let trailing = text.chars().last().is_some_and(char::is_whitespace);
    let middle = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if middle.is_empty() {
        return if leading {
            " ".to_string()
        } else {
            String::new()
        };
    }
    format!(
        "{}{}{}",
        if leading { " " } else { "" },
        middle,
        if trailing { " " } else { "" }
    )
}

fn collapse_complete(text: &str) -> String {
    text.split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_weight(value: Option<&str>) -> Option<u32> {
    match value? {
        "normal" | "regular" => Some(400),
        "bold" => Some(700),
        value => value.parse().ok(),
    }
}

fn decoration(style: &ComputedStyle, needle: &str) -> bool {
    style
        .get("text-decoration-line")
        .or_else(|| style.get("text-decoration"))
        .is_some_and(|value| value.split_whitespace().any(|part| part == needle))
}

fn parse_text_font_style(value: Option<&str>) -> Option<TextFontStyle> {
    match value? {
        "italic" | "oblique" => Some(TextFontStyle::Italic),
        "normal" => Some(TextFontStyle::Normal),
        _ => None,
    }
}

fn parse_text_align(value: &str) -> Option<TextAlign> {
    match value {
        "left" | "start" => Some(TextAlign::Left),
        "center" => Some(TextAlign::Center),
        "right" | "end" => Some(TextAlign::Right),
        "justify" => Some(TextAlign::Justify),
        _ => None,
    }
}

fn text_length_px(value: &str, style: &ComputedStyle, context: &MapCtx<'_>) -> Option<f64> {
    match parse_length(value, &text_length_context(style, context))? {
        CssLength::Px(value) => Some(value),
        CssLength::Percent(_) => None,
    }
}

fn parse_line_height(value: &str, style: &ComputedStyle, context: &MapCtx<'_>) -> Option<f64> {
    if let Ok(multiplier) = value.parse::<f64>() {
        return Some(multiplier);
    }
    match parse_length(value, &text_length_context(style, context))? {
        CssLength::Px(value) => Some(value / style.font_size),
        CssLength::Percent(percent) => Some(percent / 100.0),
    }
}

fn text_length_context(style: &ComputedStyle, context: &MapCtx<'_>) -> LengthCtx {
    LengthCtx {
        font_size: style.font_size,
        root_font_size: context.opts.base_font_size,
        viewport_w: context.opts.viewport_width,
        viewport_h: context.opts.viewport_width * 0.625,
    }
}

fn solid_fill(color: String) -> PenFill {
    PenFill::Solid(SolidFillBody {
        color,
        explain: None,
        opacity: None,
        blend_mode: None,
    })
}

#[cfg(test)]
mod tests {
    use jian_ops_schema::node::text::TextContent;
    use jian_ops_schema::node::PenNode;

    fn text_of(html: &str, css: &str) -> Vec<PenNode> {
        let dom = crate::dom::parse_dom(html);
        let (mut rules, _) =
            crate::css::cascade::parse_stylesheet(crate::css::cascade::UA_STYLESHEET, 0);
        let (author, _) = crate::css::cascade::parse_stylesheet(css, 1000);
        rules.extend(author);
        let options = crate::HtmlImportOptions::default();
        let mut context = crate::mapper::MapCtx {
            opts: &options,
            rules: &rules,
            warnings: Vec::new(),
            next_id: 0,
            node_count: 0,
        };
        let crate::dom::DomNode::Element(root) = &dom.body[0] else {
            panic!()
        };
        let Some(PenNode::Frame(frame)) = crate::mapper::map_element(&mut context, &[root], None)
        else {
            panic!()
        };
        frame.children.unwrap_or_default()
    }

    #[test]
    fn plain_paragraph_merges_to_single_plain_text() {
        let children = text_of("<div><p>hello   world</p></div>", "");
        let PenNode::Frame(paragraph) = &children[0] else {
            panic!("p should be a frame")
        };
        let PenNode::Text(text) = &paragraph.children.as_ref().unwrap()[0] else {
            panic!("expected text")
        };
        assert!(matches!(&text.content, TextContent::Plain(value) if value == "hello world"));
    }

    #[test]
    fn bold_link_run_becomes_styled_segments() {
        let children = text_of(
            "<div><p>see <b>bold</b> and <a href=\"https://x.dev\">link</a></p></div>",
            "",
        );
        let PenNode::Frame(paragraph) = &children[0] else {
            panic!()
        };
        let PenNode::Text(text) = &paragraph.children.as_ref().unwrap()[0] else {
            panic!()
        };
        let TextContent::Styled(segments) = &text.content else {
            panic!("expected styled")
        };
        assert_eq!(segments.len(), 4);
        assert_eq!(segments[1].font_weight, Some(700));
        assert_eq!(segments[3].href.as_deref(), Some("https://x.dev"));
        assert_eq!(segments[3].underline, Some(true));
        assert_eq!(segments[3].fill.as_deref(), Some("#0066cc"));
    }

    #[test]
    fn block_font_props_come_from_computed_style() {
        let children = text_of("<div><h1>Title</h1></div>", "");
        let PenNode::Frame(heading) = &children[0] else {
            panic!()
        };
        let PenNode::Text(text) = &heading.children.as_ref().unwrap()[0] else {
            panic!()
        };
        assert_eq!(text.font_size, Some(32.0));
        use jian_ops_schema::node::text::FontWeight;
        assert!(matches!(text.font_weight, Some(FontWeight::Number(700))));
    }

    #[test]
    fn mixed_inline_and_block_children_split_into_runs() {
        let children = text_of("<div>intro <b>x</b><section></section>tail</div>", "");
        assert_eq!(children.len(), 3);
        assert!(matches!(children[0], PenNode::Text(_)));
        assert!(matches!(children[1], PenNode::Frame(_)));
        assert!(matches!(children[2], PenNode::Text(_)));
    }
}
