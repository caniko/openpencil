//! `EditorCommand::MoveNode` / `CopyNode` page targeting tests.

#![cfg(test)]

use crate::command::EditorCommand;
use crate::components::ComponentLibrary;
use crate::node_id::NodeId;
use crate::pen_node_ext::PenNodeExt;
use crate::test_support::{frame, rect, state_with};
use crate::walkers::{find_node, find_node_mut};
use jian_ops_schema::node::PenNode;
use jian_ops_schema::page::PenPage;
use serde_json::json;

fn id(s: &str) -> NodeId {
    NodeId::new(s)
}

#[test]
fn move_node_can_target_requested_page_without_switching_active_page() {
    let mut s = state_with(vec![]);
    s.doc.pages = Some(vec![
        PenPage {
            id: "page-1".into(),
            name: "Page 1".into(),
            children: vec![rect("n1", "Current", 0.0, 0.0, 10.0, 10.0)],
            state: None,
            lifecycle: None,
        },
        PenPage {
            id: "page-2".into(),
            name: "Page 2".into(),
            children: vec![
                frame("n2", "Target", 0.0, 0.0, 100.0, 100.0, Vec::new()),
                rect("n3", "Moved", 0.0, 0.0, 10.0, 10.0),
            ],
            state: None,
            lifecycle: None,
        },
    ]);
    s.ui.active_page_index = 0;

    assert!(s.apply(EditorCommand::MoveNode {
        node_id: id("n3"),
        target_parent: id("n2"),
        page_id: Some("page-2".into()),
        index: None,
    }));

    let pages = s.doc.pages.as_ref().expect("pages");
    let target_children = pages[1].children[0].children().expect("frame children");
    assert_eq!(target_children.len(), 1);
    assert_eq!(target_children[0].id_str(), "n3");
    assert_eq!(s.ui.active_page_index, 0);
}

#[test]
fn move_node_can_insert_at_requested_root_index() {
    let mut s = state_with(vec![
        rect("n1", "A", 0.0, 0.0, 10.0, 10.0),
        rect("n2", "B", 0.0, 0.0, 10.0, 10.0),
        rect("n3", "C", 0.0, 0.0, 10.0, 10.0),
    ]);

    assert!(s.apply(EditorCommand::MoveNode {
        node_id: id("n3"),
        target_parent: NodeId::NONE,
        page_id: None,
        index: Some(1),
    }));

    let ids: Vec<&str> = s.active_children().iter().map(|n| n.id_str()).collect();
    assert_eq!(ids, vec!["n1", "n3", "n2"]);
}

#[test]
fn copy_node_can_target_requested_page_without_switching_active_page() {
    let mut s = state_with(vec![]);
    s.doc.pages = Some(vec![
        PenPage {
            id: "page-1".into(),
            name: "Page 1".into(),
            children: vec![rect("n1", "Current", 0.0, 0.0, 10.0, 10.0)],
            state: None,
            lifecycle: None,
        },
        PenPage {
            id: "page-2".into(),
            name: "Page 2".into(),
            children: vec![
                frame("n2", "Target", 0.0, 0.0, 100.0, 100.0, Vec::new()),
                rect("n3", "Source", 0.0, 0.0, 10.0, 10.0),
            ],
            state: None,
            lifecycle: None,
        },
    ]);
    s.ui.active_page_index = 0;

    assert!(s.apply(EditorCommand::CopyNode {
        node_id: id("n3"),
        target_parent: id("n2"),
        overrides_json: None,
        page_id: Some("page-2".into()),
    }));

    let pages = s.doc.pages.as_ref().expect("pages");
    let target_children = pages[1].children[0].children().expect("frame children");
    assert_eq!(target_children.len(), 1);
    assert_ne!(target_children[0].id_str(), "n3");
    assert_eq!(target_children[0].base().name.as_deref(), Some("Source"));
    assert_eq!(s.ui.active_page_index, 0);
}

