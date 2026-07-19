use super::*;
use std::sync::Mutex;

static THUMB_TEST_LOCK: Mutex<()> = Mutex::new(());

fn lock_thumbnail_registry() -> std::sync::MutexGuard<'static, ()> {
    THUMB_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn deeply_nested_document_with_thumb(paint_id: u64) -> String {
    let mut src =
        format!(r#"{{"version":"1.0.0","imageThumbs":{{"{paint_id}":"/9j/2Q=="}},"children":["#);
    for depth in 0..121 {
        src.push_str(&format!(
            r#"{{"type":"frame","id":"f-{depth}","name":"Frame","children":["#
        ));
    }
    for _ in 0..121 {
        src.push_str("]}");
    }
    src.push_str("]}");
    src
}

fn deeply_nested_invalid_document_with_thumb(paint_id: u64) -> String {
    let mut src =
        format!(r#"{{"version":"1.0.0","imageThumbs":{{"{paint_id}":"/9j/2Q=="}},"children":["#);
    for depth in 0..121 {
        src.push_str(&format!(
            r#"{{"type":"frame","id":"f-{depth}","name":"Frame","children":["#
        ));
    }
    src.push_str(r#"{"type":"not_a_node"}"#);
    for _ in 0..121 {
        src.push_str("]}");
    }
    src.push_str("]}");
    src
}

#[test]
fn deep_load_recognizes_image_thumbs_as_a_seeded_table() {
    let _guard = lock_thumbnail_registry();
    std::thread::Builder::new()
        .name("deep-thumb-loader-test".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let loaded = load_canonical(&deeply_nested_document_with_thumb(7_654_321))
                .expect("deep document with imageThumbs loads");
            assert!(
                !loaded.warnings.iter().any(|warning| matches!(
                    warning,
                    jian_ops_schema::LoadWarning::UnknownField { field, .. }
                        if field == "imageThumbs"
                )),
                "imageThumbs must be consumed by the deep-load seam"
            );
            assert_eq!(
                jian_ops_schema::image_thumbs::thumb_for(7_654_321),
                None,
                "deep parsing alone must not activate the thumbnail table"
            );
            let state = op_editor_core::EditorState::from_document(loaded.value);
            assert_eq!(
                &*jian_ops_schema::image_thumbs::thumb_for(7_654_321)
                    .expect("EditorState transition activates the deep seed"),
                &[0xff, 0xd8, 0xff, 0xd9]
            );
            std::mem::forget(state);
        })
        .expect("spawn deep-thumb-loader-test")
        .join()
        .expect("deep-thumb-loader-test completed");
}

#[test]
fn failed_deep_typed_load_preserves_the_thumbnail_registry() {
    let _guard = lock_thumbnail_registry();
    std::thread::Builder::new()
        .name("deep-thumb-transaction-test".into())
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let old_id = 7_654_322;
            let new_id = 7_654_323;
            jian_ops_schema::image_thumbs::store_thumb(old_id, vec![1, 2, 3]);

            assert!(
                load_canonical(&deeply_nested_invalid_document_with_thumb(new_id)).is_err(),
                "the invalid leaf must fail the deep typed parse"
            );
            assert_eq!(
                &*jian_ops_schema::image_thumbs::thumb_for(old_id)
                    .expect("prior thumbnail survives"),
                &[1, 2, 3]
            );
            assert!(
                jian_ops_schema::image_thumbs::thumb_for(new_id).is_none(),
                "the rejected deep document must not publish its thumbnail table"
            );
        })
        .expect("spawn deep-thumb-transaction-test")
        .join()
        .expect("deep-thumb-transaction-test completed");
}
