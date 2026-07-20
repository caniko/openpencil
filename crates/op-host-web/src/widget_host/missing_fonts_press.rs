//! Web missing-font modal dispatch and detection lifecycle.

use super::WidgetHost;
use op_editor_core::missing_fonts::detect_missing_fonts;
use op_editor_ui::widgets::agent_settings_fonts::FontsHit;
use op_editor_ui::widgets::{MissingFontsHit, MissingFontsPanel};
use op_editor_ui::{Point2D, Rect};

impl WidgetHost {
    pub(in crate::widget_host) fn dispatch_settings_fonts_press(&mut self, hit: FontsHit) {
        match hit {
            FontsHit::ChooseFont(row) => self
                .editor_state
                .editor_ui
                .open_missing_font_picker(row, op_editor_core::MissingFontSurface::Settings),
            FontsHit::SelectFont(index) => {
                let replacement = {
                    let ui = &self.editor_state.editor_ui;
                    op_editor_ui::widgets::property_panel_typography::font_picker_entries(
                        &ui.imported_font_families,
                        &ui.bundled_font_families,
                        &ui.system_font_families,
                        &ui.font_picker_search,
                    )
                    .get(index)
                    .map(|entry| entry.family.clone())
                };
                let expected = match self.editor_state.editor_ui.font_picker_purpose {
                    Some(op_editor_core::FontPickerPurpose::MissingFont { row, .. }) => self
                        .editor_state
                        .editor_ui
                        .missing_fonts_prompt
                        .as_ref()
                        .and_then(|prompt| prompt.entries.get(row))
                        .map(|entry| entry.family.clone()),
                    _ => None,
                };
                if let (Some(from), Some(to)) = (expected, replacement) {
                    let _ = self
                        .editor_state
                        .apply(op_editor_core::EditorCommand::ReplaceFontFamily { from, to });
                }
                self.editor_state.editor_ui.close_font_picker();
                self.refresh_missing_fonts_for_settings();
            }
            FontsHit::ImportFont(row) => {
                self.editor_state.editor_ui.missing_fonts_import_row = Some(row)
            }
            FontsHit::ClosePicker => {
                self.editor_state.editor_ui.close_font_picker();
            }
            FontsHit::RemoveImportedFont(index) => {
                if let Some(family) = self
                    .editor_state
                    .editor_ui
                    .imported_font_families
                    .get(index)
                    .cloned()
                {
                    self.editor_state.editor_ui.pending_font_remove = Some(family);
                }
            }
            FontsHit::PickerInside | FontsHit::None => {}
        }
    }

    pub(in crate::widget_host) fn try_scroll_missing_fonts_picker(
        &mut self,
        x: f32,
        y: f32,
        delta_y: f32,
        viewport_width: f32,
        viewport_height: f32,
    ) -> bool {
        if !self.editor_state.editor_ui.missing_fonts_modal_open {
            return false;
        }
        let viewport = Rect {
            origin: Point2D::new(0.0, 0.0),
            size: Point2D::new(viewport_width, viewport_height),
        };
        let point = Point2D::new(x, y);
        let resolved = MissingFontsPanel::for_editor(&self.editor_state).map(|panel| {
            let panel_rect = panel.rect(viewport_width, viewport_height);
            let picker = panel.picker_layout(panel_rect, viewport);
            let picker_scroll = picker
                .as_ref()
                .filter(|layout| layout.popup.contains(point))
                .map(|layout| layout.max_scroll);
            let rows_scroll = if picker.is_none() && panel.rows_rect(panel_rect).contains(point) {
                Some(panel.max_rows_scroll(panel_rect))
            } else {
                None
            };
            (picker_scroll, rows_scroll)
        });
        if let Some((Some(max_scroll), _)) = resolved {
            let ui = &mut self.editor_state.editor_ui;
            let next = (ui.font_picker.scroll.offset - delta_y).clamp(0.0, max_scroll);
            if next != ui.font_picker.scroll.offset {
                ui.font_picker.scroll.offset = next;
                ui.font_picker.hover = None;
                ui.font_picker_import_hover = false;
                self.mark_dirty();
            }
        } else if let Some((_, Some(max_scroll))) = resolved {
            let ui = &mut self.editor_state.editor_ui;
            let next = (ui.missing_fonts_scroll.offset - delta_y).clamp(0.0, max_scroll);
            if next != ui.missing_fonts_scroll.offset {
                ui.missing_fonts_scroll.offset = next;
                ui.missing_fonts_hover = None;
                self.mark_dirty();
            }
        }
        true
    }

