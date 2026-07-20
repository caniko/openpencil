use std::sync::Arc;

use super::WidgetHost;

pub(crate) fn normalize_browser_system_font_families(families: Vec<String>) -> Vec<String> {
    let mut out = families
        .into_iter()
        .map(|family| family.trim().to_string())
        .filter(|family| !family.is_empty())
        .collect::<Vec<_>>();
    // CanvasKit cannot select a named local face unless Local Font Access gave
    // us its bytes. Keep only the browser-backed generic stack when access is
    // unavailable; advertising Arial/Georgia/etc. here would mark them
    // resolved even though the renderer would silently use another face.
    if out.is_empty() {
        out.push("system-ui".to_string());
    }
    out.sort_by_key(|family| family.to_lowercase());
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    out
}

impl WidgetHost {
    pub(crate) fn apply_browser_system_font_families(&mut self, families: Vec<String>) {
        let families = normalize_browser_system_font_families(families);
        let ui = &mut self.editor_state.editor_ui;
        ui.system_font_families = Arc::new(families);
        ui.system_fonts_loaded = true;
        ui.font_picker.hover = None;
        self.mark_dirty();
        self.complete_pending_missing_fonts_detection();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_browser_system_fonts_like_native_host() {
        let families = normalize_browser_system_font_families(vec![
            "PingFang SC".to_string(),
            "Inter".to_string(),
            " pingfang sc ".to_string(),
            "".to_string(),
            "SF Pro".to_string(),
        ]);

        assert_eq!(families, vec!["Inter", "PingFang SC", "SF Pro"]);
    }

    #[test]
    fn applies_browser_system_fonts_to_picker_state() {
        let mut host = crate::widget_host::WidgetHost::new();

        host.apply_browser_system_font_families(vec![
            "Inter".to_string(),
            "PingFang SC".to_string(),
            "SF Pro".to_string(),
        ]);

        let ui = &host.editor_state.editor_ui;
        assert!(ui.system_fonts_loaded);
        assert_eq!(
            ui.system_font_families.as_ref(),
            &vec![
                "Inter".to_string(),
                "PingFang SC".to_string(),
                "SF Pro".to_string()
            ]
        );
    }

    #[test]
    fn empty_browser_query_exposes_only_the_renderable_generic_stack() {
        let mut host = crate::widget_host::WidgetHost::new();

        host.apply_browser_system_font_families(Vec::new());

        assert!(host.editor_state.editor_ui.system_fonts_loaded);
        let fallback_families = host.editor_state.editor_ui.system_font_families.clone();
        assert_eq!(fallback_families.as_ref(), &vec!["system-ui".to_string()]);

        let doc = serde_json::from_value(serde_json::json!({
            "version": "1.0.0",
            "children": [{
                "type": "text",
                "id": "fallback-font",
                "content": "Browser fallback",
                "fontFamily": "arial"
            }]
        }))
        .expect("document");
        host.editor_state = op_editor_core::EditorState::from_document(doc);
        host.editor_state.editor_ui.system_fonts_loaded = true;
        host.editor_state.editor_ui.system_font_families = fallback_families;
        let prompt = op_editor_core::missing_fonts::detect_missing_fonts(&host.editor_state)
            .expect("an unregistered named face remains missing");
        assert_eq!(prompt.entries[0].family, "arial");
    }
}
