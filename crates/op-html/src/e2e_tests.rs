use crate::{import_html, HtmlImportOptions};
use jian_ops_schema::node::container::LayoutMode;
use jian_ops_schema::node::PenNode;
use jian_ops_schema::sizing::{SizingBehavior, SizingKeyword};

const LANDING: &str = r#"<html><head><title>Acme</title><style>
  .hero { display:flex; flex-direction:column; align-items:center; gap:24px;
          padding:64px; background:linear-gradient(180deg,#0b1220,#1a2740); }
  .hero h1 { color:#ffffff; margin:0 }
  .cta { background-color:#3b82f6; color:#ffffff; padding:12px 24px; border-radius:8px }
  .row { display:flex; gap:16px }
</style></head><body>
  <section class="hero">
    <h1>Build faster</h1>
    <p style="color:#94a3b8">Ship <b>beautiful</b> designs</p>
    <div class="row">
      <button class="cta">Start</button>
      <input type="text" placeholder="Email"/>
    </div>
  </section>
</body></html>"#;

#[test]
fn landing_page_imports_as_editable_tree() {
    let result = import_html(LANDING, &HtmlImportOptions::default());
    assert_eq!(result.nodes.len(), 1);
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!("root must be frame")
    };
    assert_eq!(root.base.name.as_deref(), Some("Acme"));
    assert_eq!(root.container.width, Some(SizingBehavior::Number(1440.0)));
    assert_eq!(
        root.container.height,
        Some(SizingBehavior::Keyword(SizingKeyword::FitContent))
    );
    let PenNode::Frame(hero) = &root.children.as_ref().unwrap()[0] else {
        panic!()
    };
    assert_eq!(hero.container.layout, Some(LayoutMode::Vertical));
    let children = hero.children.as_ref().unwrap();
    assert!(children.len() >= 3);
    let PenNode::Frame(row) = children.last().unwrap() else {
        panic!("row")
    };
    let row_children = row.children.as_ref().unwrap();
    assert!(matches!(&row_children[0], PenNode::Frame(button)
        if button.base.role.as_deref() == Some("button")));
    assert!(matches!(&row_children[1], PenNode::TextInput(_)));
}

#[test]
fn node_limit_truncates_with_warning() {
    let mut html = String::from("<div>");
    for _ in 0..25_000 {
        html.push_str("<p>x</p>");
    }
    html.push_str("</div>");
    let result = import_html(&html, &HtmlImportOptions::default());
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("node limit")));
}

#[test]
fn document_name_option_overrides_title() {
    let options = HtmlImportOptions {
        document_name: Some("Custom".into()),
        ..Default::default()
    };
    let result = import_html(
        "<html><head><title>T</title></head><body><p>x</p></body></html>",
        &options,
    );
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!()
    };
    assert_eq!(root.base.name.as_deref(), Some("Custom"));
}
