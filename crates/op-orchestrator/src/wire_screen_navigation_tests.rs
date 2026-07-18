//! Tests for Track A's deterministic screen/nav wiring pass.

use super::*;
use jian_ops_schema::PenDocument;
use op_editor_core::EditorState;

fn state_from_json(json: &str) -> EditorState {
    let doc: PenDocument = serde_json::from_str(json).expect("valid PenDocument");
    EditorState::from_document(doc)
}

fn find_by_id<'a>(nodes: &'a [PenNode], id: &str) -> Option<&'a PenNode> {
    for node in nodes {
        if node.id_str() == id {
            return Some(node);
        }
        if let Some(children) = node.children() {
            if let Some(found) = find_by_id(children, id) {
                return Some(found);
            }
        }
    }
    None
}

fn frame_screen(node: &PenNode) -> Option<&str> {
    match node {
        PenNode::Frame(f) => f.screen.as_deref(),
        _ => None,
    }
}

fn node_events_json(node: &PenNode) -> Option<serde_json::Value> {
    serde_json::to_value(node).ok()?.get("events").cloned()
}

fn run_pass(state: &mut EditorState) {
    let mut sink = crate::loop_finalize::StateDocSink { state };
    wire_screen_navigation(&mut sink);
}

// ── Pure-helper unit tests ─────────────────────────────────────────────

#[test]
fn normalize_slug_strips_non_ascii_and_hyphenates() {
    assert_eq!(normalize_slug("Profile Screen"), "profile-screen");
    assert_eq!(normalize_slug("  Settings!! "), "settings");
    assert_eq!(normalize_slug("首页"), ""); // all-CJK -> empty, caller falls back
}

#[test]
fn labels_match_exact_and_prefix() {
    assert!(labels_match("profile", "profile"));
    assert!(labels_match("profile", "profilescreen")); // "Profile" vs "Profile Screen"
    assert!(labels_match("profilescreen", "profile"));
    assert!(!labels_match("profile", "settings"));
    assert!(!labels_match("", "profile"));
}

/// Measured (`0718-1-glm-1.op`): screen names carry a brand prefix
/// ("Wander — Trips" / "Wander — Saved") — neither normalized form is a
/// prefix of the other ("trips" vs "wandertrips"), so every one of the
/// document's nav tabs (bare "Trips" / "Saved" labels) went unbound. The
/// token fallback must recover this WITHOUT opening the door to a bare
/// substring match.
#[test]
fn labels_match_token_fallback_for_brand_prefixed_names() {
    // The exact failure this was built from: brand-prefixed screen name,
    // bare tab label, either argument order.
    assert!(labels_match("Trips", "Wander — Trips"));
    assert!(labels_match("Wander — Trips", "Trips"));
    assert!(labels_match("Saved", "Wander — Saved"));

    // A bare SUBSTRING must never count as a token — "Roadtrips" tokenizes
    // to a single ["roadtrips"] token, which never equals "trips".
    assert!(!labels_match("Trips", "Roadtrips"));
    assert!(!labels_match("Trips", "Wander — Roadtrips"));

    // Multi-word tab label: "Your Library" should match a brand-prefixed
    // screen sharing either whole word as a token.
    assert!(labels_match("Your Library", "Wander — Library"));
    assert!(labels_match("Library", "Wander — Your Library"));

    // Still ambiguous when nothing at all is shared.
    assert!(!labels_match("Explore", "Wander — Trips"));
}

/// Contract point 1: the navigate body must be a Tier-1 string-LITERAL
/// expression source — a bare `/path` lexes as division and fails to
/// compile (`Expression::compile`). The correct wire form is exactly the
/// shape asserted by jian-core's own fixtures
/// (`jian-core/tests/action_navigation.rs::push_literal_path`,
/// `jian-ops-schema/src/events.rs::push_action_with_string_body`): the
/// JSON value under `"replace"`/`"push"` is the STRING `"\"/path\""` (i.e.
/// its decoded content is `"/path"`, quote characters included).
#[test]
fn navigate_patch_produces_expression_literal_body() {
    let patch = navigate_patch("replace", "/profile");
    assert_eq!(
        patch,
        r#"{"events":{"onTap":[{"replace":"\"/profile\""}]}}"#
    );
    // Round-trip through a real JSON parser to double-check the DECODED
    // action body equals the 10-char string `"/profile"` (quotes included) —
    // exactly what `jian_core::action::actions::navigation::expr_from_value`
    // feeds to `Expression::compile`.
    let v: serde_json::Value = serde_json::from_str(&patch).unwrap();
    let body = v["events"]["onTap"][0]["replace"].as_str().unwrap();
    assert_eq!(body, "\"/profile\"");
}

