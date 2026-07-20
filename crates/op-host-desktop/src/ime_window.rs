//! Native-window IME capability and candidate-anchor synchronization.
//!
//! Candidate UIs can ask the platform for their anchor before the first
//! `Ime::Preedit` reaches the event loop. Publish the caret before enabling
//! IME, then refresh it whenever layout or the logical caret changes.

use op_editor_ui::Rect;
use op_host_native::WidgetHostNative;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::Window;

use crate::DesktopApp;

#[derive(Clone, Copy, Debug, PartialEq)]
struct ImeArea {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

#[derive(Clone, Copy)]
struct ImeWindowMetrics {
    viewport_width: f32,
    viewport_height: f32,
    cursor_x: f32,
    cursor_y: f32,
    dpi: f32,
}

impl ImeArea {
    fn from_rect(rect: Rect, dpi: f64) -> Self {
        Self {
            x: rect.origin.x as f64 * dpi,
            y: (rect.origin.y + rect.size.y) as f64 * dpi,
            width: rect.size.x as f64 * dpi,
            height: rect.size.y as f64 * dpi,
        }
    }
}

trait ImeWindowTarget {
    fn publish_ime_area(&self, area: ImeArea);
    fn publish_ime_allowed(&self, allowed: bool);
}

impl ImeWindowTarget for Window {
    fn publish_ime_area(&self, area: ImeArea) {
        Window::set_ime_cursor_area(
            self,
            PhysicalPosition::new(area.x, area.y),
            PhysicalSize::new(area.width, area.height),
        );
    }

    fn publish_ime_allowed(&self, allowed: bool) {
        Window::set_ime_allowed(self, allowed);
    }
}

#[derive(Default)]
pub(crate) struct ImeWindowSync {
    allowed: bool,
    area: Option<ImeArea>,
}

impl ImeWindowSync {
    fn sync(&mut self, host: &mut WidgetHostNative, window: &Window, metrics: ImeWindowMetrics) {
        self.sync_target(host, window, metrics);
    }

