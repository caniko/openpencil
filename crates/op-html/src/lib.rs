//! HTML → PenNode importer (structured path, CSS-subset cascade).

use jian_ops_schema::node::PenNode;

pub struct HtmlImportOptions {
    pub viewport_width: f64,
    pub base_font_size: f64,
    pub document_name: Option<String>,
}

impl Default for HtmlImportOptions {
    fn default() -> Self {
        Self {
            viewport_width: 1440.0,
            base_font_size: 16.0,
            document_name: None,
        }
    }
}

pub struct HtmlImportResult {
    pub nodes: Vec<PenNode>,
    pub warnings: Vec<String>,
}

pub fn import_html(source: &str, opts: &HtmlImportOptions) -> HtmlImportResult {
    let _ = opts;
    let mut warnings = Vec::new();
    if source.trim().is_empty() {
        warnings.push("no importable content: input HTML is empty".to_string());
        return HtmlImportResult {
            nodes: Vec::new(),
            warnings,
        };
    }
    // Pipeline lands in later tasks; non-empty input is wired up in Task 11.
    warnings.push("no importable content: importer pipeline not yet implemented".to_string());
    HtmlImportResult {
        nodes: Vec::new(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_nodes_and_a_warning() {
        let r = import_html("", &HtmlImportOptions::default());
        assert!(r.nodes.is_empty());
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("no importable content"));
    }

    #[test]
    fn options_default_values() {
        let o = HtmlImportOptions::default();
        assert_eq!(o.viewport_width, 1440.0);
        assert_eq!(o.base_font_size, 16.0);
        assert!(o.document_name.is_none());
    }
}
