//! Track C-3: screen-transition animations for APP MODE screen switches.
//!
//! `push` = the entering screen slides in from the right (240ms
//! ease-out); `pop` = the screen being left slides out to the right,
//! revealing the destination underneath (same 240ms ease-out, mirrored —
//! the plan doc names only push's easing explicitly, so pop reuses it for
//! visual symmetry); `replace` (tab switch / pill click) = a 160ms linear
//! cross-fade, both screens held in place. See the design doc's Track C-3.
//!
//! [`super::app_mode`]'s `reconcile` constructs a [`ScreenTransition`] on
//! every screen switch, snapshotting the OUTGOING scene page before it
//! overwrites `PreviewSession::scene` with the entering one.
//! [`PreviewSession::paint_framed_animated`] is the device frame's paint
//! entry point (`op-host-native::widget_host::preview_frame`) — it routes
//! straight to the steady-state [`PreviewSession::paint_framed`] once the
//! animation finishes, or composites both layers while it plays.
//!
//! "Skippable" (a nav firing again mid-animation): `reconcile` always
//! REPLACES `self.transition` outright rather than queuing — the very
//! next reconcile pass sees whatever `self.scene` had just settled to as
//! its fresh outgoing snapshot, so no half-finished slide lingers and no
//! second transition ever waits in line.

use super::PreviewSession;
use op_editor_ui::layout_scene::{LayoutScene, SceneGradient, SceneNode, ScenePage};
use op_editor_ui::widgets::{paint_scene_page_with, PaintCx, PaintSceneOptions};
use op_editor_ui::{Point2D, Rect, RenderBackend};

pub(in crate::preview) const PUSH_POP_DURATION_MS: u64 = 240;
pub(in crate::preview) const REPLACE_DURATION_MS: u64 = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::preview) enum TransitionKind {
    Push,
    Pop,
    Replace,
}

/// Classify a screen switch from the route-stack depth recorded at the
/// LAST reconcile (`AppMode::mounted_stack`) versus the depth just after
/// this switch. The router's live stack has already advanced by the time
/// `reconcile` runs (the tap mutated it synchronously), so the verb
/// itself (`push`/`replace`/`pop`) isn't available here — but depth is
/// the exact, sufficient signature of all three: deeper is only ever a
/// push, shallower only ever a pop, same depth only ever a replace/reset.
pub(in crate::preview) fn classify_transition(prev_len: usize, new_len: usize) -> TransitionKind {
    match new_len.cmp(&prev_len) {
        std::cmp::Ordering::Greater => TransitionKind::Push,
        std::cmp::Ordering::Less => TransitionKind::Pop,
        std::cmp::Ordering::Equal => TransitionKind::Replace,
    }
}

/// An in-flight screen-transition animation.
pub(in crate::preview) struct ScreenTransition {
    kind: TransitionKind,
    started_at_ms: u64,
    duration_ms: u64,
    /// The scene page as it looked immediately before this switch — the
    /// layer a push/pop slides or a replace fades out.
    outgoing: ScenePage,
}

impl ScreenTransition {
    pub(in crate::preview) fn start(
        kind: TransitionKind,
        outgoing: ScenePage,
        now_ms: u64,
    ) -> Self {
        let duration_ms = match kind {
            TransitionKind::Replace => REPLACE_DURATION_MS,
            TransitionKind::Push | TransitionKind::Pop => PUSH_POP_DURATION_MS,
        };
        Self {
            kind,
            started_at_ms: now_ms,
            duration_ms,
            outgoing,
        }
    }

    pub(in crate::preview) fn is_active(&self, now_ms: u64) -> bool {
        now_ms < self.started_at_ms.saturating_add(self.duration_ms)
    }

