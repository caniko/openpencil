//! Worker pool for paint-recorded local image decode requests.

use crate::DesktopEvent;
use op_editor_ui::widgets::canvas_viewport_image::{
    cached_bytes_for, mark_decode_done, take_pending_decodes,
};
use op_host_native::{decode_raster, NativeBackend};
use skia_safe::{ConditionallySend, Sendable};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use winit::event_loop::EventLoopProxy;

const WORKER_COUNT: usize = 2;
const MAX_QUEUED_DECODES: usize = 4;

struct DecodeJob {
    id: u64,
    bytes: Arc<[u8]>,
}

struct DecodeResult {
    id: u64,
    image: Option<Sendable<skia_safe::Image>>,
}

pub struct ImageDecodeHost {
    job_tx: Option<Sender<DecodeJob>>,
    result_rx: Receiver<DecodeResult>,
    workers: Vec<JoinHandle<()>>,
    in_flight: usize,
    wake_proxy: Arc<Mutex<Option<EventLoopProxy<DesktopEvent>>>>,
}

impl ImageDecodeHost {
    pub fn new() -> Self {
        let (job_tx, job_rx) = mpsc::channel::<DecodeJob>();
        let (result_tx, result_rx) = mpsc::channel::<DecodeResult>();
        let job_rx = Arc::new(Mutex::new(job_rx));
        let wake_proxy = Arc::new(Mutex::new(None::<EventLoopProxy<DesktopEvent>>));
        let workers = (0..WORKER_COUNT)
            .map(|index| {
                let job_rx = Arc::clone(&job_rx);
                let result_tx = result_tx.clone();
                let wake_proxy = Arc::clone(&wake_proxy);
                std::thread::Builder::new()
                    .name(format!("op-image-decode-{index}"))
                    .spawn(move || decode_worker(job_rx, result_tx, wake_proxy))
                    .expect("spawn image decode worker")
            })
            .collect();
        Self {
            job_tx: Some(job_tx),
            result_rx,
            workers,
            in_flight: 0,
            wake_proxy,
        }
    }

    pub fn set_wake_proxy(&mut self, proxy: EventLoopProxy<DesktopEvent>) {
        if let Ok(mut wake) = self.wake_proxy.lock() {
            *wake = Some(proxy);
        }
    }

    pub fn is_pending(&self) -> bool {
        self.in_flight > 0
    }

    /// Install completed rasters, then submit at most four queued ids.
    pub fn pump(&mut self, backend: &mut NativeBackend) -> bool {
        let mut changed = self.poll_results(backend);
        let free = MAX_QUEUED_DECODES.saturating_sub(self.in_flight);
        for id in take_pending_decodes(free) {
            let Some(bytes) = cached_bytes_for(id) else {
                mark_decode_done(id);
                continue;
            };
            let Some(tx) = self.job_tx.as_ref() else {
                mark_decode_done(id);
                continue;
            };
            match tx.send(DecodeJob { id, bytes }) {
                Ok(()) => {
                    self.in_flight += 1;
                    changed = true;
                }
                Err(_) => mark_decode_done(id),
            }
        }
        changed
    }

    fn poll_results(&mut self, backend: &mut NativeBackend) -> bool {
        let mut changed = false;
        while let Ok(result) = self.result_rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            if let Some(image) = result.image {
                backend.install_raster_image(result.id, image.into_inner());
            }
            mark_decode_done(result.id);
            changed = true;
        }
        changed
    }
}

impl Drop for ImageDecodeHost {
    fn drop(&mut self) {
        self.job_tx.take();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

fn decode_worker(
    jobs: Arc<Mutex<Receiver<DecodeJob>>>,
    results: Sender<DecodeResult>,
    wake_proxy: Arc<Mutex<Option<EventLoopProxy<DesktopEvent>>>>,
) {
    loop {
        let job = match jobs.lock() {
            Ok(rx) => match rx.recv() {
                Ok(job) => job,
                Err(_) => break,
            },
            Err(_) => break,
        };
        let image = decode_raster(&job.bytes).and_then(|image| image.wrap_send().ok());
        if results.send(DecodeResult { id: job.id, image }).is_err() {
            break;
        }
        if let Some(proxy) = wake_proxy.lock().ok().and_then(|wake| wake.clone()) {
            let _ = proxy.send_event(DesktopEvent::ImageDecodeReady);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use op_editor_ui::widgets::canvas_viewport_image::{
        cached_bytes_for, mark_decode_done, note_pending_decode, store_remote_image_bytes,
        take_pending_decodes,
    };
    use op_host_native::NativeBackend;
    use std::time::{Duration, Instant};

    fn encode_test_png() -> Vec<u8> {
        let mut surface = skia_safe::surfaces::raster_n32_premul((3, 2)).unwrap();
        surface.canvas().clear(skia_safe::Color::BLUE);
        surface
            .image_snapshot()
            .encode(None, skia_safe::EncodedImageFormat::PNG, 100)
            .unwrap()
            .as_bytes()
            .to_vec()
    }

    #[test]
    fn worker_round_trip_decodes_and_installs_off_thread() {
        let id = 0xDEC0_DE01;
        for stale in take_pending_decodes(usize::MAX) {
            mark_decode_done(stale);
        }
        let png = encode_test_png();
        store_remote_image_bytes(id, png);
        assert!(cached_bytes_for(id).is_some());
        note_pending_decode(id);

        let mut host = ImageDecodeHost::new();
        let mut backend = NativeBackend::with_dpi(1.0);
        assert!(!backend.image_decoded(id, &[]));
        assert!(host.pump(&mut backend), "first pump submits queued decode");

        let deadline = Instant::now() + Duration::from_secs(5);
        while !backend.image_decoded(id, &[]) {
            host.pump(&mut backend);
            assert!(Instant::now() < deadline, "decode worker never landed");
            std::thread::yield_now();
        }
        let image = backend.raster_image(id).expect("installed raster");
        assert_eq!(image.dimensions(), (3, 2).into());
    }
}
