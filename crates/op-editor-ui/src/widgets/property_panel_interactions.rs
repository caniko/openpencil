//! Interactions section — read-only screen marker + `events.onTap`
//! rows, plus the small Navigate/Back/Remove edit menu. New in this
//! change (the property panel previously showed no interaction
//! state at all, even though the generation pipeline
//! (`op-orchestrator::wire_screen_navigation`) has been wiring
//! `screen` + `events.onTap` into documents for a while).
//!
//! v1 scope, matching the plan this section ships against:
//! - `screen` is shown but not editable (a top-level screen frame's
//!   route path is read-only here; editing it is a v2 TODO).
//! - A single `onTap` action is editable via a small popover:
//!   "Navigate to <path>" (writes `replace`, one per authored
//!   `screen` on the current page) / "Back (pop)" / "Remove".
//! - Zero or one `onTap` action shows one clickable row; more than
//!   one shows every action read-only plus a single "Remove all"
//!   row (no per-action editing in v1 — see `InteractionSummary`).
//!
//! Data types (`InteractionSummary` / `TapActionSummary`) live here
//! rather than in `property_panel_snapshot.rs` (where every other
//! summary type lives) purely to avoid growing that already
//! 800-line-ceiling-busting file any further; the paint + geometry
//! helpers below follow the same section-file split as
//! `property_panel_effects.rs` / `property_panel_icon.rs`.

use crate::theme::Theme;
use crate::widgets::property_panel::PropertyPanelAction;
use crate::widgets::property_panel_inputs::{
    paint_dropdown, paint_section_divider, paint_section_label, INPUT_HEIGHT, INPUT_RADIUS, PAD_X,
    SECTION_GAP, SECTION_HEADER_HEIGHT,
};
use crate::widgets::PaintCx;
use crate::{Point2D, Rect, TextLayout};
use jian_ops_schema::events::Action;
use jian_ops_schema::node::PenNode;
use op_editor_core::pen_node_ext::PenNodeExt;

/// Which verb an authored Navigate action used. Manual edits from
/// this panel always write `Replace` (matches
/// `wire_screen_navigation`'s own tab-binding verb); `Push` is only
/// ever something a hand-authored or MCP-written document can carry,
/// shown read-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigateVerb {
    Push,
    Replace,
}

impl NavigateVerb {
    fn as_str(self) -> &'static str {
        match self {
            NavigateVerb::Push => "push",
            NavigateVerb::Replace => "replace",
        }
    }
}

/// One `events.onTap` action, summarized for display. `Other` covers
/// every action key besides navigation (`set`, `fetch`, …) — shown
/// as `Tap → <key>`, read-only (no v1 editor surface understands the
/// body of an arbitrary action).
#[derive(Debug, Clone, PartialEq)]
pub enum TapActionSummary {
    Navigate { verb: NavigateVerb, path: String },
    Pop,
    Other(String),
}

/// Interaction state for the selected node — always present (empty
/// when nothing is authored) so the section can paint its "+ Add
/// interaction" empty state. `screen` is `Some` only for a top-level
/// Frame carrying an authored `screen` route (see
/// `NodeSnapshot::from_node`'s caller, which gates this on the
/// selection being a page-root child).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InteractionSummary {
    pub screen: Option<String>,
    pub on_tap: Vec<TapActionSummary>,
}

/// Build the summary from a node's `events.onTap` + (when
/// `is_top_level`) its `screen` marker.
pub(crate) fn interactions_of(node: &PenNode, is_top_level: bool) -> InteractionSummary {
    InteractionSummary {
        screen: is_top_level
            .then(|| node.screen())
            .flatten()
            .map(str::to_string),
        on_tap: node
            .on_tap()
            .map(|actions| actions.iter().map(parse_tap_action).collect())
            .unwrap_or_default(),
    }
}

fn parse_tap_action(action: &Action) -> TapActionSummary {
    let Some((key, body)) = action.single() else {
        return TapActionSummary::Other(String::new());
    };
    match key {
        "push" => TapActionSummary::Navigate {
            verb: NavigateVerb::Push,
            path: unescape_path_literal(body),
        },
        "replace" => TapActionSummary::Navigate {
            verb: NavigateVerb::Replace,
            path: unescape_path_literal(body),
        },
        "pop" => TapActionSummary::Pop,
        other => TapActionSummary::Other(other.to_string()),
    }
}

