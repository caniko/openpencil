//! multiscreen-fanout-break fix — end-to-end `Orchestrator::run()` tests for
//! the per-screen-group N-root scaffold (item A) and the inter-group
//! concurrent executor (item D-lite).
//!
//! Wired as `#[path = "run_tests_screen_groups.rs"] mod tests_screen_groups;`
//! inside `run.rs`; stays a child module of `run`, so `use super::*`
//! resolves to `run`.

use super::*;
use crate::test_support::{
    CountingLlm, DelayedLlm, ScriptResponse, ScriptedLlm, SkippedPreValidator,
    SkippedScreenshotProvider, SkippedVisionLlmClient, VecDocSink,
};
use crate::types::LlmError;
use jian_ops_schema::node::{PenNode, TextContent};

fn stub_providers() -> ValidationProviders<'static> {
    ValidationProviders {
        pre_validator: &SkippedPreValidator,
        screenshot: &SkippedScreenshotProvider,
        vision: &SkippedVisionLlmClient,
        system_prompt: String::new(),
    }
}

fn req() -> DesignRequest {
    DesignRequest {
        prompt: "continue generating the remaining screens".into(),
        model: None,
        provider: None,
        design_md: None,
        concurrency: 1,
        append_context: None,
        validation_enabled: false,

        visual_ref_enabled: false,
    }
}

fn req_with_concurrency(concurrency: u32) -> DesignRequest {
    DesignRequest {
        concurrency,
        ..req()
    }
}

const MULTI_SCREEN_PLAN_JSON: &str = r##"{
  "rootFrame": { "id": "root", "name": "App", "width": 390, "height": 844,
                 "layout": "vertical", "gap": 0,
                 "fill": [{ "type": "solid", "color": "#FFFFFF" }] },
  "subtasks": [
    { "id": "home-hero", "label": "Home Hero", "screen": "Home",
      "region": { "width": 390, "height": 300 } },
    { "id": "profile-hero", "label": "Profile Hero", "screen": "Profile",
      "region": { "width": 390, "height": 300 } }
  ]
}"##;

// Script-gen (the default protocol): `I(parent, obj)` calls, not raw
// `_parent` JSONL. Ids are dropped — batch_design reassigns fresh ones
// anyway; the "content" string is what survives verbatim and identifies
// which screen's subtask landed where.
fn node_json(marker: &str) -> String {
    format!(
        r#"I(null, {{"type":"frame","name":"Sec","x":0,"y":0,"width":390,"height":300,"children":[{{"type":"text","content":"{marker}","fontSize":18}}]}});"#
    )
}

fn contains_text(node: &PenNode, marker: &str) -> bool {
    if let PenNode::Text(t) = node {
        if let TextContent::Plain(s) = &t.content {
            if s == marker {
                return true;
            }
        }
    }
    node.children()
        .into_iter()
        .flatten()
        .any(|c| contains_text(c, marker))
}

fn screen_marker(node: &PenNode) -> Option<&str> {
    match node {
        PenNode::Frame(f) => f.screen.as_deref(),
        _ => None,
    }
}

