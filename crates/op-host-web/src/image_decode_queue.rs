//! Native-testable bookkeeping for CanvasKit's frame-budgeted image decode drain.

use op_editor_ui::widgets::canvas_viewport_image::{
    cached_bytes_for, mark_decode_done, mark_decode_failed, take_pending_decodes,
};
use std::sync::Arc;

pub(crate) fn take_web_decode_batch(max: usize) -> Vec<(u64, Arc<[u8]>)> {
    take_pending_decodes(max)
        .into_iter()
        .filter_map(|id| match cached_bytes_for(id) {
            Some(bytes) => Some((id, bytes)),
            None => {
                mark_decode_done(id);
                None
            }
        })
        .collect()
}

pub(crate) fn finish_web_decode(id: u64, decoded: bool) {
    if decoded {
        mark_decode_done(id);
    } else {
        mark_decode_failed(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use op_editor_ui::widgets::canvas_viewport_image::{
        has_pending_decodes, note_pending_decode, store_remote_image_bytes,
    };
    use std::sync::Mutex;

    static DECODE_REGISTRY_LOCK: Mutex<()> = Mutex::new(());

    fn clear_pending_work() {
        for id in take_pending_decodes(usize::MAX) {
            mark_decode_done(id);
        }
    }

    #[test]
    fn web_decode_batch_takes_at_most_two_and_keeps_remaining_work_queued() {
        let _guard = DECODE_REGISTRY_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        clear_pending_work();
        for id in 0xC100..0xC103 {
            store_remote_image_bytes(id, vec![id as u8]);
            note_pending_decode(id);
        }

        let batch = take_web_decode_batch(2);

        assert_eq!(batch.len(), 2);
        assert_eq!(batch[0].0, 0xC100);
        assert_eq!(batch[1].0, 0xC101);
        assert!(has_pending_decodes());
        for (id, _) in batch {
            finish_web_decode(id, true);
        }
        clear_pending_work();
    }

    #[test]
    fn web_decode_failure_is_not_queued_again() {
        let _guard = DECODE_REGISTRY_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        clear_pending_work();
        let id = 0xC1_BAD;
        store_remote_image_bytes(id, b"not an encoded image".to_vec());
        note_pending_decode(id);
        let batch = take_web_decode_batch(2);
        assert_eq!(batch.len(), 1);

        finish_web_decode(id, false);
        note_pending_decode(id);

        assert!(
            take_web_decode_batch(2).is_empty(),
            "a failed bridge decode must remain negatively cached"
        );
    }
}
