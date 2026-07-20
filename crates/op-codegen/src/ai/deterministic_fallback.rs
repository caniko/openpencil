//! Deterministic last-resort code generation for the AI pipeline.
//!
//! When planning/chunks and the one-shot whole-document AI rescue all fail,
//! the existing pure Rust framework generators can still emit a usable file
//! from the selected canonical node forest.

use jian_ops_schema::PenDocument;
use op_editor_core::codegen::Framework;
use serde_json::{json, Value};

use crate::{Codegen, Compose, Flutter, Html, React, ReactNative, Svelte, SwiftUi, Vue};

pub(super) fn generate(
    nodes_json: &str,
    variables_json: Option<&str>,
    framework: Framework,
) -> Result<String, String> {
    let value: Value = serde_json::from_str(nodes_json)
        .map_err(|error| format!("selected nodes are not valid JSON: {error}"))?;
    let children = match value {
        Value::Array(children) if !children.is_empty() => children,
        Value::Object(_) => vec![value],
        Value::Array(_) => return Err("selected node list is empty".to_string()),
        _ => return Err("selected nodes must be a JSON object or array".to_string()),
    };
    let mut document: PenDocument = serde_json::from_value(json!({
        "version": "1.0.0",
        "children": children,
    }))
    .map_err(|error| format!("selected nodes are not canonical .op nodes: {error}"))?;
    if let Some(variables_json) = variables_json {
        // Variables should enrich the fallback, never make an otherwise valid
        // deterministic generation fail. Invalid or stale variable payloads
        // are therefore ignored while the selected canonical nodes survive.
        if let Ok(variables) = serde_json::from_str(variables_json) {
            document.variables = Some(variables);
        }
    }

    let code = match framework {
        Framework::React => React.generate(&document),
        Framework::Vue => Vue.generate(&document),
        Framework::Svelte => Svelte.generate(&document),
        Framework::Html => Html.generate(&document),
        Framework::Flutter => Flutter.generate(&document),
        Framework::SwiftUi => SwiftUi.generate(&document),
        Framework::Compose => Compose.generate(&document),
        Framework::ReactNative => ReactNative.generate(&document),
    };
    if code.trim().is_empty() {
        Err(format!(
            "the deterministic {} generator returned empty code",
            framework.as_wire()
        ))
    } else {
        Ok(code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NODE: &str = r#"[{"type":"frame","id":"root","name":"Root","x":0,"y":0,"width":320,"height":200,"children":[{"type":"text","id":"title","content":"Hello","x":12,"y":12,"width":100,"height":24}]}]"#;

    #[test]
    fn every_framework_has_a_non_empty_deterministic_fallback() {
        for framework in [
            Framework::React,
            Framework::Vue,
            Framework::Svelte,
            Framework::Html,
            Framework::Flutter,
            Framework::SwiftUi,
            Framework::Compose,
            Framework::ReactNative,
        ] {
            let code = generate(NODE, None, framework)
                .unwrap_or_else(|error| panic!("{} fallback failed: {error}", framework.as_wire()));
            assert!(!code.trim().is_empty(), "{}", framework.as_wire());
        }
    }

    #[test]
    fn invalid_nodes_return_an_actionable_error() {
        let error = generate("not-json", None, Framework::React).expect_err("invalid fixture");
        assert!(error.contains("not valid JSON"), "{error}");
    }

    #[test]
    fn valid_variables_are_preserved_in_css_capable_fallbacks() {
        let variables = r##"{"brand":{"type":"color","value":"#3366ff"}}"##;
        let code = generate(NODE, Some(variables), Framework::Vue).expect("vue fallback");
        assert!(code.contains("--brand: #3366ff;"), "{code}");
    }

    #[test]
    fn invalid_variables_do_not_block_node_fallback() {
        let code = generate(NODE, Some("not-json"), Framework::Html).expect("html fallback");
        assert!(code.contains("<!DOCTYPE html>"), "{code}");
    }
}