#[test]
fn copy_node_applies_root_overrides_without_overriding_fresh_id() {
    let mut s = state_with(vec![rect("n1", "Source", 0.0, 0.0, 10.0, 10.0)]);
    let source = find_node_mut(s.active_children_mut(), &id("n1")).unwrap();
    source.base_mut().runtime_id = Some("menu.source".into());
    source.base_mut().role = Some("button".into());
    source.base_mut().visual_states = Some(
        [("hover".into(), "menu.source.hover".into())]
            .into_iter()
            .collect(),
    );

    assert!(s.apply(EditorCommand::CopyNode {
        node_id: id("n1"),
        target_parent: NodeId::NONE,
        page_id: None,
        overrides_json: Some(r#"{"id":"override-id","name":"Copy","x":42,"width":88}"#.into()),
    }));

    let clone = s
        .active_children()
        .iter()
        .find(|node| node.id_str() != "n1")
        .expect("cloned node");
    assert_ne!(clone.id_str(), "override-id");
    assert_eq!(clone.base().name.as_deref(), Some("Copy"));
    assert_eq!(clone.base().x, Some(42.0));
    assert_eq!(clone.width_px(), Some(88.0));
    assert_eq!(clone.base().runtime_id, None);
    assert_eq!(clone.base().visual_states, None);
    assert_eq!(clone.base().role.as_deref(), Some("button"));
}

#[test]
fn copy_node_clears_effective_component_runtime_identity() {
    let mut child = rect("child", "Label", 0.0, 0.0, 10.0, 10.0);
    child.base_mut().runtime_id = Some("menu.label".into());
    child.base_mut().role = Some("label".into());
    let mut nested_child = rect("nested-child", "Icon", 0.0, 0.0, 10.0, 10.0);
    nested_child.base_mut().role = Some("img".into());
    let mut nested_component = frame(
        "nested-component",
        "Icon",
        0.0,
        0.0,
        10.0,
        10.0,
        vec![nested_child],
    );
    nested_component.base_mut().role = Some("group".into());
    if let PenNode::Frame(frame) = &mut nested_component {
        frame.reusable = Some(true);
    }
    let nested_reference: PenNode = serde_json::from_value(json!({
        "type": "ref",
        "id": "nested-instance",
        "ref": "nested-component"
    }))
    .unwrap();
    let mut component = frame(
        "component",
        "Menu",
        0.0,
        0.0,
        100.0,
        40.0,
        vec![child, nested_reference],
    );
    component.base_mut().role = Some("menu".into());
    if let PenNode::Frame(frame) = &mut component {
        frame.reusable = Some(true);
    }
    let reference: PenNode = serde_json::from_value(json!({
        "type": "ref",
        "id": "instance",
        "ref": "component",
        "descendants": {
            "component": {
                "runtimeId": "menu.override",
                "visualStates": {"hover": "menu.hover"}
            }
        }
    }))
    .unwrap();
    let mut s = state_with(vec![nested_component, component, reference]);
    s.components = ComponentLibrary::from_document(&s.doc);
    find_node_mut(s.active_children_mut(), &id("component"))
        .unwrap()
        .base_mut()
        .runtime_id = Some("menu.root".into());
    find_node_mut(s.active_children_mut(), &id("nested-component"))
        .unwrap()
        .base_mut()
        .runtime_id = Some("menu.icon.root".into());
    find_node_mut(s.active_children_mut(), &id("nested-child"))
        .unwrap()
        .base_mut()
        .runtime_id = Some("menu.icon".into());

    assert!(s.apply(EditorCommand::CopyNode {
        node_id: id("instance"),
        target_parent: NodeId::NONE,
        page_id: None,
        overrides_json: None,
    }));

    let clone = s
        .active_children()
        .iter()
        .find(|node| matches!(node, PenNode::Ref(r) if r.base.id != "instance"))
        .expect("cloned instance");
    let clone_id = id(clone.id_str());
    let resolved = crate::ref_resolve::resolve_refs_for_canvas(&s.doc);
    let display = find_node(&resolved.children, &clone_id).expect("resolved clone");
    assert_eq!(display.base().runtime_id, None);
    assert_eq!(display.base().visual_states, None);
    assert_eq!(display.base().role.as_deref(), Some("menu"));
    let children = display.children().unwrap();
    let child = &children[0];
    assert_eq!(child.base().runtime_id, None);
    assert_eq!(child.base().role.as_deref(), Some("label"));
    let nested = &children[1];
    assert_eq!(nested.base().runtime_id, None);
    assert_eq!(nested.base().role.as_deref(), Some("group"));
    let nested_child = nested.children().unwrap().first().unwrap();
    assert_eq!(nested_child.base().runtime_id, None);
    assert_eq!(nested_child.base().role.as_deref(), Some("img"));
}

#[test]
fn move_node_accepts_a_childless_container_but_rejects_a_leaf() {
    let doc: jian_ops_schema::PenDocument = serde_json::from_value(json!({
        "version": "1.0",
        "children": [
            {"type":"text","id":"source","content":"Move me"},
            {"type":"frame","id":"empty-frame","name":"Empty Frame"},
            {"type":"text","id":"leaf","content":"Not a parent"}
        ]
    }))
    .unwrap();
    let mut state = crate::EditorState::from_document(doc);
    assert!(state.apply(EditorCommand::MoveNode {
        node_id: id("source"),
        target_parent: id("empty-frame"),
        page_id: None,
        index: None,
    }));
    let frame = find_node(state.active_children(), &id("empty-frame")).unwrap();
    assert!(frame
        .children()
        .is_some_and(|children| children.iter().any(|node| node.id_str() == "source")));

    let before = serde_json::to_value(&state.doc).unwrap();
    assert!(!state.apply(EditorCommand::MoveNode {
        node_id: id("source"),
        target_parent: id("leaf"),
        page_id: None,
        index: None,
    }));
    assert_eq!(serde_json::to_value(&state.doc).unwrap(), before);
}

#[test]
fn copy_node_accepts_a_childless_rectangle_container() {
    let doc: jian_ops_schema::PenDocument = serde_json::from_value(json!({
        "version": "1.0",
        "children": [
            {"type":"text","id":"source","content":"Copy me"},
            {"type":"rectangle","id":"empty-rectangle","name":"Empty Rectangle"}
        ]
    }))
    .unwrap();
    let mut state = crate::EditorState::from_document(doc);
    assert!(state.apply(EditorCommand::CopyNode {
        node_id: id("source"),
        target_parent: id("empty-rectangle"),
        overrides_json: None,
        page_id: None,
    }));
    let rectangle = find_node(state.active_children(), &id("empty-rectangle")).unwrap();
    assert!(rectangle.children().is_some_and(|children| {
        children.len() == 1 && matches!(children[0], PenNode::Text(_))
    }));
}
