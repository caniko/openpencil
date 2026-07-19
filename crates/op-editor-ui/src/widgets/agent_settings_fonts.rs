//! Fonts tab of the settings modal.

use crate::theme::Theme;
use crate::widgets::editor_state_ext::translate;
use crate::widgets::missing_fonts_panel::{
    paint_missing_font_row, paint_text, row_button_rect, ROW_HEIGHT,
};
use crate::widgets::PaintCx;
use crate::{Point2D, Rect};
use jian_widgets::centered_text_baseline_y;
use op_editor_core::editor_ui_state::EditorUiState;
use op_editor_core::missing_fonts::MissingFontEntry;

const TOP_PAD: f32 = 12.0;
const SECTION_TITLE_HEIGHT: f32 = 36.0;
const EMPTY_BODY_HEIGHT: f32 = 44.0;
const SECTION_GAP: f32 = 28.0;
const BOTTOM_PAD: f32 = 24.0;
const REMOVE_HEIGHT: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontsHit {
    ChooseFile(usize),
    RemoveImportedFont(usize),
    None,
}

fn missing_entries(ui: &EditorUiState) -> &[MissingFontEntry] {
    ui.missing_fonts_prompt
        .as_ref()
        .map(|prompt| prompt.entries.as_slice())
        .unwrap_or_default()
}

fn missing_body_height(ui: &EditorUiState) -> f32 {
    let rows = missing_entries(ui).len();
    if rows == 0 {
        EMPTY_BODY_HEIGHT
    } else {
        rows as f32 * ROW_HEIGHT
    }
}

fn imported_section_top(content: Rect, ui: &EditorUiState) -> f32 {
    content.origin.y + TOP_PAD + SECTION_TITLE_HEIGHT + missing_body_height(ui) + SECTION_GAP
}

pub(crate) fn missing_row_rect(content: Rect, row: usize) -> Rect {
    Rect::xywh(
        content.origin.x,
        content.origin.y + TOP_PAD + SECTION_TITLE_HEIGHT + row as f32 * ROW_HEIGHT,
        content.size.x,
        ROW_HEIGHT,
    )
}

fn imported_row_rect(content: Rect, ui: &EditorUiState, row: usize) -> Rect {
    Rect::xywh(
        content.origin.x,
        imported_section_top(content, ui) + SECTION_TITLE_HEIGHT + row as f32 * ROW_HEIGHT,
        content.size.x,
        ROW_HEIGHT,
    )
}

pub(crate) fn imported_remove_rect(content: Rect, ui: &EditorUiState, row: usize) -> Rect {
    let row = imported_row_rect(content, ui, row);
    let width = super::missing_fonts_panel::fit_button_width(translate(ui, "common.delete"), 11.0);
    Rect::xywh(
        row.origin.x + row.size.x - width,
        row.origin.y + (ROW_HEIGHT - REMOVE_HEIGHT) / 2.0,
        width,
        REMOVE_HEIGHT,
    )
}

pub(super) fn content_height(ui: &EditorUiState) -> f32 {
    TOP_PAD
        + SECTION_TITLE_HEIGHT
        + missing_body_height(ui)
        + SECTION_GAP
        + SECTION_TITLE_HEIGHT
        + ui.imported_font_families.len() as f32 * ROW_HEIGHT
        + BOTTOM_PAD
}

pub fn hit_test(content: Rect, ui: &EditorUiState, scrolled: Point2D) -> FontsHit {
    for (row, entry) in missing_entries(ui).iter().enumerate() {
        if !entry.resolved && row_button_rect(missing_row_rect(content, row), ui).contains(scrolled)
        {
            return FontsHit::ChooseFile(row);
        }
    }
    for row in 0..ui.imported_font_families.len() {
        if imported_remove_rect(content, ui, row).contains(scrolled) {
            return FontsHit::RemoveImportedFont(row);
        }
    }
    FontsHit::None
}

pub(super) fn paint_fonts_tab(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    ui: &EditorUiState,
    content: Rect,
) {
    paint_section_title(
        cx,
        theme,
        translate(ui, "missingFonts.title"),
        content.origin.x,
        content.origin.y + TOP_PAD,
    );

    let entries = missing_entries(ui);
    if entries.is_empty() {
        paint_text(
            cx,
            translate(ui, "missingFonts.noneMissing"),
            Point2D::new(
                content.origin.x,
                content.origin.y + TOP_PAD + SECTION_TITLE_HEIGHT + 20.0,
            ),
            12.0,
            400,
            theme.muted_foreground,
        );
    } else {
        for (row, entry) in entries.iter().enumerate() {
            paint_missing_font_row(
                cx,
                theme,
                ui,
                entry,
                row,
                missing_row_rect(content, row),
                row > 0,
            );
        }
    }

    let imported_top = imported_section_top(content, ui);
    paint_section_title(
        cx,
        theme,
        translate(ui, "missingFonts.importedSection"),
        content.origin.x,
        imported_top,
    );
    for (row, family) in ui.imported_font_families.iter().enumerate() {
        let row_rect = imported_row_rect(content, ui, row);
        if row > 0 {
            cx.backend.fill_rect(
                Rect::xywh(row_rect.origin.x, row_rect.origin.y, row_rect.size.x, 1.0),
                theme.border,
            );
        }
        paint_text(
            cx,
            family,
            Point2D::new(row_rect.origin.x, row_rect.origin.y + 27.0),
            13.0,
            500,
            theme.foreground,
        );
        let remove = imported_remove_rect(content, ui, row);
        let remove_hovered = ui.missing_fonts_hover
            == Some(op_editor_core::missing_fonts::MissingFontsHover::RemoveImported(row));
        let remove_bg = if remove_hovered {
            theme.border
        } else {
            theme.muted
        };
        cx.backend.fill_round_rect(remove, 6.0, remove_bg);
        paint_text(
            cx,
            translate(ui, "common.delete"),
            Point2D::new(
                remove.origin.x + 12.0,
                centered_text_baseline_y(remove, 11.0),
            ),
            11.0,
            500,
            theme.destructive,
        );
    }
}

