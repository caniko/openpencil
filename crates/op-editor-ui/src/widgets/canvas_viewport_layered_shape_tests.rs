//! Regression coverage for canonical fill stacks on non-rectangular nodes.

use super::paint_node;
use crate::layout_scene::{
    NodeKind, SceneFillLayer, SceneGradient, SceneGradientStop, SceneNode, SceneStroke,
    SceneStrokeAlign,
};
use crate::widgets::PaintCx;
use crate::{
    Color, ImageAdjustments, ImageBlendMode, ImageDrawMode, Point2D, Rect, RenderBackend,
    TextLayout,
};
use std::sync::Arc;

#[derive(Debug, PartialEq)]
enum Op {
    RectFill(u8),
    OvalFill(u8),
    PolygonFill(u8),
    SvgFill(u8),
    Gradient(u8),
    OvalStroke,
    PolygonStroke,
    SvgStroke,
    LineStroke,
    Save,
    ClipOval,
    ClipPolygon,
    ClipSvg,
    Composite(f32, ImageBlendMode),
    Image(f32, ImageBlendMode),
    Restore,
}

#[derive(Default)]
struct ShapeCaptureBackend {
    ops: Vec<Op>,
}

impl RenderBackend for ShapeCaptureBackend {
    fn begin_frame(&mut self) {}
    fn end_frame(&mut self) {}
    fn fill_rect(&mut self, _: Rect, color: Color) {
        self.ops.push(Op::RectFill(red_byte(color)));
    }
    fn stroke_rect(&mut self, _: Rect, _: Color, _: f32) {}
    fn draw_text(&mut self, _: &TextLayout, _: Point2D) {}
    fn clip_rect(&mut self, _: Rect) {}
    fn clip_oval(&mut self, _: Rect) {
        self.ops.push(Op::ClipOval);
    }
    fn clip_polygon(&mut self, _: &[Point2D]) {
        self.ops.push(Op::ClipPolygon);
    }
    fn clip_svg_path_in_rect(&mut self, _: &str, _: Rect, _: bool) {
        self.ops.push(Op::ClipSvg);
    }
    fn stroke_line(&mut self, _: Point2D, _: Point2D, _: Color, _: f32) {
        self.ops.push(Op::LineStroke);
    }
    fn fill_round_rect(&mut self, _: Rect, _: f32, _: Color) {}
    fn fill_round_rect_linear_gradient(
        &mut self,
        _: Rect,
        _: f32,
        stops: &[(f32, Color)],
        _: f32,
        _: f32,
    ) {
        self.ops.push(Op::Gradient(red_byte(stops[0].1)));
    }
    fn stroke_round_rect(&mut self, _: Rect, _: f32, _: Color, _: f32) {}
    fn stroke_svg_path(&mut self, _: &str, _: Point2D, _: f32, _: Color, _: f32) {}
    fn fill_oval(&mut self, _: Rect, color: Color) {
        self.ops.push(Op::OvalFill(red_byte(color)));
    }
    fn stroke_oval(&mut self, _: Rect, _: Color, _: f32) {
        self.ops.push(Op::OvalStroke);
    }
    fn fill_polygon(&mut self, _: &[Point2D], color: Color) {
        self.ops.push(Op::PolygonFill(red_byte(color)));
    }
    fn stroke_polygon(&mut self, _: &[Point2D], _: Color, _: f32) {
        self.ops.push(Op::PolygonStroke);
    }
    fn fill_svg_path_in_rect_with_fill_rule(&mut self, _: &str, _: Rect, color: Color, _: bool) {
        self.ops.push(Op::SvgFill(red_byte(color)));
    }
    fn stroke_svg_path_in_rect(&mut self, _: &str, _: Rect, _: Color, _: f32) {
        self.ops.push(Op::SvgStroke);
    }
    fn draw_image_with_options_transform_and_blend(
        &mut self,
        _: Rect,
        _: u64,
        _: &[u8],
        _: ImageDrawMode,
        _: ImageAdjustments,
        opacity: f32,
        _: f32,
        _: Option<[f32; 6]>,
        blend_mode: ImageBlendMode,
    ) {
        self.ops.push(Op::Image(opacity, blend_mode));
    }
    fn save(&mut self) {
        self.ops.push(Op::Save);
    }
    fn push_composite_layer(&mut self, _: Rect, opacity: f32, mode: ImageBlendMode) {
        self.ops.push(Op::Composite(opacity, mode));
    }
    fn restore(&mut self) {
        self.ops.push(Op::Restore);
    }
    fn translate(&mut self, _: Point2D) {}
    fn resize(&mut self, _: u32, _: u32) {}
    fn dpi_scale(&self) -> f32 {
        1.0
    }
}

fn red_byte(color: Color) -> u8 {
    (color.r * 255.0).round() as u8
}

fn solid(red: u8) -> SceneFillLayer {
    SceneFillLayer::Solid {
        color: Color::rgba_u8(red, 0, 0, 1.0),
        blend_mode: ImageBlendMode::Normal,
    }
}

fn image_layer() -> SceneFillLayer {
    SceneFillLayer::Image {
        src: Arc::from(
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
        ),
        src_id: 0x5A17,
        fit: Default::default(),
        transform: None,
        adjustments: Default::default(),
        opacity: 0.75,
        blend_mode: ImageBlendMode::Multiply,
    }
}

fn linear_gradient(red: u8) -> SceneFillLayer {
    SceneFillLayer::Gradient {
        gradient: SceneGradient::Linear {
            angle_deg: 90.0,
            opacity: 1.0,
            stops: vec![
                SceneGradientStop {
                    offset: 0.0,
                    color: Color::rgba_u8(red, 0, 0, 1.0),
                },
                SceneGradientStop {
                    offset: 1.0,
                    color: Color::BLACK,
                },
            ],
        },
        blend_mode: ImageBlendMode::Normal,
    }
}

