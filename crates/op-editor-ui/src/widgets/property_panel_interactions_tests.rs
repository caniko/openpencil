//! Tests for the Interactions section's data parsing + geometry +
//! menu helpers.

use super::*;
use crate::widgets::property_panel_visibility::SectionCapabilities;
use jian_ops_schema::node::PenNode;

fn node_from(json: &str) -> PenNode {
    serde_json::from_str(json).expect("test fixture parses")
}

// ── Section capability filtering ────────────────────────────────────

#[test]
fn interactions_capability_is_on_for_every_single_select_kind() {
    use crate::layout_scene::NodeKind as K;
    for kind in [
        K::Frame,
        K::Group,
        K::Rect,
        K::Ellipse,
        K::Polygon,
        K::Line,
        K::Path,
        K::Text,
        K::Other("image".to_string()),
        K::Other("icon_font".to_string()),
        K::Other("ref".to_string()),
        K::Other("widget".to_string()),
    ] {
        assert!(
            SectionCapabilities::for_kind(&kind).interactions,
            "{kind:?} should show the Interactions section"
        );
    }
}

#[test]
fn interactions_capability_is_off_for_multi_select() {
    assert!(!SectionCapabilities::for_multi().interactions);
}

// ── events.onTap parsing ─────────────────────────────────────────────

