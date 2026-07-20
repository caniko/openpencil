use std::collections::HashSet;

use super::*;
use serde_json::json;

#[test]
fn postorder_byte_count_matches_serde_json_exactly() {
    let value = json!({
        "id": "root",
        "quoted\"key": "line one\n第二行",
        "metadata": {"nested": [true, null, 42]},
        "children": [
            {"id": "child", "children": null, "characters": "文".repeat(32)},
            null,
            "non-object child"
        ]
    });
    let info = node_info(&value).expect("node info");

    assert_eq!(info.node_count, 2);
    assert_eq!(
        info.json_bytes,
        serde_json::to_string(&value).unwrap().len()
    );
    assert_eq!(
        info.children[0].json_bytes,
        serde_json::to_string(info.children[0].node).unwrap().len()
    );
}

#[test]
fn fallback_plan_uses_selected_node_names_and_ids() {
    let plan = fallback_plan_from_nodes_json(
        r#"[{"type":"frame","id":"hero","name":"Hero","layoutMode":"horizontal","gap":12}]"#,
    )
    .expect("fallback plan");

    assert_eq!(plan.chunks.len(), 1);
    assert_eq!(plan.chunks[0].id, "chunk-1");
    assert_eq!(plan.chunks[0].node_ids, vec!["hero"]);
    assert_eq!(plan.chunks[0].suggested_component_name, "Hero");
    assert_eq!(plan.root_layout.direction, "horizontal");
    assert_eq!(plan.root_layout.gap, 12.0);
}

#[test]
fn small_plan_numbers_chunks_after_skipping_a_malformed_root() {
    let plan = fallback_plan_from_nodes_json(
        r#"[{"type":"frame","name":"Missing id"},{"type":"frame","id":"hero","name":"Hero"}]"#,
    )
    .expect("fallback plan");

    assert_eq!(plan.chunks.len(), 1);
    assert_eq!(plan.chunks[0].id, "chunk-1");
    assert_eq!(plan.chunks[0].name, "hero");
    assert_eq!(plan.chunks[0].suggested_component_name, "Hero");
}

#[test]
fn large_single_root_splits_at_semantic_subtrees_without_overlap() {
    let value = json!({
        "type": "frame",
        "id": "page",
        "name": "Page",
        "children": [
            subtree("header", "Header", 30),
            subtree("main", "Main", 80),
            subtree("footer", "Footer", 20),
        ]
    });
    let plan = plan_for(&value);

    assert_eq!(
        selected_ids(&plan),
        HashSet::from(["header", "main", "footer"])
    );
    assert_antichain(&value, &plan);
}

#[test]
fn oversized_wrappers_keep_splitting_until_descendants_fit() {
    let value = json!({
        "type": "frame",
        "id": "page",
        "children": [{
            "type": "frame",
            "id": "wrapper",
            "children": [
                subtree("features", "Features", 80),
                subtree("pricing", "Pricing", 80),
            ]
        }, subtree("footer", "Footer", 10)]
    });
    let plan = plan_for(&value);

    assert_eq!(
        selected_ids(&plan),
        HashSet::from(["features", "pricing", "footer"])
    );
    assert_antichain(&value, &plan);
}

#[test]
fn ordinary_nodes_pack_to_the_soft_node_limit() {
    let value = flat_document(1_200);
    let plan = plan_for(&value);

    assert_eq!(plan.chunks.len(), 12);
    assert!(plan
        .chunks
        .iter()
        .all(|chunk| chunk.node_ids.len() <= TARGET_CHUNK_NODES));
    assert_eq!(selected_ids(&plan).len(), 1_200);
    assert_antichain(&value, &plan);
}

#[test]
fn more_than_fifteen_required_groups_are_capped_and_balanced() {
    let value = flat_document(2_000);
    let first = plan_for(&value);
    let second = plan_for(&value);
    let loads = first
        .chunks
        .iter()
        .map(|chunk| chunk.node_ids.len())
        .collect::<Vec<_>>();

    assert_eq!(first, second, "partitioning must be deterministic");
    assert_eq!(first.chunks.len(), MAX_CHUNKS);
    assert_eq!(loads.iter().sum::<usize>(), 2_000);
    assert!(loads.iter().max().unwrap() - loads.iter().min().unwrap() <= 1);
    assert_antichain(&value, &first);
}