/// `body` is the JSON string VALUE `"\"/path\""` (embedded quote
/// characters included) — the Tier-1 EXPRESSION source for a string
/// literal, per `wire_screen_navigation`'s navigate-patch contract
/// (`op-orchestrator/src/wire_screen_navigation.rs`). Strip the
/// embedded quote pair to recover the bare path for display; any
/// other shape (e.g. a hand-authored bare path with no expression
/// quoting) is shown as-is so the row never goes blank.
fn unescape_path_literal(body: &serde_json::Value) -> String {
    let Some(s) = body.as_str() else {
        return body.to_string();
    };
    s.strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(s)
        .to_string()
}

/// Build the `events.onTap` navigate patch JSON for this panel's
/// "Navigate to…" edit — always `replace` (v1 scope). `path` must
/// already be a `/`-rooted route path; the JSON string VALUE is the
/// literal `"<path>"` (quotes included) so it compiles as a Tier-1
/// string-literal expression. Constructed by the SAME double-encoding
/// `wire_screen_navigation::navigate_patch` uses, not hand-assembled,
/// so the two writers can never drift out of sync.
pub fn navigate_patch_json(path: &str) -> String {
    let body = serde_json::to_string(path).unwrap_or_default(); // -> "\"/path\""
    let escaped_body = serde_json::to_string(&body).unwrap_or_default(); // -> "\"\\\"/path\\\"\""
    format!(r#"{{"events":{{"onTap":[{{"replace":{escaped_body}}}]}}}}"#)
}

/// The `events.onTap` back (`pop`) patch JSON.
pub const POP_PATCH_JSON: &str = r#"{"events":{"onTap":[{"pop":null}]}}"#;

/// Scan the active page's top-level frames for authored `screen`
/// paths — the source list for the popover's "Navigate to <path>"
/// rows (`来源=扫当前页顶层 frame 的 screen 值`, not a fresh scan of
/// every possible screen-shaped frame the way
/// `wire_screen_navigation::collect_screen_candidates` does — this is
/// a read-only picker source, not a wiring pass). Order matches
/// document order.
pub fn document_screen_paths(state: &op_editor_core::EditorState) -> Vec<String> {
    state
        .active_children()
        .iter()
        .filter_map(|n| n.screen().map(str::to_string))
        .collect()
}

// ── Section paint + geometry ────────────────────────────────────────

/// Number of rows the section paints: an optional Screen row, plus
/// either one clickable row (empty / single action) or N read-only
/// rows + a trailing "Remove all" row (multiple actions). Shared by
/// the height calc + the action-rect walker so they can't drift.
fn interaction_row_count(interactions: &InteractionSummary) -> usize {
    let screen_row = usize::from(interactions.screen.is_some());
    let tap_rows = match interactions.on_tap.len() {
        0 | 1 => 1,
        n => n + 1,
    };
    screen_row + tap_rows
}

/// Total vertical space the Interactions section consumes. Mirrors
/// the `n * (INPUT_HEIGHT + 6.0) + 6.0 + SECTION_GAP` row-stacking
/// math `paint_interactions_section` walks.
pub fn interactions_section_height(interactions: &InteractionSummary) -> f32 {
    let n = interaction_row_count(interactions) as f32;
    SECTION_HEADER_HEIGHT + n * (INPUT_HEIGHT + 6.0) + 6.0 + SECTION_GAP
}

/// Emit the section's clickable-row rects: the empty-state / single
/// tap row (`ToggleInteractionMenu`), or a multi-action row's
/// trailing `RemoveInteraction` row. The Screen row and read-only
/// multi-action rows contribute height but no rect — matches
/// `paint_interactions_section` exactly.
pub fn push_interaction_action_rects(
    out: &mut Vec<(PropertyPanelAction, Rect)>,
    interactions: &InteractionSummary,
    x: f32,
    y: f32,
    width: f32,
) {
    let usable_w = width - PAD_X * 2.0;
    let mut row_y = y + SECTION_HEADER_HEIGHT;
    if interactions.screen.is_some() {
        row_y += INPUT_HEIGHT + 6.0;
    }
    let row_rect = |row_y: f32| Rect {
        origin: Point2D::new(x + PAD_X, row_y),
        size: Point2D::new(usable_w, INPUT_HEIGHT),
    };
    match interactions.on_tap.len() {
        0 | 1 => {
            out.push((PropertyPanelAction::ToggleInteractionMenu, row_rect(row_y)));
        }
        n => {
            row_y += (INPUT_HEIGHT + 6.0) * n as f32;
            out.push((PropertyPanelAction::RemoveInteraction, row_rect(row_y)));
        }
    }
}

pub fn paint_interactions_section(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    interactions: &InteractionSummary,
    locale: op_editor_core::Locale,
    x: f32,
    y: f32,
    width: f32,
) -> f32 {
    let mut y = paint_section_label(
        cx,
        theme,
        translate(locale, "interactions.title", "Interactions"),
        x,
        y,
        width,
    );
    let row_w = width - PAD_X * 2.0;
    let row_rect = |y: f32| Rect {
        origin: Point2D::new(x + PAD_X, y),
        size: Point2D::new(row_w, INPUT_HEIGHT),
    };
    if let Some(screen) = &interactions.screen {
        paint_plain_row(
            cx,
            theme,
            row_rect(y),
            &format!(
                "{}: {}",
                translate(locale, "interactions.screen", "Screen"),
                screen
            ),
        );
        y += INPUT_HEIGHT + 6.0;
    }
    match interactions.on_tap.len() {
        0 => {
            paint_dropdown(
                cx,
                theme,
                row_rect(y),
                translate(locale, "interactions.addInteraction", "+ Add interaction"),
            );
            y += INPUT_HEIGHT + 6.0;
        }
        1 => {
            paint_dropdown(
                cx,
                theme,
                row_rect(y),
                &tap_action_label(locale, &interactions.on_tap[0]),
            );
            y += INPUT_HEIGHT + 6.0;
        }
        _ => {
            for action in &interactions.on_tap {
                paint_plain_row(cx, theme, row_rect(y), &tap_action_label(locale, action));
                y += INPUT_HEIGHT + 6.0;
            }
            paint_dropdown(
                cx,
                theme,
                row_rect(y),
                translate(locale, "interactions.removeAll", "Remove all"),
            );
            y += INPUT_HEIGHT + 6.0;
        }
    }
    y += 6.0;
    paint_section_divider(cx, theme, x, y, width);
    y + SECTION_GAP
}

/// Human-readable row label for one authored `onTap` action, e.g.
/// `Tap → Navigate (replace) /profile` / `Tap → Back (pop)` /
/// `Tap → set` (unknown key, read-only).
fn tap_action_label(locale: op_editor_core::Locale, action: &TapActionSummary) -> String {
    let prefix = translate(locale, "interactions.tapPrefix", "Tap →");
    match action {
        TapActionSummary::Navigate { verb, path } => format!(
            "{prefix} {} ({}) {path}",
            translate(locale, "interactions.navigate", "Navigate"),
            verb.as_str(),
        ),
        TapActionSummary::Pop => {
            format!(
                "{prefix} {}",
                translate(locale, "interactions.back", "Back (pop)")
            )
        }
        TapActionSummary::Other(key) => format!("{prefix} {key}"),
    }
}

fn paint_plain_row(cx: &mut PaintCx<'_>, theme: &Theme, rect: Rect, text: &str) {
    let layout = TextLayout::single_run(
        text,
        "system-ui",
        12.0,
        (theme.muted_foreground).to_jian(),
        Point2D::new(0.0, 0.0),
    );
    cx.backend.draw_text(
        &layout,
        Point2D::new(rect.origin.x + 2.0, rect.origin.y + 19.0),
    );
}

// ── Navigate/Back/Remove edit menu ──────────────────────────────────

pub(crate) const INTERACTION_MENU_ROW_H: f32 = 30.0;
pub(crate) const INTERACTION_MENU_W: f32 = 200.0;

/// Hit-result for a click on the open Navigate/Back/Remove menu.
#[derive(Debug, Clone, PartialEq)]
pub enum InteractionMenuHit {
    Row(PropertyPanelAction),
    Inside,
    Outside,
}

/// Menu rows in paint order: one "Navigate to <path>" row per screen
/// path authored on the current page, then "Back (pop)", then
/// "Remove" (only when there is an existing action to remove).
pub(crate) fn interaction_menu_rows(
    locale: op_editor_core::Locale,
    screen_paths: &[String],
    removable: bool,
) -> Vec<(PropertyPanelAction, String)> {
    let navigate_to = translate(locale, "interactions.navigateTo", "Navigate to");
    let mut rows: Vec<(PropertyPanelAction, String)> = screen_paths
        .iter()
        .map(|path| {
            (
                PropertyPanelAction::SetInteractionNavigate { path: path.clone() },
                format!("{navigate_to} {path}"),
            )
        })
        .collect();
    rows.push((
        PropertyPanelAction::SetInteractionPop,
        translate(locale, "interactions.back", "Back (pop)").to_string(),
    ));
    if removable {
        rows.push((
            PropertyPanelAction::RemoveInteraction,
            translate(locale, "interactions.remove", "Remove").to_string(),
        ));
    }
    rows
}

/// The menu popover rect, anchored to the tap row's rect (drops just
/// below it, right-aligned — matches `effect_add_menu_rect`).
pub(crate) fn interaction_menu_rect(anchor: Rect, row_count: usize) -> Rect {
    let h = row_count as f32 * INTERACTION_MENU_ROW_H + 8.0;
    let right = anchor.origin.x + anchor.size.x;
    Rect {
        origin: Point2D::new(right - INTERACTION_MENU_W, anchor.origin.y + anchor.size.y),
        size: Point2D::new(INTERACTION_MENU_W, h),
    }
}

/// `(action, row_rect)` for each menu row, given the menu rect.
pub(crate) fn interaction_menu_row_rects(
    menu: Rect,
    rows: &[(PropertyPanelAction, String)],
) -> Vec<(PropertyPanelAction, Rect)> {
    rows.iter()
        .enumerate()
        .map(|(i, (action, _))| {
            let ry = menu.origin.y + 4.0 + i as f32 * INTERACTION_MENU_ROW_H;
            (
                action.clone(),
                Rect {
                    origin: Point2D::new(menu.origin.x, ry),
                    size: Point2D::new(menu.size.x, INTERACTION_MENU_ROW_H),
                },
            )
        })
        .collect()
}

/// Paint the Navigate/Back/Remove popover anchored to `anchor` (the
/// tap row's rect). `hover` highlights the row index under the
/// cursor, matching the Effects add-menu's `paint_effect_add_menu`.
pub(crate) fn paint_interaction_menu(
    cx: &mut PaintCx<'_>,
    theme: &Theme,
    anchor: Rect,
    rows: &[(PropertyPanelAction, String)],
    hover: Option<usize>,
) {
    let menu = interaction_menu_rect(anchor, rows.len());
    cx.backend
        .fill_round_rect(menu, INPUT_RADIUS, theme.popover);
    cx.backend
        .stroke_round_rect(menu, INPUT_RADIUS, theme.border, 1.0);
    for (i, (_, label)) in rows.iter().enumerate() {
        let ry = menu.origin.y + 4.0 + i as f32 * INTERACTION_MENU_ROW_H;
        if hover == Some(i) {
            let row = Rect {
                origin: Point2D::new(menu.origin.x + 4.0, ry),
                size: Point2D::new(menu.size.x - 8.0, INTERACTION_MENU_ROW_H),
            };
            cx.backend.fill_round_rect(row, 6.0, theme.muted);
        }
        let text = TextLayout::single_run(
            label,
            "system-ui",
            12.0,
            theme.foreground.to_jian(),
            Point2D::new(0.0, 0.0),
        );
        cx.backend.draw_text(
            &text,
            Point2D::new(
                menu.origin.x + 12.0,
                ry + INTERACTION_MENU_ROW_H / 2.0 + 4.0,
            ),
        );
    }
}

fn translate(
    locale: op_editor_core::Locale,
    key: &'static str,
    fallback: &'static str,
) -> &'static str {
    let translated = crate::i18n::translate(locale, key);
    if translated == key {
        fallback
    } else {
        translated
    }
}

#[cfg(test)]
#[path = "property_panel_interactions_tests.rs"]
mod tests;