    fn sync_target<T: ImeWindowTarget>(
        &mut self,
        host: &mut WidgetHostNative,
        target: &T,
        metrics: ImeWindowMetrics,
    ) {
        if !host.text_input_focus_active() {
            if self.allowed {
                target.publish_ime_allowed(false);
                self.allowed = false;
            }
            return;
        }

        let rect = host
            .ime_anchor_rect(metrics.viewport_width, metrics.viewport_height)
            .unwrap_or_else(|| Rect::xywh(metrics.cursor_x, metrics.cursor_y, 1.5, 18.0));
        let area = ImeArea::from_rect(rect, metrics.dpi as f64);

        // The platform may query the candidate rectangle as soon as IME is
        // enabled, before the event loop receives its first Preedit event.
        if !self.allowed || self.area != Some(area) {
            target.publish_ime_area(area);
            self.area = Some(area);
        }
        if !self.allowed {
            target.publish_ime_allowed(true);
            self.allowed = true;
            // Wayland ignores cursor-area updates while IME is disabled.
            // Publish once more after enabling while preserving the required
            // anchor-before-enable ordering used by macOS.
            target.publish_ime_area(area);
        }
    }
}

impl DesktopApp {
    pub(crate) fn sync_native_ime(&mut self) {
        let Some(window) = self.window.as_ref() else {
            return;
        };
        self.ime_window_sync.sync(
            &mut self.host,
            window,
            ImeWindowMetrics {
                viewport_width: self.viewport_width,
                viewport_height: self.viewport_height,
                cursor_x: self.cursor_x,
                cursor_y: self.cursor_y,
                dpi: self.dpi,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{ImeArea, ImeWindowMetrics, ImeWindowSync, ImeWindowTarget};
    use op_editor_core::image_panel_state::ImageGeneratePhase;
    use op_editor_core::EditorState;
    use op_host_native::WidgetHostNative;
    use std::cell::RefCell;

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Event {
        Area(ImeArea),
        Allowed(bool),
    }

    #[derive(Default)]
    struct FakeWindow {
        events: RefCell<Vec<Event>>,
    }

    impl ImeWindowTarget for FakeWindow {
        fn publish_ime_area(&self, area: ImeArea) {
            self.events.borrow_mut().push(Event::Area(area));
        }

        fn publish_ime_allowed(&self, allowed: bool) {
            self.events.borrow_mut().push(Event::Allowed(allowed));
        }
    }

    fn image_host() -> WidgetHostNative {
        let mut host = WidgetHostNative::new();
        let mut state = EditorState::sample();
        let _ = state.insert_image_node_at_viewport("Hero photo", "https://x/y.png");
        *host.editor_state_mut() = state;
        host
    }

    fn sync(sync: &mut ImeWindowSync, host: &mut WidgetHostNative, window: &FakeWindow) {
        sync.sync_target(
            host,
            window,
            ImeWindowMetrics {
                viewport_width: 1200.0,
                viewport_height: 800.0,
                cursor_x: 25.0,
                cursor_y: 35.0,
                dpi: 2.0,
            },
        );
    }

    #[test]
    fn publishes_anchor_before_enabling_and_disables_after_close() {
        let mut host = image_host();
        host.editor_state_mut().editor_ui.image_panel.search_open = true;
        let mut state = ImeWindowSync::default();
        let window = FakeWindow::default();

        sync(&mut state, &mut host, &window);
        let events = window.events.borrow();
        assert!(matches!(
            events.as_slice(),
            [Event::Area(_), Event::Allowed(true), Event::Area(_)]
        ));
        drop(events);

        host.editor_state_mut().editor_ui.image_panel.search_open = false;
        sync(&mut state, &mut host, &window);
        assert_eq!(window.events.borrow().last(), Some(&Event::Allowed(false)));
    }

    #[test]
    fn caret_move_republishes_area_without_reenabling() {
        let mut host = image_host();
        let input = &mut host.editor_state_mut().editor_ui.image_panel.search_query;
        input.set_text("abcd");
        input.set_caret(0, 0);
        host.editor_state_mut().editor_ui.image_panel.search_open = true;
        let mut state = ImeWindowSync::default();
        let window = FakeWindow::default();

        sync(&mut state, &mut host, &window);
        let first = match window.events.borrow()[0] {
            Event::Area(area) => area,
            Event::Allowed(_) => panic!("area must be published first"),
        };
        window.events.borrow_mut().clear();

        host.editor_state_mut()
            .editor_ui
            .image_panel
            .search_query
            .set_caret(3, 0);
        sync(&mut state, &mut host, &window);
        let events = window.events.borrow();
        assert_eq!(events.len(), 1);
        let Event::Area(after) = events[0] else {
            panic!("caret movement should only republish the area");
        };
        assert!(after.x > first.x);
    }

    #[test]
    fn hidden_generate_input_disables_ime() {
        let mut host = image_host();
        let id = host
            .editor_state_mut()
            .editor_ui
            .agent_settings
            .add_image_gen_profile();
        let profile = host
            .editor_state_mut()
            .editor_ui
            .agent_settings
            .image_gen_profiles
            .iter_mut()
            .find(|profile| profile.id == id)
            .expect("new profile");
        profile.api_key = "sk-test".into();
        host.editor_state_mut().editor_ui.image_panel.generate_open = true;
        let mut state = ImeWindowSync::default();
        let window = FakeWindow::default();

        sync(&mut state, &mut host, &window);
        assert!(window.events.borrow().contains(&Event::Allowed(true)));

        host.editor_state_mut().editor_ui.image_panel.generate_phase = ImageGeneratePhase::Loading;
        sync(&mut state, &mut host, &window);
        assert_eq!(window.events.borrow().last(), Some(&Event::Allowed(false)));
    }
}
