//! Track C-2 host-glue tests: screen-switcher pill press/hover/release +
//! the single-screen (non-APP-MODE) gate.

#![cfg(test)]

use super::WidgetHostNative;
use op_editor_core::EditorState;
use op_editor_ui::widgets::ScreenSwitcherPills;
use op_editor_ui::Rect;
use std::sync::{LazyLock, Mutex, MutexGuard};

static TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

fn test_lock() -> MutexGuard<'static, ()> {
    TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

/// Two authored (already-marked) top-level screens, each named — the
/// editor-shape fixture `project_screens` normalizes on `enter`.
const TWO_SCREEN_DOC_JSON: &str = r##"{
    "version": "1.1",
    "formatVersion": "1.1",
    "id": "x",
    "app": { "name": "x", "version": "1", "id": "x" },
    "pages": [
        { "id": "canvas", "name": "Canvas", "children": [
            { "type": "frame", "id": "home", "name": "Home", "screen": "/",
              "x": 0, "y": 0, "width": 200, "height": 200, "children": [] },
            { "type": "frame", "id": "profile", "name": "Profile", "screen": "/profile",
              "x": 500, "y": 0, "width": 200, "height": 200, "children": [] }
        ] }
    ]
}"##;

const SINGLE_SCREEN_DOC_JSON: &str = r##"{
    "version": "1.1",
    "formatVersion": "1.1",
    "id": "x",
    "app": { "name": "x", "version": "1", "id": "x" },
    "pages": [
        { "id": "p0", "name": "P0", "children": [
            { "type": "frame", "id": "a", "width": 100, "height": 100 }
        ] }
    ]
}"##;

fn host_with_doc(json: &str) -> WidgetHostNative {
    let doc: jian_ops_schema::PenDocument = serde_json::from_str(json).expect("valid doc");
    let mut host = WidgetHostNative::new();
    host.install_imported_state(EditorState::from_document(doc));
    host
}

fn app_mode_host() -> WidgetHostNative {
    let mut host = host_with_doc(TWO_SCREEN_DOC_JSON);
    assert!(host.enter_preview((1200.0, 800.0)));
    assert!(
        host.preview.as_ref().unwrap().is_app_mode(),
        "two authored screens must enter APP MODE"
    );
    host
}

fn center(r: Rect) -> (f32, f32) {
    (r.origin.x + r.size.x / 2.0, r.origin.y + r.size.y / 2.0)
}

#[test]
fn single_screen_session_never_arms_the_switcher() {
    let _guard = test_lock();
    let mut host = host_with_doc(SINGLE_SCREEN_DOC_JSON);
    assert!(host.enter_preview((800.0, 600.0)));
    assert!(!host.preview.as_ref().unwrap().is_app_mode());
    assert!(!host.screen_switcher_press(0.0, 0.0, 800.0, 600.0));
    assert!(!host.screen_switcher_release());
}

#[test]
fn entries_are_stably_ordered_with_entry_path_first() {
    let _guard = test_lock();
    let host = app_mode_host();
    let entries = host.preview.as_ref().unwrap().screen_switcher_entries();
    assert_eq!(
        entries,
        vec![
            ("/".to_string(), "Home".to_string()),
            ("/profile".to_string(), "Profile".to_string()),
        ]
    );
    assert_eq!(
        host.preview.as_ref().unwrap().current_screen_index(),
        Some(0)
    );
}

#[test]
fn press_hover_release_navigates_to_the_pressed_pill() {
    let _guard = test_lock();
    let mut host = app_mode_host();
    let (viewport_w, viewport_h) = (1200.0, 800.0);
    let canvas = host.preview_canvas_rect(viewport_w, viewport_h);
    let rects = ScreenSwitcherPills::pill_rects(canvas, 2);
    let (px, py) = center(rects[1]); // "/profile" pill

    assert!(host.screen_switcher_press(px, py, viewport_w, viewport_h));
    assert_eq!(host.editor_state.editor_ui.screen_switcher_pressed, Some(1));

    host.screen_switcher_hover(px, py, viewport_w, viewport_h);
    assert_eq!(host.editor_state.editor_ui.screen_switcher_hover, Some(1));

    assert!(host.screen_switcher_release());
    assert!(host
        .editor_state
        .editor_ui
        .screen_switcher_pressed
        .is_none());

    // The release dispatched `router.replace("/profile")`; the mounted
    // screen actually swaps on the next per-frame reconcile — same as a
    // real runtime tap, which also resolves one frame later.
    let outcome = host.preview.as_mut().unwrap().reconcile(1_000);
    assert!(outcome.switched);
    assert_eq!(
        host.preview.as_ref().unwrap().current_path_for_test(),
        "/profile"
    );
}

#[test]
fn release_off_the_pressed_pill_does_not_navigate() {
    let _guard = test_lock();
    let mut host = app_mode_host();
    let (viewport_w, viewport_h) = (1200.0, 800.0);
    let canvas = host.preview_canvas_rect(viewport_w, viewport_h);
    let rects = ScreenSwitcherPills::pill_rects(canvas, 2);
    let (p0x, p0y) = center(rects[0]);
    let (p1x, p1y) = center(rects[1]);

    assert!(host.screen_switcher_press(p1x, p1y, viewport_w, viewport_h));
    // Cursor drags off the pressed pill before release.
    host.screen_switcher_hover(p0x, p0y, viewport_w, viewport_h);
    assert!(host.screen_switcher_release());

    let outcome = host.preview.as_mut().unwrap().reconcile(1_000);
    assert!(
        !outcome.switched,
        "release outside the pressed pill must not navigate"
    );
    assert_eq!(host.preview.as_ref().unwrap().current_path_for_test(), "/");
}

#[test]
fn exit_preview_clears_switcher_press_and_hover_state() {
    let _guard = test_lock();
    let mut host = app_mode_host();
    host.editor_state.editor_ui.screen_switcher_hover = Some(1);
    host.editor_state.editor_ui.screen_switcher_pressed = Some(1);
    host.exit_preview();
    assert_eq!(host.editor_state.editor_ui.screen_switcher_hover, None);
    assert_eq!(host.editor_state.editor_ui.screen_switcher_pressed, None);
}