#[test]
fn interactions_of_parses_replace_navigate() {
    let node =
        node_from(r#"{"type":"frame","id":"f1","events":{"onTap":[{"replace":"\"/profile\""}]}}"#);
    let summary = interactions_of(&node, false);
    assert_eq!(summary.on_tap.len(), 1);
    assert_eq!(
        summary.on_tap[0],
        TapActionSummary::Navigate {
            verb: NavigateVerb::Replace,
            path: "/profile".to_string(),
        }
    );
}

#[test]
fn interactions_of_parses_push_navigate() {
    let node =
        node_from(r#"{"type":"frame","id":"f1","events":{"onTap":[{"push":"\"/detail\""}]}}"#);
    let summary = interactions_of(&node, false);
    assert_eq!(
        summary.on_tap[0],
        TapActionSummary::Navigate {
            verb: NavigateVerb::Push,
            path: "/detail".to_string(),
        }
    );
}

#[test]
fn interactions_of_parses_pop() {
    let node = node_from(r#"{"type":"frame","id":"f1","events":{"onTap":[{"pop":null}]}}"#);
    let summary = interactions_of(&node, false);
    assert_eq!(summary.on_tap[0], TapActionSummary::Pop);
}

#[test]
fn interactions_of_parses_unknown_key_as_other() {
    let node =
        node_from(r#"{"type":"frame","id":"f1","events":{"onTap":[{"set":{"$state.x":"1"}}]}}"#);
    let summary = interactions_of(&node, false);
    assert_eq!(
        summary.on_tap[0],
        TapActionSummary::Other("set".to_string())
    );
}

#[test]
fn interactions_of_carries_multiple_actions_in_order() {
    let node = node_from(
        r#"{"type":"frame","id":"f1","events":{"onTap":[{"pop":null},{"replace":"\"/a\""}]}}"#,
    );
    let summary = interactions_of(&node, false);
    assert_eq!(summary.on_tap.len(), 2);
    assert_eq!(summary.on_tap[0], TapActionSummary::Pop);
    assert_eq!(
        summary.on_tap[1],
        TapActionSummary::Navigate {
            verb: NavigateVerb::Replace,
            path: "/a".to_string(),
        }
    );
}

#[test]
fn interactions_of_reports_no_actions_for_bare_node() {
    let node = node_from(r#"{"type":"frame","id":"f1"}"#);
    let summary = interactions_of(&node, false);
    assert!(summary.on_tap.is_empty());
    assert_eq!(summary.screen, None);
}

// ── screen marker, gated by `is_top_level` ──────────────────────────

#[test]
fn interactions_of_shows_screen_only_when_top_level() {
    let node = node_from(r#"{"type":"frame","id":"f1","screen":"/home"}"#);
    assert_eq!(
        interactions_of(&node, true).screen,
        Some("/home".to_string())
    );
    assert_eq!(interactions_of(&node, false).screen, None);
}

#[test]
fn interactions_of_never_shows_screen_for_non_frame() {
    // `screen` isn't a field on Rectangle at all — even `is_top_level`
    // true must not fabricate one.
    let node = node_from(r#"{"type":"rectangle","id":"r1"}"#);
    assert_eq!(interactions_of(&node, true).screen, None);
}

// ── Navigate/pop patch JSON (the write path) ────────────────────────

#[test]
fn navigate_patch_json_double_encodes_the_path_literal() {
    // The body must be the JSON string VALUE `"\"/profile\""` — quote
    // characters included — so it compiles as a Tier-1 string-literal
    // expression (see `wire_screen_navigation`'s contract).
    let patch = navigate_patch_json("/profile");
    assert_eq!(
        patch,
        r#"{"events":{"onTap":[{"replace":"\"/profile\""}]}}"#
    );
}

#[test]
fn navigate_patch_json_round_trips_through_interactions_of() {
    let path = "/settings/account";
    let patch = navigate_patch_json(path);
    let patched_node = format!(
        r#"{{"type":"frame","id":"f1",{}}}"#,
        &patch[1..patch.len() - 1]
    );
    let node = node_from(&patched_node);
    let summary = interactions_of(&node, false);
    assert_eq!(
        summary.on_tap[0],
        TapActionSummary::Navigate {
            verb: NavigateVerb::Replace,
            path: path.to_string(),
        }
    );
}

#[test]
fn pop_patch_json_matches_wire_screen_navigation_shape() {
    assert_eq!(POP_PATCH_JSON, r#"{"events":{"onTap":[{"pop":null}]}}"#);
}

// ── Section height + action-rect row math stay aligned ──────────────

#[test]
fn empty_state_has_one_clickable_row() {
    let summary = InteractionSummary::default();
    let mut out = Vec::new();
    push_interaction_action_rects(&mut out, &summary, 0.0, 0.0, 280.0);
    assert_eq!(out.len(), 1);
    assert!(matches!(
        out[0].0,
        PropertyPanelAction::ToggleInteractionMenu
    ));
}

#[test]
fn single_action_has_one_clickable_toggle_row() {
    let summary = InteractionSummary {
        screen: None,
        on_tap: vec![TapActionSummary::Pop],
    };
    let mut out = Vec::new();
    push_interaction_action_rects(&mut out, &summary, 0.0, 0.0, 280.0);
    assert_eq!(out.len(), 1);
    assert!(matches!(
        out[0].0,
        PropertyPanelAction::ToggleInteractionMenu
    ));
}

#[test]
fn multi_action_has_only_a_remove_all_row() {
    let summary = InteractionSummary {
        screen: None,
        on_tap: vec![
            TapActionSummary::Pop,
            TapActionSummary::Navigate {
                verb: NavigateVerb::Replace,
                path: "/a".to_string(),
            },
        ],
    };
    let mut out = Vec::new();
    push_interaction_action_rects(&mut out, &summary, 0.0, 0.0, 280.0);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0].0, PropertyPanelAction::RemoveInteraction));
}

#[test]
fn screen_row_adds_height_but_no_click_rect() {
    let with_screen = InteractionSummary {
        screen: Some("/home".to_string()),
        on_tap: Vec::new(),
    };
    let without_screen = InteractionSummary::default();
    assert!(
        interactions_section_height(&with_screen) > interactions_section_height(&without_screen)
    );
    let mut out = Vec::new();
    push_interaction_action_rects(&mut out, &with_screen, 0.0, 0.0, 280.0);
    // Still exactly one clickable row (the empty-state Add row) — the
    // Screen row contributes height only.
    assert_eq!(out.len(), 1);
}

// ── Navigate/Back/Remove popover rows ────────────────────────────────

#[test]
fn menu_rows_list_every_screen_path_then_back_then_remove() {
    let rows = interaction_menu_rows(
        op_editor_core::Locale::EnUs,
        &["/".to_string(), "/profile".to_string()],
        true,
    );
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows[0].0,
        PropertyPanelAction::SetInteractionNavigate {
            path: "/".to_string()
        }
    );
    assert_eq!(
        rows[1].0,
        PropertyPanelAction::SetInteractionNavigate {
            path: "/profile".to_string()
        }
    );
    assert_eq!(rows[2].0, PropertyPanelAction::SetInteractionPop);
    assert_eq!(rows[3].0, PropertyPanelAction::RemoveInteraction);
}

#[test]
fn menu_rows_omit_remove_when_not_removable() {
    let rows = interaction_menu_rows(op_editor_core::Locale::EnUs, &[], false);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, PropertyPanelAction::SetInteractionPop);
}

#[test]
fn menu_row_rects_are_stacked_and_click_target_matches_row() {
    let rows = interaction_menu_rows(op_editor_core::Locale::EnUs, &["/a".to_string()], true);
    let anchor = Rect {
        origin: Point2D::new(20.0, 100.0),
        size: Point2D::new(240.0, 30.0),
    };
    let menu = interaction_menu_rect(anchor, rows.len());
    let rects = interaction_menu_row_rects(menu, &rows);
    assert_eq!(rects.len(), rows.len());
    // Rows stack downward with no overlap.
    for pair in rects.windows(2) {
        assert!(pair[1].1.origin.y >= pair[0].1.origin.y + pair[0].1.size.y);
    }
}
