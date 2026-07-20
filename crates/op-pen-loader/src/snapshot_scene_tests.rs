use op_editor_core::EditorState;

use crate::editor_state_to_layout_scene;

#[test]
fn snapshot_absolute_overflow_does_not_expand_fixed_parent() {
    let snapshot = r##"{
      "version":1,
      "title":"Overflow snapshot",
      "viewport":{"width":320,"height":240},
      "root":{
        "kind":"element","tag":"body",
        "rect":{"x":0,"y":0,"w":100,"h":100},
        "styles":{"overflow":"visible"},
        "children":[{
          "kind":"element","tag":"div",
          "rect":{"x":80,"y":70,"w":50,"h":60},
          "styles":{"background-color":"#ff0000"},
          "children":[]
        }]
      }
    }"##;
    let imported = op_html::import_snapshot_document(
        snapshot,
        &op_html::HtmlImportOptions {
            viewport_width: 320.0,
            ..Default::default()
        },
    );
    assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);

    let state = EditorState::from_document(imported.document);
    let scene = editor_state_to_layout_scene(&state);
    let root = &scene.active_page().expect("active page").children[0];
    let overflow = &root.children[0];

    assert_eq!((root.bounds.size.x, root.bounds.size.y), (100.0, 100.0));
    assert_eq!(
        (
            overflow.bounds.origin.x,
            overflow.bounds.origin.y,
            overflow.bounds.size.x,
            overflow.bounds.size.y,
        ),
        (80.0, 70.0, 50.0, 60.0)
    );
}
