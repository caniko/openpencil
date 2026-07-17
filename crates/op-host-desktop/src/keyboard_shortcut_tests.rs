//! Track C-6 (`Cmd+P` toggles Preview) + its non-conflict with the
//! existing `Cmd+Shift+P` (Export Image) chord. Split out of
//! `main_tests.rs` to keep that file under the repo's 800-line cap.

use super::*;
use winit::keyboard::Key;

#[test]
fn cmd_p_enters_and_exits_preview() {
    let mut app = DesktopApp::new(None);
    assert!(!app.host.preview_active());

    app.zoom_modifier = true;
    app.handle_key_pressed(&Key::Character("p".into()), Some("p"));
    assert!(app.host.preview_active(), "Cmd+P must enter preview");

    app.handle_key_pressed(&Key::Character("p".into()), Some("p"));
    assert!(!app.host.preview_active(), "Cmd+P again must exit preview");
}

#[test]
fn cmd_p_works_even_while_preview_owns_the_keyboard() {
    // Preview's own takeover (Tab / arrow keys, gated on
    // `preview_active()`) only claims UNMODIFIED named keys — it must
    // never shadow the Cmd+P character chord that exits it.
    let mut app = DesktopApp::new(None);
    app.zoom_modifier = true;
    app.handle_key_pressed(&Key::Character("p".into()), Some("p"));
    assert!(app.host.preview_active());

    app.handle_key_pressed(&Key::Character("p".into()), Some("p"));
    assert!(
        !app.host.preview_active(),
        "Cmd+P must exit preview from within preview mode, not be swallowed by its keyboard takeover"
    );
}

#[test]
fn cmd_shift_p_stays_export_image_not_preview() {
    // Cmd+Shift+P is the pre-existing Export Image chord
    // (`keyboard_input.rs`'s `zoom_modifier && shift_modifier` arm) —
    // Track C-6 must not collide with it. Plain Cmd+P (no Shift) is the
    // only chord that toggles preview.
    let mut app = DesktopApp::new(None);
    app.zoom_modifier = true;
    app.shift_modifier = true;
    app.handle_key_pressed(&Key::Character("p".into()), Some("p"));
    assert!(
        !app.host.preview_active(),
        "Cmd+Shift+P must not toggle preview — it's Export Image"
    );
}
