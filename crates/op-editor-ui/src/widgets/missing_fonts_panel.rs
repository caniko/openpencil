//! Shared missing-font modal used by native and web hosts.

use crate::theme::Theme;
use crate::widgets::editor_state_ext::{theme_for, translate};
use crate::widgets::{LayoutBox, LayoutCx, PaintCx, Widget, WidgetId};
use crate::{Point2D, Rect, TextLayout};
use jian_widgets::centered_text_baseline_y;
use op_editor_core::missing_fonts::MissingFontsPrompt;
use op_editor_core::{EditorState, EditorUiState};

const PANEL_WIDTH: f32 = 480.0;
const BASE_HEIGHT: f32 = 140.0;
const ROW_HEIGHT: f32 = 44.0;
const ROWS_TOP: f32 = 68.0;
const HORIZONTAL_PAD: f32 = 20.0;
const BUTTON_WIDTH: f32 = 150.0;
const BUTTON_HEIGHT: f32 = 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingFontsHit {
    ChooseFile(usize),
    Dismiss,
    Inside,
    Outside,
}

pub struct MissingFontsPanel<'a> {
    id: WidgetId,
    theme: Theme,
    prompt: &'a MissingFontsPrompt,
    ui: &'a EditorUiState,
}

impl<'a> MissingFontsPanel<'a> {
    pub fn for_editor(state: &'a EditorState) -> Option<Self> {
        let prompt = state.editor_ui.missing_fonts_prompt.as_ref()?;
        if !state.editor_ui.missing_fonts_modal_open || prompt.entries.is_empty() {
            return None;
        }
        Some(Self {
            id: WidgetId::new(5460),
            theme: theme_for(&state.editor_ui),
            prompt,
            ui: &state.editor_ui,
        })
    }

    pub fn rect(&self, viewport_w: f32, viewport_h: f32) -> Rect {
        let height = BASE_HEIGHT + ROW_HEIGHT * self.prompt.entries.len() as f32;
        let x = ((viewport_w - PANEL_WIDTH) / 2.0).max(8.0);
        let y = ((viewport_h - height) / 2.0).max(crate::widgets::TOP_BAR_HEIGHT + 8.0);
        Rect {
            origin: Point2D::new(x, y),
            size: Point2D::new(PANEL_WIDTH, height),
        }
    }

    pub fn hit_test(&self, panel: Rect, point: Point2D) -> MissingFontsHit {
        if !panel.contains(point) {
            return MissingFontsHit::Outside;
        }

        for (row, entry) in self.prompt.entries.iter().enumerate() {
            if !entry.resolved && self.row_button_rect(panel, row).contains(point) {
                return MissingFontsHit::ChooseFile(row);
            }
        }

        if dismiss_rect(panel).contains(point) {
            MissingFontsHit::Dismiss
        } else {
            MissingFontsHit::Inside
        }
    }

    pub(crate) fn row_button_rect(&self, panel: Rect, row: usize) -> Rect {
        row_button_rect(panel, row)
    }
}

pub(crate) fn row_button_rect(panel: Rect, row: usize) -> Rect {
    Rect {
        origin: Point2D::new(
            panel.origin.x + panel.size.x - HORIZONTAL_PAD - BUTTON_WIDTH,
            panel.origin.y + ROWS_TOP + row as f32 * ROW_HEIGHT + 8.0,
        ),
        size: Point2D::new(BUTTON_WIDTH, BUTTON_HEIGHT),
    }
}

fn dismiss_rect(panel: Rect) -> Rect {
    Rect {
        origin: Point2D::new(
            panel.origin.x + panel.size.x - HORIZONTAL_PAD - 100.0,
            panel.origin.y + panel.size.y - 44.0,
        ),
        size: Point2D::new(100.0, BUTTON_HEIGHT),
    }
}

