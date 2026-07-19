use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

const REPORT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivityState {
    Active,
    SettledIdle,
}

impl ActivityState {
    fn from_queue(in_flight: usize, pending: usize) -> Self {
        if in_flight == 0 && pending == 0 {
            Self::SettledIdle
        } else {
            Self::Active
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::SettledIdle => "settled/idle",
        }
    }
}

#[derive(Default)]
struct InstallTally {
    installed_ids: HashSet<u64>,
    installs: u64,
    reinstalls: u64,
}

impl InstallTally {
    /// Return whether this paint id has already been installed in this run.
    fn record(&mut self, id: u64) -> bool {
        let reinstalled = !self.installed_ids.insert(id);
        self.installs += 1;
        if reinstalled {
            self.reinstalls += 1;
        }
        reinstalled
    }

    fn totals(&self) -> (u64, u64) {
        (self.installs, self.reinstalls)
    }
}

#[derive(Default)]
struct RateWindow {
    previous_installs: u64,
    previous_reinstalls: u64,
}

impl RateWindow {
    fn delta(&mut self, installs: u64, reinstalls: u64) -> (u64, u64) {
        let delta = (
            installs.saturating_sub(self.previous_installs),
            reinstalls.saturating_sub(self.previous_reinstalls),
        );
        self.previous_installs = installs;
        self.previous_reinstalls = reinstalls;
        delta
    }
}

#[derive(Default)]
struct SharedCounters {
    installs: AtomicU64,
    reinstalls: AtomicU64,
    in_flight: AtomicUsize,
    pending: AtomicUsize,
}

/// Opt-in native decode telemetry. When disabled, the host retains only a
/// `None` and performs no hashing, queue locking, reporting, or thread spawn.
pub(crate) struct ImageDecodeStats {
    tally: InstallTally,
    shared: Arc<SharedCounters>,
    stop_tx: Option<Sender<()>>,
    reporter: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, Copy)]
#[cfg(test)]
pub(super) struct ImageDecodeStatsSnapshot {
    pub(super) installs: u64,
    pub(super) reinstalls: u64,
    pub(super) in_flight: usize,
    pub(super) pending: usize,
    pub(super) state: &'static str,
}

impl ImageDecodeStats {
    pub(crate) fn from_env() -> Option<Self> {
        if std::env::var("OP_IMAGE_DECODE_STATS").as_deref() != Ok("1") {
            return None;
        }
        let shared = Arc::new(SharedCounters::default());
        let reporter_shared = Arc::clone(&shared);
        let (stop_tx, stop_rx) = mpsc::channel();
        let reporter = std::thread::Builder::new()
            .name("op-image-decode-stats".into())
            .spawn(move || report_loop(reporter_shared, stop_rx))
            .ok()?;
        Some(Self {
            tally: InstallTally::default(),
            shared,
            stop_tx: Some(stop_tx),
            reporter: Some(reporter),
        })
    }

    pub(crate) fn record_install(&mut self, id: u64) {
        self.tally.record(id);
        let (installs, reinstalls) = self.tally.totals();
        self.shared.installs.store(installs, Ordering::Relaxed);
        self.shared.reinstalls.store(reinstalls, Ordering::Relaxed);
    }

    pub(crate) fn update_queue(&self, in_flight: usize, pending: usize) {
        self.shared.in_flight.store(in_flight, Ordering::Relaxed);
        self.shared.pending.store(pending, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(super) fn snapshot(&self) -> ImageDecodeStatsSnapshot {
        let in_flight = self.shared.in_flight.load(Ordering::Relaxed);
        let pending = self.shared.pending.load(Ordering::Relaxed);
        ImageDecodeStatsSnapshot {
            installs: self.shared.installs.load(Ordering::Relaxed),
            reinstalls: self.shared.reinstalls.load(Ordering::Relaxed),
            in_flight,
            pending,
            state: ActivityState::from_queue(in_flight, pending).as_str(),
        }
    }
}

impl Drop for ImageDecodeStats {
    fn drop(&mut self) {
        self.stop_tx.take();
        if let Some(reporter) = self.reporter.take() {
            let _ = reporter.join();
        }
    }
}

fn report_loop(shared: Arc<SharedCounters>, stop_rx: Receiver<()>) {
    let mut rates = RateWindow::default();
    while let Err(RecvTimeoutError::Timeout) = stop_rx.recv_timeout(REPORT_INTERVAL) {
        let installs = shared.installs.load(Ordering::Relaxed);
        let reinstalls = shared.reinstalls.load(Ordering::Relaxed);
        let in_flight = shared.in_flight.load(Ordering::Relaxed);
        let pending = shared.pending.load(Ordering::Relaxed);
        let (installs_per_second, reinstalls_per_second) = rates.delta(installs, reinstalls);
        let state = ActivityState::from_queue(in_flight, pending);
        eprintln!(
            "[image-decode-stats] installs/s={installs_per_second} \
             reinstalls/s={reinstalls_per_second} installs_total={installs} \
             reinstalls_total={reinstalls} in_flight={in_flight} pending={pending} \
             state={}",
            state.as_str()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_tally_distinguishes_reinstalls_by_paint_id() {
        let mut tally = InstallTally::default();

        assert!(!tally.record(41));
        assert!(!tally.record(42));
        assert!(tally.record(41));
        assert_eq!(tally.totals(), (3, 1));
    }

    #[test]
    fn rate_window_reports_interval_deltas() {
        let mut window = RateWindow::default();

        assert_eq!(window.delta(3, 1), (3, 1));
        assert_eq!(window.delta(5, 1), (2, 0));
    }

    #[test]
    fn activity_state_is_explicitly_settled_when_both_queues_are_empty() {
        assert_eq!(ActivityState::from_queue(2, 0), ActivityState::Active);
        assert_eq!(ActivityState::from_queue(0, 3), ActivityState::Active);
        assert_eq!(ActivityState::from_queue(0, 0), ActivityState::SettledIdle);
        assert_eq!(ActivityState::SettledIdle.as_str(), "settled/idle");
    }
}