/// Core multi-root regression: a plan whose subtasks span 2 distinct
/// `screen` labels must produce 2 SEPARATE top-level roots (not one flat
/// frame), correctly named, positioned side-by-side, and with each
/// subtask's content routed into its OWN screen's root — the exact
/// "continue generating the remaining screens" bug this fix addresses.
#[test]
fn multi_screen_plan_gets_one_root_per_screen_group() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(MULTI_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("home")),
        ScriptResponse::Text(node_json("profile")),
    ]);
    let mut sink = VecDocSink::new();

    let summary = futures::executor::block_on(Orchestrator::new().run(
        req(),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("multi-screen run ok");

    let roots = sink.state.active_children();
    assert_eq!(roots.len(), 2, "one root per screen group, got {roots:?}");

    assert_eq!(roots[0].base().name.as_deref(), Some("Home"));
    assert_eq!(roots[1].base().name.as_deref(), Some("Profile"));

    let x0 = roots[0].base().x.unwrap_or(0.0);
    let x1 = roots[1].base().x.unwrap_or(0.0);
    assert!(
        x1 >= x0 + 390.0 + 80.0 - 0.01,
        "second root must be laid out to the right of the first: x0={x0} x1={x1}"
    );
    assert_eq!(
        roots[0].base().y,
        roots[1].base().y,
        "sibling screens share the same top edge"
    );

    assert!(
        contains_text(&roots[0], "home"),
        "home content lands in the Home root"
    );
    assert!(
        !contains_text(&roots[0], "profile"),
        "profile content must not leak into the Home root"
    );
    assert!(
        contains_text(&roots[1], "profile"),
        "profile content lands in the Profile root"
    );
    assert!(
        !contains_text(&roots[1], "home"),
        "home content must not leak into the Profile root"
    );

    // First surviving root is the "primary" summary root, mirroring the
    // deleted concurrent path's own convention.
    assert_eq!(summary.root_frame_id, roots[0].id_str());
    assert_eq!(summary.subtasks.len(), 2);
}

/// Co-op point with Track A (`wire_screen_navigation`, run as part of
/// `finalize_design`'s cleanup tail): once N screen-shaped roots exist,
/// EVERY one of them — not just the first — gets a `screen` route marker,
/// proving the whole set of real (post-remap) root ids reached cleanup, not
/// just the first / primary one.
#[test]
fn multi_screen_plan_roots_all_get_wired_for_app_mode_preview() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(MULTI_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("home")),
        ScriptResponse::Text(node_json("profile")),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().run(
        req(),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("multi-screen run ok");

    let roots = sink.state.active_children();
    assert_eq!(roots.len(), 2);
    for root in roots {
        assert!(
            screen_marker(root).is_some(),
            "every screen-group root must come out of cleanup App-Mode-ready: {root:?}"
        );
    }
}

// ── Item D-lite: inter-group concurrency ────────────────────────────────

/// Three screens, matching the user-reported repro shape ("continue
/// generating Search/Library/Premium") — one subtask each, so each group's
/// worker runs exactly one LLM call.
const THREE_SCREEN_PLAN_JSON: &str = r##"{
  "rootFrame": { "id": "root", "name": "App", "width": 390, "height": 844,
                 "layout": "vertical", "gap": 0,
                 "fill": [{ "type": "solid", "color": "#FFFFFF" }] },
  "subtasks": [
    { "id": "search-body", "label": "Search Body", "screen": "Search",
      "region": { "width": 390, "height": 300 } },
    { "id": "library-body", "label": "Library Body", "screen": "Library",
      "region": { "width": 390, "height": 300 } },
    { "id": "premium-body", "label": "Premium Body", "screen": "Premium",
      "region": { "width": 390, "height": 300 } }
  ]
}"##;

fn find_root_by_name<'a>(roots: &'a [PenNode], name: &str) -> &'a PenNode {
    roots
        .iter()
        .find(|r| r.base().name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no root named {name} among {roots:?}"))
}

/// `effective_concurrency(3, 3) == 3`: all 3 screen groups get their own
/// worker permit, so the executor drives them together via `join_all`
/// instead of the sequential loop. Content-correctness only (the workers'
/// genuine wall-clock overlap is proven separately with `CountingLlm` — a
/// `ScriptedLlm`-backed run never truly yields, so it cannot itself
/// distinguish "ran concurrently" from "ran group-by-group").
#[test]
fn three_screen_groups_run_concurrently_and_land_correct_content() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();
    let mut progress = Vec::new();

    let summary = futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |p| progress.push(p),
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    let roots = sink.state.active_children();
    assert_eq!(roots.len(), 3, "one root per screen group, got {roots:?}");
    for name in ["Search", "Library", "Premium"] {
        let root = find_root_by_name(roots, name);
        assert!(
            root.children().is_some_and(|c| !c.is_empty()),
            "{name} root must have received its subtask's content"
        );
    }
    assert_eq!(summary.subtasks.len(), 3);
    assert_eq!(summary.total_nodes, 3);

    // Progress completeness: every subtask reaches a terminal state
    // (Started paired with Done/Failed), regardless of interleaving order.
    for id in ["search-body", "library-body", "premium-body"] {
        assert!(
            progress
                .iter()
                .any(|p| matches!(p, Progress::SubtaskStarted { id: i, .. } if i == id)),
            "{id} must report SubtaskStarted"
        );
        assert!(
            progress.iter().any(|p| matches!(p,
                Progress::SubtaskDone { id: i, .. } | Progress::SubtaskFailed { id: i, .. }
                    if i == id)),
            "{id} must report a terminal Done/Failed"
        );
    }
}

