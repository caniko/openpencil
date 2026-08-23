use std::collections::BTreeMap;
use std::path::Path;

use op_host_services::export::render_fallback_leaf_png;
use op_pen_loader::pen_document_to_layout_scene;
use op_runtime_ui::{apply_raster_fallbacks, load_document, ExportResult};

pub(crate) fn apply(file: &str, result: &mut ExportResult) -> Result<(), String> {
    if result.raster_candidates.is_empty() {
        return Ok(());
    }
    let doc = load_document(Path::new(file)).map_err(|e| e.to_string())?;
    let mut scenes = BTreeMap::new();
    let mut rasters = Vec::new();
    for c in &result.raster_candidates {
        let scene = scenes
            .entry(c.page_index)
            .or_insert_with(|| pen_document_to_layout_scene(&doc, &BTreeMap::new(), c.page_index));
        let page = scene
            .pages
            .get(c.page_index)
            .ok_or_else(|| format!("raster page {} missing for {}", c.page_index, c.source_id))?;
        let png = render_fallback_leaf_png(page, &c.scene_id)
            .map_err(|e| format!("--raster-native failed for {}: {e}", c.source_id))?;
        rasters.push((c.source_id.clone(), png));
    }
    apply_raster_fallbacks(result, &rasters).map_err(|e| e.to_string())
}
