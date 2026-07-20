use super::*;

fn element(style: &str) -> DomElement {
    DomElement {
        tag: "p".into(),
        attrs: vec![("style".into(), style.into())],
        children: Vec::new(),
    }
}

#[test]
fn shorthands_expand_after_variable_substitution() {
    let node = element(
        "--box:10px 20px 30px 40px;\
         --edge:2px dashed rgb(1 2 3);\
         --type:italic 700 18px/1.5 Inter, sans-serif;\
         --paint:linear-gradient(90deg,#000,#fff) center/cover no-repeat #123456;\
         --flex:2 0 25%;\
         --radius:4px 8px;\
         margin:var(--box);border:var(--edge);font:var(--type);\
         background:var(--paint);flex:var(--flex);border-radius:var(--radius)",
    );
    let style = compute_style(&[&node], &[], None, 16.0);

    assert_eq!(style.get("margin-top"), Some("10px"));
    assert_eq!(style.get("margin-right"), Some("20px"));
    assert_eq!(style.get("margin-bottom"), Some("30px"));
    assert_eq!(style.get("margin-left"), Some("40px"));
    assert_eq!(style.get("border-width"), Some("2px"));
    assert_eq!(style.get("border-style"), Some("dashed"));
    assert_eq!(style.get("border-color"), Some("rgb(1 2 3)"));
    assert_eq!(style.get("font-style"), Some("italic"));
    assert_eq!(style.get("font-weight"), Some("700"));
    assert_eq!(style.get("font-size"), Some("18px"));
    assert_eq!(style.get("line-height"), Some("1.5"));
    assert_eq!(style.get("font-family"), Some("Inter, sans-serif"));
    assert_eq!(
        style.get("background-image"),
        Some("linear-gradient(90deg,#000,#fff)")
    );
    assert_eq!(style.get("background-color"), Some("#123456"));
    assert_eq!(style.get("background-size"), Some("cover"));
    assert_eq!(style.get("flex-grow"), Some("2"));
    assert_eq!(style.get("flex-shrink"), Some("0"));
    assert_eq!(style.get("flex-basis"), Some("25%"));
    assert_eq!(style.get("border-top-left-radius"), Some("4px"));
    assert_eq!(style.get("border-top-right-radius"), Some("8px"));
}

#[test]
fn deferred_shorthands_keep_cascade_order_and_invalid_values_do_not_backtrack() {
    let node = element(
        "--space:2px 3px;margin:1px;margin-left:9px;margin:var(--space);\
         padding-top:7px;padding:var(--missing)!important",
    );
    let style = compute_style(&[&node], &[], None, 16.0);
    assert_eq!(style.get("margin-top"), Some("2px"));
    assert_eq!(style.get("margin-left"), Some("3px"));
    assert_eq!(style.get("padding-top"), Some("0"));

    let node = element("--space:2px 3px;margin:var(--space);margin-left:9px");
    let style = compute_style(&[&node], &[], None, 16.0);
    assert_eq!(style.get("margin-top"), Some("2px"));
    assert_eq!(style.get("margin-left"), Some("9px"));
}

#[test]
fn unlayered_and_inline_revert_layer_fall_back_to_the_previous_origin() {
    let (mut rules, _) = parse_stylesheet_with_origin("p{color:black}", 0, StyleOrigin::UserAgent);
    let (author, _) = parse_stylesheet(
        "@layer base{p{color:blue}}p{color:red;color:revert-layer}",
        0,
    );
    rules.extend(author);

    let node = element("");
    assert_eq!(
        compute_style(&[&node], &rules, None, 16.0).get("color"),
        Some("black")
    );

    let inline = element("color:revert-layer");
    assert_eq!(
        compute_style(&[&inline], &rules, None, 16.0).get("color"),
        Some("black")
    );
}
