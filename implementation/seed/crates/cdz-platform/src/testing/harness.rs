//! The run-to-quiescence driver — run a reducer set to completion and collect the observation log
//! (`design/cadenza-platform.md` §3/§9).
//!
//! [`Harness`] is the harness executable's run loop: configure it with a program store, a reducer set to
//! spawn, and the initial events to deliver, then [`run`](Harness::run) drives the whole platform under
//! the **bach simulator** — a deterministic discrete-event scheduler with virtual time — until quiescent,
//! and returns the [`ObservationLog`] records for a checker to assert over. Driving under bach (not
//! wall-clock tokio) is the harness contract: event ordering, timestamps, and the recorded log are
//! reproducible across runs, which is exactly what a checker needs.
//!
//! It wraps the program store in a [`RecordingProgramStore`](crate::RecordingProgramStore), so every
//! reducer the kernel instantiates — the ones spawned here and the per-event system reducers the kernel
//! spawns to route effects — records the events it folds into the one log (§3/§4). Recording the
//! key-value and blob store calls a reducer makes joins the same log once the wasm host wires those
//! backends into a reducer; a native reducer that holds a
//! [`RecordingKvStore`](crate::RecordingKvStore) already records into a log the harness shares.
//!
//! **Quiescence.** bach jumps virtual time to the next scheduled event, so [`run`](Harness::run) advances
//! simulated time by [`run_for`](Harness::run_for) and lets the scheduler process every event and timer
//! that falls within it — for a bounded workload that is the whole causal chain, reached in near-zero
//! wall-clock time once the system goes idle (there is nothing left to advance to). Choose `run_for` to
//! exceed the longest chain of timers a run can produce; the default is generous because an idle system
//! costs nothing to wait on. A workload that never settles (a periodically re-arming timer) is bounded by
//! `run_for` rather than running forever.

use super::observation::{ObservationLog, Record};
use super::recording::RecordingProgramStore;
use crate::{
    BachRuntime, Delivered, HostId, InMemoryEventRegistry, InMemoryReducerGraph, ProgramHash,
    ProgramStore, ReducerId, Runtime, Spawn, System, TaskSystem,
};
use std::sync::Arc;
use std::time::Duration;

/// The default host id a [`Harness`] runs its reducers on when the caller does not set one — this node's
/// identity, stamped on the `from` of every routed message (§3).
fn default_host() -> HostId {
    HostId::of(b"cdz-platform-harness")
}

/// One simulated hour of virtual time — the default [`run_for`](Harness::run_for) horizon. Generous on
/// purpose: bach jumps to the next event, so an idle system reaches the horizon instantly, and only a
/// workload that keeps scheduling work pays for the extra span.
const DEFAULT_RUN_FOR: Duration = Duration::from_secs(3600);

/// A run of the platform: a program store, a reducer set to spawn, and the events to deliver into it.
/// [`run`](Harness::run) drives it to quiescence under the bach simulator and returns the observation log.
/// Generic over the program store, so it drives native reducer fixtures today and a wasm program store
/// once that lands, unchanged.
///
/// Built fluently: [`new`](Harness::new), then [`spawn`](Harness::spawn) / [`deliver`](Harness::deliver)
/// to describe the run, optionally [`host`](Harness::host) / [`run_for`](Harness::run_for), then
/// [`run`](Harness::run).
pub struct Harness<P> {
    programs: P,
    system_program: ProgramHash,
    host: HostId,
    spawns: Vec<Spawn>,
    deliveries: Vec<(ReducerId, Delivered)>,
    run_for: Duration,
}

impl<P: ProgramStore + 'static> Harness<P> {
    /// A harness over `programs`, routing every effect's system reducer to `system_program` by default (the
    /// event registry's default entry, §4). No reducers spawned and no events delivered yet — add them with
    /// [`spawn`](Harness::spawn) and [`deliver`](Harness::deliver).
    pub fn new(programs: P, system_program: ProgramHash) -> Self {
        Self {
            programs,
            system_program,
            host: default_host(),
            spawns: Vec::new(),
            deliveries: Vec::new(),
            run_for: DEFAULT_RUN_FOR,
        }
    }

    /// Set the host id this node runs as — stamped as the `from` host on every routed message (§3).
    #[must_use]
    pub fn host(mut self, host: HostId) -> Self {
        self.host = host;
        self
    }

    /// Set how much simulated time to drive the run for. bach jumps virtual time, so a generous horizon
    /// costs nothing once the system is idle; raise it past the longest timer chain a run can produce.
    #[must_use]
    pub fn run_for(mut self, run_for: Duration) -> Self {
        self.run_for = run_for;
        self
    }

    /// Add a reducer to spawn when the run starts. Spawned in the order added, before any delivery.
    #[must_use]
    pub fn spawn(mut self, spawn: Spawn) -> Self {
        self.spawns.push(spawn);
        self
    }

    /// Add an event to deliver into `target`'s mailbox once the reducers are spawned. Delivered in the
    /// order added — the run's initial stimulus, whose effects the platform then routes to quiescence.
    #[must_use]
    pub fn deliver(mut self, target: ReducerId, event: Delivered) -> Self {
        self.deliveries.push((target, event));
        self
    }

    /// Drive the run to quiescence under the bach simulator and return the observation log in order.
    ///
    /// Spawns each reducer, delivers each initial event, then advances simulated time by
    /// [`run_for`](Harness::run_for) so the scheduler processes every event and timer within it. The log
    /// is shared with the recording program store, so once the deterministic run ends (the primary task's
    /// simulated wait elapses) it holds every event the run produced. Returns the records — the input to a
    /// checker.
    #[must_use]
    pub fn run(self) -> Vec<Record> {
        use bach::ext::*;

        let Harness {
            programs,
            system_program,
            host,
            spawns,
            deliveries,
            run_for,
        } = self;
        let log = ObservationLog::new();
        // The handle the checker reads: the log is Arc-shared, so after the sim ends it holds every record
        // the run appended. Snapshot it once bach::sim returns (the primary task has completed by then).
        let out = log.clone();

        bach::sim(move || {
            let log = log.clone();
            async move {
                let recording = RecordingProgramStore::new(
                    programs,
                    host,
                    log,
                    BachRuntime::now as fn() -> u64,
                );
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(recording),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(InMemoryEventRegistry::new(system_program)),
                    host,
                );
                for spawn in spawns {
                    system
                        .spawn(spawn)
                        .await
                        .expect("in-memory system spawn never fails");
                }
                for (target, event) in deliveries {
                    // A false here means the target was not running to receive the delivery — a
                    // misconfigured run (delivering to an unspawned reducer), which a test wants surfaced.
                    let delivered = system
                        .deliver(target, event)
                        .await
                        .expect("in-memory system deliver never errors");
                    assert!(
                        delivered,
                        "delivered an initial event to an unspawned reducer"
                    );
                }
                // Advance simulated time so the scheduler runs every event and timer to quiescence.
                bach::time::sleep(run_for).await;
            }
            .group("harness")
            .primary()
            .spawn();
        });

        out.snapshot()
    }
}
