use jian_ops_schema::node::base::PenNodeBase;
use jian_ops_schema::node::container::{AlignItems, ContainerProps, LayoutMode};
use jian_ops_schema::node::text::{TextContent, TextGrowth};
use jian_ops_schema::node::{FrameNode, PenNode};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};

use crate::css::cascade::{compute_style_for_viewport, ComputedStyle};
use crate::css::selectors::PseudoElement;
use crate::dom::{DomElement, DomNode};
use crate::mapper::pseudo::{InlinePseudo, MappedPseudo};
use crate::mapper::{map_element, MapCtx};

use super::build_text_node_in_path;

pub(super) enum InlineRunItem<'a> {
    Dom(&'a DomNode),
    Nested {
        node: &'a DomNode,
        path: Vec<&'a DomElement>,
        parent_style: ComputedStyle,
    },
    Pseudo {
        pseudo: InlinePseudo,
        href: Option<String>,
        context: super::InlineTextContext,
    },
}

struct InlineMapState<'a, 's> {
    root_path: &'s [&'a DomElement],
    root_style: &'s ComputedStyle,
    parent_is_flex_or_grid: bool,
    nodes: Vec<PenNode>,
    inline_nodes: Vec<PenNode>,
    run: Vec<InlineRunItem<'a>>,
    has_inline_box: bool,
}

impl InlineMapState<'_, '_> {
    fn flush(&mut self, context: &mut MapCtx<'_>) {
        flush_run(
            context,
            &mut self.inline_nodes,
            &mut self.run,
            self.root_style,
            self.root_path,
        );
    }

    fn finish(&mut self, context: &mut MapCtx<'_>) {
        finish_inline_context(
            context,
            &mut self.nodes,
            &mut self.inline_nodes,
            &mut self.run,
            &mut self.has_inline_box,
            self.root_style,
            self.root_path,
        );
    }
}

pub(crate) fn map_children<'a>(
    context: &mut MapCtx<'_>,
    path: &[&'a DomElement],
    block_style: &ComputedStyle,
    dom_children: &'a [DomNode],
) -> Vec<PenNode> {
    let mut state = InlineMapState {
        root_path: path,
        root_style: block_style,
        parent_is_flex_or_grid: establishes_flex_or_grid(block_style.get("display")),
        nodes: Vec::new(),
        inline_nodes: Vec::new(),
        run: Vec::new(),
        has_inline_box: false,
    };
    map_pseudo_into_context(
        context,
        &mut state,
        path,
        block_style,
        PseudoElement::Before,
    );
    map_dom_children(context, &mut state, path, block_style, dom_children, false);
    map_pseudo_into_context(context, &mut state, path, block_style, PseudoElement::After);
    state.finish(context);
    state.nodes
}

fn map_dom_children<'a>(
    context: &mut MapCtx<'_>,
    state: &mut InlineMapState<'a, '_>,
    parent_path: &[&'a DomElement],
    parent_style: &ComputedStyle,
    dom_children: &'a [DomNode],
    nested: bool,
) {
    for child in dom_children {
        let DomNode::Element(element) = child else {
            if nested {
                state.run.push(InlineRunItem::Nested {
                    node: child,
                    path: parent_path.to_vec(),
                    parent_style: parent_style.clone(),
                });
            } else {
                state.run.push(InlineRunItem::Dom(child));
            }
            continue;
        };
        let mut child_path = parent_path.to_vec();
        child_path.push(element);
        let child_style = compute_style_for_viewport(
            &child_path,
            context.rules,
            Some(parent_style),
            context.opts.base_font_size,
            context.opts.viewport_width,
            context.opts.viewport_width * 0.625,
        );
        if child_style.get("display") == Some("none") {
            continue;
        }
        if child_style.get("display") == Some("contents") {
            map_pseudo_into_context(
                context,
                state,
                &child_path,
                &child_style,
                PseudoElement::Before,
            );
            map_dom_children(
                context,
                state,
                &child_path,
                &child_style,
                &element.children,
                true,
            );
            map_pseudo_into_context(
                context,
                state,
                &child_path,
                &child_style,
                PseudoElement::After,
            );
            continue;
        }
        let boxed_inline = !state.parent_is_flex_or_grid
            && super::inline::is_boxed_inline(element, &child_style, &child_path, context);
        if boxed_inline {
            state.flush(context);
            if let Some(node) = map_element(context, &child_path, Some(parent_style)) {
                state.inline_nodes.push(node);
                state.has_inline_box = true;
            }
        } else if !state.parent_is_flex_or_grid
            && super::inline::participates_in_inline_run(
                element,
                &child_style,
                &child_path,
                context,
            )
        {
            if nested {
                state.run.push(InlineRunItem::Nested {
                    node: child,
                    path: parent_path.to_vec(),
                    parent_style: parent_style.clone(),
                });
            } else {
                state.run.push(InlineRunItem::Dom(child));
            }
        } else {
            state.finish(context);
            if let Some(node) = map_element(context, &child_path, Some(parent_style)) {
                state.nodes.push(node);
            }
        }
    }
}

