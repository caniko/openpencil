//! Tests for the image-node section assets (view derivation +
//! popover geometry / hit-testing).

use super::property_panel_image_assets::*;
use crate::widgets::property_panel::PropertyPanelAction;
use crate::{Point2D, Rect};

use op_editor_core::image_panel_state::{
    ImageAssetCheck, ImageAssetStatus, ImageGeneratePhase, ImagePanelState, ImageSearchHit,
    ImageSearchSource,
};
use op_editor_core::{EditorState, NodeId};
use std::sync::Arc;

fn image_state() -> EditorState {
    let mut state = EditorState::sample();
    // Insert a fresh image node and select it.
    let _ = state.insert_image_node_at_viewport("Hero photo", "./assets/hero.png");
    state
}

fn selected_image_view(state: &EditorState) -> ImagePanelView {
    let node = state.selected_node().expect("image node selected");
    image_panel_view(state, node).expect("image view")
}

#[test]
fn view_seeds_query_and_prompt_from_name() {
    let state = image_state();
    let view = selected_image_view(&state);
    assert_eq!(view.search_seed, "Hero photo");
    assert_eq!(view.prompt_seed, "Hero photo");
    assert_eq!(view.src.as_deref(), Some("./assets/hero.png"));
}

#[test]
fn warning_requires_a_host_asset_check() {
    let mut state = image_state();
    // No host check yet → no warning even for a local path.
    assert!(selected_image_view(&state).warning.is_none());
    let id = state.selection.anchor.as_str().to_string();
    state.editor_ui.image_panel.asset_check = Some(ImageAssetCheck {
        node_id: id.clone(),
        src: "./assets/hero.png".into(),
        status: ImageAssetStatus::Missing,
    });
    let warning = selected_image_view(&state).warning.expect("warning");
    assert_eq!(warning.message, "Image file is missing");
    assert_eq!(warning.asset_path, "./assets/hero.png");
    // Remote URLs never warn, even with a stale check entry.
    let nid = NodeId::new(id);
    if let Some(jian_ops_schema::node::PenNode::Image(img)) =
        op_editor_core::walkers::find_node_mut(state.active_children_mut(), &nid)
    {
        img.src = "https://example.com/x.png".into();
    }
    assert!(selected_image_view(&state).warning.is_none());
}

/// A local path that DOES resolve on this machine still surfaces a
/// (softer, informational) warning — resolving here isn't the same as
/// being portable when the document is shared. `Ok` is the ONLY status
/// that suppresses the row.
#[test]
fn linked_local_status_still_warns_that_sharing_will_lose_the_image() {
    let mut state = image_state();
    let id = state.selection.anchor.as_str().to_string();
    state.editor_ui.image_panel.asset_check = Some(ImageAssetCheck {
        node_id: id,
        src: "./assets/hero.png".into(),
        status: ImageAssetStatus::LinkedLocal,
    });
    let warning = selected_image_view(&state).warning.expect("warning");
    assert_eq!(
        warning.message,
        "Linked to a local file — won't be included when this file is shared"
    );
    assert_eq!(warning.asset_path, "./assets/hero.png");
}

#[test]
fn local_asset_path_matches_ts_predicate() {
    assert!(is_local_asset_path("./a.png"));
    assert!(is_local_asset_path("/Users/x/a.png"));
    assert!(is_local_asset_path("C:\\img\\a.png"));
    assert!(!is_local_asset_path("data:image/png;base64,AA=="));
    assert!(!is_local_asset_path("https://x/y.png"));
    assert!(!is_local_asset_path("blob:abc"));
    assert!(!is_local_asset_path("/api/local-asset?path=x"));
    assert!(!is_local_asset_path("  "));
}

#[test]
fn popover_hits_resolve_controls() {
    let state = image_state();
    let panel = crate::widgets::PropertyPanel::for_selection(&state).expect("panel for image node");
    let visible = panel.visible_sections_for_test();
    let rect = Rect {
        origin: Point2D::new(600.0, 0.0),
        size: Point2D::new(280.0, 1400.0),
    };
    let mut ips = ImagePanelState {
        search_open: true,
        ..Default::default()
    };
    ips.search_query.set_text("cat");
    ips.search_results.push(ImageSearchHit {
        id: "1".into(),
        thumb_data_url: Arc::new("data:image/png;base64,AA==".into()),
        attribution: String::new(),
    });
    ips.search_source = Some(ImageSearchSource::Openverse);
    let layout = search_popover_layout(rect, visible, &ips).expect("layout");
    // Popover opens to the LEFT of the rail (TS side="left").
    assert!(layout.popup.origin.x < rect.origin.x);
    let submit_centre = Point2D::new(layout.submit.origin.x + 5.0, layout.submit.origin.y + 5.0);
    assert_eq!(
        image_popover_action_at(rect, visible, &ips, None, submit_centre),
        Some(PropertyPanelAction::RunImageSearch)
    );
    let cell0 = layout.cells[0];
    assert_eq!(
        image_popover_action_at(
            rect,
            visible,
            &ips,
            None,
            Point2D::new(cell0.origin.x + 2.0, cell0.origin.y + 2.0)
        ),
        Some(PropertyPanelAction::SelectImageSearchResult(0))
    );
    assert!(image_popovers_contain(
        rect,
        visible,
        &ips,
        None,
        submit_centre
    ));

    let input_point = Point2D::new(layout.input.origin.x + 8.0, layout.input.origin.y + 12.0);
    let (kind, offset) =
        image_popover_input_at(rect, visible, &ips, None, input_point).expect("input hit");
    assert_eq!(kind, ImagePopoverInputKind::Search);
    assert_eq!(offset, 0);

    ips.search_query.set_caret(0, 0);
    let start = image_popover_input_caret_rect(rect, visible, &ips, None).expect("caret");
    ips.search_query.set_caret(3, 0);
    let end = image_popover_input_caret_rect(rect, visible, &ips, None).expect("caret");
    assert!(end.origin.x > start.origin.x);

    let left = image_popover_input_drag_offset_at(
        rect,
        visible,
        &ips,
        None,
        ImagePopoverInputKind::Search,
        Point2D::new(layout.input.origin.x - 100.0, layout.input.origin.y - 100.0),
    );
    let right = image_popover_input_drag_offset_at(
        rect,
        visible,
        &ips,
        None,
        ImagePopoverInputKind::Search,
        Point2D::new(
            layout.input.origin.x + layout.input.size.x + 100.0,
            layout.input.origin.y + 100.0,
        ),
    );
    assert_eq!(left, Some(0));
    assert_eq!(right, Some(ips.search_query.text().len()));
}

#[test]
fn generate_popover_gates_on_profile_configuration() {
    let ips = ImagePanelState {
        generate_open: true,
        ..Default::default()
    };
    assert_eq!(
        generate_popover_view(&ips, None),
        GeneratePopoverView::NotConfigured
    );
    let unconfigured = ImageGenProfileView {
        configured: false,
        name: "P".into(),
        provider: "OpenAI",
        model: "dall-e-3".into(),
    };
    assert_eq!(
        generate_popover_view(&ips, Some(&unconfigured)),
        GeneratePopoverView::NotConfigured
    );
    let configured = ImageGenProfileView {
        configured: true,
        ..unconfigured
    };
    assert_eq!(
        generate_popover_view(&ips, Some(&configured)),
        GeneratePopoverView::Idle
    );
    let loading = ImagePanelState {
        generate_open: true,
        generate_phase: ImageGeneratePhase::Loading,
        ..Default::default()
    };
    assert_eq!(
        generate_popover_view(&loading, Some(&configured)),
        GeneratePopoverView::Loading
    );
}
