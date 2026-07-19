//! Web missing-font modal dispatch and detection lifecycle.

use super::WidgetHost;
use op_editor_core::missing_fonts::{detect_missing_fonts, refresh_prompt};
use op_editor_ui::widgets::{MissingFontsHit, MissingFontsPanel};
use op_editor_ui::{Point2D, Rect};

impl WidgetHost {
    pub(in crate::widget_host) fn dispatch_missing_fonts_press(
        &mut self,
        panel_rect: Rect,
        point: Point2D,
    ) -> bool {
        let Some(hit) = MissingFontsPanel::for_editor(&self.editor_state)
            .map(|panel| panel.hit_test(panel_rect, point))
        else {
            return false;
        };
        match hit {
            MissingFontsHit::ChooseFile(row) => {
                self.editor_state.editor_ui.missing_fonts_import_row = Some(row);
            }
            MissingFontsHit::Dismiss => {
                self.editor_state.editor_ui.missing_fonts_modal_open = false;
            }
            MissingFontsHit::Inside | MissingFontsHit::Outside => {}
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
        ui.missing_fonts_prompt = prompt;
        self.mark_dirty();
    }

    pub(crate) fn refresh_missing_fonts_prompt(&mut self) {
        let Some(mut prompt) = self.editor_state.editor_ui.missing_fonts_prompt.take() else {
            return;
        };
        let all_resolved = refresh_prompt(&mut prompt, &self.editor_state.editor_ui);
        self.editor_state.editor_ui.missing_fonts_prompt = Some(prompt);
        if all_resolved {
            self.editor_state.editor_ui.missing_fonts_modal_open = false;
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
    use op_editor_core::EditorState;
    use op_editor_ui::widgets::MissingFontsPanel;
    use op_editor_ui::Point2D;

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

    #[test]
    fn choose_file_press_records_row() {
        let mut host = host_with_missing_fonts(&["Katibeh"]);
        let panel = MissingFontsPanel::for_editor(&host.editor_state).expect("open prompt");
        let rect = panel.rect(1200.0, 800.0);
        let point = Point2D::new(rect.origin.x + rect.size.x - 95.0, rect.origin.y + 90.0);

        assert!(host.dispatch_missing_fonts_press(rect, point));
        assert_eq!(host.take_missing_fonts_import_row(), Some(0));
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

        assert!(host.dispatch_missing_fonts_press(rect, point));
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