fn map_pseudo_into_context(
    context: &mut MapCtx<'_>,
    state: &mut InlineMapState<'_, '_>,
    path: &[&DomElement],
    style: &ComputedStyle,
    pseudo: PseudoElement,
) {
    let Some(mapped) = crate::mapper::pseudo::map_pseudo(context, path, style, pseudo) else {
        return;
    };
    match mapped {
        MappedPseudo::Inline(pseudo) => state.run.push(InlineRunItem::Pseudo {
            pseudo,
            href: super::nearest_href(path),
            context: super::InlineTextContext::root(style),
        }),
        MappedPseudo::Frame {
            node,
            inline_level: true,
        } if !state.parent_is_flex_or_grid => {
            state.flush(context);
            state.inline_nodes.push(*node);
            state.has_inline_box = true;
        }
        MappedPseudo::Frame { node, .. } => {
            state.finish(context);
            state.nodes.push(*node);
        }
    }
}

pub(super) fn collect_run_segments(
    run: &[InlineRunItem<'_>],
    context: &MapCtx<'_>,
    path: &[&DomElement],
    block_style: &ComputedStyle,
    base_style: &super::SegStyle,
    base_context: super::InlineTextContext,
) -> Vec<super::RawSegment> {
    let mut segments = Vec::new();
    for item in run {
        match item {
            InlineRunItem::Dom(node) => super::collect_segments(
                node,
                context,
                path,
                block_style,
                base_style,
                base_context,
                &mut segments,
            ),
            InlineRunItem::Nested {
                node,
                path,
                parent_style,
            } => {
                let style = super::segment_style(parent_style, super::nearest_href(path));
                super::collect_segments(
                    node,
                    context,
                    path,
                    parent_style,
                    &style,
                    super::InlineTextContext::root(parent_style),
                    &mut segments,
                );
            }
            InlineRunItem::Pseudo {
                pseudo,
                href,
                context: inherited_context,
            } => super::inline::push_inline_pseudo(
                pseudo,
                href.clone(),
                *inherited_context,
                &mut segments,
            ),
        }
    }
    segments
}

fn finish_inline_context(
    context: &mut MapCtx<'_>,
    output: &mut Vec<PenNode>,
    inline_nodes: &mut Vec<PenNode>,
    run: &mut Vec<InlineRunItem<'_>>,
    has_inline_box: &mut bool,
    block_style: &ComputedStyle,
    path: &[&DomElement],
) {
    flush_run(context, inline_nodes, run, block_style, path);
    if inline_nodes.is_empty() {
        *has_inline_box = false;
        return;
    }
    if !*has_inline_box {
        if let Some(line_box) =
            wrap_smaller_inline_run_with_strut(context, inline_nodes, block_style)
        {
            output.push(line_box);
            return;
        }
        output.append(inline_nodes);
        return;
    }
    for node in inline_nodes.iter_mut() {
        if let PenNode::Text(text) = node {
            text.width = None;
            text.height = None;
            text.text_growth = Some(TextGrowth::Auto);
            text.limits = Default::default();
        }
    }
    if context.node_count >= crate::MAX_OUTPUT_NODES {
        context.warn_once("node limit reached while creating an inline formatting row");
        output.append(inline_nodes);
    } else {
        context.node_count += 1;
        let children = crate::mapper::layer_positioned_children(std::mem::take(inline_nodes));
        output.push(inline_row(context.generate_id(), children));
    }
    *has_inline_box = false;
}

fn flush_run(
    context: &mut MapCtx<'_>,
    output: &mut Vec<PenNode>,
    run: &mut Vec<InlineRunItem<'_>>,
    block_style: &ComputedStyle,
    path: &[&DomElement],
) {
    if let Some(node) = build_text_node_in_path(context, run, block_style, path) {
        output.push(node);
    }
    run.clear();
}

fn inline_row(id: String, children: Vec<PenNode>) -> PenNode {
    PenNode::Frame(FrameNode {
        base: PenNodeBase {
            id,
            name: Some("inline-row".to_string()),
            ..Default::default()
        },
        container: ContainerProps {
            width: Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)),
            height: Some(SizingBehavior::Keyword(SizingKeyword::FitContent)),
            layout: Some(LayoutMode::Horizontal),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
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
    })
}