    pub(in crate::widget_host) fn dispatch_missing_fonts_press(
        &mut self,
        panel_rect: Rect,
        viewport_rect: Rect,
        point: Point2D,
    ) -> bool {
        let Some(hit) = MissingFontsPanel::for_editor(&self.editor_state)
            .map(|panel| panel.hit_test(panel_rect, viewport_rect, point))
        else {
            return false;
        };
        match hit {
            MissingFontsHit::ChooseFont(row) => {
                self.editor_state
                    .editor_ui
                    .open_missing_font_picker(row, op_editor_core::MissingFontSurface::Prompt);
                self.editor_state.editor_ui.missing_fonts_hover = None;
            }
            MissingFontsHit::SelectFont(index) => {
                let replacement = MissingFontsPanel::for_editor(&self.editor_state)
                    .and_then(|panel| panel.picker_entries().get(index).cloned())
                    .map(|entry| entry.family);
                let expected = match self.editor_state.editor_ui.font_picker_purpose {
                    Some(op_editor_core::FontPickerPurpose::MissingFont { row, .. }) => self
                        .editor_state
                        .editor_ui
                        .missing_fonts_prompt
                        .as_ref()
                        .and_then(|prompt| prompt.entries.get(row))
                        .map(|entry| entry.family.clone()),
                    _ => None,
                };
                if let (Some(from), Some(to)) = (expected, replacement) {
                    let _ = self
                        .editor_state
                        .apply(op_editor_core::EditorCommand::ReplaceFontFamily { from, to });
                }
                self.editor_state.editor_ui.close_font_picker();
                self.replace_missing_fonts_data(true);
            }
            MissingFontsHit::ImportFont(row) => {
                self.editor_state.editor_ui.missing_fonts_import_row = Some(row);
            }
            MissingFontsHit::ClosePicker => {
                self.editor_state.editor_ui.close_font_picker();
            }
            MissingFontsHit::Dismiss => {
                self.editor_state.editor_ui.missing_fonts_modal_open = false;
                self.editor_state.editor_ui.missing_fonts_hover = None;
                self.editor_state.editor_ui.close_font_picker();
            }
            MissingFontsHit::PickerInside | MissingFontsHit::Inside | MissingFontsHit::Outside => {}
        }
        self.mark_dirty();
        true
    }

    pub(crate) fn take_missing_fonts_import_row(&mut self) -> Option<usize> {
        self.editor_state.editor_ui.missing_fonts_import_row.take()
    }

    /// Arm detection and schedule the existing CanvasKit font drain, whose
    /// queryLocalFonts path supplies the asynchronous system snapshot.
    pub fn arm_missing_fonts_detection(&mut self) {
        if self.editor_state.editor_ui.system_fonts_loaded {
            self.replace_missing_fonts_data(true);
            return;
        }
        self.editor_state.editor_ui.missing_fonts_pending_detect = true;
        self.mark_dirty();
        crate::repaint_coalescer::request();
    }

    pub(in crate::widget_host) fn refresh_missing_fonts_for_settings(&mut self) {
        if self.editor_state.editor_ui.system_fonts_loaded {
            self.replace_missing_fonts_data(false);
            return;
        }
        self.editor_state.editor_ui.missing_fonts_pending_detect = true;
        self.mark_dirty();
        crate::repaint_coalescer::request();
    }

    pub(in crate::widget_host) fn refresh_missing_fonts_after_document_change(&mut self) {
        let settings_fonts_open = self.editor_state.editor_ui.agent_settings_open
            && matches!(
                self.editor_state.editor_ui.agent_settings.tab,
                op_editor_core::AgentSettingsTab::Fonts
            );
        if settings_fonts_open {
            self.refresh_missing_fonts_for_settings();
        } else {
            self.arm_missing_fonts_detection();
        }
    }

    pub(crate) fn complete_pending_missing_fonts_detection(&mut self) {
        if !self.editor_state.editor_ui.missing_fonts_pending_detect
            || !self.editor_state.editor_ui.system_fonts_loaded
        {
            return;
        }
        let settings_fonts_open = self.editor_state.editor_ui.agent_settings_open
            && matches!(
                self.editor_state.editor_ui.agent_settings.tab,
                op_editor_core::AgentSettingsTab::Fonts
            );
        self.replace_missing_fonts_data(!settings_fonts_open);
    }

