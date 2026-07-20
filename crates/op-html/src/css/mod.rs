pub mod cascade;
mod cascade_conditions;
mod cascade_parser;
pub mod declarations;
pub mod selectors;
mod selectors_parser;

#[cfg(test)]
mod cascade_tests {
    use super::cascade::*;
    use super::selectors::PseudoElement;
    use crate::dom::DomElement;

    fn el(tag: &str, class: &str, style: &str) -> DomElement {
        let mut attrs = Vec::new();
        if !class.is_empty() {
            attrs.push(("class".into(), class.into()));
        }
        if !style.is_empty() {
            attrs.push(("style".into(), style.into()));
        }
        DomElement {
            tag: tag.into(),
            attrs,
            children: Vec::new(),
        }
    }

    #[test]
    fn cascade_variables_wide_values_and_font_math() {
        let css = ":root{--ink:#123;--a:var(--b);--b:var(--a)}\
            span{color:var(--ink)!important;border-top-color:var(--a,#fff)}";
        let (rules, _) = parse_stylesheet(css, 0);
        let root = el("div", "", "font-size:20px;font-weight:700");
        let parent = compute_style(&[&root], &rules, None, 16.0);
        let child = el(
            "span",
            "",
            "color:blue;font-size:calc(150% - 2px);font-weight:lighter",
        );
        let style = compute_style(&[&root, &child], &rules, Some(&parent), 16.0);
        assert_eq!(style.get("color"), Some("#123"));
        assert_eq!(style.get("border-top-color"), Some("#fff"));
        assert_eq!(
            (style.font_size, style.get("font-weight")),
            (28.0, Some("400"))
        );
        assert!(style.get("--a").is_none());
    }

    #[test]
    fn nested_media_strings_and_pseudo_elements() {
        let css = "@layer x{@supports(display:grid){p{display:grid}}}\
            @media screen and (max-width:800px), not screen{p{color:red}}\
            p::after{content:\"}/*literal*/\";font-size:clamp(10px,2vw,30px)}";
        let (rules, warnings) = parse_stylesheet_for_viewport(css, 0, 720.0, 900.0);
        assert!(warnings.is_empty());
        let node = el("p", "", "color:blue");
        let normal = compute_style_for_viewport(&[&node], &rules, None, 16.0, 720.0, 900.0);
        let pseudo = compute_pseudo_style_for_viewport(
            &[&node],
            &rules,
            Some(&normal),
            16.0,
            PseudoElement::After,
            720.0,
            900.0,
        );
        assert_eq!(
            (normal.get("color"), normal.get("display")),
            (Some("blue"), Some("grid"))
        );
        assert_eq!(
            (pseudo.get("content"), pseudo.font_size),
            (Some("\"}/*literal*/\""), 14.4)
        );
    }

    #[test]
    fn author_origin_beats_more_specific_ua_rule() {
        let (mut rules, _) = parse_stylesheet_with_origin(UA_STYLESHEET, 0, StyleOrigin::UserAgent);
        let (author, _) =
            parse_stylesheet_with_origin("*{font-size:10px;color:purple}", 0, StyleOrigin::Author);
        rules.extend(author);
        let heading = el("h1", "", "");
        let style = compute_style(&[&heading], &rules, None, 16.0);
        assert_eq!(
            (style.font_size, style.get("color")),
            (10.0, Some("purple"))
        );
    }

    #[test]
    fn variables_require_a_function_token_and_are_depth_bounded() {
        let css = ":root{--ink:#123456;--a:var(--b);--b:var(--a)}\
            p{color:var(--a,var(--ink));content:notvar(--ink)}";
        let (rules, _) = parse_stylesheet(css, 0);
        let node = el("p", "", "");
        let style = compute_style(&[&node], &rules, None, 16.0);
        assert_eq!(style.get("color"), Some("#123456"));
        assert_eq!(style.get("content"), Some("notvar(--ink)"));

        let mut fallback = "red".to_string();
        for index in 0..200 {
            fallback = format!("var(--missing-{index},{fallback})");
        }
        let deep = el("p", "", &format!("color:{fallback}"));
        let style = compute_style(&[&deep], &[], None, 16.0);
        assert_eq!(style.get("color"), Some("#000000"));
    }

    #[test]
    fn supports_evaluates_boolean_declarations_and_selectors() {
        let css = "@supports (display:grid) and selector(.card > span:first-child){p{color:red}}\
            @supports (display:nonsense){p{color:blue}}\
            @supports (unknown-prop:x) or (position:absolute){p{display:block}}\
            @supports not (unknown-prop:x){p{visibility:hidden}}\
            @supports (width:nonsense){p{opacity:.2}}\
            @supports selector(.card:has(> span)){p{position:relative}}";
        let (rules, warnings) = parse_stylesheet(css, 0);
        assert!(warnings.is_empty());
        let node = el("p", "", "");
        let style = compute_style(&[&node], &rules, None, 16.0);
        assert_eq!(style.get("color"), Some("red"));
        assert_eq!(style.get("display"), Some("block"));
        assert_eq!(style.get("visibility"), Some("hidden"));
        assert_eq!(style.get("opacity"), None);
        assert_eq!(style.get("position"), Some("relative"));
    }

