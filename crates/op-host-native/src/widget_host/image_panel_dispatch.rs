//! Image-node section dispatch — Search / Generate popover toggles,
//! search submit / result select, generate lifecycle, and the
//! local-asset Relink intent (TS `image-section.tsx` +
//! `image-search-popover.tsx` + `image-generate-popover.tsx`).

use super::WidgetHostNative;
use jian_ops_schema::node::PenNode;
use op_editor_core::image_panel_state::ImageGeneratePhase;
use op_editor_ui::widgets::PropertyPanel;
use op_editor_ui::Point2D;

impl WidgetHostNative {
    /// Seed for the search query / generate prompt: the node's
    /// authored `imageSearchQuery` / `imagePrompt`, else its name
    /// (TS `node.imageSearchQuery ?? node.name ?? ''`).
    fn selected_image_seed(&self, prompt: bool) -> String {
        match self.editor_state.selected_node() {
            Some(PenNode::Image(image)) => {
                let authored = if prompt {
                    image.image_prompt.as_deref()
                } else {
                    image.image_search_query.as_deref()
                };
                authored
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| image.base.name.clone())
                    .unwrap_or_default()
            }
            _ => String::new(),
        }
    }

    pub(in crate::widget_host) fn toggle_image_search_popover(&mut self) {
        let opening = !self.editor_state.editor_ui.image_panel.search_open;
        let seed = self.selected_image_seed(false);
        self.clear_image_input_selection_drag();
        if opening {
            self.blur_text_inputs_on_blank_press();
        }
        let panel = &mut self.editor_state.editor_ui.image_panel;
        // Opening either popover closes the other (TS popovers are
        // mutually exclusive portals).
        panel.close_popovers();
        if opening {
            panel.search_open = true;
            panel.search_query.set_text(seed);
            panel.search_query.touch(self.now_ms);
        }
        self.close_other_property_popovers_for_image();
    }

    pub(in crate::widget_host) fn toggle_image_generate_popover(&mut self) {
        let opening = !self.editor_state.editor_ui.image_panel.generate_open;
        let seed = self.selected_image_seed(true);
        self.clear_image_input_selection_drag();
        if opening {
            self.blur_text_inputs_on_blank_press();
        }
        let panel = &mut self.editor_state.editor_ui.image_panel;
        panel.close_popovers();
        if opening {
            // TS handleOpenChange: reset prompt + state on open.
            panel.generate_open = true;
            panel.generate_prompt.set_text(seed);
            panel.generate_prompt.touch(self.now_ms);
            panel.generate_phase = ImageGeneratePhase::Idle;
            panel.generate_preview = None;
            panel.generate_error.clear();
        }
        self.close_other_property_popovers_for_image();
    }

    /// Close the property pickers that would overlap the popovers.
    fn close_other_property_popovers_for_image(&mut self) {
        let ui = &mut self.editor_state.editor_ui;
        ui.close_fill_type_picker();
        ui.image_fill_popover_open = false;
        ui.font_weight_picker_open = false;
        ui.export_scale_picker_open = false;
        ui.export_format_picker_open = false;
        ui.property_color_variable_picker_open = None;
        self.close_font_picker();
    }

    /// Submit the search box (Enter / the icon button). No-op while a
    /// search is in flight or the query is blank (TS disables the
    /// button on both).
    pub(in crate::widget_host) fn run_image_search(&mut self) {
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.search_open || panel.search_loading || panel.search_query.text().trim().is_empty()
        {
            return;
        }
        panel.search_loading = true;
        panel.search_has_searched = true;
        panel.search_epoch = panel.search_epoch.wrapping_add(1);
    }

    /// Write the clicked result's thumbnail into the node's `src`
    /// (TS `onSelect(result.thumbUrl)`) and close the popover.
    pub(in crate::widget_host) fn select_image_search_result(&mut self, index: usize) {
        let Some(url) = self
            .editor_state
            .editor_ui
            .image_panel
            .search_results
            .get(index)
            .map(|hit| hit.thumb_data_url.as_ref().clone())
        else {
            return;
        };
        self.write_selected_image_src(&url);
        self.clear_image_input_selection_drag();
        self.editor_state.editor_ui.image_panel.close_popovers();
    }

    /// Kick off generation (drained by the desktop pump). The
    /// not-configured gate lives in the popover view; this also
    /// guards so a stale press can't start a job with no profile.
    pub(in crate::widget_host) fn run_image_generate(&mut self) {
        let configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.generate_open
            || !configured
            || panel.generate_prompt.text().trim().is_empty()
            || panel.generate_phase == ImageGeneratePhase::Loading
        {
            return;
        }
        panel.generate_phase = ImageGeneratePhase::Loading;
        panel.generate_error.clear();
        panel.generate_preview = None;
        panel.generate_epoch = panel.generate_epoch.wrapping_add(1);
    }

    pub(in crate::widget_host) fn apply_generated_image(&mut self) {
        let Some(url) = self
            .editor_state
            .editor_ui
            .image_panel
            .generate_preview
            .as_ref()
            .map(|u| u.as_ref().clone())
        else {
            return;
        };
        self.write_selected_image_src(&url);
        self.clear_image_input_selection_drag();
        self.editor_state.editor_ui.image_panel.close_popovers();
    }

    pub(in crate::widget_host) fn retry_image_generate(&mut self) {
        let panel = &mut self.editor_state.editor_ui.image_panel;
        panel.generate_phase = ImageGeneratePhase::Idle;
        panel.generate_preview = None;
        panel.generate_error.clear();
    }

    /// Open the settings modal on the Images tab (TS
    /// `setDialogOpen(true)` from the not-configured view).
    pub(in crate::widget_host) fn open_image_gen_settings(&mut self) {
        self.clear_image_input_selection_drag();
        self.editor_state.editor_ui.image_panel.close_popovers();
        self.editor_state.editor_ui.agent_settings_open = true;
        self.editor_state.editor_ui.agent_settings.tab =
            op_editor_core::agent_settings::AgentSettingsTab::Images;
    }

    /// Commit `src` onto the selected image node (with history).
    pub(in crate::widget_host) fn write_selected_image_src(&mut self, src: &str) {
        let id = self.editor_state.selection.anchor.clone();
        if !id.is_real() || src.is_empty() {
            return;
        }
        self.editor_state.commit_history();
        if let Some(PenNode::Image(image)) =
            op_editor_core::walkers::find_node_mut(self.editor_state.active_children_mut(), &id)
        {
            image.src = src.into();
        }
        self.mark_editor_state_dirty();
    }

    /// Route a printable char into whichever popover input is open.
    pub(in crate::widget_host) fn apply_image_panel_text(&mut self, c: char) -> bool {
        if c.is_control() {
            return false;
        }
        let generate_configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if panel.search_open {
            let mut text = [0u8; 4];
            panel
                .search_query
                .insert_str(c.encode_utf8(&mut text), self.now_ms);
            self.mark_dirty();
            return true;
        }
        if panel.generate_open {
            if let Some(input) = panel.active_input_mut(generate_configured) {
                let mut text = [0u8; 4];
                input.insert_str(c.encode_utf8(&mut text), self.now_ms);
                self.mark_dirty();
            }
            // Loading / preview do not paint an editor. Swallow printable
            // input so it cannot mutate either the hidden prompt or canvas.
            return true;
        }
        false
    }

    /// Backspace in the open popover's input. Swallows the key even
    /// on an empty draft so it can't fall through to node deletion.
    pub(in crate::widget_host) fn apply_image_panel_backspace(&mut self) -> bool {
        let generate_configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if panel.search_open {
            let before = panel.search_query.text().to_owned();
            panel.search_query.backspace(self.now_ms);
            if panel.search_query.text() != before {
                self.mark_dirty();
            }
            return true;
        }
        if panel.generate_open {
            if let Some(input) = panel.active_input_mut(generate_configured) {
                let before = input.text().to_owned();
                input.backspace(self.now_ms);
                if input.text() != before {
                    self.mark_dirty();
                }
            }
            return true;
        }
        false
    }

    /// Forward Delete in the visible image-popover input. The popover still
    /// consumes Delete when no glyph changes so the selected image node behind
    /// it can never be removed accidentally.
    pub(in crate::widget_host) fn apply_image_panel_delete(&mut self) -> bool {
        let generate_configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.search_open && !panel.generate_open {
            return false;
        }
        if let Some(input) = panel.active_input_mut(generate_configured) {
            let before = input.text().to_owned();
            input.delete_forward(self.now_ms);
            if input.text() != before {
                self.mark_dirty();
            }
        }
        true
    }

    /// Move the persistent image-popover caret. Consumes the arrow at text
    /// boundaries so it never falls through to canvas nudge.
    pub fn apply_image_panel_caret(&mut self, forward: bool, extend: bool) -> bool {
        let generate_configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.search_open && !panel.generate_open {
            return false;
        }
        if let Some(input) = panel.active_input_mut(generate_configured) {
            if forward {
                input.move_right(extend, self.now_ms);
            } else {
                input.move_left(extend, self.now_ms);
            }
            self.mark_dirty();
        }
        true
    }

    /// Move the visible image-popover input to its start or end. The key is
    /// still consumed when the generate view has no editable field.
    pub fn apply_image_panel_edge(&mut self, end: bool, extend: bool) -> bool {
        let configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.search_open && !panel.generate_open {
            return false;
        }
        if let Some(input) = panel.active_input_mut(configured) {
            if end {
                input.move_end(extend, self.now_ms);
            } else {
                input.move_home(extend, self.now_ms);
            }
            self.mark_dirty();
        }
        true
    }

    pub(in crate::widget_host) fn apply_image_panel_select_all(&mut self) -> bool {
        let configured = self
            .editor_state
            .editor_ui
            .agent_settings
            .image_generation_configured();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.search_open && !panel.generate_open {
            return false;
        }
        if let Some(input) = panel.active_input_mut(configured) {
            input.select_all();
            input.touch(self.now_ms);
            self.mark_dirty();
        }
        true
    }

    /// Enter while the search popover is open submits the query (TS
    /// input onKeyDown Enter → handleSearch).
    pub(in crate::widget_host) fn apply_image_panel_send(&mut self) -> bool {
        if self.editor_state.editor_ui.image_panel.search_open {
            self.run_image_search();
            self.mark_dirty();
            return true;
        }
        // Generate popover: Enter is swallowed (TS textarea would
        // insert a newline; the popup submits via the button only).
        if self.editor_state.editor_ui.image_panel.generate_open {
            return true;
        }
        false
    }

    /// A higher-z overlay or a selection-changing secondary click is taking
    /// over. Close the property image popover before that surface receives
    /// focus so keyboard, clipboard, and IME ownership cannot remain hidden
    /// underneath it.
    pub(in crate::widget_host) fn close_image_popovers_for_higher_overlay(&mut self) -> bool {
        self.clear_image_input_selection_drag();
        let panel = &mut self.editor_state.editor_ui.image_panel;
        if !panel.search_open && !panel.generate_open {
            return false;
        }
        panel.close_popovers();
        self.mark_dirty();
        true
    }

    /// Outside-click dismiss for the Search / Generate popovers.
    /// Returns `true` when the press was consumed.
    pub(in crate::widget_host) fn dismiss_image_popovers_on_press(
        &mut self,
        x: f32,
        y: f32,
        viewport_width: f32,
        viewport_height: f32,
    ) -> bool {
        use op_editor_ui::widgets::PropertyPanelAction as A;
        let panel_state = &self.editor_state.editor_ui.image_panel;
        if !panel_state.search_open && !panel_state.generate_open {
            return false;
        }
        self.refresh_layout_scene();
        if let Some(panel) = PropertyPanel::for_selection(&self.editor_state) {
            let rect = self.property_rect(viewport_width, viewport_height);
            let point = Point2D::new(x, y);
            if let Some(action) = panel.hit_test_action(rect, point) {
                if matches!(
                    action,
                    A::RunImageSearch
                        | A::SelectImageSearchResult(_)
                        | A::RunImageGenerate
                        | A::ApplyGeneratedImage
                        | A::RetryImageGenerate
                        | A::OpenImageGenSettings
                        | A::ToggleImageSearchPopover
                        | A::ToggleImageGeneratePopover
                ) {
                    self.apply_property_action(action);
                    return true;
                }
            }
            if let Some((kind, offset)) = self.image_popover_input_at(&panel, rect, point) {
                self.begin_image_input_selection_drag(kind, offset);
                return true;
            }
            if panel.image_popovers_contain(rect, point) {
                // Inside the popup body (input / textarea / empty
                // state) — swallow, keep open.
                return true;
            }
        }
        self.clear_image_input_selection_drag();
        self.editor_state.editor_ui.image_panel.close_popovers();
        self.mark_dirty();
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::widget_host::CursorHint;
    use op_editor_core::image_panel_state::{ImageGeneratePhase, ImageSearchHit};
    use op_editor_core::EditorState;
    use op_editor_ui::widgets::{PropertyPanel, PropertyPanelAction as A};
    use std::sync::Arc;

    fn host_with_image_selected() -> crate::widget_host::WidgetHostNative {
        let mut host = crate::widget_host::WidgetHostNative::new();
        let mut state = EditorState::sample();
        let _ = state.insert_image_node_at_viewport("Hero photo", "https://x/y.png");
        *host.editor_state_mut() = state;
        host
    }

    #[test]
    fn toggle_search_seeds_query_from_node_name() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageSearchPopover);
        let panel = &host.editor_state().editor_ui.image_panel;
        assert!(panel.search_open);
        assert_eq!(panel.search_query.text(), "Hero photo");
        assert!(!panel.search_has_searched);
        // Toggle again closes + clears transients.
        host.apply_property_action(A::ToggleImageSearchPopover);
        assert!(!host.editor_state().editor_ui.image_panel.search_open);
    }

    #[test]
    fn run_search_raises_epoch_and_loading() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageSearchPopover);
        let before = host.editor_state().editor_ui.image_panel.search_epoch;
        host.apply_property_action(A::RunImageSearch);
        let panel = &host.editor_state().editor_ui.image_panel;
        assert!(panel.search_loading);
        assert!(panel.search_has_searched);
        assert_eq!(panel.search_epoch, before + 1);
        // While loading, re-submit is a no-op (TS disables button).
        let epoch = panel.search_epoch;
        host.apply_property_action(A::RunImageSearch);
        assert_eq!(
            host.editor_state().editor_ui.image_panel.search_epoch,
            epoch
        );
    }

    #[test]
    fn selecting_a_result_writes_src_and_closes() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageSearchPopover);
        host.editor_state_mut()
            .editor_ui
            .image_panel
            .search_results
            .push(ImageSearchHit {
                id: "1".into(),
                thumb_data_url: Arc::new("data:image/png;base64,AA==".into()),
                attribution: String::new(),
            });
        host.apply_property_action(A::SelectImageSearchResult(0));
        let node = host.editor_state().selected_node().expect("image");
        let jian_ops_schema::node::PenNode::Image(image) = node else {
            panic!("image node expected");
        };
        assert_eq!(image.src, "data:image/png;base64,AA==");
        assert!(!host.editor_state().editor_ui.image_panel.search_open);
        // The write is undoable (commit_history ran).
        assert!(host.editor_state_mut().undo());
        let node = host.editor_state().selected_node();
        if let Some(jian_ops_schema::node::PenNode::Image(image)) = node {
            assert_eq!(image.src, "https://x/y.png");
        }
    }

    #[test]
    fn generate_gates_on_configured_profile() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageGeneratePopover);
        host.apply_property_action(A::RunImageGenerate);
        // No profile → stays idle (UI shows the not-configured view).
        assert_eq!(
            host.editor_state().editor_ui.image_panel.generate_phase,
            ImageGeneratePhase::Idle
        );
        // Configure a profile → generation kicks off.
        let id = host
            .editor_state_mut()
            .editor_ui
            .agent_settings
            .add_image_gen_profile();
        if let Some(p) = host
            .editor_state_mut()
            .editor_ui
            .agent_settings
            .image_gen_profiles
            .iter_mut()
            .find(|p| p.id == id)
        {
            p.api_key = "sk-test".into();
        }
        host.apply_property_action(A::RunImageGenerate);
        let panel = &host.editor_state().editor_ui.image_panel;
        assert_eq!(panel.generate_phase, ImageGeneratePhase::Loading);
        assert_eq!(panel.generate_epoch, 1);
    }

    #[test]
    fn open_settings_routes_to_images_tab() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageGeneratePopover);
        host.apply_property_action(A::OpenImageGenSettings);
        assert!(host.editor_state().editor_ui.agent_settings_open);
        assert_eq!(
            host.editor_state().editor_ui.agent_settings.tab,
            op_editor_core::agent_settings::AgentSettingsTab::Images
        );
        assert!(!host.editor_state().editor_ui.image_panel.generate_open);
    }

    #[test]
    fn relink_queues_the_file_action() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::RelinkImage);
        assert_eq!(
            host.editor_state().editor_ui.pending_file_action,
            Some(op_editor_core::editor_ui_state::FileAction::RelinkImage)
        );
    }

    #[test]
    fn popover_keystrokes_route_into_the_open_input() {
        let mut host = host_with_image_selected();
        assert!(!host.apply_image_panel_text('x'));
        host.apply_property_action(A::ToggleImageSearchPopover);
        host.editor_state_mut()
            .editor_ui
            .image_panel
            .search_query
            .set_text("");
        assert!(host.apply_image_panel_text('c'));
        assert!(host.apply_image_panel_text('a'));
        assert!(host.apply_image_panel_backspace());
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "c"
        );
        // Enter submits the search.
        host.editor_state_mut().ui.text_editing =
            Some(op_editor_core::NodeId::new("independently-stale-text-edit"));
        let epoch = host.editor_state().editor_ui.image_panel.search_epoch;
        assert!(host.apply_send());
        assert!(host.editor_state().editor_ui.image_panel.search_loading);
        assert_eq!(
            host.editor_state().editor_ui.image_panel.search_epoch,
            epoch + 1
        );
    }

    #[test]
    fn popover_input_supports_middle_edit_delete_and_select_all() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageSearchPopover);
        let input = &mut host.editor_state_mut().editor_ui.image_panel.search_query;
        input.set_text("abcd");
        input.set_caret(2, 0);

        assert!(host.apply_image_panel_text('你'));
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "ab你cd"
        );
        assert!(host.apply_image_panel_backspace());
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "abcd"
        );
        assert!(host.apply_image_panel_delete());
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "abd"
        );

        assert!(host.apply_select_all());
        assert!(host.apply_image_panel_text('x'));
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "x"
        );

        let input = &mut host.editor_state_mut().editor_ui.image_panel.search_query;
        input.set_text("a你b");
        input.set_caret("a你b".len(), 0);
        assert!(host.apply_image_panel_caret(false, true));
        assert!(host.apply_image_panel_caret(false, true));
        assert_eq!(host.input_copy_text().as_deref(), Some("你b"));
        assert_eq!(host.input_cut_text().as_deref(), Some("你b"));
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "a"
        );
    }

    #[test]
    fn open_popover_owns_shortcuts_and_blurs_stale_chat_focus() {
        let mut host = host_with_image_selected();
        host.editor_state_mut().chat.focused = true;
        host.apply_property_action(A::ToggleImageSearchPopover);
        assert!(!host.editor_state().chat.focused);
        assert!(host.input_active_pub());

        let selected = host.editor_state().selection.anchor.clone();
        assert!(host.apply_delete(), "Delete is consumed by the query");
        assert_eq!(host.editor_state().selection.anchor, selected);
        assert!(host.editor_state().selected_node().is_some());

        // Native menu accelerators call these host methods directly instead
        // of passing through DesktopApp's keydown guard. They must still not
        // mutate history while the query owns the keyboard.
        let snapshot = host.editor_state().snapshot_for_history();
        host.editor_state_mut().history_push_past(snapshot);
        let past_len = host.editor_state().history.past.len();
        assert!(!host.apply_undo());
        assert_eq!(host.editor_state().history.past.len(), past_len);
    }

    #[test]
    fn hidden_generate_view_does_not_expose_stale_text_selection() {
        let mut host = host_with_image_selected();
        host.editor_state_mut().chat.focused = true;
        host.editor_state_mut().chat.set_input_text("stale chat");
        host.editor_state_mut().chat.input.select_all();
        host.editor_state_mut().editor_ui.image_panel.generate_open = true;

        assert!(host.input_active_pub());
        assert!(host.input_copy_text().is_none());
        assert_eq!(host.editor_state().chat.input.text(), "stale chat");
    }

    #[test]
    fn image_popover_beats_and_clears_stale_git_focus() {
        let mut host = host_with_image_selected();
        {
            let git = &mut host.editor_state_mut().editor_ui.git_panel;
            git.open = true;
            git.commit_focused = true;
            git.commit_input.set_text("commit message");
        }
        host.apply_property_action(A::ToggleImageSearchPopover);
        assert!(!host.editor_state().editor_ui.git_panel.commit_focused);

        // Even an independently stale bit cannot split text routing from the
        // clipboard/IME resolver: the painted image overlay wins.
        host.editor_state_mut().editor_ui.git_panel.commit_focused = true;
        assert!(host.apply_text('x'));
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .text(),
            "Hero photox"
        );
        assert_eq!(
            host.editor_state().editor_ui.git_panel.commit_input.text(),
            "commit message"
        );

        host.editor_state_mut()
            .editor_ui
            .image_panel
            .search_query
            .set_text("query");
        assert!(host.apply_select_all());
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .search_query
                .highlight_range(),
            Some((0, 5))
        );
        assert!(
            host.editor_state()
                .editor_ui
                .git_panel
                .commit_input
                .highlight_range()
                .is_none(),
            "stale Git focus must not win Cmd+A"
        );
    }

    #[test]
    fn unconfigured_generate_view_swallows_keys_without_editing_hidden_prompt() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageGeneratePopover);
        let before = host
            .editor_state()
            .editor_ui
            .image_panel
            .generate_prompt
            .text()
            .to_owned();

        assert!(!host.text_input_focus_active());
        assert!(host.apply_text('x'));
        assert!(host.apply_backspace());
        assert!(host.apply_delete());
        assert_eq!(
            host.editor_state()
                .editor_ui
                .image_panel
                .generate_prompt
                .text(),
            before
        );
    }

    #[test]
    fn image_search_input_uses_text_cursor() {
        let mut host = host_with_image_selected();
        host.apply_property_action(A::ToggleImageSearchPopover);
        let viewport_w = 1200.0;
        let viewport_h = 800.0;
        let property_rect = host.property_rect(viewport_w, viewport_h);
        let panel = PropertyPanel::for_selection(host.editor_state()).expect("property panel");
        let caret = panel
            .image_popover_input_caret_rect(property_rect)
            .expect("search caret");
        assert_eq!(
            host.cursor_hint(
                caret.origin.x + 0.5,
                caret.origin.y + caret.size.y / 2.0,
                viewport_w,
                viewport_h,
            ),
            CursorHint::Text
        );
    }
}