// ── Gate ────────────────────────────────────────────────────────────────

#[test]
fn single_screen_is_untouched_zero_new_keys() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    let before = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let after = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(
        after, before,
        "gate: <2 screen-shaped frames must be a no-op"
    );
    assert!(!after.contains("screen"), "no new `screen` key grown");
}

// ── Screen marking ──────────────────────────────────────────────────────

#[test]
fn two_screens_get_entry_and_slug_path() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[]},
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let home = find_by_id(state.active_children(), "home").unwrap();
    let profile = find_by_id(state.active_children(), "profile").unwrap();
    assert_eq!(frame_screen(home), Some("/"));
    assert_eq!(frame_screen(profile), Some("/profile"));
}

#[test]
fn entry_prefers_home_like_name_over_doc_order() {
    // "Profile" is first in document order, but "Dashboard" carries an
    // entry-name hint — Dashboard must win "/" regardless of position.
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[]},
        {"type":"frame","id":"dash","name":"Dashboard","width":1200,"height":900,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let profile = find_by_id(state.active_children(), "profile").unwrap();
    let dash = find_by_id(state.active_children(), "dash").unwrap();
    assert_eq!(frame_screen(dash), Some("/"));
    assert_eq!(frame_screen(profile), Some("/profile"));
}

#[test]
fn duplicate_slugs_get_numeric_suffix() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[]},
        {"type":"frame","id":"settings-a","name":"Settings","width":390,"height":844,
         "layout":"vertical","children":[]},
        {"type":"frame","id":"settings-b","name":"Settings","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let a = find_by_id(state.active_children(), "settings-a").unwrap();
    let b = find_by_id(state.active_children(), "settings-b").unwrap();
    // Doc-order first gets the bare slug, the second gets the `-2` suffix.
    assert_eq!(frame_screen(a), Some("/settings"));
    assert_eq!(frame_screen(b), Some("/settings-2"));
}

#[test]
fn authored_screen_marker_is_never_overwritten_and_not_reused_as_entry() {
    // "Checkout" already carries an authored (non-"/") marker. Since the
    // pass never touches authored markers even to satisfy the single-entry
    // rule, "/" is picked only among the UNMARKED candidates — here that's
    // "Extras", the only other one, despite its name matching no entry hint.
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"checkout","name":"Checkout","width":390,"height":844,
         "layout":"vertical","children":[],"screen":"/checkout"},
        {"type":"frame","id":"extras","name":"Extras","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let checkout = find_by_id(state.active_children(), "checkout").unwrap();
    let extras = find_by_id(state.active_children(), "extras").unwrap();
    assert_eq!(
        frame_screen(checkout),
        Some("/checkout"),
        "authored marker untouched"
    );
    assert_eq!(
        frame_screen(extras),
        Some("/"),
        "sole unmarked candidate becomes entry"
    );
}

#[test]
fn second_run_is_idempotent() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"nav-home","name":"Bottom Nav","role":"bottom-tab-bar",
             "width":"fill_container","height":56,"layout":"horizontal","children":[
                {"type":"frame","id":"tab-home","name":"HomeTab","width":80,"height":40,
                 "children":[{"type":"text","id":"tab-home-lbl","content":"Home"}]},
                {"type":"frame","id":"tab-profile","name":"ProfileTab","width":80,"height":40,
                 "children":[{"type":"text","id":"tab-profile-lbl","content":"Profile"}]}
             ]}
         ]},
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let once = serde_json::to_string(state.active_children()).unwrap();
    run_pass(&mut state);
    let twice = serde_json::to_string(state.active_children()).unwrap();
    assert_eq!(
        once, twice,
        "running the pass twice must be a no-op the second time"
    );
}

// ── Navbar wiring ───────────────────────────────────────────────────────

fn two_screen_bottom_nav_doc() -> &'static str {
    r#"{"version":"1.0","children":[
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"nav-home","name":"Bottom Nav","role":"bottom-tab-bar",
             "width":"fill_container","height":56,"layout":"horizontal","children":[
                {"type":"frame","id":"tab-home-in-home","name":"HomeTab","width":80,"height":40,
                 "children":[{"type":"text","id":"t1","content":"Home"}]},
                {"type":"frame","id":"tab-profile-in-home","name":"ProfileTab","width":80,"height":40,
                 "children":[{"type":"text","id":"t2","content":"Profile Screen"}]},
                {"type":"frame","id":"tab-settings-in-home","name":"SettingsTab","width":80,"height":40,
                 "children":[{"type":"text","id":"t3","content":"Settings"}]}
             ]}
         ]},
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"nav-profile","name":"Bottom Nav","role":"bottom-tab-bar",
             "width":"fill_container","height":56,"layout":"horizontal","children":[
                {"type":"frame","id":"tab-home-in-profile","name":"HomeTab","width":80,"height":40,
                 "children":[{"type":"text","id":"t4","content":"Home"}]},
                {"type":"frame","id":"tab-profile-in-profile","name":"ProfileTab","width":80,"height":40,
                 "children":[{"type":"text","id":"t5","content":"Profile Screen"}]}
             ]}
         ]}
    ]}"#
}

