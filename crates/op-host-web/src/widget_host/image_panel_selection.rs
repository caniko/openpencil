//! Mouse selection for Search / Generate popover text inputs.

use super::WidgetHost;
use op_editor_ui::widgets::property_panel_image_assets::ImagePopoverInputKind;
use op_editor_ui::widgets::{PropertyPanel, TOP_BAR_HEIGHT};
use op_editor_ui::{Point2D, Rect};

#[derive(Debug, Clone, Copy)]
pub(in crate::widget_host) struct ImageInputSelectionDragState {
    pub(in crate::widget_host) kind: ImagePopoverInputKind,
    pub(in crate::widget_host) anchor: usize,
}

impl WidgetHost {
    fn cached_image_input_is_visible(&self, kind: ImagePopoverInputKind) -> bool {
        let panel = &self.editor_state.editor_ui.image_panel;
        match kind {
            ImagePopoverInputKind::Search => panel.search_open,
            ImagePopoverInputKind::Generate => {
                panel.generate_open
                    && self
                        .editor_state
                        .editor_ui
                        .agent_settings
                        .image_generation_configured()
                    && panel.active_input(true).is_some()
            }
        }
    }

    pub(in crate::widget_host) fn image_popover_input_at(
        &self,
        panel: &PropertyPanel,
        rect: Rect,
        point: Point2D,
    ) -> Option<(ImagePopoverInputKind, usize)> {
        if let Some(geometry) = self.image_input_geometry.as_ref() {
            if self.cached_image_input_is_visible(geometry.kind) {
                if let Some(offset) =
                    geometry.byte_offset_at(&self.editor_state.editor_ui.image_panel, point, false)
                {
                    return Some((geometry.kind, offset));
                }
            }
        }
        panel.image_popover_input_at(rect, point)
    }

    pub(in crate::widget_host) fn cached_image_input_caret_rect(&self) -> Option<Rect> {
        let geometry = self.image_input_geometry.as_ref()?;
        self.cached_image_input_is_visible(geometry.kind)
            .then_some(())?;
        geometry.caret_rect(&self.editor_state.editor_ui.image_panel)
    }

    pub(in crate::widget_host) fn begin_image_input_selection_drag(
        &mut self,
        kind: ImagePopoverInputKind,
        offset: usize,
    ) -> bool {
        let configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let extend = self.shift_held;
        let now_ms = self.now_ms;
        let panel = &mut self.editor_state.editor_ui.image_panel;
        let input = match kind {
            ImagePopoverInputKind::Search if panel.search_open => &mut panel.search_query,
            ImagePopoverInputKind::Generate if panel.generate_open && configured => {
                &mut panel.generate_prompt
            }
            _ => return false,
        };
        if extend {
            input.drag_to(offset, now_ms);
        } else {
            input.set_caret(offset, now_ms);
        }
        let anchor = input.selection().anchor;
        self.image_input_selection_drag = Some(ImageInputSelectionDragState { kind, anchor });
        self.mark_dirty();
        true
    }

    fn image_input_drag_offset_at_screen(
        &mut self,
        kind: ImagePopoverInputKind,
        x: f32,
        y: f32,
    ) -> Option<usize> {
        if let Some(geometry) = self.image_input_geometry.as_ref() {
            if geometry.kind == kind && self.cached_image_input_is_visible(kind) {
                if let Some(offset) = geometry.byte_offset_at(
                    &self.editor_state.editor_ui.image_panel,
                    Point2D::new(x, y),
                    true,
                ) {
                    return Some(offset);
                }
            }
        }
        self.refresh_layout_scene();
        let rect = Rect {
            origin: Point2D::new(
                self.last_viewport_w - self.editor_state.editor_ui.property_panel_width,
                TOP_BAR_HEIGHT,
            ),
            size: Point2D::new(
                self.editor_state.editor_ui.property_panel_width,
                (self.last_viewport_h - TOP_BAR_HEIGHT).max(0.0),
            ),
        };
        let panel = PropertyPanel::for_selection(&self.editor_state)?;
        panel.image_popover_input_drag_offset_at(rect, kind, Point2D::new(x, y))
    }

    pub(in crate::widget_host) fn apply_image_input_selection_drag_cursor_move(
        &mut self,
        x: f32,
        y: f32,
    ) -> bool {
        let Some(drag) = self.image_input_selection_drag else {
            return false;
        };
        let Some(focus) = self.image_input_drag_offset_at_screen(drag.kind, x, y) else {
            self.image_input_selection_drag = None;
            return false;
        };
        self.drag_image_input_selection_to(drag, focus)
    }

    pub(in crate::widget_host) fn drag_image_input_selection_to(
        &mut self,
        drag: ImageInputSelectionDragState,
        focus: usize,
    ) -> bool {
        let now_ms = self.now_ms;
        let panel = &mut self.editor_state.editor_ui.image_panel;
        let input = match drag.kind {
            ImagePopoverInputKind::Search if panel.search_open => &mut panel.search_query,
            ImagePopoverInputKind::Generate if panel.generate_open => &mut panel.generate_prompt,
            _ => return false,
        };
        let before = input.selection();
        input.set_caret(drag.anchor, now_ms);
        input.drag_to(focus, now_ms);
        if input.selection() != before {
            self.mark_dirty();
        }
        true
    }

    pub(in crate::widget_host) fn clear_image_input_selection_drag(&mut self) {
        self.image_input_selection_drag = None;
    }
}