    fn replace_missing_fonts_data(&mut self, open_modal: bool) {
        let prompt = detect_missing_fonts(&self.editor_state);
        let ui = &mut self.editor_state.editor_ui;
        ui.missing_fonts_pending_detect = false;
        ui.missing_fonts_modal_open = open_modal && prompt.is_some();
        if open_modal {
            ui.missing_fonts_scroll.offset = 0.0;
        }
        ui.missing_fonts_prompt = prompt;
        self.mark_dirty();
    }

    pub(crate) fn refresh_missing_fonts_prompt(&mut self) {
        let previous = self.editor_state.editor_ui.missing_fonts_prompt.take();
        let mut next = detect_missing_fonts(&self.editor_state);
        if let (Some(previous), Some(next)) = (previous.as_ref(), next.as_mut()) {
            for entry in &mut next.entries {
                if let Some(old) = previous
                    .entries
                    .iter()
                    .find(|old| old.family.eq_ignore_ascii_case(&entry.family))
                {
                    entry.mismatch_note = old.mismatch_note.clone();
                }
            }
        }
        self.editor_state.editor_ui.missing_fonts_prompt = next;
        if self.editor_state.editor_ui.missing_fonts_prompt.is_none() {
            let ui = &mut self.editor_state.editor_ui;
            ui.missing_fonts_modal_open = false;
            ui.missing_fonts_hover = None;
            ui.close_font_picker();
        }
        self.mark_dirty();
    }

    pub(crate) fn note_missing_font_supplied(&mut self, row: usize, actual_family: Option<&str>) {
        let mismatch_template = op_editor_ui::widgets::editor_state_ext::translate(
            &self.editor_state.editor_ui,
            "missingFonts.mismatch",
        );
        if let Some(entry) = self
            .editor_state
            .editor_ui
            .missing_fonts_prompt
            .as_mut()
            .and_then(|prompt| prompt.entries.get_mut(row))
        {
            entry.mismatch_note = actual_family
                .filter(|actual| !actual.eq_ignore_ascii_case(&entry.family))
                .map(|actual| {
                    mismatch_template
                        .replace("{actual}", actual)
                        .replace("{expected}", &entry.family)
                });
        }
        self.refresh_missing_fonts_prompt();
    }
}

#[cfg(test)]
mod tests {
    use op_editor_core::missing_fonts::{MissingFontEntry, MissingFontsPrompt};
    use op_editor_core::{AgentSettingsTab, EditorState, MissingFontSurface};
    use op_editor_ui::widgets::agent_settings_panel::AgentSettingsPanel;
    use op_editor_ui::widgets::MissingFontsPanel;
    use op_editor_ui::Point2D;
    use std::sync::Arc;

    use super::super::WidgetHost;

    fn host_with_missing_fonts(families: &[&str]) -> WidgetHost {
        let mut host = WidgetHost::new();
        host.editor_state.editor_ui.missing_fonts_prompt = Some(MissingFontsPrompt {
            entries: families
                .iter()
                .map(|family| MissingFontEntry {
                    family: (*family).to_string(),
                    run_count: 1,
                    mismatch_note: None,
                    resolved: false,
                })
                .collect(),
        });
        host.editor_state.editor_ui.missing_fonts_modal_open = true;
        host
    }

