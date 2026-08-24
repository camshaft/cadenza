//! The run-to-quiescence driver — run a set of programs to completion and collect the observation log
//! (`design/cadenza-platform.md` §3/§8/§9).
//!
//! [`Harness`] is the harness executable's run loop. A run is described in **names**, never hashes: give
//! each program **blob** a name with [`blob`](Harness::blob), spawn a **task** running a named blob with
//! [`spawn`](Harness::spawn), and deliver events to a task by name. [`run`](Harness::run) seeds a
//! content-addressed store with the named blobs, builds a program store over it, drives the whole platform
//! under the **bach simulator** — a deterministic discrete-event scheduler with virtual time — until
//! quiescent, and returns the [`Run`] (the observation log plus the name→id assignment) for a checker.
//! Driving under bach is the harness contract: event ordering, timestamps, and the recorded log are
//! reproducible across runs, which is what a checker needs.
//!
//! **Names, not hashes.** A program blob is opaque bytes named by the run (`blob(name, bytes)`); its
//! content hash is derived when a task that names it is spawned (§8). A task's reducer id is likewise the
//! hash of its genesis (its program, a spawn nonce, its parent — §3), derived at spawn. So a run writes
//! only names — a spawn names its blob and (for a child) its parent, a delivery names its target — and the
//! harness resolves each to the hash/id, mirroring how the platform itself never lets a caller pick an id.
//! The name→id assignment is returned in the [`Run`], and recorded in the log, so a checker maps a recorded
//! [`Origin`](crate::Origin) back to the name that produced it.
//!
//! **Building the store.** [`run`](Harness::run) takes a factory that builds the program store from the
//! seeded content-addressed store: for a real run that is a wasm program store loading components from the
//! store by hash; for a native test it is a store of Rust reducer factories (which ignores the seeded blobs
//! and instantiates by the same hashes). Either way the harness wraps the store in a
//! [`RecordingProgramStore`](super::recording::RecordingProgramStore), so every reducer the kernel
//! instantiates records the events it folds into the one log (§3/§4).
//!
//! **Quiescence.** bach jumps virtual time to the next scheduled event, so [`run`](Harness::run) advances
//! simulated time by [`run_for`](Harness::run_for) and lets the scheduler process every event and timer
//! that falls within it — for a bounded workload that is the whole causal chain, reached in near-zero
//! wall-clock time once the system goes idle. Choose `run_for` to exceed the longest chain of timers a run
//! can produce; the default is generous because an idle system costs nothing to wait on. A workload that
//! never settles (a periodically re-arming timer) is bounded by `run_for` rather than running forever.

use super::checker::{CheckOutcome, Checker};
use super::observation::{Entry, ObservationLog, Record, SpawnInfo};
use super::recording::RecordingProgramStore;
use crate::{
    BachRuntime, BlobStore, Bytes, ContractId, Delivered, Genesis, HostId, InMemoryBlobStore,
    InMemoryEventRegistry, InMemoryReducerGraph, Links, Origin, ProgramHash, ProgramStore,
    ReducerId, ReducerKind, Runtime, Spawn, Str, System, TaskSystem,
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

/// A task to spawn in a run: a **task name** (the caller's handle for the reducer), the **blob name** of
/// the program it runs, and its lineage/privilege/supervision. Both are names — the reducer id is derived
/// from its genesis and the program from the blob's content (§3/§8), so a caller never writes a hash. Refer
/// to the task by name in a [`deliver`](Harness::deliver) or as another spawn's [`Parent`]. The spawn nonce
/// is the task name's bytes, so sibling tasks with distinct names get distinct, reproducible ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSpec {
    name: String,
    blob: String,
    parent: Parent,
    kind: ReducerKind,
    links: Links,
}

impl SpawnSpec {
    /// A root, ordinary task named `name` running the program blob named `blob` — the shape most runs want.
    /// `blob` is a blob name registered on the harness with [`blob`](Harness::blob); the run resolves it to
    /// the blob's content hash at spawn time. Refine with [`child_of`](SpawnSpec::child_of),
    /// [`kind`](SpawnSpec::kind), and [`links`](SpawnSpec::links).
    pub fn new(name: impl Into<String>, blob: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            blob: blob.into(),
            parent: Parent::Root,
            kind: ReducerKind::Ordinary,
            links: Links::NONE,
        }
    }

    /// Make this a child of the task named `parent` (spawned earlier in the run) rather than a root.
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

    /// The task's name — the caller's handle for the reducer.
    #[must_use]
    pub fn task_name(&self) -> &str {
        &self.name
    }

    /// The name of the blob this task runs.
    #[must_use]
    pub fn blob_name(&self) -> &str {
        &self.blob
    }

    /// How this task's parent is set — a [`Root`](Parent::Root) or a [`Named`](Parent::Named) parent.
    #[must_use]
    pub fn parent(&self) -> &Parent {
        &self.parent
    }

    /// The reducer's privilege.
    #[must_use]
    pub fn reducer_kind(&self) -> ReducerKind {
        self.kind
    }

    /// The supervision links this spawn establishes.
    #[must_use]
    pub fn supervision(&self) -> Links {
        self.links
    }
}

