//! Tests for the manual per-subtask retry entry point — see
//! `retry_subtask.rs` module doc.

use super::*;
use crate::plan::Region;
use crate::test_support::{ScriptResponse, ScriptedLlm, VecDocSink};
use crate::types::AbortFlag;
use jian_ops_schema::PenDocument;

fn design_request() -> DesignRequest {
    DesignRequest {
        prompt: "retry test".into(),
        model: None,
        provider: None,
        design_md: None,
        concurrency: 1,
        append_context: None,
        validation_enabled: false,
        visual_ref_enabled: false,
    }
}

/// A live document whose sole top-level frame is the retry target's parent
/// — a different width/fill than the failed subtask's own `region`, so a
/// test can tell whether `plan_for_retry` derived its context from the
/// LIVE document (correct) or from the stale original subtask (wrong).
fn sink_with_root() -> VecDocSink {
    let doc: PenDocument = serde_json::from_str(
        r##"{ "version": "1.0", "children": [
            { "type": "frame", "id": "root", "name": "Home", "width": 1200, "height": 900,
              "layout": "vertical",
              "fill": [{ "type": "solid", "color": "#112233" }] }
        ] }"##,
    )
    .expect("doc");
    let mut sink = VecDocSink::new();
    sink.state = op_editor_core::EditorState::from_document(doc);
    sink
}

fn failed_subtask(parent_frame_id: Option<&str>) -> Subtask {
    Subtask {
        id: "browse-all-grid".into(),
        label: "Browse All Grid".into(),
        // Deliberately different from the live root (1200x900) so the
        // plan-derivation test can distinguish "read from the document"
        // from "read from the stale subtask region".
        region: Region {
            width: 390.0,
            height: 300.0,
        },
        id_prefix: "browse-all-grid".into(),
        parent_frame_id: parent_frame_id.map(str::to_string),
        elements: Some("A grid of browsable cards".into()),
        screen: Some("Home".into()),
        generated_root_id: None,
        existing_section_labels: None,
        retry_feedback: None,
    }
}

// Script-gen program text — same fixture shape `run_tests_d1.rs` uses.
fn node_json(prefix: &str) -> String {
    format!(
        r#"I(null, {{"type":"frame","name":"Sec","x":0,"y":0,"width":300,"height":120,"children":[{{"type":"text","content":"{prefix}","fontSize":18}}]}});"#
    )
}

#[test]
fn retry_succeeds_against_the_live_document_and_clears_the_subtask_field() {
    let mut sink = sink_with_root();
    let subtask = failed_subtask(Some("root"));
    let request = design_request();
    let llm = ScriptedLlm::new(vec![ScriptResponse::Text(node_json("retried"))]);
    let abort = AbortFlag::new();

    let outcome = futures::executor::block_on(retry_subtask(
        &subtask, &request, &llm, &mut sink, &abort, None, None,
    ));

    assert!(outcome.error.is_none(), "{outcome:?}");
    assert!(outcome.node_count > 0, "{outcome:?}");
    assert!(
        outcome.subtask.is_none(),
        "success must not carry the spec forward: {outcome:?}"
    );
    assert_eq!(outcome.id, "browse-all-grid");
}

#[test]
fn stale_parent_fails_fast_without_calling_the_llm() {
    struct PanicLlm;
    impl LlmClient for PanicLlm {
        fn call(
            &self,
            _req: crate::types::CallRequest,
        ) -> futures::stream::BoxStream<
            'static,
            Result<crate::types::LlmChunk, crate::types::LlmError>,
        > {
            panic!("retry_subtask must fail fast on a stale parent BEFORE calling the LLM");
        }
    }

    let mut sink = sink_with_root();
    // "gone-root" does not exist in `sink_with_root()`'s document — the
    // original run's cleanup could plausibly have replaced it.
    let subtask = failed_subtask(Some("gone-root"));
    let request = design_request();
    let llm = PanicLlm;
    let abort = AbortFlag::new();

    let outcome = futures::executor::block_on(retry_subtask(
        &subtask, &request, &llm, &mut sink, &abort, None, None,
    ));

    assert_eq!(outcome.node_count, 0);
    assert!(
        outcome.subtask.is_none(),
        "the fast-fail path returns a fresh outcome, not the persisted one: {outcome:?}"
    );
    let error = outcome.error.expect("stale parent must be reported");
    assert!(error.contains("gone-root"), "{error}");
}

#[test]
fn plan_for_retry_derives_root_frame_from_the_live_document_not_the_stale_subtask() {
    let sink = sink_with_root();
    // No parent needed for this check — only exercises `plan_for_retry`.
    let subtask = failed_subtask(None);

    let plan = plan_for_retry(&sink, &subtask);

    // Root context comes from the LIVE document's root (1200x900, #112233),
    // NOT the subtask's own (stale) region (390x300).
    assert_eq!(plan.root_frame.width, 1200.0);
    assert_eq!(plan.root_frame.height, 900.0);
    assert_eq!(
        plan.root_frame.first_solid_hex().as_deref(),
        Some("#112233")
    );

    // The subtask itself rides through completely unchanged — region,
    // elements, and screen must survive byte-for-byte.
    assert_eq!(plan.subtasks, vec![subtask.clone()]);
    assert_eq!(plan.subtasks[0].region.width, 390.0);
    assert_eq!(
        plan.subtasks[0].elements.as_deref(),
        Some(subtask.elements.as_deref().unwrap())
    );
    assert_eq!(plan.subtasks[0].screen.as_deref(), Some("Home"));
}
