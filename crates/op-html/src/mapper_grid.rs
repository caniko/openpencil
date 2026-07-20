use jian_ops_schema::node::base::{NumberOrExpression, PenNodeBase};
use jian_ops_schema::node::container::{AlignItems, ContainerProps, LayoutMode};
use jian_ops_schema::node::{FrameNode, PenNode};
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};

use crate::css::cascade::ComputedStyle;
use crate::length::{parse_length, LengthCtx};

use super::MapCtx;

const DEFAULT_AUTO_TRACK_MIN: f64 = 240.0;
const MAX_GRID_COLUMNS: usize = 256;
const MAX_TRACK_NESTING: usize = 8;

pub(super) fn grid_column_count(style: &ComputedStyle) -> Option<usize> {
    is_grid(style).then_some(())?;
    fixed_track_count(style.get("grid-template-columns")?)
}

pub(super) fn grid_row_gap(style: &ComputedStyle, context: &MapCtx<'_>) -> Option<f64> {
    gap_value(style, context, "row-gap")
}

pub(super) fn wrap_grid_rows(
    context: &mut MapCtx<'_>,
    style: &ComputedStyle,
    children: Vec<PenNode>,
) -> Vec<PenNode> {
    if children.is_empty() || !is_grid(style) {
        return children;
    }
    let Some(first_flow) = children.iter().position(|node| !is_grid_overlay(node)) else {
        return children;
    };
    let mut front = Vec::new();
    let mut flow = Vec::new();
    let mut back = Vec::new();
    for (index, node) in children.into_iter().enumerate() {
        if is_grid_overlay(&node) {
            if index < first_flow {
                front.push(node);
            } else {
                back.push(node);
            }
        } else {
            flow.push(node);
        }
    }
    let Some(columns) = effective_column_count(style, context, flow.len()) else {
        front.extend(flow);
        front.extend(back);
        return front;
    };
    if columns <= 1 {
        front.extend(flow);
        front.extend(back);
        return front;
    }

    let row_count = flow.len().div_ceil(columns);
    // The current grid element reserves its slot before synthetic rows.
    if context.node_count.saturating_add(row_count) > crate::MAX_OUTPUT_NODES {
        context.warn_once("CSS grid row wrappers omitted because the node limit was reached");
        front.extend(flow);
        front.extend(back);
        return front;
    }

    let gap = gap_value(style, context, "column-gap").unwrap_or(0.0);
    let column_gap = (gap > 0.0).then_some(NumberOrExpression::Number(gap));
    let track_widths = resolved_track_widths(style, context, columns, gap);
    let align_items = style
        .get("align-items")
        .and_then(AlignItems::from_css)
        .or(Some(AlignItems::Stretch));
    let mut source = flow.into_iter();
    let mut rows = Vec::with_capacity(row_count);
    loop {
        let mut row_children: Vec<_> = source.by_ref().take(columns).collect();
        if row_children.is_empty() {
            break;
        }
        if let Some(widths) = &track_widths {
            for (child, width) in row_children.iter_mut().zip(widths) {
                set_node_width(child, *width);
            }
        }
        let row = PenNode::Frame(FrameNode {
            base: PenNodeBase {
                id: context.generate_id(),
                name: Some("Grid row".to_string()),
                ..Default::default()
            },
            container: ContainerProps {
                width: Some(SizingBehavior::Keyword(SizingKeyword::FillContainer)),
                height: Some(SizingBehavior::Keyword(SizingKeyword::FitContent)),
                layout: Some(LayoutMode::Horizontal),
                gap: column_gap.clone(),
                align_items: align_items.clone(),
                ..Default::default()
            },
            children: Some(row_children),
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
        context.node_count += 1;
        rows.push(row);
    }
    front.extend(rows);
    front.extend(back);
    front
}

fn is_grid_overlay(node: &PenNode) -> bool {
    let base = node_base(node);
    base.x.is_some() || base.y.is_some()
}

macro_rules! base_match {
    ($node:expr; $($variant:ident),+ $(,)?) => {
        match $node { $(PenNode::$variant(node) => &node.base,)+ }
    };
}

fn node_base(node: &PenNode) -> &PenNodeBase {
    base_match!(node; Frame, Group, Rectangle, Ellipse, Line, Polygon, Path, Text,
        TextInput, Image, IconFont, TextArea, Select, Switch, Checkbox, Slider,
        RadioGroup, NumberInput, Progress, Tabs, Ref)
}

macro_rules! direct_width_slot {
    ($node:expr; $($variant:ident),+ $(,)?) => {
        match $node { $(PenNode::$variant(node) => Some(&mut node.width),)+
            _ => None }
    };
}

fn set_node_width(node: &mut PenNode, width: f64) {
    let slot = match node {
        PenNode::Frame(node) => Some(&mut node.container.width),
        PenNode::Group(node) => Some(&mut node.container.width),
        PenNode::Rectangle(node) => Some(&mut node.container.width),
        node => direct_width_slot!(node; Ellipse, Polygon, Path, Text, TextInput,
            Image, IconFont, TextArea, Select, Switch, Checkbox, Slider,
            RadioGroup, NumberInput, Progress, Tabs),
    };
    if let Some(slot) = slot {
        *slot = Some(SizingBehavior::Number(width));
    }
}

fn is_grid(style: &ComputedStyle) -> bool {
    matches!(style.get("display"), Some("grid" | "inline-grid"))
}

fn effective_column_count(
    style: &ComputedStyle,
    context: &MapCtx<'_>,
    child_count: usize,
) -> Option<usize> {
    if let Some(count) = grid_column_count(style) {
        return Some(count);
    }
    let value = style.get("grid-template-columns")?;
    let plan = adaptive_track_plan(value, style.font_size, context)?;
    let gap = gap_value(style, context, "column-gap").unwrap_or(0.0);
    let available = context.containing_width.max(0.0);
    let denominator = plan.min_group_width + gap * plan.tracks_per_repeat as f64;
    if denominator <= 0.0 || !denominator.is_finite() {
        return None;
    }
    let groups = ((available + gap) / denominator).floor().max(1.0) as usize;
    Some(
        groups
            .saturating_mul(plan.tracks_per_repeat)
            .clamp(1, MAX_GRID_COLUMNS)
            .min(child_count.max(1)),
    )
}

fn gap_value(style: &ComputedStyle, context: &MapCtx<'_>, axis_name: &str) -> Option<f64> {
    let value = style.get(axis_name).or_else(|| style.get("gap"))?;
    if value.eq_ignore_ascii_case("normal") {
        return None;
    }
    resolved_length(value, style.font_size, context, context.containing_width)
}

#[derive(Clone, Copy)]
enum TrackWidth {
    Fixed(f64),
    Flex { minimum: f64, weight: f64 },
}

fn resolved_track_widths(
    style: &ComputedStyle,
    context: &MapCtx<'_>,
    columns: usize,
    gap: f64,
) -> Option<Vec<f64>> {
    let tracks = expand_fixed_tracks(style.get("grid-template-columns")?, 0)?;
    if tracks.len() != columns {
        return None;
    }
    let padding = ["padding-left", "padding-right"]
        .into_iter()
        .filter_map(|name| {
            resolved_length(
                style.get(name)?,
                style.font_size,
                context,
                context.containing_width,
            )
        })
        .sum::<f64>();
    let content_width = (context.containing_width - padding).max(0.0);
    let specs: Vec<_> = tracks
        .iter()
        .map(|track| track_width(track, style.font_size, context, content_width))
        .collect::<Option<_>>()?;
    let mut widths: Vec<Option<f64>> = specs
        .iter()
        .map(|track| match track {
            TrackWidth::Fixed(width) => Some(*width),
            TrackWidth::Flex { .. } => None,
        })
        .collect();
    let fixed = widths.iter().flatten().sum::<f64>();
    let mut free = (content_width - gap * columns.saturating_sub(1) as f64 - fixed).max(0.0);
    let mut weight = specs
        .iter()
        .filter_map(|track| match track {
            TrackWidth::Flex { weight, .. } => Some(*weight),
            TrackWidth::Fixed(_) => None,
        })
        .sum::<f64>();
    loop {
        let frozen: Vec<_> = specs
            .iter()
            .enumerate()
            .filter_map(|(index, track)| match track {
                TrackWidth::Flex {
                    minimum,
                    weight: item_weight,
                } if widths[index].is_none()
                    && (weight <= 0.0 || free * item_weight / weight < *minimum) =>
                {
                    Some((index, *minimum, *item_weight))
                }
                _ => None,
            })
            .collect();
        if frozen.is_empty() {
            break;
        }
        for (index, minimum, item_weight) in frozen {
            widths[index] = Some(minimum);
            free = (free - minimum).max(0.0);
            weight = (weight - item_weight).max(0.0);
        }
    }
    for (index, track) in specs.iter().enumerate() {
        if let TrackWidth::Flex {
            minimum,
            weight: item_weight,
        } = track
        {
            if widths[index].is_none() {
                widths[index] = Some(if weight > 0.0 {
                    (free * item_weight / weight).max(*minimum)
                } else {
                    *minimum
                });
            }
        }
    }
    widths.into_iter().collect()
}

fn track_width(
    track: &str,
    font_size: f64,
    context: &MapCtx<'_>,
    reference: f64,
) -> Option<TrackWidth> {
    if let Some(weight) = fr_weight(track) {
        return Some(TrackWidth::Flex {
            minimum: 0.0,
            weight,
        });
    }
    if let Some(inner) = function_body(track, "minmax") {
        let (minimum, maximum) = split_top_level_once(inner, ',')?;
        let minimum = resolved_length(minimum.trim(), font_size, context, reference).unwrap_or(0.0);
        return Some(if let Some(weight) = fr_weight(maximum) {
            TrackWidth::Flex { minimum, weight }
        } else if let Some(maximum) = resolved_length(maximum.trim(), font_size, context, reference)
        {
            TrackWidth::Fixed(maximum.max(minimum))
        } else {
            TrackWidth::Flex {
                minimum,
                weight: 1.0,
            }
        });
    }
    if let Some(inner) = function_body(track, "fit-content") {
        return resolved_length(inner, font_size, context, reference).map(TrackWidth::Fixed);
    }
    if matches!(
        track.trim().to_ascii_lowercase().as_str(),
        "auto" | "min-content" | "max-content"
    ) {
        return Some(TrackWidth::Flex {
            minimum: 0.0,
            weight: 1.0,
        });
    }
    resolved_length(track, font_size, context, reference).map(TrackWidth::Fixed)
}

fn fr_weight(value: &str) -> Option<f64> {
    let lower = value.trim().to_ascii_lowercase();
    let weight = lower.strip_suffix("fr")?.trim().parse::<f64>().ok()?;
    (weight.is_finite() && weight > 0.0).then_some(weight)
}

fn resolved_length(
    value: &str,
    font_size: f64,
    context: &MapCtx<'_>,
    reference: f64,
) -> Option<f64> {
    let length = parse_length(
        value,
        &LengthCtx {
            font_size,
            root_font_size: context.opts.base_font_size,
            viewport_w: context.opts.viewport_width,
            viewport_h: context.opts.viewport_width * 0.625,
        },
    )?;
    let value = length.resolve(reference);
    (value.is_finite() && value >= 0.0).then_some(value)
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct AdaptiveTrackPlan {
    tracks_per_repeat: usize,
    min_group_width: f64,
}

fn adaptive_track_plan(
    value: &str,
    font_size: f64,
    context: &MapCtx<'_>,
) -> Option<AdaptiveTrackPlan> {
    let tokens = top_level_tokens(value)?;
    let meaningful: Vec<_> = tokens
        .iter()
        .filter(|token| !is_line_names(token))
        .collect();
    let [repeat] = meaningful.as_slice() else {
        return None;
    };
    let inner = function_body(repeat, "repeat")?;
    let (count, tracks) = split_top_level_once(inner, ',')?;
    if !matches!(
        count.trim().to_ascii_lowercase().as_str(),
        "auto-fit" | "auto-fill"
    ) {
        return None;
    }
    let track_tokens = top_level_tokens(tracks)?;
    let track_tokens: Vec<_> = track_tokens
        .iter()
        .filter(|token| !is_line_names(token))
        .collect();
    if track_tokens.is_empty() || track_tokens.len() > MAX_GRID_COLUMNS {
        return None;
    }
    let min_group_width = track_tokens
        .iter()
        .map(|token| track_min_width(token, font_size, context))
        .sum::<f64>();
    Some(AdaptiveTrackPlan {
        tracks_per_repeat: track_tokens.len(),
        min_group_width,
    })
}

fn track_min_width(track: &str, font_size: f64, context: &MapCtx<'_>) -> f64 {
    let candidate = if let Some(inner) = function_body(track, "minmax") {
        split_top_level_once(inner, ',')
            .map(|(minimum, _)| minimum.trim())
            .unwrap_or(inner)
    } else if let Some(inner) = function_body(track, "fit-content") {
        inner.trim()
    } else {
        track.trim()
    };
    resolved_length(candidate, font_size, context, context.containing_width)
        .filter(|value| *value > 0.0)
        .unwrap_or(DEFAULT_AUTO_TRACK_MIN)
}

fn fixed_track_count(value: &str) -> Option<usize> {
    Some(expand_fixed_tracks(value, 0)?.len())
}

fn expand_fixed_tracks(value: &str, depth: usize) -> Option<Vec<String>> {
    let value = value.trim();
    if depth >= MAX_TRACK_NESTING
        || value.is_empty()
        || matches!(
            value.to_ascii_lowercase().as_str(),
            "none"
                | "subgrid"
                | "masonry"
                | "inherit"
                | "initial"
                | "revert"
                | "revert-layer"
                | "unset"
        )
    {
        return None;
    }
    let tokens = top_level_tokens(value)?;
    let mut output = Vec::new();
    for token in tokens {
        if is_line_names(&token) {
            continue;
        }
        if let Some(inner) = function_body(&token, "repeat") {
            let (count, tracks) = split_top_level_once(inner, ',')?;
            let count = count.trim().parse::<usize>().ok()?;
            if count == 0 || count > MAX_GRID_COLUMNS {
                return None;
            }
            let tracks = expand_fixed_tracks(tracks, depth + 1)?;
            if tracks.len().checked_mul(count)? > MAX_GRID_COLUMNS - output.len() {
                return None;
            }
            for _ in 0..count {
                output.extend(tracks.iter().cloned());
            }
        } else if is_track_size(&token) {
            output.push(token);
        } else {
            return None;
        }
        if output.len() > MAX_GRID_COLUMNS {
            return None;
        }
    }
    (!output.is_empty()).then_some(output)
}

fn is_line_names(token: &str) -> bool {
    let token = token.trim();
    token.starts_with('[') && token.ends_with(']')
}

fn is_track_size(token: &str) -> bool {
    let lower = token.trim().to_ascii_lowercase();
    if matches!(lower.as_str(), "auto" | "min-content" | "max-content") {
        return true;
    }
    for function in ["minmax", "fit-content", "calc", "min", "max", "clamp"] {
        if function_body(&lower, function).is_some() {
            return true;
        }
    }
    is_dimension(&lower)
}

fn is_dimension(value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    let Some((at, number)) = (1..=value.len()).rev().find_map(|at| {
        value
            .is_char_boundary(at)
            .then(|| value[..at].parse::<f64>().ok().map(|number| (at, number)))?
    }) else {
        return false;
    };
    let unit = &value[at..];
    const UNITS: &[&str] = &[
        "%", "fr", "px", "em", "rem", "vw", "vh", "vmin", "vmax", "svw", "svh", "lvw", "lvh",
        "dvw", "dvh", "vi", "vb", "svi", "svb", "lvi", "lvb", "dvi", "dvb", "cm", "mm", "q", "in",
        "pt", "pc", "ch", "ex", "cap", "ic", "rch", "rex", "rcap", "ric", "lh", "rlh",
    ];
    number.is_finite() && ((unit.is_empty() && number == 0.0) || UNITS.contains(&unit))
}

fn function_body<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let value = value.trim();
    let prefix = value.get(..name.len())?;
    if value.len() <= name.len() + 2
        || !prefix.eq_ignore_ascii_case(name)
        || value.as_bytes().get(name.len()) != Some(&b'(')
        || !value.ends_with(')')
    {
        return None;
    }
    let inner = &value[name.len() + 1..value.len() - 1];
    scan_top_level(inner, None).is_some().then_some(inner)
}

fn split_top_level_once(value: &str, delimiter: char) -> Option<(&str, &str)> {
    let index = scan_top_level(value, Some(delimiter))??;
    Some((&value[..index], &value[index + delimiter.len_utf8()..]))
}

fn top_level_tokens(value: &str) -> Option<Vec<String>> {
    scan_top_level(value, None)?;
    let mut tokens = Vec::new();
    let mut start = None;
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            start.get_or_insert(index);
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            start.get_or_insert(index);
            continue;
        }
        match ch {
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.saturating_sub(1),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            ',' if parentheses == 0 && brackets == 0 => return None,
            ch if ch.is_whitespace() && parentheses == 0 && brackets == 0 => {
                if let Some(start_index) = start.take() {
                    tokens.push(value[start_index..index].trim().to_string());
                }
                continue;
            }
            _ => {}
        }
        start.get_or_insert(index);
    }
    if let Some(start) = start {
        tokens.push(value[start..].trim().to_string());
    }
    Some(tokens)
}

