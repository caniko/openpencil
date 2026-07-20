use super::*;

#[test]
fn external_css_image_uses_the_stylesheet_url_as_its_base() {
    let html = r#"<link rel="stylesheet" href="/assets/css/site.css">
        <div class="hero"></div>"#;
    let requests = std::cell::RefCell::new(Vec::new());
    let fetcher = |url: &str| -> Option<Vec<u8>> {
        requests.borrow_mut().push(url.to_string());
        match url {
            "https://example.test/assets/css/site.css" => Some(
                br#"@media (min-width: 1px) {
                    .hero { width: 20px; height: 20px;
                        background-image: url('../images/hero icon.png'); }
                }"#
                .to_vec(),
            ),
            "https://example.test/assets/images/hero%20icon.png" => {
                Some(vec![0x89, 0x50, 0x4e, 0x47, 1, 2, 3])
            }
            _ => None,
        }
    };
    let options = HtmlImportOptions {
        base_url: Some("https://example.test/documents/page.html".to_string()),
        ..Default::default()
    };

    let result = import_html_with_resources(html, &options, Some(&fetcher), None);

    assert_eq!(
        requests.into_inner(),
        [
            "https://example.test/assets/css/site.css",
            "https://example.test/assets/images/hero%20icon.png",
        ]
    );
    let PenNode::Frame(root) = &result.nodes[0] else {
        panic!("the import root should be a frame")
    };
    let PenNode::Frame(hero) = &root.children.as_ref().expect("root children")[0] else {
        panic!("the styled div should be a frame")
    };
    assert!(matches!(
        hero.container.fill.as_deref(),
        Some([PenFill::Image(image)]) if image.url.as_str().starts_with("data:image/png;base64,")
    ));
}
