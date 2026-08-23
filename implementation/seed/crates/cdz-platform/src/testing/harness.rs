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
//! **A caller names its spawns; it does not know their ids.** A reducer's id is the hash of its genesis
//! (the program, a spawn nonce, and its parent — §3), so the caller cannot write it down in advance. The
//! harness lets a run give each spawn a **name**, derives the reducer id from its genesis, and resolves a
//! delivery's target (and a spawn's parent) by that name to the assigned id. The name→id assignment is
//! returned in the [`Run`] alongside the log, so a checker can correlate a recorded [`Origin`](crate::Origin)
//! back to the name that produced it.
//!
//! It wraps the program store in a [`RecordingProgramStore`](super::recording::RecordingProgramStore), so
//! every reducer the kernel instantiates — the ones spawned here and the per-event system reducers the
//! kernel spawns to route effects — records the events it folds into the one log (§3/§4). Recording the
//! key-value and blob store calls a reducer makes joins the same log once the wasm host wires those
//! backends into a reducer.
//!
//! **Quiescence.** bach jumps virtual time to the next scheduled event, so [`run`](Harness::run) advances
//! simulated time by [`run_for`](Harness::run_for) and lets the scheduler process every event and timer
//! that falls within it — for a bounded workload that is the whole causal chain, reached in near-zero
//! wall-clock time once the system goes idle. Choose `run_for` to exceed the longest chain of timers a run
//! can produce; the default is generous because an idle system costs nothing to wait on. A workload that
//! never settles (a periodically re-arming timer) is bounded by `run_for` rather than running forever.

use super::checker::{CheckOutcome, Checker};
use super::observation::{ObservationLog, Record};
use super::recording::RecordingProgramStore;
use crate::{
    BachRuntime, Bytes, Delivered, Genesis, HostId, InMemoryEventRegistry, InMemoryReducerGraph,
    Links, ProgramHash, ProgramStore, ReducerId, ReducerKind, Runtime, Spawn, System, TaskSystem,
};
use std::collections::BTreeMap;
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

/// The canonical parent a **root** spawn's genesis is hashed against. A root is its own parent in the
/// running system (§3), which is circular for hashing a genesis (the id would depend on itself), so the
/// harness derives a root's id against this fixed anchor and then presents the reducer to the kernel as
/// its own parent (the kernel's root convention: `parent == id`, no parent link). Deterministic, so a
/// root's id is reproducible across runs.
fn root_anchor() -> ReducerId {
    ReducerId::of(b"cdz-platform.harness.genesis")
}

/// How a spawn's parent is set: a **root** (its own parent), or a **child** of an earlier spawn named in
/// this run. A child names its parent, so lineage is expressed by name and resolved to the parent's
/// derived id (§3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parent {
    /// A root reducer — its own parent, spawned at genesis.
    Root,
    /// A child of the spawn with this name (which must be spawned earlier in the run).
    Named(String),
}

/// A reducer to spawn in a run: a **name** (the caller's handle for it), the `program` it runs, and its
/// lineage/privilege/supervision. The reducer's id is derived from its genesis (§3), not chosen — refer
/// to it by name in a [`deliver`](Harness::deliver) or as another spawn's [`Parent`]. The spawn nonce is
/// the name's bytes, so sibling spawns with distinct names get distinct, reproducible ids.
#[derive(Clone, Debug)]
pub struct SpawnSpec {
    name: String,
    program: ProgramHash,
    parent: Parent,
    kind: ReducerKind,
    links: Links,
}

impl SpawnSpec {
    /// A root, ordinary reducer named `name` running `program`, with no supervision links — the shape most
    /// runs want. Refine with [`child_of`](SpawnSpec::child_of), [`kind`](SpawnSpec::kind), and
    /// [`links`](SpawnSpec::links).
    pub fn new(name: impl Into<String>, program: ProgramHash) -> Self {
        Self {
            name: name.into(),
            program,
            parent: Parent::Root,
            kind: ReducerKind::Ordinary,
            links: Links::NONE,
        }
    }

    /// Make this a child of the spawn named `parent` (spawned earlier in the run) rather than a root.
    #[must_use]
    pub fn child_of(mut self, parent: impl Into<String>) -> Self {
        self.parent = Parent::Named(parent.into());
        self
    }

    /// Set the reducer's privilege — [`Event`](ReducerKind::Event) for a privileged event/system reducer,
    /// [`Ordinary`](ReducerKind::Ordinary) otherwise (the default).
    #[must_use]
    pub fn kind(mut self, kind: ReducerKind) -> Self {
        self.kind = kind;
        self
    }

    /// Set the supervision links between this reducer and its parent.
    #[must_use]
    pub fn links(mut self, links: Links) -> Self {
        self.links = links;
        self
    }
}

/// The result of a [`Harness::run`]: the observation log, and the name→id assignment the harness made for
/// the spawns. A checker uses `ids` to map a recorded [`Origin`](crate::Origin) back to the name that
/// produced it. Both are deterministic, so two identical runs produce an equal `Run`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// Every observation the run produced, in the one global order (§9).
    pub records: Vec<Record>,
    /// The reducer id the harness assigned each named spawn (derived from its genesis, §3).
    pub ids: BTreeMap<String, ReducerId>,
}

