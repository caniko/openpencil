use jian_ops_schema::node::{FrameNode, PenNode};
use jian_ops_schema::sizing::SizingBehavior;

use crate::{import_html, HtmlImportOptions};

fn children(frame: &FrameNode) -> &[PenNode] {
    frame.children.as_deref().unwrap_or_default()
}

fn nested_position(html: &str) -> (Option<f64>, Option<f64>) {
    let result = import_html(html, &HtmlImportOptions::default());
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!()
    };
    let PenNode::Frame(positioned) = &children(root)[0] else {
        panic!()
    };
    let PenNode::Frame(static_child) = &children(positioned)[0] else {
        panic!()
    };
    let PenNode::Frame(target) = &children(static_child)[0] else {
        panic!()
    };
    (target.base.x, target.base.y)
}

#[test]
fn absolute_offsets_skip_static_ancestors() {
    let position = nested_position(
        "<div style='position:relative;width:300px;height:200px'>\
           <div style='width:100px;height:80px'>\
             <span style='position:absolute;right:0;bottom:0;width:50px;height:20px'></span>\
           </div>\
         </div>",
    );
    assert_eq!(position, (Some(250.0), Some(180.0)));
}

#[test]
fn fixed_offsets_use_the_viewport() {
    let position = nested_position(
        "<div style='position:relative;width:300px;height:200px'>\
           <div style='width:100px;height:80px'>\
             <span style='position:fixed;right:0;bottom:0;width:50px;height:20px'></span>\
           </div>\
         </div>",
    );
    assert_eq!(position, (Some(1390.0), Some(880.0)));
}

#[test]
fn absolute_viewport_unit_offsets_use_the_viewport() {
    let position = nested_position(
        "<div style='position:relative;width:300px;height:200px'>\
           <div style='width:100px;height:80px'>\
             <span style='position:absolute;left:10vw;top:10vh'></span>\
           </div>\
         </div>",
    );
    assert_eq!(position, (Some(144.0), Some(90.0)));
}

#[test]
fn right_and_bottom_offsets_may_place_a_larger_child_at_negative_coordinates() {
    let position = nested_position(
        "<div style='position:relative;width:10px;height:10px'>\
           <div><span style='position:absolute;right:0;bottom:0;width:20px;height:30px'></span></div>\
         </div>",
    );
    assert_eq!(position, (Some(-10.0), Some(-20.0)));
}

#[test]
fn relative_offsets_and_sticky_position_report_static_canvas_degradation() {
    let result = import_html(
        "<div style='display:flex'>\
           <span style='position:relative;left:5px'>first</span><span>second</span>\
           <span style='position:sticky;top:0'>sticky</span>\
         </div>",
        &HtmlImportOptions::default(),
    );
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("relative-position offsets")));
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("position:sticky")));
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!()
    };
    let PenNode::Frame(row) = &children(root)[0] else {
        panic!()
    };
    assert_eq!(
        children(row).len(),
        3,
        "flow items must stay in source order"
    );
}

#[test]
fn absolute_full_size_image_bakes_the_positioned_containing_block() {
    let result = import_html(
        "<div style='position:relative;width:300px;height:200px'>\
           <img src='data:image/png;base64,AA==' \
                style='position:absolute;inset:0;width:100%;height:100%'>\
         </div>",
        &HtmlImportOptions::default(),
    );
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!()
    };
    let PenNode::Frame(positioned) = &children(root)[0] else {
        panic!()
    };
    let PenNode::Image(image) = &children(positioned)[0] else {
        panic!()
    };
    assert_eq!(image.width, Some(SizingBehavior::Number(300.0)));
    assert_eq!(image.height, Some(SizingBehavior::Number(200.0)));
    assert_eq!((image.base.x, image.base.y), (Some(0.0), Some(0.0)));
}

#[test]
fn opposing_absolute_insets_bake_the_remaining_outer_size() {
    let result = import_html(
        "<div style='position:relative;width:300px;height:200px'>\
           <span style='position:absolute;left:10px;right:20px;top:5px;bottom:15px'></span>\
         </div>",
        &HtmlImportOptions::default(),
    );
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!()
    };
    let PenNode::Frame(positioned) = &children(root)[0] else {
        panic!()
    };
    let PenNode::Frame(overlay) = &children(positioned)[0] else {
        panic!()
    };
    assert_eq!(overlay.container.width, Some(SizingBehavior::Number(270.0)));
    assert_eq!(
        overlay.container.height,
        Some(SizingBehavior::Number(180.0))
    );
    assert_eq!((overlay.base.x, overlay.base.y), (Some(10.0), Some(5.0)));
}
