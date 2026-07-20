//! Hidden IME-capture input for the CanvasKit web shell (#54).
//!
//! The editor renders to a `<canvas>`, which is not an editable element, so a
//! browser IME (CJK / etc.) has nowhere to compose — composition events never
//! fire and the keydown handler's `is_composing()` guard drops the keystrokes,
//! leaving CJK input unusable in the browser.
//!
//! This module adds the missing piece: a hidden, focusable `<input>` that the
//! IME composes into. Its `compositionend` is wired (in `canvaskit.rs`) to
//! `WidgetHost::apply_ime`, which routes the committed string through
//! `apply_text` into whichever editor field owns the keyboard (canvas text
//! editor, chat input, property inputs …) — exactly the native host's
//! `Ime::Commit` behaviour. Preedit is intentionally not painted (matches
//! native), so only the final commit is consumed.
//!
//! Focus is gated on [`WidgetHost::input_active`](crate::widget_host): the
//! input is focused only while a text field is active, so it never pops a
//! mobile soft keyboard (or steals focus) when the user isn't editing text.
//!
//! The host repositions this element to the active caret before focus, so the
//! browser anchors its candidate window beside the visible editor instead of
//! at the viewport's top-left corner.
//!
//! Compiled under both the `web` stub and the production `canvaskit` build;
//! only the latter wires it, so its surface reads as dead code under `web`.
#![cfg_attr(not(feature = "canvaskit"), allow(dead_code))]

use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;

/// The id of the hidden IME input appended next to the canvas.
const INPUT_ID: &str = "op-ime-input";

/// Off-screen-but-focusable style. NOT `display:none` / `visibility:hidden`
/// (both block focus + IME composition); uses `opacity:0` + `pointer-events:
/// none` so it can't be seen or clicked but still accepts programmatic focus
/// and IME.
const HIDDEN_STYLE_TAIL: &str = "width:1px;height:1px;opacity:0;border:0;padding:0;\
margin:0;pointer-events:none;z-index:-1;";

/// Owns the hidden IME `<input>` and tracks its focus state so we only call
/// `focus()` / `blur()` on a transition (no per-frame focus churn).
pub(crate) struct ImeInput {
    input: HtmlInputElement,
    focused: bool,
    position: Option<(i32, i32)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusAction {
    Focus,
    Blur,
    None,
}

fn focus_action(want: bool, cached: bool, owns_dom_focus: bool) -> FocusAction {
    match (want, cached, owns_dom_focus) {
        (true, true, true) | (false, false, false) => FocusAction::None,
        (true, _, _) => FocusAction::Focus,
        (false, _, _) => FocusAction::Blur,
    }
}

impl ImeInput {
    /// Create the hidden input and append it as a sibling of `canvas` (or, if
    /// that fails, to `<body>`). Returns `None` only if the DOM is unreachable.
    pub(crate) fn create(canvas: &web_sys::HtmlCanvasElement) -> Option<Self> {
        let document = canvas
            .owner_document()
            .or_else(|| web_sys::window().and_then(|w| w.document()))?;

        // Reuse an existing input across re-mounts so we don't leak a second.
        let input: HtmlInputElement = match document.get_element_by_id(INPUT_ID) {
            Some(el) => el.dyn_into::<HtmlInputElement>().ok()?,
            None => {
                let el = document.create_element("input").ok()?;
                let el: HtmlInputElement = el.dyn_into::<HtmlInputElement>().ok()?;
                el.set_id(INPUT_ID);
                el.set_type("text");
                let _ = el.set_attribute(
                    "style",
                    &format!("position:fixed;left:0px;top:0px;{HIDDEN_STYLE_TAIL}"),
                );
                let _ = el.set_attribute("aria-hidden", "true");
                let _ = el.set_attribute("tabindex", "-1");
                // Keep the browser from autofilling / autocorrecting the
                // throwaway composition buffer.
                let _ = el.set_attribute("autocomplete", "off");
                let _ = el.set_attribute("autocorrect", "off");
                let _ = el.set_attribute("autocapitalize", "off");
                let _ = el.set_attribute("spellcheck", "false");
                if let Some(parent) = canvas.parent_node() {
                    let _ = parent.append_child(&el);
                } else if let Some(body) = document.body() {
                    let _ = body.append_child(&el);
                }
                el
            }
        };

        Some(Self {
            input,
            focused: false,
            position: None,
        })
    }

    /// The input element — `canvaskit.rs` attaches the composition listeners
    /// to it.
    pub(crate) fn input(&self) -> &HtmlInputElement {
        &self.input
    }

    fn owns_dom_focus(&self) -> bool {
        self.input
            .owner_document()
            .and_then(|document| document.active_element())
            .is_some_and(|active| {
                let active: wasm_bindgen::JsValue = active.into();
                let input: wasm_bindgen::JsValue = self.input.clone().into();
                js_sys::Object::is(&active, &input)
            })
    }

    /// Drive focus from the host's text-input-active state. The cached flag is
    /// only a churn hint: a canvas press can move real DOM focus to the canvas
    /// without changing host input ownership, so every sync also verifies
    /// `document.activeElement` and repairs focus when they diverge.
    pub(crate) fn sync_focus(&mut self, want: bool) {
        match focus_action(want, self.focused, self.owns_dom_focus()) {
            FocusAction::Focus => {
                self.focused = true;
                let _ = self.input.focus();
            }
            FocusAction::Blur => {
                self.focused = false;
                let _ = self.input.blur();
            }
            FocusAction::None => {}
        }
    }

    /// Place the invisible DOM caret at a CSS-viewport point. Integer pixels
    /// keep style churn deterministic while remaining more precise than the
    /// platform candidate-window placement itself.
    pub(crate) fn sync_position(&mut self, left: f64, top: f64) {
        let next = (left.round() as i32, top.round() as i32);
        if self.position == Some(next) {
            return;
        }
        self.position = Some(next);
        let _ = self.input.set_attribute(
            "style",
            &format!(
                "position:fixed;left:{}px;top:{}px;{HIDDEN_STYLE_TAIL}",
                next.0, next.1
            ),
        );
    }

    /// Empty the throwaway composition buffer (called on composition
    /// start/end) so it never accumulates committed text — the commit is read
    /// from the event's `data`, never the input value.
    pub(crate) fn clear(&self) {
        self.input.set_value("");
    }
}

#[cfg(test)]
mod tests {
    use super::{focus_action, FocusAction};

    #[test]
    fn focus_action_repairs_dom_focus_lost_while_host_stays_active() {
        assert_eq!(focus_action(true, true, false), FocusAction::Focus);
    }

    #[test]
    fn focus_action_blurs_reused_dom_input_when_host_is_inactive() {
        assert_eq!(focus_action(false, false, true), FocusAction::Blur);
    }

    #[test]
    fn focus_action_avoids_churn_when_cache_and_dom_agree() {
        assert_eq!(focus_action(true, true, true), FocusAction::None);
        assert_eq!(focus_action(false, false, false), FocusAction::None);
    }
}