#[test]
fn bottom_nav_tabs_wire_to_matching_screens_including_self() {
    let mut state = state_from_json(two_screen_bottom_nav_doc());
    run_pass(&mut state);

    let home_path = frame_screen(find_by_id(state.active_children(), "home").unwrap())
        .unwrap()
        .to_string();
    let profile_path = frame_screen(find_by_id(state.active_children(), "profile").unwrap())
        .unwrap()
        .to_string();

    // Home screen's own navbar: Home tab points at itself, Profile tab at Profile.
    let home_tab_in_home = find_by_id(state.active_children(), "tab-home-in-home").unwrap();
    let profile_tab_in_home = find_by_id(state.active_children(), "tab-profile-in-home").unwrap();
    assert_eq!(
        node_events_json(home_tab_in_home).unwrap()["onTap"][0]["replace"],
        serde_json::json!(format!("\"{home_path}\""))
    );
    assert_eq!(
        node_events_json(profile_tab_in_home).unwrap()["onTap"][0]["replace"],
        serde_json::json!(format!("\"{profile_path}\""))
    );

    // Profile screen's own navbar wires the same way independently.
    let home_tab_in_profile = find_by_id(state.active_children(), "tab-home-in-profile").unwrap();
    let profile_tab_in_profile =
        find_by_id(state.active_children(), "tab-profile-in-profile").unwrap();
    assert_eq!(
        node_events_json(home_tab_in_profile).unwrap()["onTap"][0]["replace"],
        serde_json::json!(format!("\"{home_path}\""))
    );
    assert_eq!(
        node_events_json(profile_tab_in_profile).unwrap()["onTap"][0]["replace"],
        serde_json::json!(format!("\"{profile_path}\""))
    );
}

#[test]
fn mismatched_tab_label_is_not_bound() {
    let mut state = state_from_json(two_screen_bottom_nav_doc());
    run_pass(&mut state);
    // "Settings" has no matching screen (only Home/Profile exist) — its tab
    // must stay unbound rather than guess.
    let settings_tab = find_by_id(state.active_children(), "tab-settings-in-home").unwrap();
    assert!(node_events_json(settings_tab).is_none());
}

/// De-identified reproduction of `0718-1-glm-1.op`: three screens already
/// spread apart (x = 0 / 430 / 860, non-overlapping), each brand-prefixed
/// ("Wander — Trips" etc.), one bottom nav with bare tab labels ("Trips" /
/// "Saved" / "Explore" / "Profile" — the last two have no matching screen
/// in this 3-screen document, matching the real file). Before the token
/// fallback this bound ZERO tabs.
fn brand_prefixed_three_screen_doc() -> &'static str {
    r#"{"version":"1.0","children":[
        {"type":"frame","id":"trips","name":"Wander — Trips","x":0,"y":0,"width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"nav-trips","name":"Tab Bar","role":"bottom-tab-bar",
             "width":"fill_container","height":72,"layout":"horizontal","children":[
                {"type":"frame","id":"tab-trips","width":80,"height":40,
                 "children":[{"type":"text","id":"t1","content":"Trips"}]},
                {"type":"frame","id":"tab-saved","width":80,"height":40,
                 "children":[{"type":"text","id":"t2","content":"Saved"}]},
                {"type":"frame","id":"tab-explore","width":80,"height":40,
                 "children":[{"type":"text","id":"t3","content":"Explore"}]},
                {"type":"frame","id":"tab-profile","width":80,"height":40,
                 "children":[{"type":"text","id":"t4","content":"Profile"}]}
             ]}
         ]},
        {"type":"frame","id":"destination","name":"Wander — Destination","x":430,"y":0,"width":390,"height":844,
         "layout":"vertical","children":[]},
        {"type":"frame","id":"saved","name":"Wander — Saved","x":860,"y":0,"width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#
}

