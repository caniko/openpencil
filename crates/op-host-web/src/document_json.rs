//! Saved-document JSON seams shared by the VS Code bridge.

use std::cell::RefCell;

use op_editor_core::PenDocument;

/// A typed document carrying a schema-owned pending thumbnail seed.
pub(crate) struct ParsedDocumentJson {
    document: Option<PenDocument>,
}

impl Drop for ParsedDocumentJson {
    fn drop(&mut self) {
        if let Some(document) = self.document.as_ref() {
            jian_ops_schema::image_thumbs::discard_for_document(document);
        }
    }
}

/// Parse `.op` JSON while preserving one shared allocation for each image
/// table payload. Thumbnail publication is deferred until the caller replaces
/// the live host document through [`with_borrowed_parsed_document`].
pub(crate) fn parse_document_json(json: &str) -> Result<ParsedDocumentJson, serde_json::Error> {
    let mut raw: serde_json::Value = serde_json::from_str(json)?;
    let pending_thumbs = jian_ops_schema::image_thumbs::take_pending_from_document(&mut raw);
    let table = jian_ops_schema::image_table::take_image_table(&mut raw);
    let mut document = jian_ops_schema::node::image_src::intern::with_load_scope(table, || {
        serde_json::from_value(raw)
    })?;
    jian_ops_schema::image_thumbs::attach_to_document(&mut document, pending_thumbs);
    Ok(ParsedDocumentJson {
        document: Some(document),
    })
}

/// Acquire a host replacement borrow and consume the parsed document.
///
/// The `EditorState` transition activates its associated seed before the
/// replacement callback can repaint. A failed borrow drops the document and
/// its pending association without changing the active registry.
pub(crate) fn with_borrowed_parsed_document<T, R>(
    host: &RefCell<T>,
    mut parsed: ParsedDocumentJson,
    replace: impl FnOnce(&mut T, PenDocument) -> R,
) -> Option<R> {
    let Ok(mut host) = host.try_borrow_mut() else {
        if let Some(document) = parsed.document.as_ref() {
            jian_ops_schema::image_thumbs::discard_for_document(document);
        }
        return None;
    };
    let document = parsed
        .document
        .take()
        .expect("parsed document is consumed at most once");
    Some(replace(&mut host, document))
}

/// Convert live inline document JSON to the compact on-disk form. Invalid
/// input passes through unchanged so snapshot error handling stays unchanged.
pub(crate) fn externalize_for_disk(doc_json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(doc_json) {
        Ok(mut value) => {
            jian_ops_schema::image_table::externalize_images(&mut value);
            value.to_string()
        }
        Err(_) => doc_json.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::Mutex;

    static THUMB_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_thumbnail_registry() -> std::sync::MutexGuard<'static, ()> {
        THUMB_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn web_document_load_commits_thumbnails_with_the_host_replacement() {
        let _guard = lock_thumbnail_registry();
        let paint_id = 8_675_309;
        let json = format!(
            r#"{{"version":"0.8.0","children":[],"imageThumbs":{{"{paint_id}":"/9j/2Q=="}}}}"#
        );
        let parsed = parse_document_json(&json).expect("valid web document");
        assert!(
            jian_ops_schema::image_thumbs::thumb_for(paint_id).is_none(),
            "parsing alone must not publish the pending document table"
        );

        let host = RefCell::new(op_editor_core::EditorState::new());
        assert!(with_borrowed_parsed_document(&host, parsed, |state, doc| {
            state.replace_document(doc);
            assert_eq!(
                &*jian_ops_schema::image_thumbs::thumb_for(paint_id)
                    .expect("replacement activates before repaint callback"),
                &[0xff, 0xd8, 0xff, 0xd9]
            );
        })
        .is_some());

        assert_eq!(
            &*jian_ops_schema::image_thumbs::thumb_for(paint_id).expect("seeded thumbnail"),
            &[0xff, 0xd8, 0xff, 0xd9]
        );
    }

    #[test]
    fn web_host_replacement_borrow_failure_preserves_thumbnail_registry() {
        let _guard = lock_thumbnail_registry();
        let old_id = 8_675_310;
        let new_id = 8_675_311;
        jian_ops_schema::image_thumbs::store_thumb(old_id, vec![1, 2, 3]);
        let json = format!(
            r#"{{"version":"0.8.0","children":[],"imageThumbs":{{"{new_id}":"/9j/2Q=="}}}}"#
        );
        let parsed = parse_document_json(&json).expect("valid web document");
        let host = RefCell::new(());
        let held = host.borrow_mut();

        assert!(with_borrowed_parsed_document(&host, parsed, |_, _| {
            panic!("replacement must not run while the host is borrowed")
        })
        .is_none());
        drop(held);
        assert_eq!(
            &*jian_ops_schema::image_thumbs::thumb_for(old_id).expect("prior thumbnail survives"),
            &[1, 2, 3]
        );
        assert!(
            jian_ops_schema::image_thumbs::thumb_for(new_id).is_none(),
            "a host borrow failure must not publish the parsed document table"
        );
    }

    #[test]
    fn failed_web_typed_parse_preserves_thumbnail_registry() {
        let _guard = lock_thumbnail_registry();
        let old_id = 8_675_312;
        let new_id = 8_675_313;
        jian_ops_schema::image_thumbs::store_thumb(old_id, vec![1, 2, 3]);
        let json = format!(
            r#"{{"version":"0.8.0","imageThumbs":{{"{new_id}":"/9j/2Q=="}},"children":[{{"type":"not_a_node"}}]}}"#
        );

        assert!(
            parse_document_json(&json).is_err(),
            "the invalid node must fail the typed parse"
        );
        assert_eq!(
            &*jian_ops_schema::image_thumbs::thumb_for(old_id).expect("prior thumbnail survives"),
            &[1, 2, 3]
        );
        assert!(
            jian_ops_schema::image_thumbs::thumb_for(new_id).is_none(),
            "a rejected web document must not publish its thumbnail table"
        );
    }
}