impl Run {
    /// The reducer id the harness assigned the spawn named `name`, if any — how a checker turns a name it
    /// knows into the id the log records carry.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<ReducerId> {
        self.ids.get(name).copied()
    }

    /// Every record produced by the spawn named `name` (matched by the id the harness assigned it): the
    /// events it folded, emitted, or closed with, and the store calls it made, in order. Empty if `name`
    /// names no spawn — a convenience for writing a checker by name rather than by raw id.
    pub fn records_from<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Record> {
        let id = self.ids.get(name).copied();
        self.records
            .iter()
            .filter(move |r| Some(r.source.reducer) == id)
    }

    /// Run `checker` over this run and return its verdict — the assertion side of the harness (§9).
    #[must_use]
    pub fn check(&self, checker: &impl Checker) -> CheckOutcome {
        checker.check(self)
    }
}

/// A run of the platform: a program store, a named reducer set to spawn, and the events to deliver into
/// it. [`run`](Harness::run) drives it to quiescence under the bach simulator and returns the [`Run`].
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
    spawns: Vec<SpawnSpec>,
    deliveries: Vec<(String, Delivered)>,
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

    /// Add a named reducer to spawn when the run starts. Spawned in the order added, before any delivery;
    /// a child's parent must be spawned before it.
    #[must_use]
    pub fn spawn(mut self, spec: SpawnSpec) -> Self {
        self.spawns.push(spec);
        self
    }

    /// Add an event to deliver into the mailbox of the spawn named `target`, once the reducers are spawned.
    /// Delivered in the order added — the run's initial stimulus, whose effects the platform then routes to
    /// quiescence.
    #[must_use]
    pub fn deliver(mut self, target: impl Into<String>, event: Delivered) -> Self {
        self.deliveries.push((target.into(), event));
        self
    }

    /// Drive the run to quiescence under the bach simulator and return the [`Run`] (log + name→id map).
    ///
    /// First resolves names to ids: each spawn's reducer id is derived from its genesis (`program`, a nonce
    /// = the name's bytes, and the parent's id — a root's against the [`root_anchor`], §3), so a delivery's
    /// target and a spawn's parent resolve to the assigned id. Then, under `bach::sim`, it spawns each
    /// reducer, delivers each initial event, and advances simulated time by [`run_for`](Harness::run_for)
    /// so the scheduler processes every event and timer within it. The log is shared with the recording
    /// program store, so once the deterministic run ends it holds every event the run produced.
    ///
    /// Panics on a misconfigured run (a duplicate spawn name, a parent or delivery target that names no
    /// spawn) — a test-authoring error, surfaced loudly rather than silently mis-delivered.
    #[must_use]
    pub fn run(self) -> Run {
        use bach::ext::*;

        let Harness {
            programs,
            system_program,
            host,
            spawns,
            deliveries,
            run_for,
        } = self;

        // Resolve names to genesis-derived ids (pure, deterministic — done before the sim). Each spawn's id
        // is the hash of its genesis; a child's parent resolves to that parent's id, a root's to the anchor.
        let mut ids: BTreeMap<String, ReducerId> = BTreeMap::new();
        let mut kernel_spawns: Vec<Spawn> = Vec::with_capacity(spawns.len());
        for spec in spawns {
            let parent_id = match &spec.parent {
                Parent::Root => root_anchor(),
                Parent::Named(parent) => *ids.get(parent).unwrap_or_else(|| {
                    panic!(
                        "spawn '{}' names parent '{parent}', which is not a spawn earlier in the run",
                        spec.name
                    )
                }),
            };
            let nonce = Bytes::copy_from_slice(spec.name.as_bytes());
            let id = Genesis {
                program: spec.program,
                nonce: nonce.clone(),
                parent: parent_id,
            }
            .id();
            if ids.insert(spec.name.clone(), id).is_some() {
                panic!("duplicate spawn name '{}'", spec.name);
            }
            // The kernel's root convention is `parent == id` (no parent link); a child's parent is its
            // resolved id. The genesis above derived the id against the anchor for a root, so the id is
            // stable regardless.
            let kernel_parent = match &spec.parent {
                Parent::Root => id,
                Parent::Named(_) => parent_id,
            };
            kernel_spawns.push(Spawn {
                id,
                program: spec.program,
                nonce,
                parent: kernel_parent,
                kind: spec.kind,
                links: spec.links,
            });
        }
        let resolved_deliveries: Vec<(ReducerId, Delivered)> = deliveries
            .into_iter()
            .map(|(target, event)| {
                let id = *ids.get(&target).unwrap_or_else(|| {
                    panic!("deliver names target '{target}', which is not a spawn in the run")
                });
                (id, event)
            })
            .collect();

        let log = ObservationLog::new();
        // The handle the checker reads: the log is Arc-shared, so after the sim ends it holds every record
        // the run appended. Snapshot it once bach::sim returns (the primary task has completed by then).
        let out = log.clone();
        let ids_out = ids.clone();

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
                for spawn in kernel_spawns {
                    system
                        .spawn(spawn)
                        .await
                        .expect("in-memory system spawn never fails");
                }
                for (target, event) in resolved_deliveries {
                    let delivered = system
                        .deliver(target, event)
                        .await
                        .expect("in-memory system deliver never errors");
                    assert!(
                        delivered,
                        "delivered an initial event to a reducer that is not running"
                    );
                }
                // Advance simulated time so the scheduler runs every event and timer to quiescence.
                bach::time::sleep(run_for).await;
            }
            .group("harness")
            .primary()
            .spawn();
        });

        Run {
            records: out.snapshot(),
            ids: ids_out,
        }
    }
}
