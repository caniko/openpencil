//! Native missing-font modal dispatch and detection lifecycle.

use super::WidgetHostNative;
use op_editor_core::missing_fonts::{detect_missing_fonts, refresh_prompt};
use op_editor_ui::widgets::{MissingFontsHit, MissingFontsPanel};
use op_editor_ui::{Point2D, Rect};

impl WidgetHostNative {
    /// Route a press to the top-most missing-font modal.
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
                self.editor_state.editor_ui.missing_fonts_hover = None;
            }
            MissingFontsHit::Inside | MissingFontsHit::Outside => {}
        }
        self.mark_dirty();
        true
    }

    /// Drain the expected-family import request raised by either prompt surface.
    pub fn take_missing_fonts_import_row(&mut self) -> Option<usize> {
        self.editor_state.editor_ui.missing_fonts_import_row.take()
    }

    /// Enumerate system fonts through the property picker's canonical routine,
    /// then detect missing document families. The pending flag remains the
    /// fallback if enumeration ever becomes asynchronous.
    pub fn arm_missing_fonts_detection(&mut self) {
        if !self.editor_state.editor_ui.system_fonts_loaded {
            self.editor_state.editor_ui.missing_fonts_pending_detect = true;
            self.ensure_system_fonts_loaded();
        }
        if self.editor_state.editor_ui.system_fonts_loaded {
            self.replace_missing_fonts_data(true);
        }
    }

    /// Recompute the Settings Fonts-tab data without opening the one-shot modal.
    pub(in crate::widget_host) fn refresh_missing_fonts_for_settings(&mut self) {
        if !self.editor_state.editor_ui.system_fonts_loaded {
            self.editor_state.editor_ui.missing_fonts_pending_detect = true;
            self.ensure_system_fonts_loaded();
        }
        if self.editor_state.editor_ui.system_fonts_loaded {
            self.replace_missing_fonts_data(false);
        }
    }

    fn replace_missing_fonts_data(&mut self, open_modal: bool) {
        let prompt = detect_missing_fonts(&self.editor_state);
        let ui = &mut self.editor_state.editor_ui;
        ui.missing_fonts_pending_detect = false;
        ui.missing_fonts_modal_open = open_modal && prompt.is_some();
        ui.missing_fonts_prompt = prompt;
        self.mark_dirty();
    }

    /// Reconcile existing rows against the latest system/imported snapshots.
    pub fn refresh_missing_fonts_prompt(&mut self) {
        let Some(mut prompt) = self.editor_state.editor_ui.missing_fonts_prompt.take() else {
            return;
        };
        let all_resolved = refresh_prompt(&mut prompt, &self.editor_state.editor_ui);
        self.editor_state.editor_ui.missing_fonts_prompt = Some(prompt);
        if all_resolved {
            self.editor_state.editor_ui.missing_fonts_modal_open = false;
            self.editor_state.editor_ui.missing_fonts_hover = None;
        }
        self.mark_dirty();
    }

    /// Record whether the supplied file declared the row's expected family,
    /// then refresh resolution from the live imported-font snapshot.
    pub fn note_missing_font_supplied(&mut self, row: usize, actual_family: Option<&str>) {
        let locale = self.editor_state.editor_ui.locale;
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
                    op_i18n::translate(locale, "missingFonts.mismatch")
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

    use super::super::WidgetHostNative;

    fn host_with_missing_fonts(families: &[&str]) -> WidgetHostNative {
        let mut host = WidgetHostNative::new();
        host.editor_state_mut().editor_ui.missing_fonts_prompt = Some(MissingFontsPrompt {
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
        host.editor_state_mut().editor_ui.missing_fonts_modal_open = true;
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
    fn choose_file_press_records_row_and_import_stays_pending() {
        let mut host = host_with_missing_fonts(&["Katibeh"]);
        let panel = MissingFontsPanel::for_editor(host.editor_state()).expect("open prompt");
        let rect = panel.rect(1200.0, 800.0);
        let point = Point2D::new(rect.origin.x + rect.size.x - 95.0, rect.origin.y + 90.0);

        assert!(host.dispatch_missing_fonts_press(rect, point));
        assert_eq!(host.take_missing_fonts_import_row(), Some(0));
    }

    #[test]
    fn dismiss_press_closes_modal_but_keeps_data_for_the_tab() {
        let mut host = host_with_missing_fonts(&["Katibeh"]);
        let panel = MissingFontsPanel::for_editor(host.editor_state()).expect("open prompt");
        let rect = panel.rect(1200.0, 800.0);
        let point = Point2D::new(
            rect.origin.x + rect.size.x - 70.0,
            rect.origin.y + rect.size.y - 30.0,
        );

        assert!(host.dispatch_missing_fonts_press(rect, point));
        assert!(!host.editor_state().editor_ui.missing_fonts_modal_open);
        assert!(host.editor_state().editor_ui.missing_fonts_prompt.is_some());
    }

    #[test]
    fn install_imported_state_enumerates_then_detects_missing_fonts() {
        let mut host = WidgetHostNative::new();
        assert!(!host.editor_state().editor_ui.system_fonts_loaded);

        host.install_imported_state(state_with_text("__OpenPencilMissingFontTest__"));

        let ui = &host.editor_state().editor_ui;
        assert!(ui.system_fonts_loaded);
        assert!(!ui.missing_fonts_pending_detect);
        assert!(ui.missing_fonts_modal_open);
        assert_eq!(
            ui.missing_fonts_prompt.as_ref().unwrap().entries[0].family,
            "__OpenPencilMissingFontTest__"
        );
    }

    #[test]
    fn settings_refresh_recomputes_data_without_opening_modal() {
        let mut host = WidgetHostNative::new();
        *host.editor_state_mut() = state_with_text("__OpenPencilSettingsMissingFontTest__");
        host.editor_state_mut().editor_ui.system_fonts_loaded = true;

        host.refresh_missing_fonts_for_settings();

        let ui = &host.editor_state().editor_ui;
        assert!(ui.missing_fonts_prompt.is_some());
        assert!(!ui.missing_fonts_modal_open);
    }
}
