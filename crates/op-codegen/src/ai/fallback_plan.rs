//! Deterministic planning fallback for AI codegen. Used only when the
//! planner responds with text that cannot be parsed as a `CodePlan`.

use serde_json::{Map, Value};

use crate::ai::parse::sanitize_name;
use crate::ai::types::{CodePlan, PlannedChunk, RootLayout};

/// Keep model requests small enough that a deterministic fallback is useful
/// even for a full imported web page / Figma frame. This is a soft target:
/// documents larger than the aggregate node/byte budget necessarily exceed it,
/// in which case the final pass balances the overflow across 15 chunks.
const TARGET_CHUNK_NODES: usize = 100;
/// Leave ample headroom for prompt instructions, dependency contracts, and
/// asset hints below the pipeline's hard per-request prompt limit.
const TARGET_CHUNK_JSON_BYTES: usize = 20 * 1024;
const MAX_CHUNKS: usize = 15;

struct NodeInfo<'a> {
    node: &'a Value,
    node_count: usize,
    json_bytes: usize,
    children: Vec<NodeInfo<'a>>,
}

#[derive(Clone, Copy)]
struct PartitionRoot<'a> {
    node: &'a Value,
    node_count: usize,
    json_bytes: usize,
    semantic: bool,
    order: usize,
}

pub(crate) fn fallback_plan_from_nodes_json(nodes_json: &str) -> Option<CodePlan> {
    let value: Value = serde_json::from_str(nodes_json).ok()?;
    let roots: Vec<&Value> = match &value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![&value],
        _ => Vec::new(),
    };

    let root_infos = roots
        .iter()
        .filter_map(|node| node_info(node))
        .collect::<Vec<_>>();
    let valid_root_count = root_infos
        .iter()
        .filter(|info| node_id(info.node).is_some())
        .count();
    let needs_partition =
        valid_root_count > MAX_CHUNKS || root_infos.iter().any(|info| !info_fits_target(info));

    // Preserve the old fallback byte-for-byte for ordinary selections: one
    // chunk per top-level selected node, with the same labels and roles.
    let chunks = if needs_partition {
        partitioned_chunks(&root_infos)
    } else {
        top_level_chunks(&roots)
    };
    if chunks.is_empty() {
        return None;
    }

    Some(CodePlan {
        chunks,
        shared_styles: Vec::new(),
        root_layout: root_layout_from_node(roots.first().copied()),
    })
}

fn top_level_chunks(roots: &[&Value]) -> Vec<PlannedChunk> {
    let mut chunks = Vec::new();
    for node in roots {
        if let Some(chunk) = chunk_from_node(node, chunks.len() + 1) {
            chunks.push(chunk);
        }
    }
    chunks
}

fn node_info(node: &Value) -> Option<NodeInfo<'_>> {
    let map = node.as_object()?;
    let mut children = Vec::new();
    let children_json_bytes = map.get("children").and_then(Value::as_array).map(|items| {
        items
            .iter()
            .enumerate()
            .fold(2usize, |bytes, (index, child)| {
                let (child_bytes, child_info) = match node_info(child) {
                    Some(info) => (info.json_bytes, Some(info)),
                    None => (serialized_json_len(child), None),
                };
                if let Some(info) = child_info {
                    children.push(info);
                }
                bytes
                    .saturating_add(usize::from(index > 0))
                    .saturating_add(child_bytes)
            })
    });
    let node_count = children.iter().fold(1usize, |count, child| {
        count.saturating_add(child.node_count)
    });
    let json_bytes = map
        .iter()
        .enumerate()
        .fold(2usize, |bytes, (index, (key, value))| {
            let value_bytes = if key == "children" {
                children_json_bytes.unwrap_or_else(|| serialized_json_len(value))
            } else {
                serialized_json_len(value)
            };
            bytes
                .saturating_add(usize::from(index > 0))
                .saturating_add(serialized_json_len(key.as_str()))
                .saturating_add(1) // `:`
                .saturating_add(value_bytes)
        });
    Some(NodeInfo {
        node,
        node_count,
        json_bytes,
        children,
    })
}

fn serialized_json_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |json| json.len())
}