#[test]
fn rich_text_is_partitioned_by_serialized_byte_size() {
    let rich_text = "文".repeat(5_500);
    let value = json!({
        "type": "frame",
        "id": "page",
        "children": (0..3).map(|index| json!({
            "type": "text",
            "id": format!("text-{index}"),
            "characters": rich_text,
            "style": {"fontFamily": "Inter", "fontSize": 16}
        })).collect::<Vec<_>>()
    });
    let plan = plan_for(&value);

    assert_eq!(plan.chunks.len(), 3);
    assert!(plan.chunks.iter().all(|chunk| {
        serialized_chunk_bytes(&value, &chunk.node_ids) <= TARGET_CHUNK_JSON_BYTES
    }));
    assert_antichain(&value, &plan);
}

#[test]
fn an_oversized_leaf_is_retained_for_the_final_prompt_size_guard() {
    let value = json!({
        "type": "text",
        "id": "giant-text",
        "characters": "x".repeat(TARGET_CHUNK_JSON_BYTES + 1_024)
    });
    let plan = plan_for(&value);

    assert_eq!(plan.chunks.len(), 1);
    assert_eq!(plan.chunks[0].node_ids, ["giant-text"]);
    assert!(serialized_chunk_bytes(&value, &plan.chunks[0].node_ids) > TARGET_CHUNK_JSON_BYTES);
}

fn plan_for(value: &Value) -> CodePlan {
    fallback_plan_from_nodes_json(&serde_json::to_string(value).unwrap()).expect("fallback plan")
}

fn subtree(id: &str, name: &str, node_count: usize) -> Value {
    assert!(node_count > 0);
    json!({
        "type": "frame",
        "id": id,
        "name": name,
        "children": (1..node_count).map(|index| json!({
            "type": "rectangle",
            "id": format!("{id}-child-{index}")
        })).collect::<Vec<_>>()
    })
}

fn flat_document(child_count: usize) -> Value {
    json!({
        "type": "frame",
        "id": "root",
        "children": (0..child_count).map(|index| json!({
            "type": "rectangle",
            "id": format!("leaf-{index}")
        })).collect::<Vec<_>>()
    })
}

fn selected_ids(plan: &CodePlan) -> HashSet<&str> {
    plan.chunks
        .iter()
        .flat_map(|chunk| chunk.node_ids.iter().map(String::as_str))
        .collect()
}

fn assert_antichain(value: &Value, plan: &CodePlan) {
    let selected = selected_ids(plan);
    let total = plan
        .chunks
        .iter()
        .map(|chunk| chunk.node_ids.len())
        .sum::<usize>();
    assert_eq!(selected.len(), total, "a root was selected more than once");

    fn visit(node: &Value, selected: &HashSet<&str>, ancestor_selected: bool) {
        match node {
            Value::Array(items) => {
                for item in items {
                    visit(item, selected, ancestor_selected);
                }
            }
            Value::Object(map) => {
                let current_selected = map
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| selected.contains(id));
                assert!(!(ancestor_selected && current_selected));
                if let Some(children) = map.get("children") {
                    visit(children, selected, ancestor_selected || current_selected);
                }
            }
            _ => {}
        }
    }
    visit(value, &selected, false);
}

fn serialized_chunk_bytes(value: &Value, ids: &[String]) -> usize {
    fn find<'a>(value: &'a Value, target: &str) -> Option<&'a Value> {
        match value {
            Value::Array(items) => items.iter().find_map(|item| find(item, target)),
            Value::Object(map) => {
                if map.get("id").and_then(Value::as_str) == Some(target) {
                    Some(value)
                } else {
                    map.get("children")
                        .and_then(|children| find(children, target))
                }
            }
            _ => None,
        }
    }

    let nodes = ids
        .iter()
        .filter_map(|id| find(value, id))
        .collect::<Vec<_>>();
    serde_json::to_string(&nodes).unwrap().len()
}