/// The result of a [`Harness::run`]: the observation log, and the name→id assignment the harness made for
/// the spawns. A checker uses `ids` to map a recorded [`Origin`](crate::Origin) back to the task name that
/// produced it. Both are deterministic, so two identical runs produce an equal `Run`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// Every observation the run produced, in the one global order (§9).
    pub records: Vec<Record>,
    /// The reducer id the harness assigned each named task (derived from its genesis, §3).
    pub ids: BTreeMap<String, ReducerId>,
}

impl Run {
    /// The reducer id the harness assigned the task named `name`, if any — how a checker turns a name it
    /// knows into the id the log records carry.
    #[must_use]
    pub fn id(&self, name: &str) -> Option<ReducerId> {
        self.ids.get(name).copied()
    }

    /// Every record produced by the task named `name` (matched by the id the harness assigned it): the
    /// events it folded, emitted, or closed with, and the store calls it made, in order. Empty if `name`
    /// names no task — a convenience for writing a checker by name rather than by raw id.
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

/// A run of the platform: named program blobs, a named task set to spawn, and the events to deliver into
/// them. [`run`](Harness::run) seeds a content-addressed store with the blobs, builds a program store over
/// it, drives to quiescence under the bach simulator, and returns the [`Run`].
///
/// Built fluently: [`new`](Harness::new), then [`blob`](Harness::blob) to register program bytes,
/// [`spawn`](Harness::spawn) / [`deliver`](Harness::deliver) to describe the run, optionally
/// [`host`](Harness::host) / [`run_for`](Harness::run_for), then [`run`](Harness::run).
pub struct Harness {
    /// The blob name of the system reducer every effect routes to by default (the event registry's default
    /// entry, §4). Like every program it is named, not hashed — resolved to its content hash at run time,
    /// so it must be registered with [`blob`](Harness::blob).
    system: String,
    host: HostId,
    /// Blob name → its opaque program bytes. A spawn names a blob; the run seeds these into the store and
    /// resolves a blob name to the bytes' content hash at spawn time.
    blobs: BTreeMap<String, Bytes>,
    spawns: Vec<SpawnSpec>,
    deliveries: Vec<(String, Delivered)>,
    run_for: Duration,
    /// The observation log to record into, if the caller supplied one with [`log`](Harness::log). `None`
    /// means the run creates its own fresh log. A caller supplies a log to *share* it with the store
    /// backends it wires into the program store — a [`RecordingKvStore`](super::RecordingKvStore) /
    /// [`RecordingBlobStore`](super::RecordingBlobStore) built over the same log — so a reducer's KV and blob
    /// calls land in the one ordered log alongside the events it folds (§7/§8/§9), not just the events.
    log: Option<ObservationLog>,
    /// The event registry to install, if the caller set one with [`registry`](Harness::registry): the default
    /// handler's blob name plus per-contract `(contract, handler blob name)` overrides (§4). `None` keeps the
    /// harness default — every contract routes to the `system` reducer. Names resolve to content hashes at
    /// run time, alongside the blob/spawn names.
    event_registry: Option<(String, Vec<(ContractId, String)>)>,
}

impl Harness {
    /// A harness routing every effect to the system reducer named by the blob `system` (the event
    /// registry's default entry, §4). `system` is a blob *name* like any other program — register its bytes
    /// with [`blob`](Harness::blob), and the run resolves the name to its content hash. No blobs, tasks, or
    /// deliveries yet.
    pub fn new(system: impl Into<String>) -> Self {
        Self {
            system: system.into(),
            host: default_host(),
            blobs: BTreeMap::new(),
            spawns: Vec::new(),
            deliveries: Vec::new(),
            run_for: DEFAULT_RUN_FOR,
            log: None,
            event_registry: None,
        }
    }

