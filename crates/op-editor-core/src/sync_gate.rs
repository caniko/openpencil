//! Pure sync-concurrency state machine shared by the periodic push tick,
//! the pull tick, and the bridge snapshot path. Baseline semantics: the
//! gate remembers the exact (generation, revision) pair last known to
//! match the daemon (after a pull apply, or after a push confirmation —
//! carrying the pair captured AT SERIALIZATION TIME, never "now"). Any
//! current pair differing from the baseline means unpushed local edits:
//! pulls are gated and a push is due. An oversize skip, failed POST, or
//! version conflict simply never advances the baseline, so the gate
//! cannot reopen and remote state cannot overwrite local edits.

/// Bytes cap for the periodic push channel (see [`SyncGate::periodic_push_allowed`]).
const PERIODIC_PUSH_CAP_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct SyncGate {
    last_synced: Option<(u64, u64)>, // (generation, revision); None = never synced
    conflict: Option<u64>,           // server version at conflict time
    // `opened` not yet emittable; holds the open's TARGET GENERATION —
    // open completion is generation-scoped so a pre-existing in-flight
    // push (whose confirmation carries a pair serialized BEFORE the open's
    // replace_document, i.e. an older generation) can never complete an
    // unrelated pending open prematurely.
    open_pending: Option<u64>,
    open_pull_block: bool, // pulls blocked while the open's probe/push is in flight
    // AcceptRemote resolution records the pair observed at resolve time AND
    // retains the conflict's server version; the resolving pull is allowed
    // ONLY while the pair is unchanged, so edits landing between backup and
    // pull-apply cannot be overwritten — and a broken window can re-enter
    // the conflict flow with the retained version (no wedged state).
    accept_expected: Option<((u64, u64), u64)>, // (expected pair, server version)
    // CONSUMABLE edge latches, distinct from the condition fields above: a
    // compare-based observer (cached-vs-current) misses edges that rise AND
    // fall between two ticks (e.g. a fast open completing, or a conflict
    // resolved before the observer ran). These latch the event so it can be
    // drained exactly once, independent of whatever the "current" state (
    // `open_pending`/`conflict`) happens to read at observation time.
    opened_edge: Option<u64>,   // generation to announce via `opened`
    conflict_edge: Option<u64>, // server version to announce via `sync-conflict`
}

impl SyncGate {
    /// After a pull apply (new baseline = post-apply pair) or a push
    /// confirmation (baseline = the pair serialized into that push).
    /// Clears conflict + accept_expected + any UNDRAINED conflict latch
    /// (a resolved conflict must not be announced); clears
    /// open_pending/open_pull_block ONLY when `generation >= open target
    /// generation` (older confirmations leave the pending open untouched).
    pub fn note_synced(&mut self, generation: u64, revision: u64) {
        self.last_synced = Some((generation, revision));
        self.conflict = None;
        self.accept_expected = None;
        self.conflict_edge = None;
        if let Some(target) = self.open_pending {
            if generation >= target {
                self.open_pending = None;
                self.open_pull_block = false;
                // Latch the NOW-LIVE generation, not the open's original
                // target — they differ when an AcceptRemote pull-apply
                // completes the open with a newer replacement generation.
                self.opened_edge = Some(generation);
            }
        }
    }

    /// Also clears accept_expected.
    pub fn note_conflict(&mut self, server_version: u64) {
        self.conflict = Some(server_version);
        self.accept_expected = None;
        self.conflict_edge = Some(server_version);
    }

    /// Set SYNCHRONOUSLY in the OpenDocument prologue, right after
    /// replace_document minted the new generation and BEFORE the first
    /// await (sets open_pending = Some(target_generation) + open_pull_block):
    /// during bootstrap the baseline is None so pull_allowed would be true —
    /// without the block a 400 ms pull can overwrite the just-replaced local
    /// document while the open's probe/push is in flight.
    pub fn note_open_pending(&mut self, target_generation: u64) {
        self.open_pending = Some(target_generation);
        self.open_pull_block = true;
    }