#[test]
fn brand_prefixed_screen_names_get_their_matching_tabs_bound() {
    let mut state = state_from_json(brand_prefixed_three_screen_doc());
    run_pass(&mut state);

    let trips_path = frame_screen(find_by_id(state.active_children(), "trips").unwrap())
        .unwrap()
        .to_string();
    let saved_path = frame_screen(find_by_id(state.active_children(), "saved").unwrap())
        .unwrap()
        .to_string();

    let trips_tab = find_by_id(state.active_children(), "tab-trips").unwrap();
    assert_eq!(
        node_events_json(trips_tab).unwrap()["onTap"][0]["replace"],
        serde_json::json!(format!("\"{trips_path}\"")),
        "\"Trips\" must bind to \"Wander — Trips\" via the token fallback"
    );
    let saved_tab = find_by_id(state.active_children(), "tab-saved").unwrap();
    assert_eq!(
        node_events_json(saved_tab).unwrap()["onTap"][0]["replace"],
        serde_json::json!(format!("\"{saved_path}\"")),
        "\"Saved\" must bind to \"Wander — Saved\" via the token fallback"
    );

    // "Explore" / "Profile" have no matching screen in this 3-screen
    // document (same as the real file) — must stay unbound, not guess.
    let explore_tab = find_by_id(state.active_children(), "tab-explore").unwrap();
    assert!(node_events_json(explore_tab).is_none());
    let profile_tab = find_by_id(state.active_children(), "tab-profile").unwrap();
    assert!(node_events_json(profile_tab).is_none());

    // Destination (a push-style detail screen, no bottom nav of its own)
    // still gets tagged even though it has nothing to bind.
    assert!(frame_screen(find_by_id(state.active_children(), "destination").unwrap()).is_some());
}

#[test]
fn existing_tab_events_are_left_alone() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"nav-home","name":"Bottom Nav","role":"bottom-tab-bar",
             "width":"fill_container","height":56,"layout":"horizontal","children":[
                {"type":"frame","id":"tab-profile","name":"ProfileTab","width":80,"height":40,
                 "events":{"onTap":[{"custom_action":null}]},
                 "children":[{"type":"text","id":"t1","content":"Profile"}]}
             ]}
         ]},
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    let before = node_events_json(find_by_id(state.active_children(), "tab-profile").unwrap());
    run_pass(&mut state);
    let after = node_events_json(find_by_id(state.active_children(), "tab-profile").unwrap());
    assert_eq!(
        after, before,
        "a node with existing events is never touched"
    );
}

// ── Back-button wiring ──────────────────────────────────────────────────

#[test]
fn header_back_icon_gets_pop() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"icon_font","id":"back-icon","name":"Back","iconFontName":"arrow-left",
             "width":24,"height":24}
         ]},
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let back = find_by_id(state.active_children(), "back-icon").unwrap();
    assert_eq!(
        node_events_json(back).unwrap(),
        serde_json::json!({"onTap": [{"pop": null}]})
    );
}

/// An authored interactive control owns its whole subtree: the arrow icon
/// INSIDE an already-bound back button must not receive a second `pop`
/// (both handlers firing on one tap would double-pop the route stack).
#[test]
fn icon_inside_authored_back_button_is_not_double_bound() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"back-btn","name":"Back Button","width":44,"height":44,
             "events":{"onTap":[{"pop":null}]},
             "children":[
                {"type":"icon_font","id":"inner-icon","name":"arrow-left","iconFontName":"arrow-left",
                 "width":24,"height":24}
             ]}
         ]},
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let icon = find_by_id(state.active_children(), "inner-icon").unwrap();
    assert!(
        node_events_json(icon).is_none(),
        "a descendant of an authored interactive control must never be wired"
    );
}

#[test]
fn back_icon_outside_header_region_is_not_bound() {
    let json = r#"{"version":"1.0","children":[
        {"type":"frame","id":"profile","name":"Profile","width":390,"height":844,
         "layout":"vertical","children":[
            {"type":"frame","id":"filler","name":"Filler","width":"fill_container","height":700,
             "children":[]},
            {"type":"icon_font","id":"chevron-icon","name":"chevron-left","iconFontName":"chevron-left",
             "width":24,"height":24}
         ]},
        {"type":"frame","id":"home","name":"Home","width":390,"height":844,
         "layout":"vertical","children":[]}
    ]}"#;
    let mut state = state_from_json(json);
    run_pass(&mut state);
    let chevron = find_by_id(state.active_children(), "chevron-icon").unwrap();
    assert!(
        node_events_json(chevron).is_none(),
        "a back-shaped icon well below the header band must not be bound"
    );
}