fn paint_section_title(cx: &mut PaintCx<'_>, theme: &Theme, title: &str, x: f32, y: f32) {
    paint_text(
        cx,
        title,
        Point2D::new(x, y + 20.0),
        15.0,
        500,
        theme.foreground,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use crate::widgets::PaintCx;
    use crate::{Color, Point2D, Rect, RenderBackend, TextLayout};
    use op_editor_core::missing_fonts::{MissingFontEntry, MissingFontsPrompt};
    use op_editor_core::EditorState;
    use std::sync::Arc;

    #[derive(Default)]
    struct CaptureBackend {
        text: Vec<String>,
    }

    impl RenderBackend for CaptureBackend {
        fn begin_frame(&mut self) {}
        fn end_frame(&mut self) {}
        fn fill_rect(&mut self, _: Rect, _: Color) {}
        fn stroke_rect(&mut self, _: Rect, _: Color, _: f32) {}
        fn draw_text(&mut self, layout: &TextLayout, _: Point2D) {
            self.text
                .extend(layout.runs().iter().map(|run| run.content.clone()));
        }
        fn clip_rect(&mut self, _: Rect) {}
        fn stroke_line(&mut self, _: Point2D, _: Point2D, _: Color, _: f32) {}
        fn fill_round_rect(&mut self, _: Rect, _: f32, _: Color) {}
        fn stroke_round_rect(&mut self, _: Rect, _: f32, _: Color, _: f32) {}
        fn stroke_svg_path(&mut self, _: &str, _: Point2D, _: f32, _: Color, _: f32) {}
        fn save(&mut self) {}
        fn restore(&mut self) {}
        fn translate(&mut self, _: Point2D) {}
        fn resize(&mut self, _: u32, _: u32) {}
        fn dpi_scale(&self) -> f32 {
            1.0
        }
    }

    fn content_rect() -> Rect {
        Rect::xywh(20.0, 20.0, 460.0, 640.0)
    }

    fn state_with_missing(families: &[&str]) -> EditorState {
        let mut state = EditorState::new();
        state.editor_ui.missing_fonts_prompt = Some(MissingFontsPrompt {
            entries: families
                .iter()
                .map(|family| MissingFontEntry {
                    family: (*family).to_owned(),
                    run_count: 2,
                    mismatch_note: None,
                    resolved: false,
                })
                .collect(),
        });
        state
    }

    fn painted_text(state: &EditorState) -> Vec<String> {
        let mut backend = CaptureBackend::default();
        let mut cx = PaintCx {
            backend: &mut backend,
        };
        paint_fonts_tab(&mut cx, &Theme::dark(), &state.editor_ui, content_rect());
        backend.text
    }

    #[test]
    fn tab_renders_missing_rows_from_prompt_data() {
        let state = state_with_missing(&["Katibeh", "Inter Tight"]);
        let text = painted_text(&state);

        assert!(text.iter().any(|value| value == "Katibeh"));
        assert!(text.iter().any(|value| value == "Inter Tight"));
    }

    #[test]
    fn tab_renders_none_missing_copy_when_prompt_is_empty() {
        let state = EditorState::new();
        let text = painted_text(&state);

        assert!(text
            .iter()
            .any(|value| value == translate(&state.editor_ui, "missingFonts.noneMissing")));
    }

    #[test]
    fn choose_file_hit_maps_to_missing_row() {
        let state = state_with_missing(&["Katibeh"]);
        let row = missing_row_rect(content_rect(), 0);
        let button = row_button_rect(row, &state.editor_ui);
        let point = Point2D::new(
            button.origin.x + button.size.x / 2.0,
            button.origin.y + button.size.y / 2.0,
        );

        assert_eq!(
            hit_test(content_rect(), &state.editor_ui, point),
            FontsHit::ChooseFile(0)
        );
    }

    #[test]
    fn imported_rows_expose_remove_hits() {
        let mut state = EditorState::new();
        state.editor_ui.imported_font_families = Arc::new(vec!["Katibeh".into()]);
        let remove = imported_remove_rect(content_rect(), &state.editor_ui, 0);
        let point = Point2D::new(
            remove.origin.x + remove.size.x / 2.0,
            remove.origin.y + remove.size.y / 2.0,
        );

        assert_eq!(
            hit_test(content_rect(), &state.editor_ui, point),
            FontsHit::RemoveImportedFont(0)
        );
    }
}