fn info_fits_target(info: &NodeInfo<'_>) -> bool {
    info.node_count <= TARGET_CHUNK_NODES && info.json_bytes <= TARGET_CHUNK_JSON_BYTES
}

fn node_id(node: &Value) -> Option<&str> {
    node.as_object()?
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn partitioned_chunks(root_infos: &[NodeInfo<'_>]) -> Vec<PlannedChunk> {
    let mut selected = Vec::new();
    let mut next_order = 0usize;
    for root in root_infos {
        collect_partition_roots(root, false, &mut selected, &mut next_order);
    }
    group_partition_roots(selected)
        .iter()
        .enumerate()
        .filter_map(|(index, group)| chunk_from_group(group, index + 1))
        .collect()
}

/// Select an antichain of subtree roots. Once a node is selected we never
/// descend into it; once it is too large we select only descendants. This
/// makes every generated node appear in at most one chunk payload.
fn collect_partition_roots<'a>(
    info: &NodeInfo<'a>,
    allow_semantic_refinement: bool,
    out: &mut Vec<PartitionRoot<'a>>,
    next_order: &mut usize,
) {
    let has_id = node_id(info.node).is_some();
    let refine_semantics =
        allow_semantic_refinement && info_fits_target(info) && should_refine_for_semantics(info);
    if has_id && info_fits_target(info) && !refine_semantics {
        push_partition_root(info, out, next_order);
        return;
    }

    let before = out.len();
    for child in &info.children {
        collect_partition_roots(child, true, out, next_order);
    }

    // Malformed imported trees can contain children without ids. If none of
    // them yielded an addressable subtree, retaining the addressable parent is
    // more useful than dropping that whole branch, even if it exceeds target.
    if before == out.len() && has_id {
        push_partition_root(info, out, next_order);
    }
}

fn push_partition_root<'a>(
    info: &NodeInfo<'a>,
    out: &mut Vec<PartitionRoot<'a>>,
    next_order: &mut usize,
) {
    out.push(PartitionRoot {
        node: info.node,
        node_count: info.node_count,
        json_bytes: info.json_bytes,
        semantic: is_semantic_node(info.node),
        order: *next_order,
    });
    *next_order = next_order.saturating_add(1);
}

/// A transparent `div`/Frame wrapper under the size limit should not hide
/// meaningful Header/Main/Footer-style children. Keep a semantic node intact,
/// but unwrap a generic node when it directly contains two or more semantic
/// regions. The top-level small-document compatibility path bypasses this.
fn should_refine_for_semantics(info: &NodeInfo<'_>) -> bool {
    !is_semantic_node(info.node)
        && info
            .children
            .iter()
            .filter(|child| is_semantic_node(child.node))
            .take(2)
            .count()
            >= 2
}

fn is_semantic_node(node: &Value) -> bool {
    let Some(map) = node.as_object() else {
        return false;
    };
    [
        "role",
        "name",
        "tag",
        "tagName",
        "htmlTag",
        "semanticRole",
        "type",
    ]
    .iter()
    .filter_map(|key| map.get(*key).and_then(Value::as_str))
    .any(contains_semantic_token)
}

fn contains_semantic_token(value: &str) -> bool {
    const TOKENS: &[&str] = &[
        "article",
        "aside",
        "banner",
        "card",
        "carousel",
        "categories",
        "checkout",
        "content",
        "contentinfo",
        "dialog",
        "faq",
        "features",
        "footer",
        "form",
        "gallery",
        "grid",
        "header",
        "hero",
        "list",
        "main",
        "menu",
        "modal",
        "nav",
        "navigation",
        "pricing",
        "products",
        "region",
        "section",
        "sidebar",
        "table",
        "tabs",
        "testimonials",
        "toolbar",
    ];

    let mut normalized = String::with_capacity(value.len());
    let mut previous_was_lower_or_digit = false;
    for character in value.chars() {
        if character.is_ascii_uppercase() && previous_was_lower_or_digit {
            normalized.push(' ');
        }
        if character.is_ascii_alphanumeric() {
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
        previous_was_lower_or_digit = character.is_ascii_lowercase() || character.is_ascii_digit();
    }
    normalized
        .split_whitespace()
        .any(|token| TOKENS.contains(&token))
}

fn group_partition_roots<'a>(roots: Vec<PartitionRoot<'a>>) -> Vec<Vec<PartitionRoot<'a>>> {
    if roots.is_empty() {
        return Vec::new();
    }

    let mut groups = Vec::new();
    let mut ordinary_run = Vec::new();
    for root in roots {
        if root.semantic {
            flush_ordinary_run(&mut ordinary_run, &mut groups);
            groups.push(vec![root]);
        } else {
            ordinary_run.push(root);
        }
    }
    flush_ordinary_run(&mut ordinary_run, &mut groups);

    if groups.len() <= MAX_CHUNKS {
        groups
    } else {
        // More semantic/size groups than the public plan can carry. LPT
        // (largest-processing-time first) gives deterministic, well-balanced
        // bins; ties use document order. Within/between bins we restore page
        // order so prompt and assembly output remain stable.
        rebalance_partition_roots(groups.into_iter().flatten().collect())
    }
}