    pub fn open_pending(&self) -> bool {
        self.open_pending.is_some()
    }

    /// Accept-remote resolution: forget the baseline, clear the conflict
    /// AND any undrained conflict latch (retaining the server version
    /// inside accept_expected), lift open_pull_block, and record `current`
    /// as the expected pair for the resolving pull — but KEEP open_pending:
    /// it clears when that pull's apply calls note_synced (bridge's cue to
    /// emit `opened`).
    pub fn resolve_accept_remote(&mut self, current: (u64, u64)) {
        let server_version = self.conflict.take();
        self.last_synced = None;
        self.conflict_edge = None;
        self.open_pull_block = false;
        self.accept_expected = server_version.map(|v| (current, v));
    }

    /// Wiring escape hatch for a broken accept window: returns the retained
    /// server version when accept_expected is set and `current` moved past
    /// it (an edit landed before the resolving pull applied). The pull tick
    /// checks this; Some(v) means "re-enter the conflict flow": call
    /// note_conflict(v) (which clears accept_expected) so the user decides
    /// again — the gate can never wedge at baseline=None/conflict=None.
    pub fn accept_window_broken(&self, current: (u64, u64)) -> Option<u64> {
        match self.accept_expected {
            Some((expected, server_version)) if current != expected => Some(server_version),
            _ => None,
        }
    }

    /// CONSUMABLE edge latches — the bridge's tick observer drains these
    /// instead of comparing cached values (a compare-based observer misses
    /// edges that rise AND fall between ticks, e.g. a fast open completing
    /// before the next tick; a latch cannot lose the event):
    /// set by note_synced when it clears a pending open. Holds the
    /// GENERATION PASSED TO note_synced — i.e. the now-live document's
    /// generation, NOT the open's original target (they differ when an
    /// AcceptRemote pull-apply completes the open with a newer replacement
    /// generation; `opened` must name the document actually on screen).
    pub fn take_opened_edge(&mut self) -> Option<u64> {
        self.opened_edge.take()
    }

    /// set by note_conflict (holds the server version) — drain to emit
    /// `sync-conflict`. CLEARED by note_synced and resolve_accept_remote:
    /// a conflict resolved before the observer drained the latch must NOT
    /// be announced (stale emission would pop a ghost dialog on the host).
    /// A back-to-back second conflict simply sets the latch again after
    /// the resolution cleared it — still reported.
    pub fn take_conflict_edge(&mut self) -> Option<u64> {
        self.conflict_edge.take()
    }

    /// Pulls allowed when open_pull_block is clear, no conflict pending,
    /// AND no unpushed local edits. None baseline (mount) allows the
    /// initial pull — EXCEPT when accept_expected is set: then the pull is
    /// allowed only while `current == accept_expected` (an intervening edit
    /// moves the pair; the wiring observes the blocked pull and re-notes
    /// the conflict so the user decides again).
    pub fn pull_allowed(&self, current: (u64, u64)) -> bool {
        if self.open_pull_block || self.conflict.is_some() {
            return false;
        }
        if let Some((expected, _)) = self.accept_expected {
            return current == expected;
        }
        match self.last_synced {
            None => true,
            Some(baseline) => current == baseline,
        }
    }

    /// Push due when NO conflict is pending (a stored conflict suspends the
    /// periodic retry until explicit resolution — spec's "no silent retry"),
    /// a baseline exists (daemon-authority: never push before the first
    /// apply), and the current pair moved past it.
    pub fn needs_push(&self, current: (u64, u64)) -> bool {
        if self.conflict.is_some() {
            return false;
        }
        match self.last_synced {
            None => false,
            Some(baseline) => current != baseline,
        }
    }

    pub fn conflict(&self) -> Option<u64> {
        self.conflict
    }

    /// len <= 2 MiB cap.
    pub fn periodic_push_allowed(len: usize) -> bool {
        len <= PERIODIC_PUSH_CAP_BYTES
    }