    #[test]
    fn cascade_layers_reverse_order_for_important_declarations() {
        let css = "@layer reset,theme;\
            @layer reset{p{color:red!important;background-color:red}}\
            @layer theme{p{color:blue!important;background-color:blue}}\
            p{color:green!important;background-color:green}";
        let (rules, _) = parse_stylesheet(css, 0);
        let node = el("p", "", "");
        let style = compute_style(&[&node], &rules, None, 16.0);
        assert_eq!(style.get("color"), Some("red"));
        assert_eq!(style.get("background-color"), Some("green"));
    }

    #[test]
    fn nested_layers_stay_with_their_parent_and_direct_rules_are_last() {
        let css = "@layer first,last;\
            @layer first{@layer child{p{color:red;background-color:red;opacity:.5!important}}\
                         p{background-color:orange;opacity:.7!important}}\
            @layer last{p{color:blue}}";
        let (rules, _) = parse_stylesheet(css, 0);
        let node = el("p", "", "");
        let style = compute_style(&[&node], &rules, None, 16.0);
        assert_eq!(style.get("color"), Some("blue"));
        assert_eq!(style.get("background-color"), Some("orange"));
        assert_eq!(style.get("opacity"), Some(".5"));
    }

    #[test]
    fn important_origins_use_the_reverse_origin_order() {
        let (mut rules, _) =
            parse_stylesheet_with_origin("p{color:black!important}", 0, StyleOrigin::UserAgent);
        let (author, _) =
            parse_stylesheet_with_origin("p{color:red!important}", 0, StyleOrigin::Author);
        rules.extend(author);
        let node = el("p", "", "");
        assert_eq!(
            compute_style(&[&node], &rules, None, 16.0).get("color"),
            Some("black")
        );
    }

    #[test]
    fn named_layer_order_survives_across_stylesheets() {
        let mut parser = StylesheetParser::new(StyleOrigin::Author, 0);
        let (mut rules, _) = parser.parse("@layer first,second;@layer second{p{color:blue}}");
        let (later, _) = parser.parse("@layer first{p{color:red}}");
        rules.extend(later);
        let node = el("p", "", "");
        assert_eq!(
            compute_style(&[&node], &rules, None, 16.0).get("color"),
            Some("blue")
        );
    }

    #[test]
    fn revert_and_revert_layer_reveal_lower_cascade_levels() {
        let (mut rules, _) =
            parse_stylesheet_with_origin("p{color:black}", 0, StyleOrigin::UserAgent);
        let (author, _) = parse_stylesheet(
            "@layer base,override;@layer base{p{background-color:red}}\
             @layer override{p{background-color:blue;background-color:revert-layer}}\
             p{color:red;color:revert}",
            0,
        );
        rules.extend(author);
        let node = el("p", "", "");
        let style = compute_style(&[&node], &rules, None, 16.0);
        assert_eq!(style.get("color"), Some("black"));
        assert_eq!(style.get("background-color"), Some("red"));
    }

    #[test]
    fn media_range_syntax_and_parser_limits_are_safe() {
        let css = "@media (width <= 720px){p{color:red}}\
            @media (400px < width < 900px){p{display:block}}";
        let (rules, warnings) = parse_stylesheet_for_viewport(css, 0, 600.0, 800.0);
        assert!(warnings.is_empty());
        let node = el("p", "", "");
        let style = compute_style(&[&node], &rules, None, 16.0);
        assert_eq!(
            (style.get("color"), style.get("display")),
            (Some("red"), Some("block"))
        );

        let deeply_nested = format!(
            "{}p{{color:red}}{}",
            "@supports (display:grid){".repeat(70),
            "}".repeat(70)
        );
        let (_, warnings) = parse_stylesheet(&deeply_nested, 0);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("depth limit")));
    }

    #[test]
    fn custom_property_token_blocks_survive_but_css_nesting_warns() {
        let css = "p{--tokens:{foreground:red;meta:{x:y}};content:var(--tokens);\
            &:hover{color:blue}}@container card (width>1px){p{color:red}}";
        let (rules, warnings) = parse_stylesheet(css, 0);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("CSS nesting")));
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("@container")));
        let node = el("p", "", "");
        assert_eq!(
            compute_style(&[&node], &rules, None, 16.0).get("content"),
            Some("{foreground:red;meta:{x:y}}")
        );
    }
}

mod declaration_syntax;
