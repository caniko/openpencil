//! Missing-font detection for the post-load prompt.
//!
//! This module is platform-free: the diff runs against the family snapshots
//! both hosts maintain on [`EditorUiState`] (system fonts plus imported fonts),
//! so native and web share the same detection logic.

use crate::{EditorState, EditorUiState};
use jian_ops_schema::font_plan::FontPlan;

#[derive(Debug, Clone, PartialEq)]
pub struct MissingFontEntry {
    pub family: String,
    pub run_count: u32,
    /// Set after a supply attempt whose file declared another family.
    pub mismatch_note: Option<String>,
    pub resolved: bool,
}

/// Hoverable controls of the missing-fonts modal and the shared row
/// component — cursor-move hover wash state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingFontsHover {
    ChooseFile(usize),
    RemoveImported(usize),
    Dismiss,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MissingFontsPrompt {
    pub entries: Vec<MissingFontEntry>,
}

fn family_available(family: &str, ui: &EditorUiState) -> bool {
    let matches = |candidate: &String| candidate.eq_ignore_ascii_case(family);
    ui.imported_font_families.iter().any(matches) || ui.system_font_families.iter().any(matches)
}

/// Diff the document's families against the current available-family snapshots.
///
/// Returns `None` when every family is available or system enumeration has not
/// finished yet. Hosts distinguish those cases through
/// `missing_fonts_pending_detect` and retry after enumeration completes.
pub fn detect_missing_fonts(state: &EditorState) -> Option<MissingFontsPrompt> {
    if !state.editor_ui.system_fonts_loaded {
        return None;
    }

    let plan = FontPlan::scan(&state.doc);
    let entries = plan
        .families()
        .filter(|(family, _)| !family.is_empty())
        .filter(|(family, _)| !family_available(family, &state.editor_ui))
        .map(|(family, usage)| MissingFontEntry {
            family: family.to_owned(),
            run_count: usage.run_count,
            mismatch_note: None,
            resolved: false,
        })
        .collect::<Vec<_>>();

    (!entries.is_empty()).then_some(MissingFontsPrompt { entries })
}

/// Re-check prompt entries against the current available-family snapshots.
///
/// Returns `true` when every row is resolved so the host can close the modal.
pub fn refresh_prompt(prompt: &mut MissingFontsPrompt, ui: &EditorUiState) -> bool {
    for entry in &mut prompt.entries {
        if !entry.resolved && family_available(&entry.family, ui) {
            entry.resolved = true;
            entry.mismatch_note = None;
        }
    }
    prompt.entries.iter().all(|entry| entry.resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn missing_family_is_detected_with_run_count() {
        let mut state = state_with_text("Katibeh");
        state.editor_ui.system_fonts_loaded = true;
        state.editor_ui.system_font_families = std::sync::Arc::new(vec!["Arial".into()]);

        let prompt = detect_missing_fonts(&state).expect("prompt");
        assert_eq!(prompt.entries.len(), 1);
        assert_eq!(prompt.entries[0].family, "Katibeh");
        assert!(prompt.entries[0].run_count >= 1);
    }

    #[test]
    fn available_families_produce_no_prompt() {
        let mut state = state_with_text("Arial");
        state.editor_ui.system_fonts_loaded = true;
        state.editor_ui.system_font_families = std::sync::Arc::new(vec!["Arial".into()]);

        assert!(detect_missing_fonts(&state).is_none());
    }

    #[test]
    fn imported_families_count_as_available() {
        let mut state = state_with_text("Katibeh");
        state.editor_ui.system_fonts_loaded = true;
        state.editor_ui.imported_font_families = std::sync::Arc::new(vec!["Katibeh".into()]);

        assert!(detect_missing_fonts(&state).is_none());
    }

    #[test]
    fn family_matching_is_ascii_case_insensitive() {
        let mut state = state_with_text("Katibeh");
        state.editor_ui.system_fonts_loaded = true;
        state.editor_ui.system_font_families = std::sync::Arc::new(vec!["KATIBEH".into()]);

        assert!(detect_missing_fonts(&state).is_none());
    }

    #[test]
    fn unloaded_system_fonts_defer_detection() {
        let state = state_with_text("Katibeh");
        assert!(
            detect_missing_fonts(&state).is_none(),
            "defer instead of reporting a false positive"
        );
    }

    #[test]
    fn refresh_marks_resolved_and_reports_all_done() {
        let mut prompt = MissingFontsPrompt {
            entries: vec![MissingFontEntry {
                family: "Katibeh".into(),
                run_count: 3,
                mismatch_note: Some("wrong family".into()),
                resolved: false,
            }],
        };
        let ui = EditorUiState {
            imported_font_families: std::sync::Arc::new(vec!["Katibeh".into()]),
            ..EditorUiState::default()
        };

        assert!(refresh_prompt(&mut prompt, &ui), "all rows resolved");
        assert!(prompt.entries[0].resolved);
        assert_eq!(prompt.entries[0].mismatch_note, None);
    }

    #[test]
    fn refresh_keeps_unavailable_rows_unresolved() {
        let mut prompt = MissingFontsPrompt {
            entries: vec![MissingFontEntry {
                family: "Katibeh".into(),
                run_count: 1,
                mismatch_note: None,
                resolved: false,
            }],
        };

        assert!(!refresh_prompt(&mut prompt, &EditorUiState::default()));
        assert!(!prompt.entries[0].resolved);
    }
}