    /// Next wake time for the host's animation loop — same shape as
    /// `op_editor_ui::widgets::CanvasLayoutTransition::next_deadline_ms`,
    /// though the host doesn't actually need this: `next_animation_deadline_ms`
    /// already ticks ~30fps for the whole `self.preview.is_some()` window.
    /// Kept for parity / a future host that wants a tighter wake schedule.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::preview) fn next_deadline_ms(&self, now_ms: u64) -> Option<u64> {
        if !self.is_active(now_ms) {
            return None;
        }
        Some((now_ms.saturating_add(16)).min(self.started_at_ms + self.duration_ms))
    }

    fn linear_t(&self, now_ms: u64) -> f32 {
        let elapsed = now_ms.saturating_sub(self.started_at_ms);
        (elapsed as f32 / self.duration_ms.max(1) as f32).clamp(0.0, 1.0)
    }

    /// Ease-out-cubic progress for the push/pop slide — the same formula
    /// `op_editor_ui::widgets::CanvasLayoutTransition` uses, copied rather
    /// than imported: that helper's `translate_scene_subtree` diffs ONE
    /// scene's nodes by id across a layout change (a per-node problem);
    /// sliding two independent whole pages by a viewport-origin offset
    /// (see `paint_framed_animated`) only needs this one-line formula, not
    /// the subtree walker, so there's nothing else worth reusing across
    /// the crate boundary.
    fn eased_t(&self, now_ms: u64) -> f32 {
        let t = self.linear_t(now_ms);
        1.0 - (1.0 - t).powi(3)
    }
}

/// Scale every alpha-bearing channel in this subtree by `factor` (the
/// Replace cross-fade). Multiplies `opacity` (the one channel the
/// painter reads directly, for raster images) plus the alpha ALREADY
/// baked into `fill` / `stroke` / each gradient variant's own `opacity`
/// at scene-build time (see `SceneNode::opacity`'s doc).
///
/// Known gap: drop-shadow (`effects`) and SkSL shader fills keep full
/// alpha through the 160ms window. Accepted: there is no render-backend
/// primitive for true offscreen alpha compositing (the trait has no
/// save-layer/blend op), so a complete fade would mean extending that
/// trait — out of scope here — and shadows are rare on the nav/tab
/// chrome that is actually visible during a tab-switch cross-fade.
fn fade_scene_node(node: &mut SceneNode, factor: f32) {
    node.opacity *= factor;
    node.fill = node.fill.map(|c| c.with_alpha(c.a * factor));
    if let Some(stroke) = node.stroke.as_mut() {
        stroke.color = stroke.color.with_alpha(stroke.color.a * factor);
    }
    if let Some(gradient) = node.gradient.as_mut() {
        match gradient {
            SceneGradient::Linear { opacity, .. }
            | SceneGradient::Radial { opacity, .. }
            | SceneGradient::Mesh { opacity, .. } => *opacity *= factor,
        }
    }
    for child in &mut node.children {
        fade_scene_node(child, factor);
    }
}

/// Paint one root, optionally faded, inside `content_clip`. Mirrors the
/// clip/cull/paint sequence `PreviewSession::paint_framed` uses for its
/// single (steady-state) layer.
#[allow(clippy::too_many_arguments)]
fn paint_content_layer(
    backend: &mut dyn RenderBackend,
    page: &ScenePage,
    root_id: &str,
    origin: Point2D,
    fit: f32,
    content_clip: Rect,
    cull: Rect,
    fade: Option<f32>,
) {
    let faded_page;
    let page_ref: &ScenePage = match fade {
        Some(factor) => {
            let mut clone = page.clone();
            if let Some(root) = clone.children.iter_mut().find(|c| c.id == root_id) {
                fade_scene_node(root, factor);
            }
            faded_page = clone;
            &faded_page
        }
        None => page,
    };
    backend.save();
    backend.clip_rect(content_clip);
    {
        let mut cx = PaintCx { backend };
        paint_scene_page_with(
            &mut cx,
            page_ref,
            origin,
            fit,
            cull,
            PaintSceneOptions {
                only_root: Some(root_id),
                skip_node: None,
            },
        );
    }
    backend.restore();
}

