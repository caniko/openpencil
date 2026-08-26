use crate::theme::Theme;
use crate::widgets::property_panel::{NodeSnapshot, PropertyPanelAction};
use crate::widgets::property_panel_inputs::{
    paint_input_with_prefix_focused_state, paint_section_divider, paint_section_label,
    INPUT_HEIGHT, PAD_X, SECTION_HEADER_HEIGHT, TAB_HEIGHT,
};
use crate::widgets::property_panel_sections::EditContext;
use crate::widgets::PaintCx;
use crate::{Point2D, Rect, TextLayout};
use op_editor_core::PropertyFocus;

const ROW_ADVANCE: f32 = INPUT_HEIGHT + 6.0;

const TEXT_ROWS: [(PropertyFocus, &str); 9] = [
    (PropertyFocus::RuntimeId, "ID"),
    (PropertyFocus::RuntimeRole, "Role"),
    (PropertyFocus::RuntimeAccessibilityLabel, "Label"),
    (PropertyFocus::RuntimeTabIndex, "Tab"),
    (PropertyFocus::RuntimeStateDefault, "Default"),
    (PropertyFocus::RuntimeStateHover, "Hover"),
    (PropertyFocus::RuntimeStatePressed, "Pressed"),
    (PropertyFocus::RuntimeStateDisabled, "Disabled"),
    (PropertyFocus::RuntimeStateFocused, "Focused"),
];

pub fn content_height() -> f32 {
    TAB_HEIGHT + SECTION_HEADER_HEIGHT + ROW_ADVANCE * 10.0 + 28.0
}

fn fallback(snapshot: &NodeSnapshot, focus: PropertyFocus) -> &str {
    let runtime = &snapshot.runtime_ui;
    match focus {
        PropertyFocus::RuntimeId => &runtime.runtime_id,
        PropertyFocus::RuntimeRole => &runtime.role,
        PropertyFocus::RuntimeAccessibilityLabel => &runtime.accessibility_label,
        PropertyFocus::RuntimeTabIndex => &runtime.tab_index,
        PropertyFocus::RuntimeStateDefault => &runtime.default_state,
        PropertyFocus::RuntimeStateHover => &runtime.hover_state,
        PropertyFocus::RuntimeStatePressed => &runtime.pressed_state,
        PropertyFocus::RuntimeStateDisabled => &runtime.disabled_state,
        PropertyFocus::RuntimeStateFocused => &runtime.focused_state,
        _ => "",
    }
}

pub fn input_rects(panel_rect: Rect) -> Vec<(PropertyFocus, Rect)> {
    let x = panel_rect.origin.x + PAD_X;
    let width = panel_rect.size.x - PAD_X * 2.0;
    let mut y = panel_rect.origin.y + TAB_HEIGHT + SECTION_HEADER_HEIGHT;
    let mut rects = Vec::with_capacity(TEXT_ROWS.len());
    for (index, (focus, _)) in TEXT_ROWS.into_iter().enumerate() {
        if index == 4 {
            y += ROW_ADVANCE;
        }
        rects.push((
            focus,
            Rect {
                origin: Point2D::new(x, y),
                size: Point2D::new(width, INPUT_HEIGHT),
            },
        ));
        y += ROW_ADVANCE;
    }
    rects
}

pub fn enabled_action_rect(panel_rect: Rect, enabled: bool) -> (PropertyPanelAction, Rect) {
    (
        PropertyPanelAction::ToggleRuntimeEnabled(!enabled),
        Rect {
            origin: Point2D::new(
                panel_rect.origin.x + PAD_X,
                panel_rect.origin.y + TAB_HEIGHT + SECTION_HEADER_HEIGHT + ROW_ADVANCE * 4.0,
            ),
            size: Point2D::new(panel_rect.size.x - PAD_X * 2.0, INPUT_HEIGHT),
        },
    )
}

pub fn paint_runtime_panel(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    snapshot: &NodeSnapshot,
    edit: &EditContext<'_>,
    x: f32,
    y: f32,
    width: f32,
) {
    let usable_width = width - PAD_X * 2.0;
    let mut y = paint_section_label(cx, theme, "Runtime UI", x, y, width);
    for (index, (focus, prefix)) in TEXT_ROWS.into_iter().enumerate() {
        if index == 4 {
            paint_enabled_row(cx, theme, x + PAD_X, y, snapshot.runtime_ui.enabled);
            y += ROW_ADVANCE;
        }
        let rect = Rect {
            origin: Point2D::new(x + PAD_X, y),
            size: Point2D::new(usable_width, INPUT_HEIGHT),
        };
        paint_input_with_prefix_focused_state(
            cx,
            theme,
            rect,
            prefix,
            edit.value_for(focus, fallback(snapshot, focus)),
            edit.focus == Some(focus),
            edit.caret_at(focus),
            edit.select_all_at(focus),
            edit.input_at(focus),
            edit.now_ms,
        );
        y += ROW_ADVANCE;
    }
    y += 12.0;
    paint_section_divider(cx, theme, x, y, width);
}

fn paint_enabled_row(cx: &mut PaintCx<'_>, theme: &Theme, x: f32, y: f32, enabled: bool) {
    let checkbox = Rect {
        origin: Point2D::new(x, y + 7.0),
        size: Point2D::new(16.0, 16.0),
    };
    jian_widgets::components::checkbox::Checkbox {
        checked: enabled,
        enabled: true,
    }
    .paint(
        cx.backend,
        checkbox,
        &crate::widgets::button::tokens_from_theme(theme),
    );
    let text = TextLayout::single_run(
        "Enabled",
        "system-ui",
        12.0,
        theme.foreground.to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend
        .draw_text(&text, Point2D::new(x + 24.0, y + 21.0));
}