fn flush_ordinary_run<'a>(
    run: &mut Vec<PartitionRoot<'a>>,
    groups: &mut Vec<Vec<PartitionRoot<'a>>>,
) {
    let mut group = Vec::new();
    let mut group_nodes = 0usize;
    // `chunk_nodes_json` serializes a JSON array. Account for `[]` plus one
    // comma per additional root; the 4 KiB safety margin absorbs minor prompt
    // framing beyond this exact node-array estimate.
    let mut group_bytes = 2usize;
    for root in run.drain(..) {
        let separator = usize::from(!group.is_empty());
        let next_nodes = group_nodes.saturating_add(root.node_count);
        let next_bytes = group_bytes
            .saturating_add(separator)
            .saturating_add(root.json_bytes);
        if !group.is_empty()
            && (next_nodes > TARGET_CHUNK_NODES || next_bytes > TARGET_CHUNK_JSON_BYTES)
        {
            groups.push(std::mem::take(&mut group));
            group_nodes = 0;
            group_bytes = 2;
        }
        group_nodes = group_nodes.saturating_add(root.node_count);
        group_bytes = group_bytes
            .saturating_add(usize::from(!group.is_empty()))
            .saturating_add(root.json_bytes);
        group.push(root);
    }
    if !group.is_empty() {
        groups.push(group);
    }
}