fn scan_top_level(value: &str, delimiter: Option<char>) -> Option<Option<usize>> {
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices() {
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
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        match ch {
            '(' => parentheses += 1,
            ')' => parentheses = parentheses.checked_sub(1)?,
            '[' => brackets += 1,
            ']' => brackets = brackets.checked_sub(1)?,
            _ if delimiter == Some(ch) && parentheses == 0 && brackets == 0 => {
                return Some(Some(index));
            }
            _ => {}
        }
    }
    (parentheses == 0 && brackets == 0 && quote.is_none() && !escaped).then_some(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn style(columns: &str, gap: Option<&str>) -> ComputedStyle {
        let mut props = BTreeMap::from([
            ("display".to_string(), "grid".to_string()),
            ("grid-template-columns".to_string(), columns.to_string()),
        ]);
        if let Some(gap) = gap {
            props.insert("gap".to_string(), gap.to_string());
            props.insert("row-gap".to_string(), gap.to_string());
            props.insert("column-gap".to_string(), gap.to_string());
        }
        ComputedStyle {
            props,
            font_size: 16.0,
        }
    }

    fn context<'a>(options: &'a crate::HtmlImportOptions) -> MapCtx<'a> {
        MapCtx {
            opts: options,
            rules: &[],
            warnings: Vec::new(),
            next_id: 100,
            node_count: 0,
            containing_width: options.viewport_width,
            containing_height: options.viewport_width * 0.625,
            containing_width_is_definite: true,
            positioned_width: options.viewport_width,
            positioned_height: options.viewport_width * 0.625,
        }
    }

    fn child(id: usize) -> PenNode {
        serde_json::from_str(&format!(
            r#"{{"type":"frame","id":"child-{id}","children":[]}}"#
        ))
        .unwrap()
    }

    fn row(node: &PenNode) -> &FrameNode {
        let PenNode::Frame(row) = node else {
            panic!("expected grid row")
        };
        row
    }

    fn width(node: &PenNode) -> f64 {
        let PenNode::Frame(node) = node else {
            panic!("expected frame")
        };
        let Some(SizingBehavior::Number(width)) = node.container.width else {
            panic!("expected numeric width")
        };
        width
    }

    #[test]
    fn counts_safe_explicit_tracks_and_rejects_dynamic_lists() {
        assert_eq!(
            grid_column_count(&style("repeat(4, minmax(0, 1fr))", None)),
            Some(4)
        );
        assert_eq!(
            grid_column_count(&style(
                "[start] 120px [middle] minmax(0, 1fr) fit-content(20rem) [end]",
                None,
            )),
            Some(3)
        );
        for value in [
            "none",
            "subgrid",
            "repeat(auto-fit, minmax(200px, 1fr))",
            "repeat(0, 1fr)",
            "repeat(257, 1fr)",
            "1fr, 2fr",
            "var(--tracks)",
        ] {
            assert_eq!(grid_column_count(&style(value, None)), None, "{value}");
        }
    }

    #[test]
    fn wraps_rows_without_stretching_the_incomplete_tail() {
        let options = crate::HtmlImportOptions {
            viewport_width: 1000.0,
            ..Default::default()
        };
        let mut context = context(&options);
        let rows = wrap_grid_rows(
            &mut context,
            &style("repeat(2, 1fr)", Some("12px")),
            (0..5).map(child).collect(),
        );
        assert_eq!(rows.len(), 3);
        assert_eq!(context.node_count, 3);
        let lengths: Vec<_> = rows
            .iter()
            .map(|node| row(node).children.as_ref().unwrap().len())
            .collect();
        assert_eq!(lengths, [2, 2, 1]);
        assert_eq!(row(&rows[0]).container.layout, Some(LayoutMode::Horizontal));
        assert_eq!(
            row(&rows[0]).container.gap,
            Some(NumberOrExpression::Number(12.0))
        );
        let tail = &row(&rows[2]).children.as_ref().unwrap()[0];
        assert!((width(tail) - 494.0).abs() < 0.001);
    }

    #[test]
    fn assigns_non_uniform_pixel_percent_fr_and_minmax_tracks() {
        let options = crate::HtmlImportOptions {
            viewport_width: 1000.0,
            ..Default::default()
        };
        let mut context = context(&options);
        let rows = wrap_grid_rows(
            &mut context,
            &style("120px 25% minmax(100px, 1fr) 2fr", Some("10px")),
            (0..4).map(child).collect(),
        );
        let children = row(&rows[0]).children.as_ref().unwrap();
        let widths: Vec<_> = children.iter().map(width).collect();
        assert_eq!(widths, [120.0, 250.0, 200.0, 400.0]);
    }

    #[test]
    fn flows_static_generated_items_but_keeps_positioned_pseudos_outside_rows() {
        let options = crate::HtmlImportOptions::default();
        let mut context = context(&options);
        let mut front = child(90);
        let PenNode::Frame(node) = &mut front else {
            unreachable!()
        };
        node.base.name = Some("::before".into());
        node.base.x = Some(4.0);
        let mut after = child(91);
        let PenNode::Frame(node) = &mut after else {
            unreachable!()
        };
        node.base.name = Some("::after".into());
        let output = wrap_grid_rows(
            &mut context,
            &style("repeat(2, 1fr)", None),
            vec![front, child(0), child(1), after],
        );
        assert_eq!(node_base(&output[0]).id, "child-90");
        assert_eq!(row(&output[1]).children.as_ref().unwrap().len(), 2);
        let tail = row(&output[2]).children.as_ref().unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(node_base(&tail[0]).id, "child-91");
    }

    #[test]
    fn estimates_auto_fit_gaps_and_preserves_the_node_budget() {
        let options = crate::HtmlImportOptions::default();
        let mut context = context(&options);
        let mut grid_style = style("repeat(2, 1fr)", None);
        grid_style.props.insert("row-gap".into(), "8px".into());
        grid_style.props.insert("column-gap".into(), "16px".into());
        assert_eq!(grid_row_gap(&grid_style, &context), Some(8.0));
        assert_eq!(gap_value(&grid_style, &context, "column-gap"), Some(16.0));
        assert_eq!(
            effective_column_count(
                &style("repeat(auto-fit, minmax(220px, 1fr))", Some("20px")),
                &context,
                7,
            ),
            Some(6)
        );
        context.node_count = crate::MAX_OUTPUT_NODES;
        let children = vec![child(0), child(1)];
        assert_eq!(
            wrap_grid_rows(&mut context, &grid_style, children.clone()),
            children
        );
        assert_eq!(context.warnings.len(), 1);
    }
}