/// One screen group's subtask fails ALL attempts (main ladder + the
/// end-of-run salvage retry) while the other two groups succeed — the
/// failure must stay ISOLATED to its own root; it must never abort or empty
/// out its siblings' content, and its own (now-empty) scaffold root must
/// still survive (only an ALL-roots-empty run deletes scaffolding).
#[test]
fn one_group_failure_does_not_take_down_the_others() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        // "content blocked" is non-retryable (crate::retry::is_non_retryable)
        // — the ladder stops after attempt 1, consuming exactly one slot.
        ScriptResponse::Fail(LlmError {
            message: "content blocked by policy".into(),
            aborted: false,
        }),
        ScriptResponse::Text(node_json("p")),
        // The end-of-run salvage pass retries every zero-node subtask
        // regardless of why it failed — one more slot, kept failing.
        ScriptResponse::Fail(LlmError {
            message: "content blocked by policy".into(),
            aborted: false,
        }),
    ]);
    let mut sink = VecDocSink::new();

    let summary = futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("partial-failure run must still succeed overall");

    let roots = sink.state.active_children();
    assert_eq!(
        roots.len(),
        3,
        "a failed group's root is never deleted while siblings have content"
    );

    let search = find_root_by_name(roots, "Search");
    let library = find_root_by_name(roots, "Library");
    let premium = find_root_by_name(roots, "Premium");
    assert!(
        contains_text(search, "s"),
        "Search must be unaffected by Library's failure"
    );
    assert!(
        contains_text(premium, "p"),
        "Premium must be unaffected by Library's failure"
    );
    // Library's root still SURVIVES (not deleted — only an all-roots-empty
    // run deletes scaffolding) but never received subtask content — a
    // mobile scaffold root may carry pre-inserted chrome (e.g. a status
    // bar), so "failed" is checked by the ABSENCE of the subtask's own
    // marker text, not by raw emptiness.
    assert!(
        !contains_text(library, "l"),
        "Library's failed subtask must never backfill with the wrong content"
    );

    assert_eq!(summary.subtasks.len(), 3);
    assert!(
        summary.subtasks.iter().any(|o| o.node_count == 0),
        "exactly one subtask's final outcome stays zero-node"
    );
    assert_eq!(
        summary.subtasks.iter().filter(|o| o.node_count > 0).count(),
        2,
        "the other two subtasks succeed"
    );
}