    fn state_with_text(family: &str) -> EditorState {
        let doc: jian_ops_schema::PenDocument = serde_json::from_str(&format!(
            r#"{{"version":"0.8.0","children":[
                {{"type":"text","id":"t1","name":"t","x":0,"y":0,"width":10,"height":10,
                  "content":"hi","fontFamily":"{family}"}}]}}"#
        ))
        .expect("document");
        EditorState::from_document(doc)
    }

    fn populate_scrollable_font_list(host: &mut WidgetHost) {
        host.editor_state.editor_ui.system_font_families = Arc::new(
            (0..40)
                .map(|index| format!("System Font {index:02}"))
                .collect(),
        );
    }

    #[test]
    fn choose_font_press_opens_the_shared_picker_for_the_row() {
        let mut host = host_with_missing_fonts(&["Katibeh"]);
        let panel = MissingFontsPanel::for_editor(&host.editor_state).expect("open prompt");
        let rect = panel.rect(1200.0, 800.0);
        let point = Point2D::new(rect.origin.x + rect.size.x - 50.0, rect.origin.y + 90.0);

        let viewport = op_editor_ui::Rect::xywh(0.0, 0.0, 1200.0, 800.0);
        assert!(host.dispatch_missing_fonts_press(rect, viewport, point));
        assert_eq!(
            host.editor_state().editor_ui.font_picker_purpose,
            Some(op_editor_core::FontPickerPurpose::MissingFont {
                row: 0,
                surface: op_editor_core::MissingFontSurface::Prompt,
            })
        );
    }

    #[test]
    fn wheel_scrolls_prompt_and_settings_font_pickers_down() {
        const VIEWPORT_W: f32 = 1200.0;
        const VIEWPORT_H: f32 = 800.0;
        let viewport = op_editor_ui::Rect::xywh(0.0, 0.0, VIEWPORT_W, VIEWPORT_H);

        for surface in [MissingFontSurface::Prompt, MissingFontSurface::Settings] {
            let mut host = host_with_missing_fonts(&["Katibeh"]);
            populate_scrollable_font_list(&mut host);
            host.editor_state
                .editor_ui
                .open_missing_font_picker(0, surface);

            let popup = match surface {
                MissingFontSurface::Prompt => {
                    let panel = MissingFontsPanel::for_editor(&host.editor_state).expect("prompt");
                    let panel_rect = panel.rect(VIEWPORT_W, VIEWPORT_H);
                    let layout = panel
                        .picker_layout(panel_rect, viewport)
                        .expect("prompt picker");
                    assert!(layout.max_scroll > 80.0);
                    layout.popup
                }
                MissingFontSurface::Settings => {
                    let ui = &mut host.editor_state.editor_ui;
                    ui.missing_fonts_modal_open = false;
                    ui.agent_settings_open = true;
                    ui.agent_settings.tab = AgentSettingsTab::Fonts;
                    let panel = AgentSettingsPanel::for_web_editor(&host.editor_state);
                    let panel_rect = panel.rect(VIEWPORT_W, VIEWPORT_H);
                    let layout = panel
                        .font_picker_layout(panel_rect)
                        .expect("settings picker");
                    assert!(layout.max_scroll > 80.0);
                    layout.popup
                }
            };
            let point = Point2D::new(
                popup.origin.x + popup.size.x / 2.0,
                popup.origin.y + popup.size.y / 2.0,
            );
            let zoom_before = host.editor_state.viewport.zoom;

            assert!(host.apply_wheel(point.x, point.y, -80.0, VIEWPORT_W, VIEWPORT_H,));

            assert_eq!(host.editor_state.editor_ui.font_picker.scroll.offset, 80.0);
            assert_eq!(host.editor_state.viewport.zoom, zoom_before);
        }
    }

    #[test]
    fn dismiss_keeps_prompt_data() {
        let mut host = host_with_missing_fonts(&["Katibeh"]);
        let panel = MissingFontsPanel::for_editor(&host.editor_state).expect("open prompt");
        let rect = panel.rect(1200.0, 800.0);
        let point = Point2D::new(
            rect.origin.x + rect.size.x - 70.0,
            rect.origin.y + rect.size.y - 30.0,
        );

        assert!(host.dispatch_missing_fonts_press(
            rect,
            op_editor_ui::Rect::xywh(0.0, 0.0, 1200.0, 800.0),
            point,
        ));
        assert!(!host.editor_state.editor_ui.missing_fonts_modal_open);
        assert!(host.editor_state.editor_ui.missing_fonts_prompt.is_some());
    }

    #[test]
    fn ingest_with_loaded_snapshot_detects_immediately() {
        let mut host = WidgetHost::new();
        host.editor_state.editor_ui.system_fonts_loaded = true;

        host.install_ingested_state(state_with_text("__OpenPencilWebMissingFontTest__"));

        let ui = &host.editor_state.editor_ui;
        assert!(ui.missing_fonts_modal_open);
        assert_eq!(
            ui.missing_fonts_prompt.as_ref().unwrap().entries[0].family,
            "__OpenPencilWebMissingFontTest__"
        );
    }

    #[test]
    fn ingest_without_snapshot_stays_pending_until_query_result_lands() {
        let mut host = WidgetHost::new();

        host.install_ingested_state(state_with_text("__OpenPencilWebDeferredFontTest__"));
        assert!(host.editor_state.editor_ui.missing_fonts_pending_detect);

        host.apply_browser_system_font_families(vec!["Arial".to_string()]);

        let ui = &host.editor_state.editor_ui;
        assert!(!ui.missing_fonts_pending_detect);
        assert!(ui.missing_fonts_modal_open);
        assert_eq!(
            ui.missing_fonts_prompt.as_ref().unwrap().entries[0].family,
            "__OpenPencilWebDeferredFontTest__"
        );
    }
}
