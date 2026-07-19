//! One-off raw-field probe: find nodes by name and print their raw Kiwi
//! fields (visibility, scroll behavior, clip, parent order) plus their
//! direct children summary. Used for the 汇总稿 tabbar-occlusion triage.
//!
//! Usage:
//! `cargo run -p op-figma --example probe_frame -- <file.fig> <name> [<parent-name-filter>]`

#![allow(dead_code)]

#[path = "../src/container.rs"]
mod container;
#[path = "../src/corner_geometry.rs"]
mod corner_geometry;
#[path = "../src/figma_types.rs"]
mod figma_types;
#[path = "../src/kiwi.rs"]
mod kiwi;
#[path = "../src/tree.rs"]
mod tree;
#[path = "../src/vector_decoder.rs"]
mod vector_decoder;
#[path = "../src/zip_reader.rs"]
mod zip_reader;

use figma_types::parse_fig_file;
use kiwi::FigValue;
use tree::{build_tree, TreeNode};

const INTERESTING: &[&str] = &[
    "visible",
    "opacity",
    "scrollBehavior",
    "scrollDirection",
    "overflowDirection",
    "frameMaskDisabled",
    "resizeToFit",
    "mask",
    "maskType",
    "clipsContent",
    "size",
    "transform",
    "parentIndex",
    "type",
    "horizontalConstraint",
    "verticalConstraint",
    "constraints",
    "proportionsConstrained",
    "textAutoResize",
    "fontSize",
    "blendMode",
    "styleType",
];

fn summarize(value: &FigValue) -> String {
    let mut out = Vec::new();
    for key in INTERESTING {
        if let Some(v) = value.get(key) {
            out.push(format!("{key}={}", render(v)));
        }
    }
    out.join(" ")
}

fn render(v: &FigValue) -> String {
    if let Some(s) = v.as_str() {
        return format!("{s:?}");
    }
    if let Some(n) = v.as_f64() {
        return format!("{n}");
    }
    if let Some(b) = v.as_bool() {
        return format!("{b}");
    }
    // Compound: render one level of scalar fields.
    let mut parts = Vec::new();
    for key in [
        "x",
        "y",
        "m00",
        "m01",
        "m02",
        "m10",
        "m11",
        "m12",
        "position",
        "guid",
        "sessionID",
        "localID",
    ] {
        if let Some(inner) = v.get(key) {
            if let Some(n) = inner.as_f64() {
                parts.push(format!("{key}:{n}"));
            } else if let Some(s) = inner.as_str() {
                parts.push(format!("{key}:{s:?}"));
            }
        }
    }
    if parts.is_empty() {
        "<compound>".into()
    } else {
        format!("{{{}}}", parts.join(","))
    }
}

fn walk<'a>(
    node: &'a TreeNode,
    target: &str,
    parent_filter: Option<&str>,
    parent_name: &str,
    hits: &mut Vec<(&'a TreeNode, String)>,
) {
    let name = node.figma.get_str("name").unwrap_or("");
    if name == target && parent_filter.is_none_or(|f| parent_name.contains(f)) {
        hits.push((node, parent_name.to_string()));
    }
    for child in &node.children {
        walk(child, target, parent_filter, name, hits);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("fig path");
    let target = args.next().expect("target name");
    let parent_filter = args.next();

    let bytes = std::fs::read(&path).expect("read fig");
    let parsed = parse_fig_file(&bytes).expect("parse fig");
    let tree = build_tree(&parsed.node_changes)
        .into_iter()
        .collect::<Vec<_>>();

    let mut hits = Vec::new();
    for root in &tree {
        walk(root, &target, parent_filter.as_deref(), "<root>", &mut hits);
    }
    println!("hits={}", hits.len());
    for (node, parent) in hits.iter().take(4) {
        println!("== {target} (parent {parent}) ==");
        println!("  self: {}", summarize(&node.figma));
        println!("  children ({}):", node.children.len());
        for child in &node.children {
            let name = child.figma.get_str("name").unwrap_or("?");
            println!("    - {name:?}: {}", summarize(&child.figma));
        }
    }
}