    /// Uncapped.
    pub fn snapshot_push_allowed(_len: usize) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_during_inflight_push_stays_gated() {
        let mut g = SyncGate::default();
        assert!(g.pull_allowed((1, 0))); // mount: initial pull allowed
        g.note_synced(1, 0); // first apply baselines
                             // edit A -> revision 1: push due, pull gated
        assert!(g.needs_push((1, 1)));
        assert!(!g.pull_allowed((1, 1)));
        // push A in flight; edit B bumps revision to 2; A's confirmation
        // carries the pair captured at serialization time (1, 1):
        g.note_synced(1, 1);
        assert!(!g.pull_allowed((1, 2))); // B still unpushed — gate stays closed
        assert!(g.needs_push((1, 2)));
        g.note_synced(1, 2); // push B confirmed
        assert!(g.pull_allowed((1, 2)));
    }

    #[test]
    fn ui_only_changes_do_not_close_the_gate() {
        // revision is content-only (state.rs): an unchanged pair keeps pulls open
        let mut g = SyncGate::default();
        g.note_synced(1, 5);
        assert!(g.pull_allowed((1, 5)));
        assert!(!g.needs_push((1, 5))); // no deadlock: nothing to push, gate open
    }

    #[test]
    fn never_pushes_before_first_sync() {
        let g = SyncGate::default();
        assert!(!g.needs_push((0, 3))); // daemon authority preserved
    }

    #[test]
    fn open_pending_gates_pulls_through_bootstrap() {
        let mut g = SyncGate::default();
        assert!(g.pull_allowed((1, 0))); // bootstrap: baseline None, pulls open
        g.note_open_pending(2); // prologue: replace_document minted gen 2
        assert!(!g.pull_allowed((2, 0))); // pull may not overwrite the open
        g.note_synced(2, 0); // open's own push confirmed
        assert!(!g.open_pending());
        assert!(g.pull_allowed((2, 0)));
    }

    #[test]
    fn fast_open_completion_is_latched_not_lost() {
        // open rises AND completes between two observer ticks — a compare-based
        // observer would see false→false and drop `opened`; the latch cannot:
        let mut g = SyncGate::default();
        g.note_open_pending(2);
        g.note_synced(2, 0);
        assert_eq!(g.take_opened_edge(), Some(2)); // observer drains exactly once
        assert_eq!(g.take_opened_edge(), None);
    }

    #[test]
    fn back_to_back_conflicts_both_latch() {
        let mut g = SyncGate::default();
        g.note_synced(1, 0);
        g.note_conflict(7);
        assert_eq!(g.take_conflict_edge(), Some(7));
        g.resolve_accept_remote((1, 1));
        g.note_conflict(8); // second conflict lands before any observer tick
        assert_eq!(g.take_conflict_edge(), Some(8)); // not lost
        assert_eq!(g.take_conflict_edge(), None);
    }

    #[test]
    fn resolved_conflict_latch_is_not_announced() {
        // conflict latched, then resolved BEFORE the observer drained it —
        // a stale emission would pop a ghost dialog on the host:
        let mut g = SyncGate::default();
        g.note_synced(1, 0);
        g.note_conflict(7);
        g.resolve_accept_remote((1, 1)); // resolution clears the pending latch
        assert_eq!(g.take_conflict_edge(), None);

        g.note_conflict(9);
        g.note_synced(1, 2); // UseLocal-style completion also clears it
        assert_eq!(g.take_conflict_edge(), None);
    }

    #[test]
    fn older_push_confirmation_cannot_complete_a_pending_open() {
        // Interleaving: a periodic push was in flight (serialized at gen 1)
        // when OpenDocument minted gen 2 and started waiting on push_busy.
        let mut g = SyncGate::default();
        g.note_synced(1, 3); // steady state before the open
        g.note_open_pending(2); // open targets generation 2
        g.note_synced(1, 4); // the OLD push's confirmation lands
        assert!(g.open_pending()); // must NOT complete the open (gen 1 < 2)
        assert!(!g.pull_allowed((2, 0)));
        g.note_synced(2, 0); // the open's own confirmation
        assert!(!g.open_pending());
    }