    /// Install a custom event registry (§4): `default` is the blob name of the default event handler (the
    /// program governing any contract without an override), and `overrides` maps contract-ids to handler
    /// blob names. Both resolve to content hashes at run time. Without this the run routes every contract to
    /// the `system` reducer (the harness default). All named programs must be registered with
    /// [`blob`](Harness::blob).
    #[must_use]
    pub fn registry(
        mut self,
        default: impl Into<String>,
        overrides: Vec<(ContractId, String)>,
    ) -> Self {
        self.event_registry = Some((default.into(), overrides));
        self
    }

    /// Record the run into `log` rather than a fresh internal one — so a caller can *share* one observation
    /// log across the run's events and the store backends it wires into the program store. Build a
    /// [`RecordingKvStore`](super::RecordingKvStore) / [`RecordingBlobStore`](super::RecordingBlobStore) over
    /// the same `log`, hand those to the store the run's `make_store` builds (e.g. a wasm program store's
    /// per-reducer KV/blob factories), and every KV and blob call a reducer makes lands in the returned
    /// [`Run`]'s log interleaved with the events it folds — the one ordered log a checker reads (§9). Without
    /// this, the run's log holds only events, since the store backends have no handle to it.
    #[must_use]
    pub fn log(mut self, log: ObservationLog) -> Self {
        self.log = Some(log);
        self
    }

