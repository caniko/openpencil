//! Desktop-side font import / removal — drains the requests raised by
//! the property-panel font picker (`PropertyPanelAction::ImportFont` /
//! `RemoveImportedFont`).
//!
//! The picker + registry live cross-platform (`op-editor-ui`,
//! `jian-skia`), but the disk-backed [`crate::fonts::FontStore`] and the
//! native file dialog are desktop-only, so op-host-native surfaces the
//! request via pending flags and this module performs the IO — the same
//! delegation shape `theme_preset_host` uses for `.optheme` files.

use crate::fonts::{FontStore, MAX_FONT_BYTES};
use op_host_native::WidgetHostNative;

/// Drain a pending font import / removal. Blocking (rfd / disk IO),
/// like `theme_preset_host::drain_preset_io`. Returns `true` when an
/// action ran so the caller requests a redraw.
pub fn drain_font_requests(host: &mut WidgetHostNative) -> bool {
    let mut ran = false;
    if let Some(row) = host.take_missing_fonts_import_row() {
        import_missing_font_dialog(host, row);
        ran = true;
    }
    // Import first: opening the dialog is the user-visible action.
    if host.take_font_import_request() {
        import_font_dialog(host);
        ran = true;
    }
    if let Some(family) = host.take_font_remove_request() {
        remove_font(host, &family);
        ran = true;
    }
    ran
}

/// Pick and import a font for one missing-family row, retaining a mismatch
/// note when the file declares another family.
fn import_missing_font_dialog(host: &mut WidgetHostNative, row: usize) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Font", &["ttf", "otf"])
        .pick_file()
    else {
        return;
    };
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() as usize > MAX_FONT_BYTES {
            show_font_error_dialog(&format!(
                "{} is too large ({:.1} MiB; max {} MiB).",
                path.display(),
                meta.len() as f64 / (1024.0 * 1024.0),
                MAX_FONT_BYTES / (1024 * 1024)
            ));
            return;
        }
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            show_font_error_dialog(&format!("Could not read {}:\n\n{error}", path.display()));
            return;
        }
    };
    let actual_family = jian_skia::parse_imported_font_meta(&bytes).map(|meta| meta.family);
    let store = match FontStore::user() {
        Ok(store) => store,
        Err(error) => {
            show_font_error_dialog(&format!("Could not open the font store:\n\n{error}"));
            return;
        }
    };
    match store.import(bytes) {
        Ok(_) => {
            host.refresh_imported_fonts();
            host.note_missing_font_supplied(row, actual_family.as_deref());
        }
        Err(detail) => {
            show_font_error_dialog(&format!("Could not import {}:\n\n{detail}", path.display()));
        }
    }
}

/// Pick a `.ttf` / `.otf` file and register it through `FontStore`.
/// Cancel silently returns; a read / parse failure pops an error dialog.
fn import_font_dialog(host: &mut WidgetHostNative) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Font", &["ttf", "otf"])
        .pick_file()
    else {
        return;
    };
    // Reject an oversize file by its metadata BEFORE reading it into
    // memory (the post-read `FontStore::import` cap is the backstop).
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() as usize > MAX_FONT_BYTES {
            show_font_error_dialog(&format!(
                "{} is too large ({:.1} MiB; max {} MiB).",
                path.display(),
                meta.len() as f64 / (1024.0 * 1024.0),
                MAX_FONT_BYTES / (1024 * 1024)
            ));
            return;
        }
    }
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) => {
            show_font_error_dialog(&format!("Could not read {}:\n\n{e}", path.display()));
            return;
        }
    };
    let store = match FontStore::user() {
        Ok(store) => store,
        Err(e) => {
            show_font_error_dialog(&format!("Could not open the font store:\n\n{e}"));
            return;
        }
    };
    match store.import(bytes) {
        Ok(_) => {
            // Rebuild the picker snapshot so the new family shows at once.
            host.refresh_imported_fonts();
        }
        Err(detail) => {
            show_font_error_dialog(&format!("Could not import {}:\n\n{detail}", path.display()));
        }
    }
}

/// Remove every imported face of `family` from the registry + disk.
fn remove_font(host: &mut WidgetHostNative, family: &str) {
    let store = match FontStore::user() {
        Ok(store) => store,
        Err(e) => {
            show_font_error_dialog(&format!("Could not open the font store:\n\n{e}"));
            return;
        }
    };
    store.remove(family);
    host.refresh_imported_fonts();
}

/// A minimal native error dialog for font IO. `persistence`'s dialog is
/// keyed to Open/Save/Export document errors, which don't fit here, so
/// this uses a plain rfd message box.
fn show_font_error_dialog(detail: &str) {
    rfd::MessageDialog::new()
        .set_title("Font import failed")
        .set_description(detail)
        .set_level(rfd::MessageLevel::Error)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;
    use op_editor_core::missing_fonts::{MissingFontEntry, MissingFontsPrompt};

    fn state_with_text(family: &str) -> op_editor_core::EditorState {
        let doc = serde_json::from_value(serde_json::json!({
            "version": "0.8.0",
            "children": [{
                "type": "text",
                "id": "t1",
                "name": "t",
                "x": 0,
                "y": 0,
                "width": 10,
                "height": 10,
                "content": "hi",
                "fontFamily": family
            }]
        }))
        .expect("document");
        op_editor_core::EditorState::from_document(doc)
    }

    #[test]
    fn supplied_font_with_another_family_records_a_mismatch_note() {
        let bytes = include_bytes!("../assets/fonts/InstrumentSerif-Regular.ttf");
        let actual = jian_skia::parse_imported_font_meta(bytes)
            .expect("fixture metadata")
            .family;
        let mut host = WidgetHostNative::new();
        *host.editor_state_mut() = state_with_text("Katibeh");
        host.editor_state_mut().editor_ui.system_fonts_loaded = true;
        host.editor_state_mut().editor_ui.missing_fonts_prompt = Some(MissingFontsPrompt {
            entries: vec![MissingFontEntry {
                family: "Katibeh".to_string(),
                run_count: 1,
                mismatch_note: None,
                resolved: false,
            }],
        });
        host.editor_state_mut().editor_ui.missing_fonts_modal_open = true;

        host.note_missing_font_supplied(0, Some(&actual));

        let note = host
            .editor_state()
            .editor_ui
            .missing_fonts_prompt
            .as_ref()
            .unwrap()
            .entries[0]
            .mismatch_note
            .as_deref()
            .expect("mismatch note");
        assert!(note.contains("Instrument Serif"));
        assert!(note.contains("Katibeh"));
    }
}