    #[test]
    fn accept_remote_keeps_open_pending_until_the_pull_applies() {
        let mut g = SyncGate::default();
        g.note_open_pending(2);
        g.note_conflict(9); // open's push conflicted twice
        g.resolve_accept_remote((2, 0));
        assert!(g.open_pending()); // still pending: opened not yet emittable
        assert!(g.pull_allowed((2, 0))); // the resolving pull may proceed
        g.note_synced(3, 0); // remote applied (gen 3 >= target 2)
        assert!(!g.open_pending());
        // the latch names the NOW-LIVE generation (3), not the open's target (2):
        assert_eq!(g.take_opened_edge(), Some(3));
    }

    #[test]
    fn accept_remote_blocks_the_pull_if_an_edit_intervenes() {
        let mut g = SyncGate::default();
        g.note_synced(1, 1);
        g.note_conflict(9);
        g.resolve_accept_remote((1, 2)); // user accepted remote at pair (1,2)
        assert!(g.pull_allowed((1, 2))); // unchanged: pull may apply
        assert_eq!(g.accept_window_broken((1, 2)), None);
        // an edit lands BEFORE the pull applies — those bytes must not be lost:
        assert!(!g.pull_allowed((1, 3)));
        // the gate itself hands the wiring the retained server version so the
        // conflict flow can restart (no manual bookkeeping, no wedged state):
        assert_eq!(g.accept_window_broken((1, 3)), Some(9));
        g.note_conflict(9); // wiring re-enters the conflict flow
        assert_eq!(g.accept_window_broken((1, 3)), None); // window consumed
        assert!(!g.pull_allowed((1, 3)));
    }

    #[test]
    fn generation_only_replacement_rebaselines_without_push() {
        // open-document with byte-identical content: generation moves, bytes
        // match the daemon. The wiring layer detects the byte match via
        // should_push and must re-baseline directly instead of pushing —
        // note_synced with the new pair reopens the gate:
        let mut g = SyncGate::default();
        g.note_synced(1, 0);
        assert!(g.needs_push((2, 0))); // pair moved (generation bump)
        assert!(!g.pull_allowed((2, 0)));
        g.note_synced(2, 0); // wiring re-baselines on identical bytes
        assert!(!g.needs_push((2, 0)));
        assert!(g.pull_allowed((2, 0))); // no deadlock
    }

    #[test]
    fn oversize_or_failed_push_keeps_gate_closed() {
        let mut g = SyncGate::default();
        g.note_synced(1, 0);
        // edit -> (1,1); periodic tick skipped the push (oversize) — no
        // confirmation, baseline unchanged, gate must stay closed:
        assert!(!SyncGate::periodic_push_allowed(3 * 1024 * 1024));
        assert!(!g.pull_allowed((1, 1)));
        // snapshot channel is uncapped; its confirmation advances the baseline:
        assert!(SyncGate::snapshot_push_allowed(3 * 1024 * 1024));
        g.note_synced(1, 1);
        assert!(g.pull_allowed((1, 1)));
    }

    #[test]
    fn conflict_holds_gate_until_explicit_resolution() {
        let mut g = SyncGate::default();
        g.note_synced(1, 0);
        g.note_conflict(12); // (1,1) push rejected
        assert_eq!(g.conflict(), Some(12));
        assert!(!g.pull_allowed((1, 1)));
        // A stored conflict SUSPENDS the periodic push — no silent auto-retry
        // on the next 2s tick; only explicit resolution may push again:
        assert!(!g.needs_push((1, 1)));
        g.resolve_accept_remote((1, 1));
        assert_eq!(g.conflict(), None);
        assert!(g.pull_allowed((1, 1))); // baseline forgotten: next pull re-baselines
    }
}