impl PreviewSession {
    /// Device-frame paint entry point (`widget_host::preview_frame`
    /// calls this in place of [`PreviewSession::paint_framed`]): routes
    /// straight through when no transition is playing, or composites the
    /// outgoing + entering screens for the in-flight animation.
    ///
    /// Deliberately simplified versus the steady-state `paint_framed` for
    /// the animation's short window: no pinned-nav strip and no focus
    /// caret paint while `is_active` — both resume the very next frame
    /// the animation ends. Both layers otherwise go through the exact
    /// same painter (`paint_scene_page_with`) `paint_framed` itself calls.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_framed_animated(
        &self,
        backend: &mut dyn RenderBackend,
        only_root: &str,
        content_clip: Rect,
        content_origin: Point2D,
        fit: f32,
        pinned: Option<&super::present::PinnedPaint>,
        now_ms: u64,
    ) {
        let Some(transition) = self.transition.as_ref().filter(|t| t.is_active(now_ms)) else {
            self.paint_framed(
                backend,
                only_root,
                content_clip,
                content_origin,
                fit,
                pinned,
                now_ms,
            );
            return;
        };
        let Some(outgoing_root) = transition.outgoing.children.first() else {
            self.paint_framed(
                backend,
                only_root,
                content_clip,
                content_origin,
                fit,
                pinned,
                now_ms,
            );
            return;
        };
        let outgoing_id = outgoing_root.id.clone();

        let overlaid_entering;
        let entering_scene: &LayoutScene = if self.runtime.widget_states.iter().next().is_none()
            && self.binding_sites.is_empty()
        {
            &self.scene
        } else {
            overlaid_entering = self.overlay_runtime_state(&self.scene);
            &overlaid_entering
        };
        let Some(entering_page) = entering_scene.active_page() else {
            return;
        };

        const CULL_MARGIN: f32 = 64.0;
        let cull = Rect {
            origin: Point2D::new(
                content_clip.origin.x - CULL_MARGIN,
                content_clip.origin.y - CULL_MARGIN,
            ),
            size: Point2D::new(
                content_clip.size.x + CULL_MARGIN * 2.0,
                content_clip.size.y + CULL_MARGIN * 2.0,
            ),
        };

        match transition.kind {
            TransitionKind::Push => {
                // Bottom: outgoing, static (already correctly placed).
                // Top: entering, sliding in from the right toward 0.
                paint_content_layer(
                    backend,
                    &transition.outgoing,
                    &outgoing_id,
                    content_origin,
                    fit,
                    content_clip,
                    cull,
                    None,
                );
                let dx = content_clip.size.x * (1.0 - transition.eased_t(now_ms));
                let entering_origin = Point2D::new(content_origin.x + dx, content_origin.y);
                paint_content_layer(
                    backend,
                    entering_page,
                    only_root,
                    entering_origin,
                    fit,
                    content_clip,
                    cull,
                    None,
                );
            }
            TransitionKind::Pop => {
                // Bottom: entering (the destination, already revealed).
                // Top: outgoing, sliding right off-screen to uncover it.
                paint_content_layer(
                    backend,
                    entering_page,
                    only_root,
                    content_origin,
                    fit,
                    content_clip,
                    cull,
                    None,
                );
                let dx = content_clip.size.x * transition.eased_t(now_ms);
                let outgoing_origin = Point2D::new(content_origin.x + dx, content_origin.y);
                paint_content_layer(
                    backend,
                    &transition.outgoing,
                    &outgoing_id,
                    outgoing_origin,
                    fit,
                    content_clip,
                    cull,
                    None,
                );
            }
            TransitionKind::Replace => {
                let t = transition.linear_t(now_ms);
                paint_content_layer(
                    backend,
                    &transition.outgoing,
                    &outgoing_id,
                    content_origin,
                    fit,
                    content_clip,
                    cull,
                    Some(1.0 - t),
                );
                paint_content_layer(
                    backend,
                    entering_page,
                    only_root,
                    content_origin,
                    fit,
                    content_clip,
                    cull,
                    Some(t),
                );
            }
        }
    }

    /// Whether a Track C-3 transition is currently playing, using the
    /// clock the host last pushed via `set_now_ms`. `input.rs`'s pointer
    /// dispatch gates on this — DISCARDING taps/drags/wheel while a
    /// screen-transition animation plays, rather than queuing them, is
    /// the "simple and doesn't break anything" choice: a tap mid-slide
    /// has no stable target to land on anyway (the content is physically
    /// moving), and the window is short (160-240ms) — a queued tap would
    /// risk firing against whichever screen happens to be mounted once
    /// the animation ends, which is not necessarily what the user was
    /// aiming at when they tapped.
    pub(in crate::preview) fn transition_active(&self) -> bool {
        self.transition
            .as_ref()
            .is_some_and(|t| t.is_active(self.last_now_ms))
    }

    /// Test-only: whether a transition is currently playing at `now_ms`.
    #[cfg(all(test, not(target_os = "windows")))]
    pub(in crate::preview) fn transition_active_for_test(&self, now_ms: u64) -> bool {
        self.transition
            .as_ref()
            .is_some_and(|t| t.is_active(now_ms))
    }

    /// Test-only: the kind of the current (possibly finished) transition.
    #[cfg(all(test, not(target_os = "windows")))]
    pub(in crate::preview) fn transition_kind_for_test(&self) -> Option<TransitionKind> {
        self.transition.as_ref().map(|t| t.kind)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(root_id: &str) -> ScenePage {
        use op_editor_ui::layout_scene::NodeKind;
        let mut root = SceneNode::leaf(root_id, NodeKind::Frame);
        root.bounds = Rect::xywh(0.0, 0.0, 390.0, 844.0);
        ScenePage {
            id: "p".into(),
            name: "P".into(),
            children: vec![root],
        }
    }

    #[test]
    fn classify_by_stack_depth() {
        assert_eq!(classify_transition(1, 2), TransitionKind::Push);
        assert_eq!(classify_transition(2, 1), TransitionKind::Pop);
        assert_eq!(classify_transition(1, 1), TransitionKind::Replace);
        assert_eq!(classify_transition(3, 5), TransitionKind::Push);
    }

    #[test]
    fn push_pop_use_240ms_replace_uses_160ms() {
        let push = ScreenTransition::start(TransitionKind::Push, page("a"), 0);
        assert!(push.is_active(239));
        assert!(!push.is_active(240));

        let pop = ScreenTransition::start(TransitionKind::Pop, page("a"), 0);
        assert!(pop.is_active(239));
        assert!(!pop.is_active(240));

        let replace = ScreenTransition::start(TransitionKind::Replace, page("a"), 0);
        assert!(replace.is_active(159));
        assert!(!replace.is_active(160));
    }

    #[test]
    fn eased_t_starts_at_zero_ends_at_one() {
        let t = ScreenTransition::start(TransitionKind::Push, page("a"), 1_000);
        assert_eq!(t.eased_t(1_000), 0.0);
        assert!((t.eased_t(1_240) - 1.0).abs() < 1e-6);
        // Monotonic ease-out: past the midpoint, more than half done.
        assert!(t.eased_t(1_120) > 0.5);
    }

    #[test]
    fn next_deadline_ms_ticks_until_done() {
        let t = ScreenTransition::start(TransitionKind::Replace, page("a"), 1_000);
        assert_eq!(t.next_deadline_ms(1_000), Some(1_016));
        // Never overshoots the animation's own end.
        assert_eq!(t.next_deadline_ms(1_150), Some(1_160));
        assert_eq!(
            t.next_deadline_ms(1_160),
            None,
            "finished — no more wakeups"
        );
    }

    #[test]
    fn linear_t_is_unclamped_progress_for_replace() {
        let t = ScreenTransition::start(TransitionKind::Replace, page("a"), 1_000);
        assert_eq!(t.linear_t(1_000), 0.0);
        assert!((t.linear_t(1_080) - 0.5).abs() < 1e-6);
        assert_eq!(t.linear_t(1_160), 1.0);
        assert_eq!(t.linear_t(5_000), 1.0, "clamped past the end");
    }

    #[test]
    fn fade_scales_fill_stroke_and_gradient_alpha() {
        use op_editor_ui::layout_scene::{SceneGradientStop, SceneStroke};
        use op_editor_ui::Color;
        let mut node = SceneNode::leaf("n", op_editor_ui::layout_scene::NodeKind::Rect);
        node.opacity = 1.0;
        node.fill = Some(Color {
            r: 1.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        });
        node.stroke = Some(SceneStroke {
            color: Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.8,
            },
            width: 1.0,
            sides: None,
            align: Default::default(),
        });
        node.gradient = Some(SceneGradient::Linear {
            angle_deg: 0.0,
            opacity: 1.0,
            stops: vec![SceneGradientStop {
                offset: 0.0,
                color: Color::WHITE,
            }],
        });
        fade_scene_node(&mut node, 0.5);
        assert!((node.opacity - 0.5).abs() < 1e-6);
        assert!((node.fill.unwrap().a - 0.5).abs() < 1e-6);
        assert!((node.stroke.unwrap().color.a - 0.4).abs() < 1e-6);
        match node.gradient.unwrap() {
            SceneGradient::Linear { opacity, .. } => assert!((opacity - 0.5).abs() < 1e-6),
            _ => panic!("expected linear gradient"),
        }
    }

    #[test]
    fn fade_recurses_into_children() {
        use op_editor_ui::Color;
        let mut child = SceneNode::leaf("c", op_editor_ui::layout_scene::NodeKind::Rect);
        child.fill = Some(Color::BLACK);
        let mut parent = SceneNode::leaf("p", op_editor_ui::layout_scene::NodeKind::Frame);
        parent.children = vec![child];
        fade_scene_node(&mut parent, 0.25);
        assert!((parent.children[0].fill.unwrap().a - 0.25).abs() < 1e-6);
    }
}
