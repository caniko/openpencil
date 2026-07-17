//! Track M-1 host glue: enter/exit Preview mode, and the canvas ↔
//! device-frame merge animation that plays across the toggle. Split
//! out of the `widget_host.rs` spine (which was already well past the
//! repo's 800-line cap before this file existed) to keep the new
//! addition from growing that debt further.
//!
//! The merge animation's pure state machine
//! (`preview::ModeTransition`) is a sibling concern to
//! `preview::transition::ScreenTransition` (screen-to-screen, INSIDE
//! an active session) — this one bridges canvas (design) mode and
//! device-frame preview mode instead. See that module's doc for the
//! full design (why it reuses `compute_frame_geometry` instead of its
//! own transform, and why content never needs its own fade).

use super::WidgetHostNative;

impl WidgetHostNative {
    /// Whether the canvas is currently in Preview (Play) mode with a
    /// live runtime.
    pub fn preview_active(&self) -> bool {
        self.preview.is_some() && self.editor_state.editor_ui.preview_mode
    }

    /// Enter Preview (Play) mode: flip the editor flag + build a live
    /// jian runtime from the current document (which is NOT mutated).
    /// Layout is solved per-root from each root frame's own authored
    /// size (mirroring the design canvas), so `canvas_size` no longer
    /// drives the flex solve — it is retained only for API
    /// compatibility; the visible viewport affects paint transform
    /// (pan / zoom / clip), not layout. On a build failure the editor
    /// stays in design mode and the error is recorded in
    /// `preview_warnings`. Returns `true` on success.
    ///
    /// Track M-1: the screen's CURRENT canvas-space rect (before any
    /// state changes below touch it) is captured first so the merge
    /// animation has a real starting point; `initialize_device_preview`
    /// a few lines down needs the session installed to compute the
    /// destination device-frame rect, so this can't be reordered.
    pub fn enter_preview(&mut self, canvas_size: (f32, f32)) -> bool {
        // The user changed their mind mid-close: an Exit merge
        // animation was still playing (`self.preview` deliberately
        // kept alive for it — see `exit_preview`'s doc), so finish
        // that teardown synchronously right here instead of waiting
        // for `settle_mode_transition` to notice next frame. Without
        // this, `self.preview.is_some()` below would still be true
        // from the exiting session and this call would silently no-op
        // against stale state (wrong device pick, no re-infer) rather
        // than genuinely re-entering.
        if matches!(
            self.preview_mode_transition.as_ref().map(|t| t.kind()),
            Some(crate::preview::ModeTransitionKind::Exit)
        ) {
            self.finish_exit_teardown();
        }
        if self.preview.is_some() {
            return true;
        }
        match crate::preview::PreviewSession::enter(
            &self.editor_state.doc,
            canvas_size,
            &self.editor_state.ui.variables.active_theme,
            self.editor_state.ui.active_page_index,
            self.editor_state.editor_ui.preserve_authored_geometry,
        ) {
            Ok(mut session) => {
                let source_rect = session.framed_root().map(|(_, rect)| {
                    self.doc_rect_to_screen_rect(rect, canvas_size.0, canvas_size.1)
                });
                session.set_now_ms(self.now_ms);
                self.editor_state.editor_ui.enter_preview();
                self.editor_state.editor_ui.preview_warnings = session.warnings().to_vec();
                self.preview = Some(session);
                self.initialize_device_preview();
                // APP MODE: center the viewport on the entry screen (a
                // workbench-mode session has no screen rect, so this is
                // a no-op there).
                self.center_preview_entry_if_canvas(canvas_size);
                // Only a FRAMED entry (Phone/Desktop) has a device
                // silhouette to merge into; plain Canvas-mode preview
                // paints the same scene painter at the canvas's own
                // pan/zoom, so there is nothing to animate toward.
                if let Some(settled) = self.preview_device_frame.as_ref().map(|f| f.frame) {
                    self.preview_mode_transition = Some(crate::preview::ModeTransition::start(
                        crate::preview::ModeTransitionKind::Enter,
                        source_rect,
                        settled,
                        self.now_ms,
                    ));
                }
                self.mark_dirty();
                true
            }
            Err(message) => {
                // Stay in design mode; surface the failure.
                self.editor_state.editor_ui.preview_mode = false;
                self.editor_state.editor_ui.preview_warnings = vec![format!("preview: {message}")];
                self.mark_dirty();
                false
            }
        }
    }

