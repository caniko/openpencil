//! Multi-layer fill painter for layout-scene nodes.

use crate::layout_scene::{SceneFillLayer, SceneGradient, SceneNode};
use crate::widgets::canvas_viewport_image::paint_image_node;
use crate::widgets::canvas_viewport_overlay::{
    paint_gradient_rect, paint_node_stroke, paint_shader_rect, scaled_non_uniform_corner_radii,
};
use crate::widgets::PaintCx;
use crate::{ImageBlendMode, Rect};
use std::sync::Arc;

/// Paint the complete front-to-back fill stack and then the node outline.
/// Returns `false` for legacy scenes whose single-fill fields should be used.
pub(super) fn paint_fill_layers_then_stroke(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    world_rect: Rect,
    zoom: f32,
) -> bool {
    let painted = paint_fill_layers_with(cx, node, world_rect, |cx, layer| {
        paint_rect_fill_layer(cx, node, layer, world_rect, zoom);
    });
    if painted {
        paint_node_stroke(cx, node, world_rect, zoom);
    }
    painted
}

/// Paint the complete front-to-back fill stack through a shape-specific
/// callback. Ordering, node opacity, and per-layer blend isolation stay shared
/// while callers choose the geometry used for each layer.
pub(super) fn paint_fill_layers_with(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    world_rect: Rect,
    mut paint_layer: impl FnMut(&mut PaintCx<'_>, &SceneFillLayer),
) -> bool {
    if node.fill_layers.is_empty() {
        return false;
    }

    // Node opacity belongs to the assembled background, not to each paint.
    // A non-Normal layer also needs this transparent outer backdrop even at
    // opacity 1, otherwise it would blend with canvas content behind the node.
    let isolate_stack = node.opacity < 1.0
        || node
            .fill_layers
            .iter()
            .any(|layer| layer.blend_mode() != ImageBlendMode::Normal);
    if isolate_stack {
        cx.backend
            .push_composite_layer(world_rect, node.opacity, ImageBlendMode::Normal);
    }
    // Canonical fills are front-to-back, whereas raster painting proceeds
    // back-to-front. This keeps CSS's first background-image topmost.
    for layer in node.fill_layers.iter().rev() {
        let blend_mode = layer.blend_mode();
        let isolated = blend_mode != ImageBlendMode::Normal;
        if isolated {
            cx.backend.push_composite_layer(world_rect, 1.0, blend_mode);
        }
        paint_layer(cx, layer);
        if isolated {
            cx.backend.restore();
        }
    }
    if isolate_stack {
        cx.backend.restore();
    }
    true
}

/// Paint a stack inside an exact shape clip. The clip save surrounds the
/// node-opacity and blend layers, so image/gradient AABB painters cannot leak
/// beyond the node silhouette and every restore remains strictly nested.
pub(super) fn paint_clipped_fill_layers_with(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    world_rect: Rect,
    clip: impl FnOnce(&mut dyn crate::RenderBackend),
    paint_layer: impl FnMut(&mut PaintCx<'_>, &SceneFillLayer),
) -> bool {
    if node.fill_layers.is_empty() {
        return false;
    }
    cx.backend.save();
    clip(cx.backend);
    let painted = paint_fill_layers_with(cx, node, world_rect, paint_layer);
    cx.backend.restore();
    painted
}

fn paint_rect_fill_layer(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    layer: &SceneFillLayer,
    world_rect: Rect,
    zoom: f32,
) {
    let radius = node.corner_radius * zoom;
    let radius = if radius > 0.5 { radius } else { 0.0 };
    let corner_radii = scaled_non_uniform_corner_radii(node, zoom);

    match layer {
        SceneFillLayer::Solid { color, .. } => {
            if let Some(radii) = corner_radii {
                cx.backend
                    .fill_round_rect_per_corner(world_rect, radii, *color);
            } else if radius > 0.0 {
                cx.backend.fill_round_rect(world_rect, radius, *color);
            } else {
                cx.backend.fill_rect(world_rect, *color);
            }
        }
        SceneFillLayer::Gradient { gradient, .. } => {
            paint_gradient_rect(cx, gradient, world_rect, radius, corner_radii);
        }
        SceneFillLayer::Shader { shader, .. } => {
            paint_shader_rect(cx, shader, world_rect, radius, corner_radii);
        }
        SceneFillLayer::Image { .. } => {
            paint_image_fill_layer(cx, node, layer, world_rect, zoom);
        }
    }
}

/// Paint one image layer without standalone-image placeholder art. Shape
/// callers use their existing clipping/degradation behavior around this AABB
/// paint while sharing the decode and compositing path with rectangles.
pub(super) fn paint_image_fill_layer(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    layer: &SceneFillLayer,
    world_rect: Rect,
    zoom: f32,
) -> bool {
    let SceneFillLayer::Image {
        src,
        src_id,
        fit,
        transform,
        adjustments,
        opacity,
        ..
    } = layer
    else {
        return false;
    };
    let mut image = SceneNode::leaf(node.id.as_str(), node.kind.clone());
    image.corner_radius = node.corner_radius;
    image.corner_radii = node.corner_radii;
    image.image_src = Some(Arc::clone(src));
    image.image_src_id = *src_id;
    image.image_fit = *fit;
    image.image_transform = *transform;
    image.image_adjustments = *adjustments;
    image.image_blend_mode = ImageBlendMode::Normal;
    image.opacity = *opacity;
    // A CSS/background fill miss is transparent. It still queues its decode,
    // but standalone image placeholder art must not cover lower layers.
    paint_image_node(cx, &image, world_rect, zoom, src, false);
    true
}

/// Paint a non-solid layer across its destination AABB. Callers install an
/// exact silhouette clip first, which turns the existing rectangle gradient,
/// shader, and image backends into faithful shape fills without duplicating
/// those rich paint implementations.
pub(super) fn paint_clipped_shape_rich_fill_layer(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    layer: &SceneFillLayer,
    world_rect: Rect,
    zoom: f32,
) -> bool {
    match layer {
        SceneFillLayer::Solid { .. } => false,
        SceneFillLayer::Gradient { gradient, .. } => {
            paint_gradient_rect(cx, gradient, world_rect, 0.0, None);
            true
        }
        SceneFillLayer::Shader { shader, .. } => {
            paint_shader_rect(cx, shader, world_rect, 0.0, None);
            true
        }
        SceneFillLayer::Image { .. } => {
            paint_image_fill_layer(cx, node, layer, world_rect, zoom);
            true
        }
    }
}

/// Solid fallback used by shapes whose backend has no gradient/shader
/// primitive for that geometry. This matches the loader's legacy projection,
/// but does it per layer so the complete stack is retained.
pub(super) fn fill_layer_fallback_color(layer: &SceneFillLayer) -> Option<crate::Color> {
    let fold_opacity = |color: crate::Color, opacity: f32| crate::Color {
        a: color.a * opacity.clamp(0.0, 1.0),
        ..color
    };
    match layer {
        SceneFillLayer::Solid { color, .. } => Some(*color),
        SceneFillLayer::Gradient { gradient, .. } => match gradient {
            SceneGradient::Linear { opacity, stops, .. }
            | SceneGradient::Radial { opacity, stops, .. } => {
                stops.first().map(|stop| fold_opacity(stop.color, *opacity))
            }
            SceneGradient::Mesh {
                colors, opacity, ..
            } => colors
                .first()
                .copied()
                .map(|color| fold_opacity(color, *opacity)),
        },
        SceneFillLayer::Shader { shader, .. } => {
            Some(fold_opacity(shader.fallback, shader.opacity))
        }
        SceneFillLayer::Image { .. } => None,
    }
}

/// Paint an SVG path's complete canonical fill stack. One exact path clip
/// shapes every AABB layer, preserving the silhouette without applying edge AA
/// twice; shaders keep their documented fallback on unsupported backends.
pub(super) fn paint_svg_path_fill_layers(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    world_rect: Rect,
    zoom: f32,
    d: &str,
) -> bool {
    paint_clipped_fill_layers_with(
        cx,
        node,
        world_rect,
        |backend| backend.clip_svg_path_in_rect(d, world_rect, node.even_odd_fill),
        |cx, layer| match layer {
            SceneFillLayer::Solid { color, .. } => {
                // The path clip owns the only anti-aliased silhouette edge.
                // Re-drawing the same path here would square edge coverage.
                cx.backend.fill_rect(world_rect, *color);
            }
            SceneFillLayer::Gradient { gradient, .. } => {
                // Gradient coordinates are defined by `world_rect` on both
                // path backends. Paint the AABB and let the exact clip shape it
                // so linear/radial paths do not receive AA twice.
                paint_gradient_rect(cx, gradient, world_rect, 0.0, None);
            }
            SceneFillLayer::Shader { shader, .. } => {
                paint_shader_rect(cx, shader, world_rect, 0.0, None);
            }
            SceneFillLayer::Image { .. } => {
                paint_image_fill_layer(cx, node, layer, world_rect, zoom);
            }
        },
    )
}

pub(super) fn paint_svg_path_gradient(
    cx: &mut PaintCx<'_>,
    node: &SceneNode,
    gradient: &SceneGradient,
    world_rect: Rect,
    d: &str,
    mesh_fallback: Option<crate::Color>,
) {
    match gradient {
        SceneGradient::Linear {
            angle_deg,
            opacity,
            stops,
        } => {
            let flat: Vec<(f32, crate::Color)> =
                stops.iter().map(|s| (s.offset, s.color)).collect();
            cx.backend
                .fill_svg_path_in_rect_linear_gradient_with_fill_rule(
                    d,
                    world_rect,
                    &flat,
                    *angle_deg,
                    *opacity,
                    node.even_odd_fill,
                );
        }
        SceneGradient::Radial {
            cx: gx,
            cy,
            radius,
            opacity,
            stops,
        } => {
            let flat: Vec<(f32, crate::Color)> =
                stops.iter().map(|s| (s.offset, s.color)).collect();
            cx.backend
                .fill_svg_path_in_rect_radial_gradient_with_fill_rule(
                    d,
                    world_rect,
                    &flat,
                    *gx,
                    *cy,
                    *radius,
                    *opacity,
                    node.even_odd_fill,
                );
        }
        // There is no per-vertex SVG-path mesh primitive. Preserve the
        // documented first-vertex fallback for this individual layer.
        SceneGradient::Mesh {
            colors, opacity, ..
        } => {
            let first_vertex = colors.first().copied().map(|color| crate::Color {
                a: color.a * opacity.clamp(0.0, 1.0),
                ..color
            });
            if let Some(color) = mesh_fallback.or(first_vertex) {
                cx.backend.fill_svg_path_in_rect_with_fill_rule(
                    d,
                    world_rect,
                    color,
                    node.even_odd_fill,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout_scene::{NodeKind, SceneStroke, SceneStrokeAlign};
    use crate::{Color, Point2D, RenderBackend, TextLayout};

    #[derive(Debug, PartialEq)]
    enum Op {
        Composite(Rect, f32, ImageBlendMode),
        Fill(u8),
        Stroke,
        Restore,
    }

    #[derive(Default)]
    struct CaptureBackend {
        ops: Vec<Op>,
    }

    impl RenderBackend for CaptureBackend {
        fn begin_frame(&mut self) {}
        fn end_frame(&mut self) {}
        fn fill_rect(&mut self, _: Rect, color: Color) {
            self.ops.push(Op::Fill((color.r * 255.0).round() as u8));
        }
        fn stroke_rect(&mut self, _: Rect, _: Color, _: f32) {
            self.ops.push(Op::Stroke);
        }
        fn draw_text(&mut self, _: &TextLayout, _: Point2D) {}
        fn clip_rect(&mut self, _: Rect) {}
        fn stroke_line(&mut self, _: Point2D, _: Point2D, _: Color, _: f32) {
            self.ops.push(Op::Stroke);
        }
        fn fill_round_rect(&mut self, _: Rect, _: f32, _: Color) {}
        fn stroke_round_rect(&mut self, _: Rect, _: f32, _: Color, _: f32) {
            self.ops.push(Op::Stroke);
        }
        fn stroke_svg_path(&mut self, _: &str, _: Point2D, _: f32, _: Color, _: f32) {}
        fn save(&mut self) {}
        fn push_composite_layer(&mut self, bounds: Rect, opacity: f32, mode: ImageBlendMode) {
            self.ops.push(Op::Composite(bounds, opacity, mode));
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

    fn solid(red: u8, blend_mode: ImageBlendMode) -> SceneFillLayer {
        SceneFillLayer::Solid {
            color: Color {
                r: f32::from(red) / 255.0,
                g: 0.0,
                b: 0.0,
                a: 1.0,
            },
            blend_mode,
        }
    }

    #[test]
    fn canonical_front_to_back_layers_paint_back_to_front_with_blends() {
        let mut node = SceneNode::leaf("layered", NodeKind::Rect);
        node.opacity = 0.5;
        node.stroke = Some(SceneStroke {
            color: Color::BLACK,
            width: 1.0,
            sides: None,
            align: SceneStrokeAlign::Center,
        });
        node.fill_layers = vec![
            solid(3, ImageBlendMode::Screen),
            solid(2, ImageBlendMode::Normal),
            solid(1, ImageBlendMode::Multiply),
        ];
        let mut backend = CaptureBackend::default();
        let bounds = Rect::xywh(0.0, 0.0, 20.0, 20.0);
        assert!(paint_fill_layers_then_stroke(
            &mut PaintCx {
                backend: &mut backend,
            },
            &node,
            bounds,
            1.0,
        ));
        assert_eq!(
            backend.ops,
            vec![
                Op::Composite(bounds, 0.5, ImageBlendMode::Normal),
                Op::Composite(bounds, 1.0, ImageBlendMode::Multiply),
                Op::Fill(1),
                Op::Restore,
                Op::Fill(2),
                Op::Composite(bounds, 1.0, ImageBlendMode::Screen),
                Op::Fill(3),
                Op::Restore,
                Op::Restore,
                Op::Stroke,
            ]
        );
    }

    #[test]
    fn unavailable_background_image_is_transparent_over_lower_fill() {
        let mut node = SceneNode::leaf("layered-image", NodeKind::Rect);
        node.fill_layers = vec![
            SceneFillLayer::Image {
                src: Arc::from("placeholder://missing-background"),
                src_id: 42,
                fit: Default::default(),
                transform: None,
                adjustments: Default::default(),
                opacity: 1.0,
                blend_mode: ImageBlendMode::Normal,
            },
            solid(7, ImageBlendMode::Normal),
        ];
        let mut backend = CaptureBackend::default();

        assert!(paint_fill_layers_then_stroke(
            &mut PaintCx {
                backend: &mut backend,
            },
            &node,
            Rect::xywh(0.0, 0.0, 20.0, 20.0),
            1.0,
        ));

        assert_eq!(backend.ops, vec![Op::Fill(7)]);
    }
}