fn layered_node(kind: NodeKind) -> SceneNode {
    let mut node = SceneNode::leaf("layered-shape", kind);
    node.bounds = Rect::xywh(0.0, 0.0, 100.0, 80.0);
    // Canonical order is front-to-back, so red=1 must paint first.
    node.fill_layers = vec![solid(2), solid(1)];
    node.stroke = Some(SceneStroke {
        color: Color::BLACK,
        width: 2.0,
        sides: None,
        align: SceneStrokeAlign::Center,
    });
    node
}

fn image_node(kind: NodeKind) -> SceneNode {
    let mut node = SceneNode::leaf("clipped-image", kind);
    node.bounds = Rect::xywh(0.0, 0.0, 100.0, 80.0);
    node.opacity = 0.5;
    node.fill_layers = vec![image_layer()];
    node
}

fn painted(node: &SceneNode) -> Vec<Op> {
    let mut backend = ShapeCaptureBackend::default();
    paint_node(
        &mut PaintCx {
            backend: &mut backend,
        },
        node,
        Point2D::ZERO,
        1.0,
        Rect::xywh(-100.0, -100.0, 1000.0, 1000.0),
    );
    backend.ops
}

#[test]
fn ellipse_paints_every_layer_before_its_outline() {
    assert_eq!(
        painted(&layered_node(NodeKind::Ellipse)),
        vec![
            Op::Save,
            Op::ClipOval,
            Op::RectFill(1),
            Op::RectFill(2),
            Op::Restore,
            Op::OvalStroke,
        ]
    );
}

#[test]
fn polygon_paints_every_layer_before_its_outline() {
    assert_eq!(
        painted(&layered_node(NodeKind::Polygon)),
        vec![
            Op::Save,
            Op::ClipPolygon,
            Op::RectFill(1),
            Op::RectFill(2),
            Op::Restore,
            Op::PolygonStroke,
        ]
    );
}

#[test]
fn svg_path_paints_every_layer_before_its_outline() {
    let mut node = layered_node(NodeKind::Path);
    node.svg_path = Some("M0 0H10V10H0Z".to_string());
    assert_eq!(
        painted(&node),
        vec![
            Op::Save,
            Op::ClipSvg,
            Op::RectFill(1),
            Op::RectFill(2),
            Op::Restore,
            Op::SvgStroke,
        ]
    );
}

#[test]
fn closed_point_path_paints_every_layer_before_its_outline() {
    let mut node = layered_node(NodeKind::Path);
    node.path_closed = true;
    node.points = vec![
        Point2D::new(0.0, 0.0),
        Point2D::new(100.0, 0.0),
        Point2D::new(50.0, 80.0),
    ];
    let ops = painted(&node);
    assert_eq!(
        &ops[..4],
        &[Op::Save, Op::ClipPolygon, Op::RectFill(1), Op::RectFill(2),],
        "the path fill stack must paint back-to-front"
    );
    assert!(
        ops.get(4) == Some(&Op::Restore)
            && !ops[5..].is_empty()
            && ops[5..].iter().all(|op| *op == Op::LineStroke),
        "only the path outline may paint above the fill stack: {ops:?}"
    );
}

fn expected_image_ops(clip: Op) -> Vec<Op> {
    vec![
        Op::Save,
        clip,
        Op::Composite(0.5, ImageBlendMode::Normal),
        Op::Composite(1.0, ImageBlendMode::Multiply),
        Op::Image(0.75, ImageBlendMode::Normal),
        Op::Restore,
        Op::Restore,
        Op::Restore,
    ]
}

#[test]
fn image_layers_clip_inside_blend_and_node_opacity_layers_for_every_shape() {
    assert_eq!(
        painted(&image_node(NodeKind::Ellipse)),
        expected_image_ops(Op::ClipOval)
    );
    assert_eq!(
        painted(&image_node(NodeKind::Polygon)),
        expected_image_ops(Op::ClipPolygon)
    );

    let mut point_path = image_node(NodeKind::Path);
    point_path.path_closed = true;
    point_path.points = vec![
        Point2D::new(0.0, 0.0),
        Point2D::new(100.0, 0.0),
        Point2D::new(50.0, 80.0),
    ];
    assert_eq!(painted(&point_path), expected_image_ops(Op::ClipPolygon));

    let mut svg_path = image_node(NodeKind::Path);
    svg_path.svg_path = Some("M0 0H10V10H0Z".to_string());
    assert_eq!(painted(&svg_path), expected_image_ops(Op::ClipSvg));
}

#[test]
fn clipped_shape_keeps_full_gradient_layer_in_stack_order() {
    let mut node = layered_node(NodeKind::Ellipse);
    node.stroke = None;
    node.fill_layers = vec![solid(3), linear_gradient(2), solid(1)];
    assert_eq!(
        painted(&node),
        vec![
            Op::Save,
            Op::ClipOval,
            Op::RectFill(1),
            Op::Gradient(2),
            Op::RectFill(3),
            Op::Restore,
        ]
    );
}

#[test]
fn layered_svg_gradient_uses_its_clip_as_the_only_shape_edge() {
    let mut node = layered_node(NodeKind::Path);
    node.stroke = None;
    node.svg_path = Some("M0 0H10V10H0Z".to_string());
    node.fill_layers = vec![solid(3), linear_gradient(2), solid(1)];

    assert_eq!(
        painted(&node),
        vec![
            Op::Save,
            Op::ClipSvg,
            Op::RectFill(1),
            Op::Gradient(2),
            Op::RectFill(3),
            Op::Restore,
        ]
    );
}