    /// Exit Preview mode. The document is byte-identical to before
    /// entering (the runtime never touched it). Idempotent.
    ///
    /// Track M-1: when a device frame is on screen, this does NOT drop
    /// the runtime immediately — it starts the reverse merge animation
    /// (device-frame rect → the screen's canvas-space rect) and leaves
    /// `self.preview` alive so there is still a live scene to paint
    /// while it plays. `settle_mode_transition` (called once per frame
    /// from the paint entry point, mirroring the screen-switch
    /// `reconcile` call right above it) does the ACTUAL teardown once
    /// the animation finishes. Calling `exit_preview` again while the
    /// exit animation is already playing is a no-op (idempotent).
    pub fn exit_preview(&mut self) {
        let Some(settled) = self.preview_device_frame.as_ref().map(|f| f.frame) else {
            // No device frame on screen (Canvas-mode preview, or
            // preview was never entered) — nothing to merge back into,
            // so exit immediately exactly as before.
            self.finish_exit_teardown();
            self.mark_dirty();
            return;
        };
        if self.preview.is_none()
            || matches!(
                self.preview_mode_transition.as_ref().map(|t| t.kind()),
                Some(crate::preview::ModeTransitionKind::Exit)
            )
        {
            return; // already out, or already exiting
        }
        let dest_rect = self.preview.as_ref().and_then(|session| {
            session.framed_root().map(|(_, rect)| {
                self.doc_rect_to_screen_rect(rect, self.last_viewport_w, self.last_viewport_h)
            })
        });
        self.preview_mode_transition = Some(crate::preview::ModeTransition::start(
            crate::preview::ModeTransitionKind::Exit,
            dest_rect,
            settled,
            self.now_ms,
        ));
        self.mark_dirty();
    }

    /// Map a DOC-space rect through the canvas's current pan/zoom into
    /// SCREEN space — same formula `canvas_viewport.rs`'s drop-indicator
    /// painter uses (`canvas_origin + pan + doc * zoom`). Track M-1's
    /// merge animation is the only caller outside that widget.
    fn doc_rect_to_screen_rect(
        &self,
        rect: op_editor_ui::Rect,
        viewport_w: f32,
        viewport_h: f32,
    ) -> op_editor_ui::Rect {
        let (cx0, cy0, _cw, _ch) = self.canvas_region(viewport_w, viewport_h);
        let vp = &self.editor_state.viewport;
        op_editor_ui::Rect {
            origin: op_editor_ui::Point2D::new(
                cx0 + vp.pan_x + rect.origin.x * vp.zoom,
                cy0 + vp.pan_y + rect.origin.y * vp.zoom,
            ),
            size: op_editor_ui::Point2D::new(rect.size.x * vp.zoom, rect.size.y * vp.zoom),
        }
    }

    /// Once a Track M-1 merge animation finishes, finalize whichever
    /// direction it was playing: Enter just clears the (by-then
    /// no-op) transition marker; Exit performs the teardown that was
    /// deferred at `exit_preview` time. Called once per frame from the
    /// paint entry point, right alongside the screen-switch
    /// `reconcile` call. A no-op while a transition is still active,
    /// or when none is playing.
    pub(crate) fn settle_mode_transition(&mut self) {
        let Some(transition) = self.preview_mode_transition.as_ref() else {
            return;
        };
        if transition.is_active(self.now_ms) {
            return;
        }
        if transition.kind() == crate::preview::ModeTransitionKind::Exit {
            self.finish_exit_teardown();
        } else {
            self.preview_mode_transition = None;
        }
    }

    /// The actual Preview-mode teardown deferred by `exit_preview` —
    /// shared by `settle_mode_transition` (the animation finished on
    /// its own) and `enter_preview` (the user re-opened before it did).
    fn finish_exit_teardown(&mut self) {
        self.preview = None;
        self.clear_device_preview_state();
        self.preview_press_active = false;
        self.preview_last_doc = None;
        self.editor_state.editor_ui.exit_preview();
        self.preview_mode_transition = None;
    }

    /// Whether input should be discarded because a Track M-1 merge
    /// animation is playing — mirrors `PreviewSession::transition_active`'s
    /// "discard, don't queue" rationale: the canvas/device-frame rect is
    /// physically moving, so a tap has no stable target to land on.
    pub(crate) fn mode_transition_active(&self) -> bool {
        self.preview_mode_transition
            .as_ref()
            .is_some_and(|t| t.is_active(self.now_ms))
    }
}
