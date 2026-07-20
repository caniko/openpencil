use super::*;
use crate::css::cascade::StyleRule;
use std::collections::BTreeMap;

fn computed(properties: &[(&str, &str)]) -> ComputedStyle {
    ComputedStyle {
        props: properties
            .iter()
            .map(|(name, value)| ((*name).into(), (*value).into()))
            .collect::<BTreeMap<_, _>>(),
        font_size: 16.0,
    }
}

fn run<T>(action: impl FnOnce(&mut MapCtx<'_>) -> T) -> (T, Vec<String>) {
    let options = crate::HtmlImportOptions::default();
    let rules = Vec::<StyleRule>::new();
    let mut context = MapCtx {
        opts: &options,
        rules: &rules,
        warnings: Vec::new(),
        next_id: 0,
        node_count: 0,
        containing_width: options.viewport_width,
        containing_height: options.viewport_width * 0.625,
        containing_width_is_definite: true,
        positioned_width: options.viewport_width,
        positioned_height: options.viewport_width * 0.625,
    };
    let output = action(&mut context);
    (output, context.warnings)
}

#[test]
fn layered_background_keeps_css_order_color_and_variable_stops() {
    let style = computed(&[
        ("--tone", "oklch(62% 0.2 25)"),
        (
            "background-image",
            "linear-gradient(90deg,var(--tone) 10% 20%,rgb(0 0 255 / 50%) 80%),url(\"a,b.png\")",
        ),
        ("background-repeat", "no-repeat,no-repeat"),
        ("background-size", "cover,contain"),
        ("background-color", "#123456"),
    ]);
    let (fills, warnings) = run(|context| map_fill(&style, context).unwrap());
    let PenFill::LinearGradient(gradient) = &fills[0] else {
        panic!("top CSS layer must remain the first fill")
    };
    assert_eq!(gradient.angle, Some(90.0));
    assert_eq!(gradient.stops.len(), 3);
    assert_eq!(
        (gradient.stops[0].offset, gradient.stops[1].offset),
        (0.1, 0.2)
    );
    assert!(matches!(&fills[1], PenFill::Image(image)
        if image.mode == Some(ImageFillMode::Fit)));
    assert!(matches!(&fills[2], PenFill::Solid(fill) if fill.color == "#123456"));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("background-size on gradients"));
}

#[test]
fn radial_gradient_maps_position_radius_and_implicit_stop() {
    let style = computed(&[]);
    let (fill, warnings) = run(|context| {
        map_gradient(
            "radial-gradient(circle 40% at top right,red 0%,blue,green 100%)",
            &style,
            context,
        )
        .unwrap()
    });
    let PenFill::RadialGradient(gradient) = fill else {
        panic!()
    };
    assert_eq!(
        (gradient.cx, gradient.cy, gradient.radius),
        (Some(1.0), Some(0.0), Some(0.4))
    );
    assert_eq!(gradient.stops[1].offset, 0.5);
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn multiple_outer_and_inset_shadows_are_preserved() {
    let style = computed(&[
        ("color", "#334455"),
        (
            "box-shadow",
            "inset 0 1px 2px 3px rgb(0 0 0 / 25%),4px 5px #ff0000",
        ),
    ]);
    let (effects, warnings) = run(|context| map_effects(&style, context).unwrap());
    assert_eq!(effects.len(), 2);
    assert!(matches!(&effects[0], PenEffect::Shadow(shadow)
        if shadow.inner == Some(true) && shadow.spread == 3.0 && shadow.color == "#00000040"));
    assert!(matches!(&effects[1], PenEffect::Shadow(shadow)
        if shadow.inner.is_none() && shadow.offset_x == 4.0 && shadow.color == "#ff0000"));
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn per_side_border_widths_survive_and_unrepresentable_differences_warn() {
    let style = computed(&[
        ("border-top-style", "dashed"),
        ("border-top-width", "2px"),
        ("border-top-color", "red"),
        ("border-right-style", "none"),
        ("border-right-width", "7px"),
        ("border-bottom-style", "dotted"),
        ("border-bottom-width", "4px"),
        ("border-bottom-color", "blue"),
        ("border-left-style", "solid"),
        ("border-left-width", "6px"),
        ("border-left-color", "blue"),
    ]);
    let (stroke, warnings) = run(|context| map_stroke(&style, context).unwrap());
    assert_eq!(
        stroke.thickness,
        StrokeThickness::PerSide([2.0, 0.0, 4.0, 6.0])
    );
    assert_eq!(stroke.align, Some(StrokeAlign::Inside));
    assert!(stroke.dash_pattern.is_some());
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("border colors")));
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("border styles")));
}

#[test]
fn border_style_uses_medium_width_and_initial_current_color() {
    let style = computed(&[("border-style", "solid")]);
    let (stroke, warnings) = run(|context| map_stroke(&style, context).unwrap());
    assert_eq!(stroke.thickness, StrokeThickness::Uniform(3.0));
    assert!(matches!(stroke.fill.as_ref().unwrap().first(),
        Some(PenFill::Solid(fill)) if fill.color == "#000000"));
    assert!(warnings.is_empty());
}

#[test]
fn transparent_fill_is_skipped_and_unsupported_visuals_warn_once() {
    let transparent = computed(&[("background-color", "rgb(1 2 3 / 0)")]);
    let (fill, _) = run(|context| map_fill(&transparent, context));
    assert!(fill.is_none());

    let unsupported = computed(&[
        ("background-image", "conic-gradient(red,blue)"),
        ("background-position", "25% bottom"),
        ("filter", "brightness(2) brightness(3)"),
    ]);
    let (_, warnings) = run(|context| {
        map_fill(&unsupported, context);
        map_effects(&unsupported, context);
    });
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning.contains("filter function"))
            .count(),
        1
    );
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("conic gradients")));
    assert!(warnings
        .iter()
        .any(|warning| warning.contains("background-position")));
}

#[test]
fn omitted_gradient_stops_are_interpolated() {
    let mut stops = vec![
        ParsedStop {
            color: "a".into(),
            offset: 0.0,
            explicit: false,
        },
        ParsedStop {
            color: "b".into(),
            offset: 0.0,
            explicit: false,
        },
        ParsedStop {
            color: "c".into(),
            offset: 0.0,
            explicit: false,
        },
    ];
    distribute_stops(&mut stops);
    assert_eq!(
        (stops[0].offset, stops[1].offset, stops[2].offset),
        (0.0, 0.5, 1.0)
    );
}