fn paint_text(
    cx: &mut PaintCx<'_>,
    text: &str,
    origin: Point2D,
    font_size: f32,
    weight: u16,
    color: crate::Color,
) {
    let layout = TextLayout::single_run(
        text,
        "system-ui",
        font_size,
        color.to_jian(),
        Point2D::new(0.0, 0.0),
    )
    .with_font_weight(weight);
    cx.backend.draw_text(&layout, origin);
}

impl Widget for MissingFontsPanel<'_> {
    fn id(&self) -> WidgetId {
        self.id
    }

    fn layout(&self, _cx: &LayoutCx) -> LayoutBox {
        LayoutBox {
            rect: Rect {
                origin: Point2D::new(0.0, 0.0),
                size: Point2D::new(
                    PANEL_WIDTH,
                    BASE_HEIGHT + ROW_HEIGHT * self.prompt.entries.len() as f32,
                ),
            },
        }
    }

    fn paint(&self, cx: &mut PaintCx<'_>, panel: Rect) {
        cx.backend.fill_round_rect(panel, 14.0, self.theme.card);
        cx.backend
            .stroke_round_rect(panel, 14.0, self.theme.border, 1.0);

        paint_text(
            cx,
            translate(self.ui, "missingFonts.title"),
            Point2D::new(panel.origin.x + HORIZONTAL_PAD, panel.origin.y + 28.0),
            16.0,
            600,
            self.theme.foreground,
        );
        paint_text(
            cx,
            translate(self.ui, "missingFonts.subtitle"),
            Point2D::new(panel.origin.x + HORIZONTAL_PAD, panel.origin.y + 50.0),
            11.0,
            400,
            self.theme.muted_foreground,
        );

        for (row, entry) in self.prompt.entries.iter().enumerate() {
            let row_y = panel.origin.y + ROWS_TOP + row as f32 * ROW_HEIGHT;
            if row > 0 {
                cx.backend.fill_rect(
                    Rect {
                        origin: Point2D::new(panel.origin.x + HORIZONTAL_PAD, row_y),
                        size: Point2D::new(panel.size.x - HORIZONTAL_PAD * 2.0, 1.0),
                    },
                    self.theme.border,
                );
            }
            paint_text(
                cx,
                &entry.family,
                Point2D::new(panel.origin.x + HORIZONTAL_PAD, row_y + 17.0),
                13.0,
                600,
                self.theme.foreground,
            );
            let usage = translate(self.ui, "missingFonts.usage")
                .replace("{n}", &entry.run_count.to_string());
            paint_text(
                cx,
                &usage,
                Point2D::new(panel.origin.x + HORIZONTAL_PAD, row_y + 33.0),
                11.0,
                400,
                self.theme.muted_foreground,
            );

            let action = self.row_button_rect(panel, row);
            if entry.resolved {
                let chip = Rect {
                    origin: Point2D::new(action.origin.x + action.size.x - 76.0, action.origin.y),
                    size: Point2D::new(76.0, action.size.y),
                };
                cx.backend
                    .fill_round_rect(chip, 6.0, self.theme.row_selected_primary);
                paint_text(
                    cx,
                    translate(self.ui, "missingFonts.resolved"),
                    Point2D::new(chip.origin.x + 12.0, centered_text_baseline_y(chip, 11.0)),
                    11.0,
                    500,
                    self.theme.primary,
                );
            } else {
                cx.backend.fill_round_rect(action, 6.0, self.theme.muted);
                paint_text(
                    cx,
                    translate(self.ui, "missingFonts.chooseFile"),
                    Point2D::new(
                        action.origin.x + 12.0,
                        centered_text_baseline_y(action, 11.0),
                    ),
                    11.0,
                    500,
                    self.theme.foreground,
                );
            }

            if let Some(note) = &entry.mismatch_note {
                paint_text(
                    cx,
                    note,
                    Point2D::new(panel.origin.x + HORIZONTAL_PAD, row_y + 43.0),
                    11.0,
                    400,
                    self.theme.destructive,
                );
            }
        }

        let dismiss = dismiss_rect(panel);
        paint_text(
            cx,
            translate(self.ui, "missingFonts.dismiss"),
            Point2D::new(
                dismiss.origin.x + 12.0,
                centered_text_baseline_y(dismiss, 12.0),
            ),
            12.0,
            500,
            self.theme.muted_foreground,
        );
    }

    fn access_node(&self) -> accesskit::Node {
        let mut node = accesskit::Node::new(accesskit::Role::Dialog);
        node.set_label(translate(self.ui, "missingFonts.title"));
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use op_editor_core::missing_fonts::MissingFontEntry;

    fn prompt_state(rows: usize) -> EditorState {
        let mut state = EditorState::new();
        state.editor_ui.missing_fonts_prompt = Some(MissingFontsPrompt {
            entries: (0..rows)
                .map(|row| MissingFontEntry {
                    family: format!("F{row}"),
                    run_count: 1,
                    mismatch_note: None,
                    resolved: false,
                })
                .collect(),
        });
        state.editor_ui.missing_fonts_modal_open = true;
        state
    }

    #[test]
    fn choose_file_button_hit_maps_to_row() {
        let state = prompt_state(2);
        let panel = MissingFontsPanel::for_editor(&state).expect("open");
        let rect = panel.rect(1200.0, 800.0);
        let button = panel.row_button_rect(rect, 1);
        let centre = Point2D::new(
            button.origin.x + button.size.x / 2.0,
            button.origin.y + button.size.y / 2.0,
        );

        assert_eq!(panel.hit_test(rect, centre), MissingFontsHit::ChooseFile(1));
    }

    #[test]
    fn outside_click_does_not_choose() {
        let state = prompt_state(1);
        let panel = MissingFontsPanel::for_editor(&state).expect("open");
        let rect = panel.rect(1200.0, 800.0);

        assert_eq!(
            panel.hit_test(rect, Point2D::new(1.0, 1.0)),
            MissingFontsHit::Outside
        );
    }

    #[test]
    fn resolved_row_has_no_button() {
        let mut state = prompt_state(1);
        state
            .editor_ui
            .missing_fonts_prompt
            .as_mut()
            .expect("prompt")
            .entries[0]
            .resolved = true;
        let panel = MissingFontsPanel::for_editor(&state).expect("open");
        let rect = panel.rect(1200.0, 800.0);
        let button = panel.row_button_rect(rect, 0);
        let centre = Point2D::new(
            button.origin.x + button.size.x / 2.0,
            button.origin.y + button.size.y / 2.0,
        );

        assert_eq!(panel.hit_test(rect, centre), MissingFontsHit::Inside);
    }

    #[test]
    fn dismiss_button_has_a_dedicated_hit() {
        let state = prompt_state(1);
        let panel = MissingFontsPanel::for_editor(&state).expect("open");
        let rect = panel.rect(1200.0, 800.0);
        let dismiss = dismiss_rect(rect);
        let centre = Point2D::new(
            dismiss.origin.x + dismiss.size.x / 2.0,
            dismiss.origin.y + dismiss.size.y / 2.0,
        );

        assert_eq!(panel.hit_test(rect, centre), MissingFontsHit::Dismiss);
    }

    #[test]
    fn closed_or_empty_prompt_builds_none() {
        let state = EditorState::new();
        assert!(MissingFontsPanel::for_editor(&state).is_none());

        let mut state = prompt_state(0);
        assert!(MissingFontsPanel::for_editor(&state).is_none());
        state.editor_ui.missing_fonts_prompt = Some(MissingFontsPrompt {
            entries: vec![MissingFontEntry {
                family: "Katibeh".into(),
                run_count: 1,
                mismatch_note: None,
                resolved: false,
            }],
        });
        state.editor_ui.missing_fonts_modal_open = false;
        assert!(MissingFontsPanel::for_editor(&state).is_none());
    }
}