    /// Register a program blob under `name` — opaque bytes a spawn can name. The run seeds it into the
    /// content-addressed store and a spawn that names it resolves to its content hash (§8). Registering the
    /// same name again replaces the bytes.
    #[must_use]
    pub fn blob(mut self, name: impl Into<String>, bytes: Bytes) -> Self {
        self.blobs.insert(name.into(), bytes);
        self
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

    /// Add a task to spawn when the run starts. Spawned in the order added, before any delivery; a child's
    /// parent must be spawned before it.
    #[must_use]
    pub fn spawn(mut self, spec: SpawnSpec) -> Self {
        self.spawns.push(spec);
        self
    }

    /// Add an event to deliver into the mailbox of the task named `target`, once the tasks are spawned.
    /// Delivered in the order added — the run's initial stimulus, whose effects the platform then routes to
    /// quiescence.
    #[must_use]
    pub fn deliver(mut self, target: impl Into<String>, event: Delivered) -> Self {
        self.deliveries.push((target.into(), event));
        self
    }

    /// Drive the run to quiescence under the bach simulator and return the [`Run`] (log + name→id map).
    ///
    /// `make_store` builds the program store from the content-addressed store the harness seeds with the
    /// run's blobs: for a real run, `|cas| WasmProgramStore::new(cas, …)`; for a native test, a closure that
    /// ignores the store and returns a store of Rust reducer factories keyed by the same content hashes.
    ///
    /// Resolves names to ids first (a blob name → its bytes' content hash; a task name → its genesis id; a
    /// delivery/parent name → the assigned id), then under `bach::sim` seeds the store, builds the program
    /// store, spawns each task, delivers each event, and advances simulated time by
    /// [`run_for`](Harness::run_for). The log is shared with the recording store, so once the deterministic
    /// run ends it holds every event the run produced.
    ///
    /// Panics on a misconfigured run (a duplicate task name, or a blob / parent / delivery target that names
    /// nothing registered) — a test-authoring error, surfaced loudly rather than silently mis-run.
    #[must_use]
    pub fn run<P, F>(self, make_store: F) -> Run
    where
        P: ProgramStore + 'static,
        F: FnOnce(Arc<dyn BlobStore>) -> P + Send + 'static,
    {
        use bach::ext::*;

        let Harness {
            system,
            host,
            blobs,
            spawns,
            deliveries,
            run_for,
            log: external_log,
            event_registry,
        } = self;

        // Resolve names to hashes/ids (pure, deterministic — done before the sim). A blob name resolves to
        // its bytes' content hash (§8); a task's reducer id is the hash of its genesis; a child's parent and
        // a delivery target resolve to the assigned id; the system reducer blob name resolves like any other.
        let blob_ids: BTreeMap<&str, ProgramHash> = blobs
            .iter()
            .map(|(name, bytes)| (name.as_str(), ProgramHash::of(bytes)))
            .collect();
        let system_program = *blob_ids.get(system.as_str()).unwrap_or_else(|| {
            panic!("system reducer blob '{system}' is not registered with Harness::blob")
        });
        // Build the event registry (§4): a custom one if the run installed it (default + per-contract
        // overrides, resolved by blob name to content hash), else the harness default of every contract →
        // the system reducer. Resolved here (borrowing `blob_ids`) into an owned registry moved into the sim.
        let registry = match event_registry {
            None => InMemoryEventRegistry::new(system_program),
            Some((default_name, overrides)) => {
                let default = *blob_ids.get(default_name.as_str()).unwrap_or_else(|| {
                    panic!("registry default handler blob '{default_name}' is not registered with Harness::blob")
                });
                let mut registry = InMemoryEventRegistry::new(default);
                for (contract, program_name) in &overrides {
                    let program = *blob_ids.get(program_name.as_str()).unwrap_or_else(|| {
                        panic!(
                            "registry handler blob '{program_name}' is not registered with Harness::blob"
                        )
                    });
                    registry.set_override(*contract, program);
                }
                registry
            }
        };
        let mut ids: BTreeMap<String, ReducerId> = BTreeMap::new();
        // Each kernel spawn paired with the task name the run gave it, so the run can record the name→id
        // assignment into the log before spawning.
        let mut kernel_spawns: Vec<(Str, Spawn)> = Vec::with_capacity(spawns.len());
        for spec in spawns {
            let parent_id = match &spec.parent {
                Parent::Root => root_anchor(),
                Parent::Named(parent) => *ids.get(parent).unwrap_or_else(|| {
                    panic!(
                        "task '{}' names parent '{parent}', which is not a task earlier in the run",
                        spec.name
                    )
                }),
            };
            // Resolve the task's blob name to its content hash — the name→hash side of the same name-not-hash
            // model the reducer id uses (§3/§8).
            let program = *blob_ids.get(spec.blob.as_str()).unwrap_or_else(|| {
                panic!(
                    "task '{}' names blob '{}', which is not registered with Harness::blob",
                    spec.name, spec.blob
                )
            });
            let nonce = Bytes::copy_from_slice(spec.name.as_bytes());
            let id = Genesis {
                program,
                nonce: nonce.clone(),
                parent: parent_id,
            }
            .id();
            if ids.insert(spec.name.clone(), id).is_some() {
                panic!("duplicate task name '{}'", spec.name);
            }
            // The kernel's root convention is `parent == id` (no parent link); a child's parent is its
            // resolved id. The genesis above derived the id against the anchor for a root, so the id is
            // stable regardless.
            let kernel_parent = match &spec.parent {
                Parent::Root => id,
                Parent::Named(_) => parent_id,
            };
            kernel_spawns.push((
                Str::from(spec.name.as_str()),
                Spawn {
                    id,
                    program,
                    nonce,
                    parent: kernel_parent,
                    kind: spec.kind,
                    links: spec.links,
                },
            ));
        }
        let resolved_deliveries: Vec<(ReducerId, Delivered)> = deliveries
            .into_iter()
            .map(|(target, event)| {
                let id = *ids.get(&target).unwrap_or_else(|| {
                    panic!("deliver names target '{target}', which is not a task in the run")
                });
                (id, event)
            })
            .collect();

        // Record into the caller's log if one was supplied (so a caller's recording store backends share it),
        // else a fresh one for this run.
        let log = external_log.unwrap_or_else(ObservationLog::new);
        // The handle the checker reads: the log is Arc-shared, so after the sim ends it holds every record
        // the run appended. Snapshot it once bach::sim returns (the primary task has completed by then).
        let out = log.clone();
        let ids_out = ids.clone();

        bach::sim(move || {
            let log = log.clone();
            async move {
                // Seed the content-addressed store with the run's opaque blobs, then let the factory build
                // the program store over it (a wasm store loads components from here by hash; a native store
                // ignores it and instantiates by the same hashes).
                let mut cas = InMemoryBlobStore::new();
                for bytes in blobs.into_values() {
                    cas.put(bytes).await;
                }
                let store = make_store(Arc::new(cas));
                let recording = RecordingProgramStore::new(
                    store,
                    host,
                    log.clone(),
                    BachRuntime::now as fn() -> u64,
                );
                let system = TaskSystem::<BachRuntime>::new(
                    Arc::new(recording),
                    Arc::new(InMemoryReducerGraph::new()),
                    Arc::new(registry),
                    host,
                );
                // Record the name→id assignment into the log first, so the log is self-describing: a reader
                // derefs a name to the reducer id it was given (the record's source) without any out-of-band
                // map. These lead the log, ahead of any reducer's birth or events.
                for (name, spawn) in &kernel_spawns {
                    log.record(
                        BachRuntime::now(),
                        Origin {
                            reducer: spawn.id,
                            host,
                        },
                        Entry::Spawn(SpawnInfo {
                            name: name.clone(),
                            program: spawn.program,
                            parent: spawn.parent,
                            kind: spawn.kind,
                        }),
                    );
                }
                for (_name, spawn) in kernel_spawns {
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
