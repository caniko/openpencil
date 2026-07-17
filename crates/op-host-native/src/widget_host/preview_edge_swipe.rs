//! Track C-4: iOS-style edge-swipe-to-pop for the device-frame preview.
//!
//! A press that starts within `EDGE_ZONE_PX` of the framed content's LEFT
//! edge arms a candidate; a subsequent drag whose cumulative rightward
//! screen-space delta crosses `POP_THRESHOLD_PX` fires `router.pop()` —
//! the same navigation path an authored `{"pop": null}` back button uses
//! — and cancels the underlying pointer gesture so the runtime doesn't
//! also complete a tap/drag on whatever was under the finger.
//!
//! Only armed in App Mode with a route stack deeper than the entry
//! screen (`PreviewSession::can_pop`), so the gesture never fights the
//! device frame's own vertical scroll (`apply_device_scroll`, wheel-only
//! today) or a horizontal drag inside ordinary content — those never
//! start within the 24px dead zone in the first place.

use jian_core::gesture::pointer::PointerPhase;

/// Screen-space distance from the framed content's left edge a press
/// must start within to arm an edge-swipe candidate.
const EDGE_ZONE_PX: f32 = 24.0;
/// Cumulative rightward drag distance that fires the pop.
const POP_THRESHOLD_PX: f32 = 60.0;

impl super::WidgetHostNative {
    /// Arm (or clear) an edge-swipe candidate for a fresh press at
    /// `screen_x`. Called from `preview_dispatch_press` right after the
    /// underlying pointer Down is forwarded.
    pub(in crate::widget_host) fn arm_edge_swipe_candidate(&mut self, screen_x: f32) {
        self.preview_edge_swipe_start_x = None;
        if !self.device_mode_active() {
            return;
        }
        let Some(frame) = self.preview_device_frame.as_ref() else {
            return;
        };
        let edge = frame.content_span_x.0;
        if screen_x < edge || screen_x - edge >= EDGE_ZONE_PX {
            return;
        }
        if !self.preview.as_ref().is_some_and(|p| p.can_pop()) {
            return;
        }
        self.preview_edge_swipe_start_x = Some(screen_x);
    }

    /// Check an armed candidate against the held drag's current
    /// `screen_x`. Returns `true` exactly once per gesture — the instant
    /// the threshold is crossed — and fires the pop as a side effect;
    /// every other call (before arming, before the threshold, or after
    /// it already fired once) is a no-op returning `false`.
    pub(in crate::widget_host) fn maybe_fire_edge_swipe(&mut self, screen_x: f32) -> bool {
        let Some(start_x) = self.preview_edge_swipe_start_x else {
            return false;
        };
        if screen_x - start_x < POP_THRESHOLD_PX {
            return false;
        }
        self.preview_edge_swipe_start_x = None;
        if let Some(preview) = self.preview.as_ref() {
            preview.pop_screen();
        }
        true
    }

    /// Clear any armed candidate — called on release so a gesture that
    /// never crossed the threshold doesn't leak into the NEXT press.
    pub(in crate::widget_host) fn disarm_edge_swipe(&mut self) {
        self.preview_edge_swipe_start_x = None;
    }

    /// Cancel the in-flight preview pointer gesture after an edge-swipe
    /// fires: dispatches `Cancel` (releasing the runtime's gesture
    /// anchor the same way an `Up` would) and clears the press-active
    /// flag so the eventual OS-level release is a no-op rather than
    /// replaying a stale `Up`.
    pub(in crate::widget_host) fn cancel_preview_gesture_for_edge_swipe(&mut self) {
        if let Some((x, y)) = self.preview_last_doc {
            if let Some(p) = self.preview.as_mut() {
                p.dispatch_pointer_phase(x, y, PointerPhase::Cancel);
            }
        }
        self.preview_press_active = false;
    }
}