/// A block container contributes an invisible font/line-height strut to each
/// inline line box. Jian measures a lone styled run from that run's promoted
/// font size, so a smaller child would otherwise collapse below the parent
/// strut (for example 12px text inside a 15px/1.5 block).
fn wrap_smaller_inline_run_with_strut(
    context: &mut MapCtx<'_>,
    inline_nodes: &mut Vec<PenNode>,
    block_style: &ComputedStyle,
) -> Option<PenNode> {
    let [PenNode::Text(text)] = inline_nodes.as_slice() else {
        return None;
    };
    let font_size = text.font_size?;
    let line_height = text.layout_line_height_multiplier()?;
    let content_has_break = match &text.content {
        TextContent::Plain(value) => value.contains('\n'),
        TextContent::Styled(segments) => segments.iter().any(|segment| segment.text.contains('\n')),
    };
    let strut_height = block_style.font_size * line_height;
    let text_height = font_size * line_height;
    if text.text_growth != Some(TextGrowth::Auto)
        || content_has_break
        || !strut_height.is_finite()
        || strut_height <= text_height + f64::EPSILON
        || context.node_count.saturating_add(2) > crate::MAX_OUTPUT_NODES
    {
        return None;
    }

    context.node_count += 2;
    let text = inline_nodes.pop()?;
    let strut = empty_strut(context.generate_id(), strut_height);
    Some(anonymous_line_box(
        context.generate_id(),
        context.containing_width_is_definite,
        vec![strut, text],
    ))
}

fn empty_strut(id: String, height: f64) -> PenNode {
    PenNode::Frame(FrameNode {
        base: PenNodeBase {
            id,
            name: Some("line-height strut".to_string()),
            ..Default::default()
        },
        container: ContainerProps {
            width: Some(SizingBehavior::Number(0.0)),
            height: Some(SizingBehavior::Number(height)),
            ..Default::default()
        },
        children: Some(Vec::new()),
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
    })
}

fn anonymous_line_box(id: String, definite_width: bool, children: Vec<PenNode>) -> PenNode {
    PenNode::Frame(FrameNode {
        base: PenNodeBase {
            id,
            name: Some("anonymous line box".to_string()),
            ..Default::default()
        },
        container: ContainerProps {
            width: Some(SizingBehavior::Keyword(if definite_width {
                SizingKeyword::FillContainer
            } else {
                SizingKeyword::FitContent
            })),
            height: Some(SizingBehavior::Keyword(SizingKeyword::FitContent)),
            layout: Some(LayoutMode::Horizontal),
            align_items: Some(AlignItems::Center),
            ..Default::default()
        },
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
    })
}

fn establishes_flex_or_grid(display: Option<&str>) -> bool {
    matches!(
        display.map(str::trim),
        Some(
            "flex"
                | "inline-flex"
                | "grid"
                | "inline-grid"
                | "block flex"
                | "inline flex"
                | "block grid"
                | "inline grid"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{import_html, HtmlImportOptions};

    fn named_frame<'a>(nodes: &'a [PenNode], name: &str) -> Option<&'a FrameNode> {
        for node in nodes {
            let PenNode::Frame(frame) = node else {
                continue;
            };
            if frame.base.name.as_deref() == Some(name) {
                return Some(frame);
            }
            if let Some(found) = named_frame(frame.children.as_deref().unwrap_or_default(), name) {
                return Some(found);
            }
        }
        None
    }

    #[test]
    fn smaller_inline_run_keeps_the_parent_line_box_strut() {
        let result = import_html(
            r#"<style>
                .category { display:flex }
                .copy { font:15px/1.5 sans-serif }
                .copy b { display:block }
                .copy span { font-size:12px }
            </style>
            <a class="category"><div class="copy"><b>Title</b><span>Detail</span></div></a>"#,
            &HtmlImportOptions::default(),
        );
        let line_box = named_frame(&result.nodes, "anonymous line box").expect("line box");
        assert_eq!(
            line_box.container.width,
            Some(SizingBehavior::Keyword(SizingKeyword::FitContent))
        );
        assert_eq!(line_box.container.align_items, Some(AlignItems::Center));
        let [PenNode::Frame(strut), PenNode::Text(text)] =
            line_box.children.as_deref().expect("line-box children")
        else {
            panic!("line box should contain a strut and its inline text")
        };
        assert_eq!(strut.container.width, Some(SizingBehavior::Number(0.0)));
        assert_eq!(strut.container.height, Some(SizingBehavior::Number(22.5)));
        assert_eq!(text.font_size, Some(12.0));
        assert_eq!(text.line_height, Some(1.5));
        assert_eq!(text.text_growth, Some(TextGrowth::Auto));
    }
}
