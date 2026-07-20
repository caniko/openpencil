use super::WidgetHostNative;
use op_editor_core::agent_settings::SettingsFocus;
use op_editor_core::{EditorState, NodeId};
use op_editor_ui::widgets::{LayerPanel, LayerPanelHit, TopBar, TopBarHit, TOP_BAR_HEIGHT};
use op_editor_ui::{Point2D, Rect};

const VIEWPORT_W: f32 = 1200.0;
const VIEWPORT_H: f32 = 800.0;

fn topbar_point(host: &mut WidgetHostNative, target: TopBarHit) -> Point2D {
    let rect = Rect::xywh(0.0, 0.0, VIEWPORT_W, TOP_BAR_HEIGHT);
    let mut topbar = TopBar::for_editor_ui(&host.editor_state().editor_ui);
    topbar.chip_text_w = Some(host.topbar_chip_text_w(&topbar));
    let mut x = rect.origin.x;
    while x < rect.origin.x + rect.size.x {
        let point = Point2D::new(x, TOP_BAR_HEIGHT / 2.0);
        if topbar.hit_test(rect, point) == Some(target) {
            return point;
        }
        x += 1.0;
    }
    panic!("no topbar point for {target:?}");
}

fn layer_row_point(host: &WidgetHostNative, target: &NodeId) -> Point2D {
    let rect = Rect::xywh(
        0.0,
        TOP_BAR_HEIGHT,
        host.editor_state().editor_ui.layer_panel_width,
        VIEWPORT_H - TOP_BAR_HEIGHT,
    );
    let panel = LayerPanel::from_editor(host.editor_state());
    let mut y = rect.origin.y;
    while y < rect.origin.y + rect.size.y {
        let point = Point2D::new(48.0, y);
        if panel.hit_test(rect, point) == Some(LayerPanelHit::Layer(target.clone())) {
            return point;
        }
        y += 1.0;
    }
    panic!("no layer row for {target:?}");
}

#[test]
fn topbar_modal_takes_input_ownership_from_image_search() {
    let mut host = WidgetHostNative::new();
    let mut state = EditorState::sample();
    let _ = state.insert_image_node_at_viewport("Hero photo", "https://x/y.png");
    state.editor_ui.image_panel.search_open = true;
    state
        .editor_ui
        .image_panel
        .search_query
        .set_text("hero query");
    *host.editor_state_mut() = state;
    let point = topbar_point(&mut host, TopBarHit::OpenAgentSettings);

    assert!(host.apply_press(point.x, point.y, VIEWPORT_W, VIEWPORT_H));
    assert!(host.editor_state().editor_ui.agent_settings_open);
    assert!(!host.editor_state().editor_ui.image_panel.search_open);

    let ui = &mut host.editor_state_mut().editor_ui;
    ui.agent_settings.focus = Some(SettingsFocus::McpPort);
    ui.settings_input.set_text("123");
    assert!(host.apply_ime_commit("4"));
    assert_eq!(host.editor_state().editor_ui.settings_input.text(), "1234");
    assert_eq!(
        host.editor_state()
            .editor_ui
            .image_panel
            .search_query
            .text(),
        "hero query"
    );
}

#[test]
fn selection_changing_right_press_closes_image_search() {
    let doc = jian_ops_schema::load_str(
        r#"{"version":"1.0.0","children":[
          {"type":"rectangle","id":"n1","name":"Other","x":0,"y":0,"width":100,"height":50},
          {"type":"image","id":"n2","name":"Hero","x":120,"y":0,"width":100,"height":100,"src":"https://x/y.png"}
        ]}"#,
    )
    .expect("fixture")
    .value;
    let mut host = WidgetHostNative::new();
    *host.editor_state_mut() = EditorState::from_document(doc);
    host.editor_state_mut()
        .set_single_selection(NodeId::new("n2"));
    host.editor_state_mut().editor_ui.image_panel.search_open = true;
    host.editor_state_mut()
        .editor_ui
        .image_panel
        .search_query
        .set_text("hero query");
    let point = layer_row_point(&host, &NodeId::new("n1"));

    assert!(host.apply_right_press(point.x, point.y, VIEWPORT_W, VIEWPORT_H));
    assert_eq!(host.editor_state().selection.anchor, NodeId::new("n1"));
    assert!(!host.editor_state().editor_ui.image_panel.search_open);
    assert!(!host.apply_text('x'));
    assert_eq!(
        host.editor_state()
            .editor_ui
            .image_panel
            .search_query
            .text(),
        "hero query"
    );
}
