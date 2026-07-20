//! IME and paste text routing for the web widget host.

use super::WidgetHost;
use op_editor_ui::widgets::{AIChatPlaceholder, PropertyPanel, TOP_BAR_HEIGHT};
use op_editor_ui::{Point2D, Rect};

impl WidgetHost {
    /// Whether the browser's hidden IME capture input should own DOM focus.
    /// A generate popup without a configured provider has no visible editor,
    /// even though the popup still swallows canvas shortcuts.
    pub(crate) fn text_input_focus_active(&self) -> bool {
        let panel = &self.editor_state.editor_ui.image_panel;
        if panel.search_open || panel.generate_open {
            let configured = self
                .editor_state
                .editor_ui
                .agent_settings
                .image_generation_configured();
            return panel.active_input(configured).is_some();
        }
        self.input_active()
    }

    /// Focused caret rectangle in logical canvas coordinates. Search/Generate
    /// and chat expose exact geometry; older fields fall back to the last
    /// pointer position so browser candidates never anchor at viewport (0, 0).
    pub(crate) fn ime_anchor_rect(&self) -> Option<Rect> {
        let configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let image_popover_open = self.editor_state.editor_ui.image_panel.search_open
            || self.editor_state.editor_ui.image_panel.generate_open;
        if image_popover_open {
            self.editor_state
                .editor_ui
                .image_panel
                .active_input(configured)?;
            if let Some(rect) = self.cached_image_input_caret_rect() {
                return Some(rect);
            }
            let panel = PropertyPanel::for_selection(&self.editor_state)?;
            let property_rect = Rect {
                origin: Point2D::new(
                    self.last_viewport_w - self.editor_state.editor_ui.property_panel_width,
                    TOP_BAR_HEIGHT,
                ),
                size: Point2D::new(
                    self.editor_state.editor_ui.property_panel_width,
                    (self.last_viewport_h - TOP_BAR_HEIGHT).max(0.0),
                ),
            };
            return panel.image_popover_input_caret_rect(property_rect);
        }
        if self.editor_state.chat.focused && !self.non_chat_input_owns_keyboard() {
            let chat_rect = self.ai_chat_rect(self.last_viewport_w, self.last_viewport_h)?;
            let chat = AIChatPlaceholder::from_editor_at(&self.editor_state, self.now_ms);
            return Some(chat.input_caret_rect(chat_rect));
        }
        self.text_input_focus_active()
            .then(|| Rect::xywh(self.last_cursor_x, self.last_cursor_y, 1.5, 18.0))
    }

    /// IME composition forwarding. Only the final COMMIT lands in the
    /// focused input — preedit text is not painted (matches the native
    /// host, which routes winit `Ime::Commit` through `apply_text`
    /// char-by-char in `app_handler.rs` and ignores `Ime::Preedit`).
    /// Routing therefore covers every `apply_text` focus branch.
    /// Wired in `canvaskit.rs` to the hidden IME input's `compositionend`
    /// (the `canvaskit` build); unused under the `web` compile stub.
    #[cfg_attr(not(feature = "canvaskit"), allow(dead_code))]
    pub fn apply_ime(&mut self, event: &op_editor_ui::ImeEvent) -> bool {
        if !matches!(event.kind, op_editor_ui::ImeKind::CompositionEnd) {
            return false;
        }
        self.apply_paste_text(&event.text)
    }

    /// Route a multi-character text payload (IME commit, clipboard paste)
    /// into whichever input owns the keyboard, char-by-char through
    /// `apply_text` so every focus branch + per-field filter applies.
    pub fn apply_paste_text(&mut self, text: &str) -> bool {
        let mut consumed = false;
        for c in text.chars() {
            if !c.is_control() && self.apply_text(c) {
                consumed = true;
            }
        }
        consumed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_with_image() -> WidgetHost {
        let mut host = WidgetHost::new();
        let _ = host
            .editor_state
            .insert_image_node_at_viewport("Hero photo", "https://x/y.png");
        host.last_viewport_w = 1200.0;
        host.last_viewport_h = 800.0;
        host
    }

    #[test]
    fn image_search_web_ime_anchor_tracks_the_persistent_caret() {
        let mut host = host_with_image();
        host.editor_state.editor_ui.image_panel.search_open = true;
        let input = &mut host.editor_state.editor_ui.image_panel.search_query;
        input.set_text("abcd");
        input.set_caret(0, 0);
        let start = host.ime_anchor_rect().expect("search anchor");

        host.editor_state
            .editor_ui
            .image_panel
            .search_query
            .set_caret(3, 0);
        let after_three = host.ime_anchor_rect().expect("search anchor");
        assert!(after_three.origin.x > start.origin.x + 5.0);
        assert_eq!(after_three.origin.y, start.origin.y);
        assert!(host.text_input_focus_active());
    }

    #[test]
    fn unconfigured_generate_view_does_not_focus_browser_ime_or_edit_prompt() {
        let mut host = host_with_image();
        let panel = &mut host.editor_state.editor_ui.image_panel;
        panel.generate_open = true;
        panel.generate_prompt.set_text("Hero photo");

        assert!(!host.text_input_focus_active());
        assert!(host.ime_anchor_rect().is_none());
        assert!(host.apply_text('x'));
        assert_eq!(
            host.editor_state
                .editor_ui
                .image_panel
                .generate_prompt
                .text(),
            "Hero photo"
        );
    }
}