/// Proves the workers genuinely overlap in wall-clock time (not just
/// "structurally concurrent code that happens to run group-by-group"):
/// `CountingLlm`'s stream yields `Pending` once before resolving, which lets
/// `join_all` poll a SIBLING worker while the first is still "in flight" —
/// `max_concurrent()` only exceeds 1 if the executor actually interleaves
/// them.
#[test]
fn concurrency_genuinely_overlaps_across_groups() {
    let llm = CountingLlm::new(vec![
        THREE_SCREEN_PLAN_JSON.into(),
        node_json("s"),
        node_json("l"),
        node_json("p"),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    assert!(
        llm.max_concurrent() > 1,
        "3 screen groups with concurrency=3 must overlap — max_concurrent() was {}",
        llm.max_concurrent()
    );
}

/// `concurrency=1` regression lock: even with 3 screen groups on the
/// canvas, a `request.concurrency` of 1 must take the UNTOUCHED sequential
/// loop — `effective_concurrency(1, 3) == 1` — so calls never overlap.
#[test]
fn concurrency_one_keeps_the_sequential_path_even_with_multiple_groups() {
    let llm = CountingLlm::new(vec![
        THREE_SCREEN_PLAN_JSON.into(),
        node_json("s"),
        node_json("l"),
        node_json("p"),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(1),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("sequential 3-group run ok");

    assert_eq!(
        llm.max_concurrent(),
        1,
        "concurrency=1 must never let two subtask calls overlap"
    );
    assert_eq!(sink.state.active_children().len(), 3);
}

// ── Three-piece visibility fix (2026-07-17) ─────────────────────────────

/// Order in which each of `markers` FIRST appears across `applied`'s
/// `InsertSubtree` commands — proves REPLAY order (which command actually
/// landed in the real sink first), independent of plan order.
fn insert_marker_order(applied: &[EditorCommand], markers: &[&str]) -> Vec<String> {
    let mut order = Vec::new();
    for cmd in applied {
        if let EditorCommand::InsertSubtree { nodes, .. } = cmd {
            for marker in markers {
                if nodes.iter().any(|n| contains_text(n, marker))
                    && !order.contains(&marker.to_string())
                {
                    order.push(marker.to_string());
                }
            }
        }
    }
    order
}

/// Item 2 ("按组完成即 replay"): the group whose LLM call resolves FIRST
/// replays into the real document FIRST, regardless of its plan index.
/// `library-body` is plan index 1 (after `search-body`), but `DelayedLlm`
/// makes its call resolve with zero extra polls while Search's takes many —
/// the replayed `InsertSubtree` order must show Library before Search.
#[test]
fn faster_group_replays_before_a_slower_earlier_plan_index_group() {
    let llm = DelayedLlm::new(vec![
        (0, THREE_SCREEN_PLAN_JSON.into()), // planning call — no delay
        (8, node_json("s")),                // search-body: many extra polls
        (0, node_json("l")),                // library-body: resolves immediately
        (4, node_json("p")),                // premium-body: some delay
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    // Final document is correct regardless of replay interleaving.
    let roots = sink.state.active_children();
    for name in ["Search", "Library", "Premium"] {
        find_root_by_name(roots, name);
    }

    let order = insert_marker_order(&sink.applied, &["s", "l", "p"]);
    let l_pos = order
        .iter()
        .position(|m| m == "l")
        .expect("library replayed");
    let s_pos = order
        .iter()
        .position(|m| m == "s")
        .expect("search replayed");
    assert!(
        l_pos < s_pos,
        "the faster (Library) group must replay before the slower, EARLIER-plan-index \
         (Search) group: replay order was {order:?}"
    );
}

/// Item 1 ("进度实时 drain"): `on_progress` must observe a group's terminal
/// event WHILE its siblings are still in flight — not only after every
/// worker has resolved. `DelayedLlm::finished()` (a counter incremented the
/// instant each call's stream produces its chunk, read from OUTSIDE the
/// callback) is what actually distinguishes "delivered live" from
/// "delivered late but in the right order" — an mpsc channel preserves send
/// order either way, so final-Vec ordering alone can't prove this.
#[test]
fn progress_is_delivered_while_slower_siblings_are_still_running() {
    let llm = std::sync::Arc::new(DelayedLlm::new(vec![
        (0, THREE_SCREEN_PLAN_JSON.into()),
        (10, node_json("s")),
        (0, node_json("l")),
        (6, node_json("p")),
    ]));
    let mut sink = VecDocSink::new();
    let llm_for_progress = std::sync::Arc::clone(&llm);
    // Snapshot `llm.finished()` at the moment Library's terminal event
    // arrives — if delivery is live, Search (delay=10) and Premium
    // (delay=6) cannot have resolved yet, so the count must be < 3.
    let mut finished_at_library_done: Option<usize> = None;

    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(3),
        &mut sink,
        &*llm,
        &mut |p| {
            if matches!(&p, Progress::SubtaskDone { id, .. } if id == "library-body") {
                finished_at_library_done = Some(llm_for_progress.finished());
            }
        },
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    let observed = finished_at_library_done.expect("library-body must report SubtaskDone");
    assert!(
        observed < 3,
        "Library's SubtaskDone must reach on_progress before every group's LLM call has \
         resolved (observed {observed}/3 finished) — a post-hoc batched drain would always \
         observe 3/3"
    );
}

/// Item 4: a fact-line precedes the concurrent phase so ⚡Nx's effect is
/// legible in the progress panel instead of silent.
#[test]
fn concurrent_run_announces_group_and_worker_count_upfront() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();
    let mut progress = Vec::new();

    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |p| progress.push(p),
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    assert!(
        progress.iter().any(|p| matches!(
            p,
            Progress::ConcurrentGroupsStarted {
                group_count: 3,
                workers: 3,
            }
        )),
        "expected a ConcurrentGroupsStarted{{group_count:3, workers:3}} event: {progress:?}"
    );
    // Never fires on the sequential path.
    let mut seq_progress = Vec::new();
    let seq_llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut seq_sink = VecDocSink::new();
    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(1),
        &mut seq_sink,
        &seq_llm,
        &mut |p| seq_progress.push(p),
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("sequential 3-group run ok");
    assert!(
        !seq_progress
            .iter()
            .any(|p| matches!(p, Progress::ConcurrentGroupsStarted { .. })),
        "concurrency=1 must never announce a concurrent phase"
    );
}

/// Item 3: when the groups genuinely run concurrently, each screen's root
/// gets its OWN distinct agent-indicator identity (colour + name) — not one
/// shared identity for all of them.
#[test]
fn concurrent_groups_get_distinct_agent_identities() {
    let _guard = crate::agent_indicator_test_support::lock();
    let epoch = op_editor_core::agent_indicators::begin();
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().with_indicator_epoch(epoch).run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    let roots = sink.state.active_children();
    let search_id = find_root_by_name(roots, "Search").id_str().to_string();
    let library_id = find_root_by_name(roots, "Library").id_str().to_string();
    let premium_id = find_root_by_name(roots, "Premium").id_str().to_string();

    let snap = op_editor_core::agent_indicators::snapshot();
    let search_tag = snap.frames.get(&search_id).expect("Search root tagged");
    let library_tag = snap.frames.get(&library_id).expect("Library root tagged");
    let premium_tag = snap.frames.get(&premium_id).expect("Premium root tagged");
    assert_ne!(
        search_tag, library_tag,
        "each group must get its OWN identity"
    );
    assert_ne!(search_tag, premium_tag);
    assert_ne!(library_tag, premium_tag);

    op_editor_core::agent_indicators::end_if_epoch(epoch);
}

/// Regression lock (dual-cursor-identity fix, 2026-07-17): the SEQUENTIAL
/// path (concurrency=1, even with multiple screen groups) must tag NO root
/// with its own identity — every reveal falls through to the host-confirmed
/// `cursor_agent`, the single source of truth for a single-agent run. Before
/// this fix the sequential path minted its OWN "Kiki" (seed-0) identity and
/// tagged every root with it — a second, orchestrator-only identity source
/// independent of the transcript's confirmed identity, which surfaced as two
/// simultaneous cursors once `canvas_agent_cursor.rs`'s ownership-tag
/// precedence was flipped (three-piece visibility fix) to stop hiding it.
#[test]
fn sequential_multi_screen_run_tags_no_frame_identity() {
    let _guard = crate::agent_indicator_test_support::lock();
    let epoch = op_editor_core::agent_indicators::begin();
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().with_indicator_epoch(epoch).run(
        req_with_concurrency(1),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("sequential 3-group run ok");

    let roots = sink.state.active_children();
    let search_id = find_root_by_name(roots, "Search").id_str().to_string();
    let library_id = find_root_by_name(roots, "Library").id_str().to_string();
    let premium_id = find_root_by_name(roots, "Premium").id_str().to_string();

    let snap = op_editor_core::agent_indicators::snapshot();
    assert!(
        snap.frames.is_empty(),
        "sequential path must tag NO root — orchestrator identity minting is \
         concurrent-only now: {:?}",
        snap.frames
    );
    // Simulate the host's transcript-identity confirmation (the ONE real
    // source for a single-agent run) and prove every root's waypoints
    // resolve to it once tagged this way — `canvas_agent_cursor.rs`'s
    // `confirmed` fallback for untagged nodes.
    op_editor_core::agent_indicators::confirm_cursor_agent(epoch, "#4ECDC4", "Nova");
    let confirmed = op_editor_core::agent_indicators::snapshot()
        .cursor_agent
        .expect("confirmed just now");
    assert_eq!(confirmed.name, "Nova");
    for id in [&search_id, &library_id, &premium_id] {
        assert!(
            !snap.frames.contains_key(id),
            "root {id} must stay untagged so it inherits the confirmed identity"
        );
    }

    op_editor_core::agent_indicators::end_if_epoch(epoch);
}

/// Dual-cursor-identity fix companion: when groups genuinely run
/// CONCURRENTLY, the orchestrator confirms the FIRST group's identity as the
/// canonical `cursor_agent` — so the transcript speaker (which the host
/// resolves from `cursor_agent` when nothing is stamped on the message yet)
/// matches one of the N visible group cursors (the primary/first group)
/// instead of an unrelated third persona.
#[test]
fn concurrent_run_confirms_primary_group_identity_as_cursor_agent() {
    let _guard = crate::agent_indicator_test_support::lock();
    let epoch = op_editor_core::agent_indicators::begin();
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().with_indicator_epoch(epoch).run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    let roots = sink.state.active_children();
    let search_id = find_root_by_name(roots, "Search").id_str().to_string();

    let snap = op_editor_core::agent_indicators::snapshot();
    let search_tag = snap.frames.get(&search_id).expect("Search root tagged");
    let confirmed = snap
        .cursor_agent
        .expect("concurrent phase must confirm a primary identity");
    assert_eq!(
        &confirmed, search_tag,
        "the confirmed cursor_agent must match the FIRST (primary) group's identity"
    );

    op_editor_core::agent_indicators::end_if_epoch(epoch);
}

/// Web-streaming half of the dual-cursor-identity fix (2026-07-17):
/// `web_chat_standard.rs` confirms + announces an identity to its SSE client
/// BEFORE calling `Orchestrator::run()` at all (the transcript needs a
/// persona immediately, well before groups/concurrency is known). This test
/// simulates exactly that — pre-confirm a `cursor_agent`, THEN run a
/// genuinely concurrent 3-group plan — and proves the primary group's
/// identity ADOPTS the pre-confirmed one instead of the orchestrator
/// silently overwriting it with a freshly minted "Kiki". Content-level
/// mirror of `agent_identity::tests::with_primary_keeps_the_primary_as_the_first_identity`,
/// exercised through the real `Orchestrator::run()` path.
#[test]
fn concurrent_run_adopts_a_pre_confirmed_identity_as_the_primary() {
    let _guard = crate::agent_indicator_test_support::lock();
    let epoch = op_editor_core::agent_indicators::begin();
    // Simulate the web route: confirm + (conceptually) stream this identity
    // to the client BEFORE `run()` is ever called.
    op_editor_core::agent_indicators::confirm_cursor_agent(epoch, "#4ECDC4", "Nova");

    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();

    futures::executor::block_on(Orchestrator::new().with_indicator_epoch(epoch).run(
        req_with_concurrency(3),
        &mut sink,
        &llm,
        &mut |_| {},
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("3-group concurrent run ok");

    let roots = sink.state.active_children();
    let search_id = find_root_by_name(roots, "Search").id_str().to_string();
    let library_id = find_root_by_name(roots, "Library").id_str().to_string();
    let premium_id = find_root_by_name(roots, "Premium").id_str().to_string();

    let snap = op_editor_core::agent_indicators::snapshot();
    assert_eq!(
        snap.cursor_agent,
        Some(op_editor_core::agent_indicators::AgentTag {
            color: "#4ECDC4".into(),
            name: "Nova".into(),
        }),
        "the pre-confirmed identity must survive the concurrent phase unchanged"
    );
    assert_eq!(
        snap.frames.get(&search_id),
        snap.cursor_agent.as_ref(),
        "the primary (first) group's tag must equal the adopted pre-confirmed identity"
    );
    // The other two groups still get their own DISTINCT identities.
    let library_tag = snap.frames.get(&library_id).expect("Library tagged");
    let premium_tag = snap.frames.get(&premium_id).expect("Premium tagged");
    assert_ne!(library_tag.color, "#4ECDC4");
    assert_ne!(premium_tag.color, "#4ECDC4");
    assert_ne!(library_tag.color, premium_tag.color);

    op_editor_core::agent_indicators::end_if_epoch(epoch);
}

// ── Sequential-execution self-diagnostic (2026-07-17 root-cause hunt) ──────

/// `ScreenGroupsSequential` fires exactly once, with the raw (unclamped)
/// `request.concurrency` the turn actually carried — the dual of
/// `concurrent_run_announces_group_and_worker_count_upfront`. This is the
/// line that lets a future real-world repro self-diagnose: if it shows
/// `requested_workers: 1`, the ⚡Nx picker's value never reached
/// `DesignRequest.concurrency` for that turn; `effective_concurrency`
/// itself is proven correct by `multi_group_is_capped_by_both_clamp_and_group_count`
/// (concurrent_tests.rs) — it CANNOT return 1 here unless `request.concurrency`
/// really was ≤1.
#[test]
fn sequential_run_announces_group_count_and_requested_workers() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(THREE_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("s")),
        ScriptResponse::Text(node_json("l")),
        ScriptResponse::Text(node_json("p")),
    ]);
    let mut sink = VecDocSink::new();
    let mut progress = Vec::new();

    futures::executor::block_on(Orchestrator::new().run(
        req_with_concurrency(1),
        &mut sink,
        &llm,
        &mut |p| progress.push(p),
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("sequential 3-group run ok");

    let hits: Vec<_> = progress
        .iter()
        .filter(|p| matches!(p, Progress::ScreenGroupsSequential { .. }))
        .collect();
    assert_eq!(hits.len(), 1, "must fire exactly once: {progress:?}");
    assert!(
        matches!(
            hits[0],
            Progress::ScreenGroupsSequential {
                group_count: 3,
                requested_workers: 1,
            }
        ),
        "expected group_count:3, requested_workers:1, got {:?}",
        hits[0]
    );
    // Never fires alongside the concurrent phase's own announcement.
    assert!(!progress
        .iter()
        .any(|p| matches!(p, Progress::ConcurrentGroupsStarted { .. })));
}

/// Regression: an ordinary single-screen plan (the overwhelming majority of
/// runs — no `screen` tags at all, `groups.len() <= 1`) must NEVER emit this
/// diagnostic line. It exists to flag an ANOMALY (multi-group plan that
/// still went sequential), not to narrate the always-sequential common case.
#[test]
fn single_screen_run_never_announces_the_sequential_diagnostic() {
    let llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(MULTI_SCREEN_PLAN_JSON.into()),
        ScriptResponse::Text(node_json("home")),
        ScriptResponse::Text(node_json("profile")),
    ]);
    let mut sink = VecDocSink::new();
    let mut progress = Vec::new();

    // MULTI_SCREEN_PLAN_JSON actually has 2 screens — use the plain `req()`
    // fixture with concurrency=1 (its default) so this exercises the
    // 2-group-but-sequential case too, alongside a genuinely single-group
    // plan below.
    futures::executor::block_on(Orchestrator::new().run(
        req(),
        &mut sink,
        &llm,
        &mut |p| progress.push(p),
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("2-group sequential run ok");
    assert!(
        progress
            .iter()
            .any(|p| matches!(p, Progress::ScreenGroupsSequential { group_count: 2, .. })),
        "a 2-group plan run sequentially SHOULD still announce (only single-group is exempt)"
    );

    // Genuinely single-group plan (no `screen` tags): must stay silent.
    let single_llm = ScriptedLlm::new(vec![
        ScriptResponse::Text(
            r##"{
  "rootFrame": { "id": "root", "name": "App", "width": 1200, "height": 800,
                 "layout": "vertical", "gap": 0,
                 "fill": [{ "type": "solid", "color": "#FFFFFF" }] },
  "subtasks": [
    { "id": "hero", "label": "Hero", "region": { "width": 1200, "height": 400 } }
  ]
}"##
            .into(),
        ),
        ScriptResponse::Text(node_json("hero")),
    ]);
    let mut single_sink = VecDocSink::new();
    let mut single_progress = Vec::new();
    futures::executor::block_on(Orchestrator::new().run(
        req(),
        &mut single_sink,
        &single_llm,
        &mut |p| single_progress.push(p),
        &AbortFlag::new(),
        &stub_providers(),
    ))
    .expect("single-group run ok");
    assert!(
        !single_progress
            .iter()
            .any(|p| matches!(p, Progress::ScreenGroupsSequential { .. })),
        "single-group plans must never announce: {single_progress:?}"
    );
}
