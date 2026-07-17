//! multiscreen-fanout-break fix (item A) tests for `plan_normalize::normalize`'s
//! screen-grouping. Split into its own sibling file (rather than growing the
//! inline `mod tests { ... }` in `plan_normalize.rs`, already near the
//! 800-line cap) — self-contained fixture helpers mirror that module's
//! `req` / `subtask` / `plan` exactly.

use super::*;
use crate::plan::{OrchestratorPlan, Region, RootFrameSpec, Subtask};
use crate::types::DesignRequest;

fn req() -> DesignRequest {
    DesignRequest {
        prompt: "x".into(),
        model: None,
        provider: None,
        design_md: None,
        concurrency: 1,
        append_context: None,
        validation_enabled: true,

        visual_ref_enabled: false,
    }
}

fn subtask_with_screen(id: &str, label: &str, screen: Option<&str>) -> Subtask {
    Subtask {
        id: id.into(),
        label: label.into(),
        region: Region {
            width: 100.0,
            height: 100.0,
        },
        id_prefix: String::new(),
        parent_frame_id: None,
        elements: None,
        screen: screen.map(str::to_string),
        generated_root_id: None,
        existing_section_labels: None,
    }
}

fn plan(width: f64, subtasks: Vec<Subtask>) -> OrchestratorPlan {
    OrchestratorPlan {
        root_frame: RootFrameSpec {
            id: "root".into(),
            name: "P".into(),
            width,
            height: 800.0,
            layout: None,
            gap: None,
            padding: None,
            fill: None,
        },
        subtasks,
        style_guide_name: None,
    }
}

/// multiscreen-fanout-break regression lock: ≥2 distinct `screen` labels
/// must NOT collapse onto the shared `root_id` — each group gets its own
/// placeholder id, and every group's subtasks share it.
#[test]
fn normalize_groups_subtasks_by_screen_when_multiple_screens_present() {
    let mut p = plan(
        1200.0,
        vec![
            subtask_with_screen("home-hero", "Home Hero", Some("Home")),
            subtask_with_screen("profile-hero", "Profile Hero", Some("Profile")),
            subtask_with_screen("home-feat", "Home Features", Some("Home")),
        ],
    );
    normalize(&mut p, &req());

    let home_parent = p.subtasks[0].parent_frame_id.clone();
    let profile_parent = p.subtasks[1].parent_frame_id.clone();
    assert_ne!(
        home_parent, profile_parent,
        "distinct screens must get distinct placeholder roots"
    );
    assert_ne!(
        home_parent.as_deref(),
        Some("root"),
        "must NOT be the shared root_id"
    );
    assert_eq!(
        p.subtasks[2].parent_frame_id, home_parent,
        "same-screen subtasks share their group's root"
    );
}

/// Regression lock (spec point 2, zero-tags case): no subtask carries a
/// `screen` label → single-root behavior stays byte-identical to today.
#[test]
fn normalize_zero_screen_labels_keeps_single_root() {
    let mut p = plan(
        1200.0,
        vec![
            subtask_with_screen("hero", "Hero", None),
            subtask_with_screen("feat", "Features", None),
        ],
    );
    normalize(&mut p, &req());
    for st in &p.subtasks {
        assert_eq!(st.parent_frame_id.as_deref(), Some("root"));
    }
}

/// Regression lock (spec point 2, all-same-tag case): every subtask tagged
/// with the SAME screen must also stay single-root — grouping only fans out
/// on ≥2 DISTINCT screen values.
#[test]
fn normalize_all_same_screen_label_keeps_single_root() {
    let mut p = plan(
        1200.0,
        vec![
            subtask_with_screen("hero", "Hero", Some("Home")),
            subtask_with_screen("feat", "Features", Some("Home")),
        ],
    );
    normalize(&mut p, &req());
    for st in &p.subtasks {
        assert_eq!(st.parent_frame_id.as_deref(), Some("root"));
    }
}