fn rebalance_partition_roots<'a>(mut roots: Vec<PartitionRoot<'a>>) -> Vec<Vec<PartitionRoot<'a>>> {
    let bin_count = MAX_CHUNKS.min(roots.len());
    roots.sort_by(|a, b| {
        partition_pressure(b)
            .cmp(&partition_pressure(a))
            .then_with(|| a.order.cmp(&b.order))
    });

    let mut bins = vec![Vec::new(); bin_count];
    let mut loads = vec![(0usize, 2usize); bin_count];
    for root in roots {
        let lightest = loads
            .iter()
            .enumerate()
            .min_by_key(|(index, (nodes, bytes))| {
                let next_nodes = nodes.saturating_add(root.node_count);
                let next_bytes = bytes
                    .saturating_add(usize::from(!bins[*index].is_empty()))
                    .saturating_add(root.json_bytes);
                (load_pressure(next_nodes, next_bytes), *index)
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        loads[lightest].0 = loads[lightest].0.saturating_add(root.node_count);
        loads[lightest].1 = loads[lightest]
            .1
            .saturating_add(usize::from(!bins[lightest].is_empty()))
            .saturating_add(root.json_bytes);
        bins[lightest].push(root);
    }

    for bin in &mut bins {
        bin.sort_by_key(|root| root.order);
    }
    bins.retain(|bin| !bin.is_empty());
    bins.sort_by_key(|bin| bin.first().map(|root| root.order).unwrap_or(usize::MAX));
    bins
}

fn partition_pressure(root: &PartitionRoot<'_>) -> usize {
    load_pressure(root.node_count, root.json_bytes)
}

/// Compare the two independent limits without floats. A value of
/// `TARGET_CHUNK_NODES * TARGET_CHUNK_JSON_BYTES` means one full target load.
fn load_pressure(nodes: usize, bytes: usize) -> usize {
    nodes
        .saturating_mul(TARGET_CHUNK_JSON_BYTES)
        .max(bytes.saturating_mul(TARGET_CHUNK_NODES))
}

fn chunk_from_group(group: &[PartitionRoot<'_>], index: usize) -> Option<PlannedChunk> {
    if group.len() == 1 {
        return chunk_from_node(group[0].node, index);
    }

    let anchor = group.iter().find(|root| root.semantic).unwrap_or(&group[0]);
    let anchor_map = anchor.node.as_object()?;
    let label = format!("{} Group {index}", node_label(anchor_map, index));
    let suggested = {
        let sanitized = sanitize_name(&label);
        if sanitized.is_empty() {
            format!("Chunk{index}")
        } else {
            sanitized
        }
    };
    let node_ids = group
        .iter()
        .filter_map(|root| node_id(root.node).map(str::to_string))
        .collect::<Vec<_>>();
    if node_ids.is_empty() {
        return None;
    }

    let roles = group
        .iter()
        .filter_map(|root| {
            let map = root.node.as_object()?;
            map.get("role")
                .and_then(Value::as_str)
                .or_else(|| map.get("type").and_then(Value::as_str))
        })
        .collect::<Vec<_>>();
    let role = roles
        .first()
        .filter(|first| roles.iter().all(|role| role == *first))
        .copied()
        .unwrap_or("section")
        .to_string();

    Some(PlannedChunk {
        id: format!("chunk-{index}"),
        name: label_to_kebab(&label, index),
        node_ids,
        role,
        suggested_component_name: suggested,
        dependencies: Vec::new(),
    })
}

fn chunk_from_node(node: &Value, index: usize) -> Option<PlannedChunk> {
    let map = node.as_object()?;
    let id = map
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let label = node_label(map, index);
    let suggested = {
        let sanitized = sanitize_name(&label);
        if sanitized.is_empty() {
            format!("Chunk{index}")
        } else {
            sanitized
        }
    };

    Some(PlannedChunk {
        id: format!("chunk-{index}"),
        name: label_to_kebab(&label, index),
        node_ids: vec![id.to_string()],
        role: map
            .get("role")
            .and_then(Value::as_str)
            .or_else(|| map.get("type").and_then(Value::as_str))
            .unwrap_or("component")
            .to_string(),
        suggested_component_name: suggested,
        dependencies: Vec::new(),
    })
}

fn node_label(map: &Map<String, Value>, index: usize) -> String {
    let has_name = map
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .is_some();
    let mut label = map
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| map.get("role").and_then(Value::as_str))
        .or_else(|| map.get("type").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::trim)
        .unwrap_or("chunk")
        .to_string();
    if !has_name {
        label.push_str(&format!(" {index}"));
    }
    label
}

fn label_to_kebab(label: &str, index: usize) -> String {
    let mut out = String::new();
    let mut last_was_sep = true;
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('-');
            last_was_sep = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        format!("chunk-{index}")
    } else {
        out
    }
}

fn root_layout_from_node(node: Option<&Value>) -> RootLayout {
    let Some(map) = node.and_then(Value::as_object) else {
        return default_root_layout();
    };
    RootLayout {
        direction: root_direction(map),
        gap: first_number(
            map,
            &["gap", "itemSpacing", "spacing", "rowGap", "columnGap"],
        )
        .unwrap_or(0.0),
        responsive: true,
    }
}

fn default_root_layout() -> RootLayout {
    RootLayout {
        direction: "vertical".to_string(),
        gap: 0.0,
        responsive: true,
    }
}

fn root_direction(map: &Map<String, Value>) -> String {
    let raw = map
        .get("layoutMode")
        .or_else(|| map.get("layoutDirection"))
        .or_else(|| map.get("flexDirection"))
        .or_else(|| map.get("direction"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match raw.to_ascii_lowercase().as_str() {
        "horizontal" | "row" => "horizontal".to_string(),
        _ => "vertical".to_string(),
    }
}

fn first_number(map: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| match map.get(*key)? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    })
}

#[cfg(test)]
#[path = "fallback_plan_tests.rs"]
mod tests;
