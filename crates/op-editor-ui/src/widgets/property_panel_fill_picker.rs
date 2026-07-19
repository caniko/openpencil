use crate::theme::Theme;
use crate::widgets::property_panel_inputs::TAB_HEIGHT;
use crate::{Point2D, Rect};
use jian_widgets::components::select::{Select, SelectHit, SelectState};
use jian_widgets::Tokens;
use op_editor_core::FillType;

pub const FILL_TYPE_ROW_HEIGHT: f32 = 30.0;
pub const FILL_TYPE_COUNT: usize = 4;
const FILL_TYPE_PICKER_GAP: f32 = 4.0;
pub const FILL_TYPES: [FillType; FILL_TYPE_COUNT] = [
    FillType::Solid,
    FillType::LinearGradient,
    FillType::RadialGradient,
    FillType::Image,
];

pub fn fill_type_at(idx: usize) -> Option<FillType> {
    FILL_TYPES.get(idx).copied()
}

/// Type-dropdown rect for a fill row. The caller must pass the
/// `ToggleFillTypePicker(index)` action rect emitted by the shared
/// property-panel action walker.
pub fn fill_type_dropdown_rect(action_rect: Rect) -> Rect {
    action_rect
}

/// Visible part of the property rail where scrolling section overlays
/// may paint. The pinned Design / Code tabs are deliberately excluded.
pub fn fill_type_picker_viewport(panel_rect: Rect) -> Rect {
    let top = panel_rect.origin.y + TAB_HEIGHT;
    let bottom = panel_rect.origin.y + panel_rect.size.y;
    Rect {
        origin: Point2D::new(panel_rect.origin.x, top),
        size: Point2D::new(panel_rect.size.x.max(0.0), (bottom - top).max(0.0)),
    }
}

/// Picker overlay rect anchored to the fill's type dropdown and clamped
/// to the visible property-panel section viewport. Prefer below, flip
/// above when needed, then clamp as a last resort when neither side fits.
pub fn fill_type_picker_rect(action_rect: Rect, viewport: Rect) -> Rect {
    let dropdown = fill_type_dropdown_rect(action_rect);
    let width = dropdown.size.x.max(0.0).min(viewport.size.x.max(0.0));
    let height = (FILL_TYPE_ROW_HEIGHT * FILL_TYPE_COUNT as f32).min(viewport.size.y.max(0.0));
    let viewport_right = viewport.origin.x + viewport.size.x.max(0.0);
    let viewport_bottom = viewport.origin.y + viewport.size.y.max(0.0);
    let max_x = (viewport_right - width).max(viewport.origin.x);
    let x = dropdown.origin.x.clamp(viewport.origin.x, max_x);
    let below = dropdown.origin.y + dropdown.size.y + FILL_TYPE_PICKER_GAP;
    let above = dropdown.origin.y - FILL_TYPE_PICKER_GAP - height;
    let y = if below + height <= viewport_bottom {
        below
    } else if above >= viewport.origin.y {
        above
    } else {
        (viewport_bottom - height).max(viewport.origin.y)
    };
    Rect {
        origin: Point2D::new(x, y),
        size: Point2D::new(width, height),
    }
}

pub fn fill_type_picker_hit(
    state: &SelectState,
    action_rect: Rect,
    viewport: Rect,
    point: Point2D,
    theme: &Theme,
) -> SelectHit {
    let picker = fill_type_picker_rect(action_rect, viewport);
    Select::hit(
        state,
        popup_anchor(picker),
        picker,
        FILL_TYPE_COUNT,
        point,
        &tokens_from_theme(theme),
    )
}

pub(crate) fn popup_anchor(popup: Rect) -> Rect {
    Rect {
        origin: popup.origin,
        size: Point2D::new(popup.size.x, 0.0),
    }
}

pub(crate) fn tokens_from_theme(theme: &Theme) -> Tokens {
    crate::widgets::button::tokens_from_theme(theme)
}
