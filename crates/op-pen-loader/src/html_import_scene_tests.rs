use jian_scene::layout_scene::{SceneFillLayer, SceneNode};
use op_editor_core::render_backend::ImageBlendMode;
use op_editor_core::EditorState;

use crate::editor_state_to_layout_scene;

fn layered_node(nodes: &[SceneNode]) -> Option<&SceneNode> {
    nodes.iter().find_map(|node| {
        (node.fill_layers.len() >= 3)
            .then_some(node)
            .or_else(|| layered_node(&node.children))
    })
}

#[test]
fn html_background_stack_and_per_layer_blends_reach_layout_scene() {
    let html = r#"
      <div style="
        width: 120px;
        height: 80px;
        background-image:
          linear-gradient(90deg, #ff0000 0%, #0000ff 100%),
          url('data:image/svg+xml;base64,PHN2Zy8+');
        background-color: #112233;
        background-repeat: no-repeat, no-repeat;
        background-size: cover, contain;
        background-blend-mode: screen, multiply;
      "></div>
    "#;
    let imported =
        op_html::import_html_document(html, &op_html::HtmlImportOptions::default(), None, None);
    let state = EditorState::from_document(imported.document);
    let scene = editor_state_to_layout_scene(&state);
    let node = layered_node(&scene.active_page().expect("active page").children)
        .expect("HTML background node with all three fill layers");

    assert_eq!(node.fill_layers.len(), 3);
    assert!(matches!(
        &node.fill_layers[0],
        SceneFillLayer::Gradient {
            blend_mode: ImageBlendMode::Screen,
            ..
        }
    ));
    assert!(matches!(
        &node.fill_layers[1],
        SceneFillLayer::Image {
            blend_mode: ImageBlendMode::Multiply,
            fit: jian_scene::layout_scene::SceneImageFit::Fit,
            ..
        }
    ));
    assert!(matches!(
        &node.fill_layers[2],
        SceneFillLayer::Solid {
            color,
            blend_mode: ImageBlendMode::Normal,
        } if (color.r - 0x11 as f32 / 255.0).abs() < 0.001
          && (color.g - 0x22 as f32 / 255.0).abs() < 0.001
          && (color.b - 0x33 as f32 / 255.0).abs() < 0.001
    ));
}
