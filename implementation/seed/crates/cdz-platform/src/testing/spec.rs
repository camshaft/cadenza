//! Decode a whole harness run from a **Cadenza binary AST** (`design/cadenza-platform.md` §9).
//!
//! The integration-test executable's input is not an argv convention — it is a single Cadenza value that
//! *describes the entire run*: the program blobs, the tasks to spawn, and the event registry (its default
//! handler and any per-contract overrides). A value, not a
//! command line, keeps the harness language-neutral (the description is itself a serializable Cadenza value)
//! and self-contained. [`HarnessSpec::decode`] reads the binary AST (`cadenza_ast::codec::decode`) and
//! interprets it as a run; [`HarnessSpec::build`] turns it into a ready-to-run [`Harness`], resolving any
//! blob given by *path* through a caller-supplied loader (so the decoder itself touches no filesystem).
//!
//! ## The value shape
//! A run is a record (`("record" (= <field> <value>)…)`, the canonical Cadenza record — [`crate`]'s
//! `contract_value`):
//!
//! ```text
//! ("record"
//!   (= registry ("record" (= default "sys")))  ; the event registry: its default handler blob NAME (§4)
//!   (= run-for 3600000000000)                 ; optional; virtual-time horizon in NANOSECONDS (default 1h)
//!   (= blobs   ("list" <blob>…))              ; the program blobs, by name
//!   (= spawns  ("list" <spawn>…))             ; the tasks to spawn, in order
//!   (= deliver ("list" <delivery>…))          ; optional; the initial events to inject, in order
//!   (= checker "check")                        ; optional; the blob name of the checker reducer (§9)
//!   (= edges   ("list" <edge>…))))             ; optional; reducer-graph transform chains to seed (§4)
//! ```
//!
//! An `<edge>` seeds one transform chain (§4): for the emitting task `from`, effects on `contract` route
//! through the ordered chain of transform tasks `to`. All three are task names (resolved to ids like a
//! spawn); the graph is otherwise empty, so an emitter with no edge for a contract has an empty chain (which
//! the default event handler answers with `missing-handler`):
//! ```text
//! ("record" (= from "emitter") (= contract "AbC…base62") (= to ("list" "transform-a" "transform-b")))
//! ```
//!
//! The `checker`, if present, names a program blob the harness runs over the completed observation log to
//! decide pass/fail: it is delivered the whole log and emits a verdict. The harness just executes it as a
//! wasm reducer — it knows nothing of how the checker was authored (a declarative set of checks compiled to
//! a Cadenza reducer, or hand-written); that transform is separate, upstream, and never seen here.
//!
//! A `<delivery>` names a `target` task and carries exactly one event to inject into it — a `message` (an
//! effect folded through `on_message`) or a `notification` (a control-plane event folded through
//! `on_notification`). Both carry a `contract` (a contract-id) and a `payload`; a message also
//! takes an optional `token` (the caller's continuation token, default empty). A message `payload` is either
//! opaque bytes, a `Value("<Type>", <value>)` reference (the given Cadenza value encoded to the canonical
//! binary form a guest `Value.decode`s type-directed against `<Type>` — a STRUCTURED payload a reducer decodes
//! and dispatches on by schema, not opaque bytes), or a resolve reference to a named blob — `BlobBytes(<name>)`
//! (its content bytes) or `BlobHash(<name>)` (its content hash) — that resolves at build from the run's blob
//! table (so a reducer can
//! be handed another program's hash to `run` it, or a shared blob's bytes, by name; §3/§8). A `contract` is
//! written either
//! as its raw 33 tagged bytes, or as a **base62** string (the §8 text form) — the string form is what a
//! name→id rewrite substitutes for a contract name (contract-name resolution is done outside the platform and
//! rewritten into the spec, so a resolved name arrives as its base62 id):
//! ```text
//! ("record" (= target "root") (= message ("record" (= contract b"…33 bytes") (= payload b"…") (= token b"…"))))
//! ("record" (= target "root") (= notification ("record" (= contract "AbC…base62") (= payload b"…"))))
//! ```
//!
//! A `<blob>` names a program and gives its bytes **either inline or by path** — exactly one:
//! ```text
//! ("record" (= name "greeter") (= bytes b"\x00…"))   ; inline opaque bytes
//! ("record" (= name "greeter") (= path  "greeter.wasm"))   ; a file the loader reads
//! ```
//!
//! A `<spawn>` names a task and the blob it runs, with optional lineage and privilege:
//! ```text
//! ("record"
//!   (= name   "root")        ; the task's handle in the log
//!   (= blob   "greeter")     ; a blob name declared above
//!   (= parent "root")        ; optional — a task spawned earlier; absent ⇒ a root
//!   (= kind   "event")       ; optional — "ordinary" (default) or "event" (a privileged event reducer)
//!   (= links  ("record" (= parentWatchesChild 1))))  ; optional — supervision links to the parent (§7)
//! ```
//! The `links` sub-record carries two `0`/`1` flags — `parentWatchesChild` and `childWatchesParent` — each of
//! which, when set, establishes a `watch_exit` edge so the runtime delivers one reducer's lifecycle event
//! (`Exited`/`Crashed`) to the other. Absent flags default to false (no supervision).
//!
//! Both the list head `("list" …)` and the bare name head `(list …)` are accepted, as they denote the same
//! construct. Every field is read by name, so order is not load-bearing. A malformed description is a
//! [`SpecError`], never a panic.

use super::harness::{Harness, Parent, SpawnSpec};
use crate::contract_value::{
    ascribe, bare_ctor, bytes_leaf, read_bytes, read_uint, record, record_field, uint_leaf,
};
use crate::{
    Bytes, ContractId, Delivered, Error, Hash, HostId, Links, Message, Notification, Origin,
    ProgramHash, ReducerId, ReducerKind, Response,
};
use cadenza_ast::ast::{Arenas, Builder, CompoundCtor, Leaf, Struct, StructId};
use cadenza_ast::codec;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// The synthetic [`Origin`] stamped on a delivered [`Message`]'s `from` — an initial event injected by the
/// harness has no real sending reducer, so the harness attributes it to a fixed, reproducible external
/// origin. (A configurable `from` is a later slice; a delivered message's sender is rarely load-bearing for
/// the reducer under test, which routes on the contract, not the injector.)
fn external_origin() -> Origin {
    Origin {
        reducer: ReducerId::of(b"cdz-platform.harness.external"),
        host: HostId::of(b"cdz-platform.harness.external"),
    }
}

/// Where a program blob's bytes come from: inline in the description, or a path a loader reads. The decoder
/// yields the source unresolved so it does no filesystem I/O; [`HarnessSpec::build`] resolves a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlobSource {
    /// The opaque program bytes, carried inline in the AST.
    Inline(Bytes),
    /// A filesystem path to read the program bytes from, relative to the caller's working directory.
    Path(String),
}

/// A program blob in a run: its name (a spawn refers to it by this) and where its bytes come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobSpec {
    /// The blob name — a spawn's `blob` field names this.
    pub name: String,
    /// Inline bytes or a path.
    pub source: BlobSource,
}

/// A whole harness run decoded from a Cadenza binary AST: the event registry (its default handler and any
/// per-contract overrides), the optional virtual-time horizon, the program blobs, and the tasks to spawn.
/// [`build`](HarnessSpec::build) turns it into a runnable [`Harness`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessSpec {
    /// How much virtual time to drive the run for; `None` uses the harness default (one simulated hour).
    pub run_for: Option<Duration>,
    /// The program blobs, in declaration order.
    pub blobs: Vec<BlobSpec>,
    /// The tasks to spawn, in order (a child's parent must appear before it).
    pub spawns: Vec<SpawnSpec>,
    /// The initial events to inject once the tasks are spawned, each paired with the task name to deliver it
    /// into, in order. Empty for a run whose only stimulus is the reducers' births.
    pub deliveries: Vec<(String, DeliveryEvent)>,
    /// The blob name of the run's **checker** program, if any — a reducer the harness runs over the completed
    /// observation log to decide pass/fail (§9). It is delivered the whole log and emits a verdict; the
    /// harness just executes it as a wasm reducer, knowing nothing of how it was authored (a declarative
    /// set of checks compiled to a Cadenza reducer program, or hand-written). `None` for a run with no
    /// end-of-run check.
    pub checker: Option<String>,
    /// A **pure run** to perform instead of the spawn/deliver/checker flow, if present (§3): run one program
    /// as a pure function of a single input — empty capabilities, every effect denied — and assert its output.
    /// The executable runs the program through the `run` primitive and passes iff the output equals
    /// [`PureRun::expect_output`]; the spawn/deliver/checker fields are unused for such a run. `None` for an
    /// ordinary run.
    pub pure_run: Option<PureRun>,
    /// **Unnamed** content-addressed component dependencies to seed into the run's content-addressed store,
    /// each by its content hash — the value-heap runtime a Cadenza guest imports
    /// (`cadenza:runtime/heap@…+<hash>`) and that runtime's own NFC dependency. Unlike [`blobs`](Self::blobs)
    /// these carry no name (no spawn refers to them); they exist only so a guest's content-addressed imports
    /// resolve in the store (`host::…::bind_dependencies`). The run is thereby **self-contained** — every
    /// component it needs travels in the spec — rather than pulling the runtime from an ambient store at run
    /// time. Empty for a run of only self-contained (Rust) guests.
    pub deps: Vec<BlobSource>,
    /// The **event registry** the run installs (§4): its [`default`](RegistrySpec::default) event handler —
    /// the program every contract routes to unless overridden — plus zero or more per-contract overrides. This
    /// is the sole source of the default handler; a run stands up a specific default handler here and routes
    /// named contracts to named handlers, then asserts how an effect is routed/answered.
    pub registry: RegistrySpec,
    /// The reducer-graph transform chains to seed before the run (§4): each edge configures, for an emitting
    /// task and a contract, the ordered chain of transform tasks its effects on that contract route through.
    /// The graph is otherwise empty, so an emitter with no edge for a contract has an empty chain — which the
    /// default event handler answers with `missing-handler` (the reject arm). An edge with a non-empty chain
    /// is the forward arm: the default handler routes the effect to the first transform. All tasks are named
    /// (resolved to ids at run time, like spawns); empty for a run that seeds no chains.
    pub edges: Vec<GraphEdge>,
}

/// One reducer-graph transform chain to seed (`design/cadenza-platform.md` §3/§4): for the emitting task
/// [`from`](GraphEdge::from), effects on [`contract`](GraphEdge::contract) route through the ordered chain of
/// transform tasks [`to`](GraphEdge::to). Mirrors the graph's `set-edges` (whole-chain replace) keyed by
/// [`EdgeKind::for_contract`](crate::graph::EdgeKind::for_contract). All are task **names** resolved to ids at
/// run time; an empty `to` seeds no chain (the same as declaring no edge).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    /// The emitting task whose effects on `contract` this chain governs.
    pub from: String,
    /// The contract whose chain this configures.
    pub contract: ContractId,
    /// The ordered transform tasks the effect routes through (chain order).
    pub to: Vec<String>,
}

/// The event registry a run installs (`design/cadenza-platform.md` §4): a **default** event handler that
/// governs any contract without an override, plus zero or more per-contract **handler** overrides. An effect
/// on a contract resolves to its override if one is installed, else the default handler (which may itself
/// decline and reply with a rejection — an unregistered contract is *forwarded* to the default, never
/// auto-faulted). All programs are blob **names** (resolved to content hashes at run time).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySpec {
    /// The blob name of the default event handler — the program whose event reducer governs any contract
    /// without an override.
    pub default: String,
    /// Per-contract overrides: `(contract-id, handler blob name)`, in declaration order. A contract with an
    /// override routes to that handler instead of the default.
    pub handlers: Vec<(ContractId, String)>,
}

/// A pure run to perform and assert over (`design/cadenza-platform.md` §3): run [`program`](PureRun::program)
/// as a pure function of [`input`](PureRun::input) on [`contract`](PureRun::contract) — the `run` primitive
/// instantiates it with an empty capability set, so every effect it emits is denied (dropped, never routed)
/// and its only output is the fold's result — and assert the output equals [`expect_output`](PureRun::expect_output).
/// This is how a conformance run exercises run-as-effect and effect-denial (§3) without any event routing: no
/// spawn, no delivery, just `run(program, contract, input) == Ok(expect_output)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PureRun {
    /// The blob name of the program to run (declared in [`HarnessSpec::blobs`]).
    pub program: String,
    /// The contract-id the input is delivered against (the `run` message's `id`).
    pub contract: ContractId,
    /// The input value, as opaque bytes.
    pub input: Bytes,
    /// The output the run must produce for the conformance run to pass.
    pub expect_output: Bytes,
}

/// An event the harness injects into a task's mailbox as a run's initial stimulus (§4). The schema's
/// delivery vocabulary — its own type rather than the platform's full [`Delivered`], so it names exactly
/// what a harness description can express: a [`Message`] (folded through `on_message`), a [`Notification`]
/// (`on_notification`), or a [`Response`] (`on_response`). A `Response` is normally a reply to a request the
/// reducer itself made; the harness injects one so a conformance run can exercise the `on_response` path —
/// including a runtime-failure (`Err`) answer — without a live responder, correlating it to the request by
/// `token`. [`build`](HarnessSpec::build) turns each into a [`Delivered`], stamping a delivered message with
/// the harness's synthetic external [`Origin`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryEvent {
    /// An effect to fold through the target's `on_message`.
    Message {
        /// The contract-id of the effect.
        contract: ContractId,
        /// The effect's input value, opaque bytes. A `BlobBytes(<name>)` / `BlobHash(<name>)` resolve
        /// reference in the source is resolved to these bytes by [`resolve_references`] at decode, so by here
        /// the payload is always literal bytes.
        payload: Bytes,
        /// The caller's continuation token (the reducer's reply is correlated back by it); empty by default.
        token: Bytes,
        /// The sender the kernel stamps on the delivered message (`msg.from`), so a run can exercise a
        /// reducer that routes or validates on *who* sent the effect. `None` uses the harness's synthetic
        /// external origin — the default, since a reducer usually routes on the contract, not the injector.
        from: Option<Origin>,
    },
    /// A control-plane event to fold through the target's `on_notification`.
    Notification {
        /// The contract-id of the notification's schema.
        contract: ContractId,
        /// The notification value, opaque bytes.
        payload: Bytes,
    },
    /// A reply to fold through the target's `on_response` — a response to a request the reducer performed.
    /// Injected so a run can exercise `on_response` without a live responder, correlated by `token`.
    Response {
        /// The contract-id the reply answers.
        contract: ContractId,
        /// The continuation token correlating the reply to its request; empty by default.
        token: Bytes,
        /// The reply: `Ok` the contract's output value (opaque bytes), or `Err` a runtime-level failure.
        answer: Result<Bytes, Error>,
    },
}

impl DeliveryEvent {
    /// Realize this as a platform [`Delivered`], stamping a message with the harness's synthetic external
    /// origin (an injected event has no real sending reducer). Any blob reference in the payload was already
    /// resolved to bytes by [`resolve_references`] at decode.
    fn into_delivered(self) -> Delivered {
        match self {
            DeliveryEvent::Message {
                contract,
                payload,
                token,
                from,
            } => Delivered::Message(Message {
                id: contract,
                payload,
                from: from.unwrap_or_else(external_origin),
                continuation_token: token,
            }),
            DeliveryEvent::Notification { contract, payload } => {
                Delivered::Notification(Notification {
                    id: contract,
                    payload,
                })
            }
            DeliveryEvent::Response {
                contract,
                token,
                answer,
            } => Delivered::Response(Response {
                id: contract,
                continuation_token: token,
                payload: answer,
            }),
        }
    }
}

/// Why a harness description could not be read. Each carries enough to point the author at the defect —
/// the decoder is total, so a bad description is a rejected value, not a panic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpecError {
    /// The bytes are not a decodable canonical Cadenza AST.
    Undecodable,
    /// The root value is not a `("record" …)`.
    NotARecord,
    /// A required field is absent.
    MissingField(&'static str),
    /// A field is present but the wrong shape (`want` describes what was expected).
    WrongType {
        /// The field name.
        field: &'static str,
        /// A short description of the shape the decoder expected.
        want: &'static str,
    },
    /// A blob record names neither `bytes` nor `path`, or names both — exactly one source is required.
    BlobSource {
        /// The blob's name (or `"?"` if that too was missing).
        blob: String,
    },
    /// A spawn's `kind` field is not `"ordinary"` or `"event"`.
    UnknownKind {
        /// The unrecognized kind string.
        kind: String,
    },
    /// A delivery record names neither `message` nor `notification`, or names both — exactly one event kind
    /// is required.
    DeliveryKind {
        /// The delivery's target task name (or `"?"` if that too was missing).
        target: String,
    },
    /// A spawn — or the registry's default handler or an override — names a program blob not declared in
    /// `blobs`. A cross-reference error found by [`validate`](HarnessSpec::validate), not a shape error.
    UnknownBlob {
        /// What referred to the blob: a task name, `"registry default"`, or `"registry handler"`.
        referrer: String,
        /// The undeclared blob name.
        blob: String,
    },
    /// A spawn's `parent` names a task not spawned earlier in the run (lineage must be ordered — a parent
    /// appears before its child).
    UnknownParent {
        /// The child task.
        task: String,
        /// The parent name that resolves to no earlier task.
        parent: String,
    },
    /// Two spawns share a task name — a task's name is its handle in the run, so it must be unique.
    DuplicateTask {
        /// The repeated task name.
        task: String,
    },
    /// A delivery's `target` names a task the run does not spawn.
    UnknownTarget {
        /// The delivery target that resolves to no task.
        target: String,
    },
    /// A graph edge's `from` emitter or a `to` transform names a task the run does not spawn.
    UnknownEdgeTask {
        /// The edge endpoint (emitter or transform) that resolves to no task.
        task: String,
    },
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpecError::Undecodable => write!(f, "input is not a decodable Cadenza binary AST"),
            SpecError::NotARecord => {
                write!(f, "the harness description must be a (record …) value")
            }
            SpecError::MissingField(field) => write!(f, "missing required field `{field}`"),
            SpecError::WrongType { field, want } => {
                write!(f, "field `{field}` has the wrong shape (expected {want})")
            }
            SpecError::BlobSource { blob } => write!(
                f,
                "blob `{blob}` must give exactly one of `bytes` (inline) or `path`"
            ),
            SpecError::UnknownKind { kind } => {
                write!(
                    f,
                    "unknown reducer kind `{kind}` (expected `ordinary` or `event`)"
                )
            }
            SpecError::DeliveryKind { target } => write!(
                f,
                "delivery to `{target}` must give exactly one of `message` or `notification`"
            ),
            SpecError::UnknownBlob { referrer, blob } => write!(
                f,
                "`{referrer}` names blob `{blob}`, which is not declared in `blobs`"
            ),
            SpecError::UnknownParent { task, parent } => write!(
                f,
                "task `{task}` names parent `{parent}`, which is not a task spawned earlier in the run"
            ),
            SpecError::DuplicateTask { task } => write!(f, "duplicate task name `{task}`"),
            SpecError::UnknownTarget { target } => {
                write!(f, "delivery target `{target}` is not a task the run spawns")
            }
            SpecError::UnknownEdgeTask { task } => {
                write!(
                    f,
                    "graph edge names task `{task}`, which is not a task the run spawns"
                )
            }
        }
    }
}

impl std::error::Error for SpecError {}

impl HarnessSpec {
    /// Decode a harness description from a Cadenza binary AST. Total: any malformation is a [`SpecError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, SpecError> {
        Self::decode_with(bytes, |_| None)
    }

    /// Decode a run, resolving `BlobBytes(<name>)` / `BlobHash(<name>)` reference pseudo-functions BEFORE
    /// interpreting the value — so a reference resolves wherever it appears (any field), not special-cased per
    /// field (`design/cadenza-platform.md` §3/§8). `load` materializes a `path`-form blob's bytes (the harness
    /// supplies guest programs by path); [`decode`](Self::decode) passes a no-op loader for the all-inline case.
    pub fn decode_with(
        bytes: &[u8],
        load: impl FnMut(&str) -> Option<Bytes>,
    ) -> Result<Self, SpecError> {
        let arenas = codec::decode(bytes).ok_or(SpecError::Undecodable)?;
        let resolved = resolve_references(&arenas, load)?;
        Self::read(&resolved, resolved.root)
    }

    /// Interpret an already-decoded AST node as a harness description — the shape-reading core, split out so
    /// it is testable against an in-memory `Arenas` without a full encode/decode round-trip.
    pub fn read(arenas: &Arenas, root: StructId) -> Result<Self, SpecError> {
        if !is_record(arenas, root) {
            return Err(SpecError::NotARecord);
        }

        let run_for = match record_field(arenas, root, "run-for") {
            None => None,
            Some(id) => Some(Duration::from_nanos(read_uint(arenas, id).ok_or(
                SpecError::WrongType {
                    field: "run-for",
                    want: "an unsigned integer of nanoseconds",
                },
            )?)),
        };

        let blobs = match record_field(arenas, root, "blobs") {
            None => Vec::new(),
            Some(id) => list_items(arenas, id)
                .ok_or(SpecError::WrongType {
                    field: "blobs",
                    want: "a (list …) of blob records",
                })?
                .iter()
                .map(|&b| read_blob(arenas, b))
                .collect::<Result<_, _>>()?,
        };

        let spawns = match record_field(arenas, root, "spawns") {
            None => Vec::new(),
            Some(id) => list_items(arenas, id)
                .ok_or(SpecError::WrongType {
                    field: "spawns",
                    want: "a (list …) of spawn records",
                })?
                .iter()
                .map(|&s| read_spawn(arenas, s))
                .collect::<Result<_, _>>()?,
        };

        let deliveries = match record_field(arenas, root, "deliver") {
            None => Vec::new(),
            Some(id) => list_items(arenas, id)
                .ok_or(SpecError::WrongType {
                    field: "deliver",
                    want: "a (list …) of delivery records",
                })?
                .iter()
                .map(|&d| read_delivery(arenas, d))
                .collect::<Result<_, _>>()?,
        };

        let checker = str_field(arenas, root, "checker")?.map(str::to_string);

        let pure_run = match record_field(arenas, root, "pure-run") {
            None => None,
            Some(id) => Some(read_pure_run(arenas, id)?),
        };

        let deps = match record_field(arenas, root, "deps") {
            None => Vec::new(),
            Some(id) => list_items(arenas, id)
                .ok_or(SpecError::WrongType {
                    field: "deps",
                    want: "a (list …) of dependency records",
                })?
                .iter()
                .map(|&d| read_dep(arenas, d))
                .collect::<Result<_, _>>()?,
        };

        let registry = match record_field(arenas, root, "registry") {
            None => return Err(SpecError::MissingField("registry")),
            Some(id) => read_registry(arenas, id)?,
        };

        let edges = match record_field(arenas, root, "edges") {
            None => Vec::new(),
            Some(id) => list_items(arenas, id)
                .ok_or(SpecError::WrongType {
                    field: "edges",
                    want: "a (list …) of edge records",
                })?
                .iter()
                .map(|&e| read_edge(arenas, e))
                .collect::<Result<_, _>>()?,
        };

        Ok(HarnessSpec {
            run_for,
            blobs,
            spawns,
            deliveries,
            checker,
            pure_run,
            deps,
            registry,
            edges,
        })
    }

    /// Check the description's internal name references before it is run: the registry's default handler and
    /// every spawn's blob are declared in `blobs`, each spawn's `parent` is a task spawned earlier, task names
    /// are unique, and every delivery targets a spawned task. Where [`decode`](HarnessSpec::decode) checks the
    /// *shape* of the value, this checks the *cross-references* a run resolves — so a caller (the executable)
    /// can reject a malformed run with a pointed [`SpecError`] and a clean exit, rather than letting the run
    /// fail deep in name resolution. Pure: it reads only the spec's own fields and touches no store.
    pub fn validate(&self) -> Result<(), SpecError> {
        use std::collections::BTreeSet;
        let blobs: BTreeSet<&str> = self.blobs.iter().map(|b| b.name.as_str()).collect();
        // Spawns resolve in order: a parent must appear before its child, and each task name is claimed once.
        let mut spawned: BTreeSet<&str> = BTreeSet::new();
        for spawn in &self.spawns {
            if !blobs.contains(spawn.blob_name()) {
                return Err(SpecError::UnknownBlob {
                    referrer: spawn.task_name().to_string(),
                    blob: spawn.blob_name().to_string(),
                });
            }
            if let Parent::Named(parent) = spawn.parent()
                && !spawned.contains(parent.as_str())
            {
                return Err(SpecError::UnknownParent {
                    task: spawn.task_name().to_string(),
                    parent: parent.clone(),
                });
            }
            if !spawned.insert(spawn.task_name()) {
                return Err(SpecError::DuplicateTask {
                    task: spawn.task_name().to_string(),
                });
            }
        }
        for (target, _event) in &self.deliveries {
            if !spawned.contains(target.as_str()) {
                return Err(SpecError::UnknownTarget {
                    target: target.clone(),
                });
            }
        }
        // A pure run names a program to run; it must be a declared blob (like any spawn). The run is
        // standalone (no spawn/delivery), so nothing else about it cross-references the spawn set.
        if let Some(pure_run) = &self.pure_run
            && !blobs.contains(pure_run.program.as_str())
        {
            return Err(SpecError::UnknownBlob {
                referrer: "pure-run".to_string(),
                blob: pure_run.program.clone(),
            });
        }
        // The registry names its default handler and each per-contract override handler by blob name; every
        // one must be declared, like any spawn.
        if !blobs.contains(self.registry.default.as_str()) {
            return Err(SpecError::UnknownBlob {
                referrer: "registry default".to_string(),
                blob: self.registry.default.clone(),
            });
        }
        for (_contract, program) in &self.registry.handlers {
            if !blobs.contains(program.as_str()) {
                return Err(SpecError::UnknownBlob {
                    referrer: "registry handler".to_string(),
                    blob: program.clone(),
                });
            }
        }
        // A graph edge seeds a transform chain between spawned tasks: its `from` emitter and every `to`
        // transform must be a task the run spawns (like a delivery target).
        for edge in &self.edges {
            if !spawned.contains(edge.from.as_str()) {
                return Err(SpecError::UnknownEdgeTask {
                    task: edge.from.clone(),
                });
            }
            for to in &edge.to {
                if !spawned.contains(to.as_str()) {
                    return Err(SpecError::UnknownEdgeTask { task: to.clone() });
                }
            }
        }
        Ok(())
    }

    /// Turn the description into a runnable [`Harness`], resolving each `path`-sourced blob through
    /// `load_path` (an inline blob passes straight through, so `load_path` is only ever called for a path).
    /// The loader's error type `E` is threaded out unchanged — the binary passes a filesystem read; a test
    /// passing only inline blobs can use any `E` since the loader is never invoked.
    pub fn build<E>(
        self,
        mut load_path: impl FnMut(&str) -> Result<Bytes, E>,
    ) -> Result<Harness, E> {
        let mut harness = Harness::new(self.registry.default);
        if let Some(run_for) = self.run_for {
            harness = harness.run_for(run_for);
        }
        for blob in self.blobs {
            let bytes = match blob.source {
                BlobSource::Inline(bytes) => bytes,
                BlobSource::Path(path) => load_path(&path)?,
            };
            harness = harness.blob(blob.name, bytes);
        }
        for spawn in self.spawns {
            harness = harness.spawn(spawn);
        }
        // Payloads carry literal bytes here — any `BlobBytes`/`BlobHash` reference was resolved by
        // `resolve_references` at decode.
        for (target, event) in self.deliveries {
            harness = harness.deliver(target, event.into_delivered());
        }
        // The default handler is already set via `Harness::new` above; layer per-contract overrides only when
        // there are any.
        if !self.registry.handlers.is_empty() {
            harness = harness.registry(self.registry.handlers);
        }
        // Seed the reducer-graph transform chains, if any (§4) — resolved from names to ids at run time.
        if !self.edges.is_empty() {
            harness = harness.edges(
                self.edges
                    .into_iter()
                    .map(|e| (e.from, e.contract, e.to))
                    .collect(),
            );
        }
        Ok(harness)
    }

    /// Encode this description to a Cadenza binary AST — the exact inverse of [`decode`](HarnessSpec::decode),
    /// producing the bytes the executable reads. A field at its default (an absent `run-for`, an ordinary
    /// spawn kind, a root parent, an empty message token) is omitted, so `decode(spec.encode())` recovers an
    /// equal `HarnessSpec` — for a spec whose delivered messages carry the harness's synthetic `from` origin
    /// (the only `from` decode produces). This is how a test or build step produces a `harness.ast`
    /// programmatically rather than by hand.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Builder::new();
        let root = self.to_ast(&mut b);
        codec::encode(&b.finish(root))
    }

    /// Build the `("record" …)` node for this description in `b`, returning its id. Split from
    /// [`encode`](HarnessSpec::encode) so a value can also be nested in a larger AST if ever needed.
    fn to_ast(&self, b: &mut Builder) -> StructId {
        let mut fields: Vec<(&str, StructId)> = Vec::with_capacity(4);
        if let Some(run_for) = self.run_for {
            let ns = u64::try_from(run_for.as_nanos()).unwrap_or(u64::MAX);
            let run_for = uint_leaf(b, ns);
            fields.push(("run-for", run_for));
        }
        if !self.blobs.is_empty() {
            let items = self.blobs.iter().map(|blob| blob_to_ast(b, blob)).collect();
            let blobs = list_value(b, items);
            fields.push(("blobs", blobs));
        }
        if !self.spawns.is_empty() {
            let items = self
                .spawns
                .iter()
                .map(|spawn| spawn_to_ast(b, spawn))
                .collect();
            let spawns = list_value(b, items);
            fields.push(("spawns", spawns));
        }
        if !self.deliveries.is_empty() {
            let items = self
                .deliveries
                .iter()
                .map(|(target, event)| delivery_to_ast(b, target, event))
                .collect();
            let deliver = list_value(b, items);
            fields.push(("deliver", deliver));
        }
        if let Some(checker) = &self.checker {
            let checker = str_leaf(b, checker);
            fields.push(("checker", checker));
        }
        if let Some(pure_run) = &self.pure_run {
            let pure_run = pure_run_to_ast(b, pure_run);
            fields.push(("pure-run", pure_run));
        }
        if !self.deps.is_empty() {
            let items = self
                .deps
                .iter()
                .map(|source| dep_to_ast(b, source))
                .collect();
            let deps = list_value(b, items);
            fields.push(("deps", deps));
        }
        let registry = registry_to_ast(b, &self.registry);
        fields.push(("registry", registry));
        if !self.edges.is_empty() {
            let items = self.edges.iter().map(|edge| edge_to_ast(b, edge)).collect();
            let edges = list_value(b, items);
            fields.push(("edges", edges));
        }
        record(b, fields)
    }
}

/// The `("record" (= default …) (= handlers ("list" …)?))` node for a registry — the inverse of
/// [`read_registry`]. Empty `handlers` is omitted (the decode default).
fn registry_to_ast(b: &mut Builder, registry: &RegistrySpec) -> StructId {
    let default = str_leaf(b, &registry.default);
    let mut fields = vec![("default", default)];
    if !registry.handlers.is_empty() {
        let items = registry
            .handlers
            .iter()
            .map(|(contract, program)| {
                let contract = bytes_leaf(b, contract.hash().as_bytes());
                let program = str_leaf(b, program);
                record(b, vec![("contract", contract), ("program", program)])
            })
            .collect();
        let handlers = list_value(b, items);
        fields.push(("handlers", handlers));
    }
    record(b, fields)
}

/// The `("record" (= from …) (= contract b"…33") (= to ("list" …)))` node for one edge — the inverse of
/// [`read_edge`]. The contract crosses as its raw 33 tagged bytes (like a delivery's contract).
fn edge_to_ast(b: &mut Builder, edge: &GraphEdge) -> StructId {
    let from = str_leaf(b, &edge.from);
    let contract = bytes_leaf(b, edge.contract.hash().as_bytes());
    let to_items = edge.to.iter().map(|t| str_leaf(b, t)).collect();
    let to = list_value(b, to_items);
    record(b, vec![("from", from), ("contract", contract), ("to", to)])
}

/// The `("record" (= bytes …)|(= path …))` node for one unnamed dependency — the inverse of [`read_dep`]
/// (a blob source with no `name`).
fn dep_to_ast(b: &mut Builder, source: &BlobSource) -> StructId {
    let field = match source {
        BlobSource::Inline(bytes) => ("bytes", bytes_leaf(b, bytes)),
        BlobSource::Path(path) => ("path", str_leaf(b, path)),
    };
    record(b, vec![field])
}

/// The `("record" (= program …) (= contract …) (= input …) (= expect-output …))` node for a pure run — the
/// inverse of [`read_pure_run`]. The contract crosses as its raw bytes (the byte form `read_contract_id` also
/// accepts, alongside the base62-string form a name→id rewrite produces).
fn pure_run_to_ast(b: &mut Builder, pure_run: &PureRun) -> StructId {
    let program = str_leaf(b, &pure_run.program);
    let contract = bytes_leaf(b, pure_run.contract.hash().as_bytes());
    let input = bytes_leaf(b, &pure_run.input);
    let expect_output = bytes_leaf(b, &pure_run.expect_output);
    record(
        b,
        vec![
            ("program", program),
            ("contract", contract),
            ("input", input),
            ("expect-output", expect_output),
        ],
    )
}

/// A string leaf `"text"`.
fn str_leaf(b: &mut Builder, text: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(text)))
}

/// A `#list(e…)` value — the M2 NATIVE list constructor (`Leaf::Ctor(CompoundCtor::List)` head). The M3
/// reader-flip (#6528) dropped the legacy STRING-head `("list" …)` form, so fixtures build the native head.
fn list_value(b: &mut Builder, items: Vec<StructId>) -> StructId {
    b.compound(CompoundCtor::List, &items)
}

/// The `("record" (= name …) (= bytes …)|(= path …))` node for one blob — the inverse of [`read_blob`].
fn blob_to_ast(b: &mut Builder, blob: &BlobSpec) -> StructId {
    let name = str_leaf(b, &blob.name);
    let source = match &blob.source {
        BlobSource::Inline(bytes) => ("bytes", bytes_leaf(b, bytes)),
        BlobSource::Path(path) => ("path", str_leaf(b, path)),
    };
    record(b, vec![("name", name), source])
}

/// The `("record" (= name …) (= blob …) (= parent …)? (= kind …)? (= links …)?)` node for one spawn — the
/// inverse of [`read_spawn`]. A root parent, an ordinary kind, and no supervision links are the decode
/// defaults, so they are omitted; only a `true` link flag is written, so a round-trip recovers the same links.
fn spawn_to_ast(b: &mut Builder, spawn: &SpawnSpec) -> StructId {
    let name = str_leaf(b, spawn.task_name());
    let blob = str_leaf(b, spawn.blob_name());
    let mut fields = vec![("name", name), ("blob", blob)];
    if let Parent::Named(parent) = spawn.parent() {
        let parent = str_leaf(b, parent);
        fields.push(("parent", parent));
    }
    if spawn.reducer_kind() == ReducerKind::Event {
        let kind = str_leaf(b, "event");
        fields.push(("kind", kind));
    }
    let links = spawn.supervision();
    if links != Links::NONE {
        let mut flags: Vec<(&str, StructId)> = Vec::new();
        if links.parent_watches_child {
            flags.push(("parentWatchesChild", uint_leaf(b, 1)));
        }
        if links.child_watches_parent {
            flags.push(("childWatchesParent", uint_leaf(b, 1)));
        }
        let links = record(b, flags);
        fields.push(("links", links));
    }
    record(b, fields)
}

/// The items of a `("list" e…)` value — accepting both the string head `"list"` and the bare name head
/// `list`, which denote the same construct.
fn list_items(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    // All three list spellings — native ctor-leaf head (rcdzc-compiled), `list` name alias, `("list" …)`
    // string — via `compound_form_of`.
    arenas.compound_form_of(id, CompoundCtor::List)
}

/// Whether `id` is a record value — the NAME-headed `(record …)` (the canonical Cadenza value form) or the
/// STRING-headed `("record" …)` (the `cdz convert --to binary` surface form a HarnessSpec arrives as). Liberal
/// about the head so a description reads either way, matching [`record_field`]/[`list_items`].
fn is_record(arenas: &Arenas, id: StructId) -> bool {
    // Recognize all THREE record spellings — the M2 native ctor-leaf head (what rcdzc compiles the harness
    // description to), the `record` name alias, and the legacy `("record" …)` string — via `compound_form_of`.
    // Before this, the native head read as "not a record", blocking §9 harness-run.
    arenas.compound_form_of(id, CompoundCtor::Record).is_some()
}

/// A record's field read as a string, if present. `Ok(None)` when the field is absent; a present-but-not-a-
/// string field is a [`SpecError::WrongType`].
fn str_field<'a>(
    arenas: &'a Arenas,
    record: StructId,
    field: &'static str,
) -> Result<Option<&'a str>, SpecError> {
    match record_field(arenas, record, field) {
        None => Ok(None),
        Some(id) => arenas.as_str(id).map(Some).ok_or(SpecError::WrongType {
            field,
            want: "a string",
        }),
    }
}

/// A record's field read as a `u64`, if present. `Ok(None)` when the field is absent; a present-but-not-a-
/// uint field is a [`SpecError::WrongType`].
fn uint_field(
    arenas: &Arenas,
    record: StructId,
    field: &'static str,
) -> Result<Option<u64>, SpecError> {
    match record_field(arenas, record, field) {
        None => Ok(None),
        Some(id) => read_uint(arenas, id).map(Some).ok_or(SpecError::WrongType {
            field,
            want: "a uint",
        }),
    }
}

/// Read one blob record: a `name` and exactly one of `bytes` (inline) or `path`.
fn read_blob(arenas: &Arenas, id: StructId) -> Result<BlobSpec, SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "blobs",
            want: "a (record …) per blob",
        });
    }
    let name = str_field(arenas, id, "name")?
        .ok_or(SpecError::MissingField("name"))?
        .to_string();
    let inline = record_field(arenas, id, "bytes");
    let path = record_field(arenas, id, "path");
    let source = match (inline, path) {
        (Some(b), None) => {
            BlobSource::Inline(read_bytes(arenas, b).ok_or(SpecError::WrongType {
                field: "bytes",
                want: "a bytes literal",
            })?)
        }
        (None, Some(p)) => BlobSource::Path(
            arenas
                .as_str(p)
                .ok_or(SpecError::WrongType {
                    field: "path",
                    want: "a string",
                })?
                .to_string(),
        ),
        _ => return Err(SpecError::BlobSource { blob: name }),
    };
    Ok(BlobSpec { name, source })
}

/// Read one spawn record into a [`SpawnSpec`]: a `name` and a `blob` (required), an optional `parent`
/// (absent ⇒ a root), and an optional `kind` (`"ordinary"` default, or `"event"`).
fn read_spawn(arenas: &Arenas, id: StructId) -> Result<SpawnSpec, SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "spawns",
            want: "a (record …) per spawn",
        });
    }
    let name = str_field(arenas, id, "name")?.ok_or(SpecError::MissingField("name"))?;
    let blob = str_field(arenas, id, "blob")?.ok_or(SpecError::MissingField("blob"))?;
    let mut spec = SpawnSpec::new(name, blob);
    if let Some(parent) = str_field(arenas, id, "parent")? {
        spec = spec.child_of(parent);
    }
    if let Some(kind) = str_field(arenas, id, "kind")? {
        spec = spec.kind(match kind {
            "ordinary" => ReducerKind::Ordinary,
            "event" => ReducerKind::Event,
            other => {
                return Err(SpecError::UnknownKind {
                    kind: other.to_string(),
                });
            }
        });
    }
    // Optional supervision links (§7): a `links` sub-record whose two flags say which lifecycle events flow
    // between this task and its parent. Each true flag becomes a `watch_exit` edge the system reads on exit to
    // deliver a peer's `Exited`/`Crashed` event. A flag is a `0`/`1` uint (the log's flag convention); absent ⇒
    // false, so a spawn with no `links` field keeps the default of no supervision in either direction.
    if let Some(links_id) = record_field(arenas, id, "links") {
        if !is_record(arenas, links_id) {
            return Err(SpecError::WrongType {
                field: "links",
                want: "a (record …) of supervision flags",
            });
        }
        spec = spec.links(Links {
            parent_watches_child: uint_field(arenas, links_id, "parentWatchesChild")?.unwrap_or(0)
                != 0,
            child_watches_parent: uint_field(arenas, links_id, "childWatchesParent")?.unwrap_or(0)
                != 0,
        });
    }
    Ok(spec)
}

/// Read one delivery record: a `target` task name and exactly one event — a `message` or a `notification`
/// sub-record. Returns the target and the [`DeliveryEvent`].
fn read_delivery(arenas: &Arenas, id: StructId) -> Result<(String, DeliveryEvent), SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "deliver",
            want: "a (record …) per delivery",
        });
    }
    let target = str_field(arenas, id, "target")?
        .ok_or(SpecError::MissingField("target"))?
        .to_string();
    let message = record_field(arenas, id, "message");
    let notification = record_field(arenas, id, "notification");
    let response = record_field(arenas, id, "response");
    let event = match (message, notification, response) {
        (Some(m), None, None) => DeliveryEvent::Message {
            contract: read_contract_id(arenas, m)?,
            payload: read_required_bytes(arenas, m, "payload")?,
            token: read_optional_token(arenas, m)?,
            from: read_optional_from(arenas, m)?,
        },
        (None, Some(n), None) => DeliveryEvent::Notification {
            contract: read_contract_id(arenas, n)?,
            payload: read_required_bytes(arenas, n, "payload")?,
        },
        (None, None, Some(r)) => DeliveryEvent::Response {
            contract: read_contract_id(arenas, r)?,
            token: read_optional_token(arenas, r)?,
            answer: read_answer(arenas, r)?,
        },
        _ => return Err(SpecError::DeliveryKind { target }),
    };
    Ok((target, event))
}

/// Read a message record's optional `from` sender — a `("record" (= reducer b"…") (= host b"…"))` of two
/// hash-byte fields, or `None` when absent (the harness then stamps its synthetic external origin). A
/// present-but-malformed `from` is a [`SpecError`].
fn read_optional_from(arenas: &Arenas, event: StructId) -> Result<Option<Origin>, SpecError> {
    let Some(from) = record_field(arenas, event, "from") else {
        return Ok(None);
    };
    let reducer = read_required_bytes(arenas, from, "reducer")?;
    let host = read_required_bytes(arenas, from, "host")?;
    Ok(Some(Origin {
        reducer: ReducerId::try_from(reducer.as_ref()).map_err(|_| SpecError::WrongType {
            field: "from",
            want: "a 33-byte reducer id",
        })?,
        host: HostId::try_from(host.as_ref()).map_err(|_| SpecError::WrongType {
            field: "from",
            want: "a 33-byte host id",
        })?,
    }))
}

/// The `("record" (= reducer b"…") (= host b"…"))` node for a message's `from` sender — the inverse of the
/// `from` read in [`read_optional_from`].
fn origin_to_ast(b: &mut Builder, origin: &Origin) -> StructId {
    let reducer = bytes_leaf(b, origin.reducer.hash().as_bytes());
    let host = bytes_leaf(b, origin.host.hash().as_bytes());
    record(b, vec![("reducer", reducer), ("host", host)])
}

/// Read an event record's optional `token` field — a bytes literal, or empty when absent (the decode
/// default a message/response shares). A present-but-not-bytes token is a [`SpecError`].
fn read_optional_token(arenas: &Arenas, event: StructId) -> Result<Bytes, SpecError> {
    match record_field(arenas, event, "token") {
        None => Ok(Bytes::new()),
        Some(t) => read_bytes(arenas, t).ok_or(SpecError::WrongType {
            field: "token",
            want: "a bytes literal",
        }),
    }
}

/// Read a response record's `answer` — the platform result form `(Ok <bytes>)` (the output value) or
/// `(Err <error>)` (a runtime failure, an error-tag string). Absent/malformed is a [`SpecError`].
fn read_answer(arenas: &Arenas, response: StructId) -> Result<Result<Bytes, Error>, SpecError> {
    let want = "(Ok <bytes>) or (Err <error>)";
    let id = record_field(arenas, response, "answer").ok_or(SpecError::MissingField("answer"))?;
    let Struct::List(items) = arenas.get(id) else {
        return Err(SpecError::WrongType {
            field: "answer",
            want,
        });
    };
    let (&head, tail) = items.split_first().ok_or(SpecError::WrongType {
        field: "answer",
        want,
    })?;
    match arenas.as_name(head) {
        Some("Ok") => {
            let [v] = <[StructId; 1]>::try_from(tail).map_err(|_| SpecError::WrongType {
                field: "answer",
                want: "a single bytes payload in (Ok …)",
            })?;
            Ok(Ok(read_bytes(arenas, v).ok_or(SpecError::WrongType {
                field: "answer",
                want: "a bytes literal in (Ok …)",
            })?))
        }
        Some("Err") => {
            let [e] = <[StructId; 1]>::try_from(tail).map_err(|_| SpecError::WrongType {
                field: "answer",
                want: "a single error tag in (Err …)",
            })?;
            let tag = arenas.as_str(e).ok_or(SpecError::WrongType {
                field: "answer",
                want: "an error-tag string in (Err …)",
            })?;
            Ok(Err(read_error_tag(tag)?))
        }
        _ => Err(SpecError::WrongType {
            field: "answer",
            want,
        }),
    }
}

/// Map a runtime-failure error tag to its [`Error`] — the inverse of [`error_tag`].
fn read_error_tag(tag: &str) -> Result<Error, SpecError> {
    match tag {
        "timeout" => Ok(Error::Timeout),
        "missing-handler" => Ok(Error::MissingHandler),
        "schema-violation" => Ok(Error::SchemaViolation),
        "faulted" => Ok(Error::Faulted),
        _ => Err(SpecError::WrongType {
            field: "answer",
            want: "a known error tag: timeout | missing-handler | schema-violation | faulted",
        }),
    }
}

/// The error tag for a runtime-failure [`Error`] — the wire string in `(Err <tag>)`.
fn error_tag(e: Error) -> &'static str {
    match e {
        Error::Timeout => "timeout",
        Error::MissingHandler => "missing-handler",
        Error::SchemaViolation => "schema-violation",
        Error::Faulted => "faulted",
    }
}

/// Read the required `contract` field of an event record as a [`ContractId`] (its 33 raw hash bytes).
/// Read a `registry` sub-record into a [`RegistrySpec`]: a required `default` handler blob name, and an
/// optional `handlers` list of per-contract overrides — each a record `{ contract = <id>, program = <name> }`.
/// Liberal about the record head.
fn read_registry(arenas: &Arenas, id: StructId) -> Result<RegistrySpec, SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "registry",
            want: "a (record …) with default and optional handlers",
        });
    }
    let default = str_field(arenas, id, "default")?
        .ok_or(SpecError::MissingField("default"))?
        .to_string();
    let handlers = match record_field(arenas, id, "handlers") {
        None => Vec::new(),
        Some(list) => list_items(arenas, list)
            .ok_or(SpecError::WrongType {
                field: "handlers",
                want: "a (list …) of handler records",
            })?
            .iter()
            .map(|&h| read_handler(arenas, h))
            .collect::<Result<_, _>>()?,
    };
    Ok(RegistrySpec { default, handlers })
}

/// Read one `handlers` record into a `(contract-id, program name)` override — a record with a `contract`
/// (contract-id, by base62 string or raw bytes) and a `program` (handler blob name). Liberal about the head.
fn read_handler(arenas: &Arenas, id: StructId) -> Result<(ContractId, String), SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "handlers",
            want: "a (record …) with contract and program",
        });
    }
    let contract = read_contract_id(arenas, id)?;
    let program = str_field(arenas, id, "program")?
        .ok_or(SpecError::MissingField("program"))?
        .to_string();
    Ok((contract, program))
}

/// Read one `edges` record into a [`GraphEdge`] — a record with a `from` (emitting task name), a `contract`
/// (contract-id, by base62 string or raw bytes), and a `to` (a `(list …)` of transform task names in chain
/// order). Liberal about the record head.
fn read_edge(arenas: &Arenas, id: StructId) -> Result<GraphEdge, SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "edges",
            want: "a (record …) with from, contract and to",
        });
    }
    let from = str_field(arenas, id, "from")?
        .ok_or(SpecError::MissingField("from"))?
        .to_string();
    let contract = read_contract_id(arenas, id)?;
    let to_list = record_field(arenas, id, "to").ok_or(SpecError::MissingField("to"))?;
    let to = list_items(arenas, to_list)
        .ok_or(SpecError::WrongType {
            field: "to",
            want: "a (list …) of transform task names",
        })?
        .iter()
        .map(|&t| {
            arenas
                .as_str(t)
                .map(str::to_string)
                .ok_or(SpecError::WrongType {
                    field: "to",
                    want: "a task name string",
                })
        })
        .collect::<Result<_, _>>()?;
    Ok(GraphEdge { from, contract, to })
}

/// Read one `deps` record into a [`BlobSource`] — an UNNAMED content-addressed component: a record with
/// exactly one of `bytes` (inline) or `path` (a file the executable reads), like a blob but with no `name`
/// (nothing refers to a dep by name; it is resolved by content hash). Liberal about the record head.
fn read_dep(arenas: &Arenas, id: StructId) -> Result<BlobSource, SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "deps",
            want: "a (record …) with exactly one of bytes / path",
        });
    }
    let inline = record_field(arenas, id, "bytes");
    let path = record_field(arenas, id, "path");
    match (inline, path) {
        (Some(b), None) => Ok(BlobSource::Inline(read_bytes(arenas, b).ok_or(
            SpecError::WrongType {
                field: "bytes",
                want: "a bytes literal",
            },
        )?)),
        (None, Some(p)) => Ok(BlobSource::Path(
            arenas
                .as_str(p)
                .ok_or(SpecError::WrongType {
                    field: "path",
                    want: "a string",
                })?
                .to_string(),
        )),
        _ => Err(SpecError::BlobSource {
            blob: "deps".to_string(),
        }),
    }
}

/// Read a `pure-run` sub-record into a [`PureRun`]: a `program` blob name, the `contract` the input is run
/// against, the `input` bytes, and the `expect-output` bytes. Liberal about the record head (name- or
/// string-headed), so it reads both the canonical value form and the `cdz convert` surface form.
fn read_pure_run(arenas: &Arenas, id: StructId) -> Result<PureRun, SpecError> {
    if !is_record(arenas, id) {
        return Err(SpecError::WrongType {
            field: "pure-run",
            want: "a (record …) with program, contract, input, expect-output",
        });
    }
    let program = str_field(arenas, id, "program")?
        .ok_or(SpecError::MissingField("program"))?
        .to_string();
    let contract = read_contract_id(arenas, id)?;
    let input = read_required_bytes(arenas, id, "input")?;
    let expect_output = read_required_bytes(arenas, id, "expect-output")?;
    Ok(PureRun {
        program,
        contract,
        input,
        expect_output,
    })
}

fn read_contract_id(arenas: &Arenas, event: StructId) -> Result<ContractId, SpecError> {
    let id = record_field(arenas, event, "contract").ok_or(SpecError::MissingField("contract"))?;
    // A contract-id crosses either as its raw 33 tagged bytes, or as a base62 string (§8) — the text form
    // a name→id rewrite substitutes for a contract name (the platform does no name resolution; that mapping
    // is produced outside and rewritten into the spec, so a resolved name arrives here as its base62 id).
    if let Some(text) = arenas.as_str(id) {
        let hash = text.parse::<Hash>().map_err(|_| SpecError::WrongType {
            field: "contract",
            want: "a base62 contract-id",
        })?;
        Ok(ContractId::from_hash(hash))
    } else {
        let bytes = read_bytes(arenas, id).ok_or(SpecError::WrongType {
            field: "contract",
            want: "a base62 string or a 33-byte contract-id",
        })?;
        ContractId::try_from(bytes.as_ref()).map_err(|_| SpecError::WrongType {
            field: "contract",
            want: "a 33-byte contract-id",
        })
    }
}

/// The run's blobs by name — the table a `BlobBytes`/`BlobHash` reference resolves against. An inline
/// (`bytes = b"…"`) blob contributes its bytes directly; a `path = "…"` blob contributes the bytes `load`
/// materializes from that path (the harness supplies guest programs by path, so a reference to one resolves
/// only once the loader can read it — at build, not decode). A path a caller supplies no loader for (or that
/// fails to read) is absent, so a reference to it is a clean rejection rather than empty bytes.
fn blob_table(
    arenas: &Arenas,
    mut load: impl FnMut(&str) -> Option<Bytes>,
) -> BTreeMap<String, Bytes> {
    let mut table = BTreeMap::new();
    let Some(blobs) =
        record_field(arenas, arenas.root, "blobs").and_then(|id| list_items(arenas, id))
    else {
        return table;
    };
    for &blob in blobs {
        let Ok(Some(name)) = str_field(arenas, blob, "name") else {
            continue;
        };
        let bytes = if let Some(bytes) =
            record_field(arenas, blob, "bytes").and_then(|id| read_bytes(arenas, id))
        {
            Some(bytes)
        } else if let Some(path) =
            record_field(arenas, blob, "path").and_then(|id| arenas.as_str(id))
        {
            load(path)
        } else {
            None
        };
        if let Some(bytes) = bytes {
            table.insert(name.to_string(), bytes);
        }
    }
    table
}

/// Resolve the reference pseudo-functions in a decoded harness-spec value: rewrite every APPLICATION node
/// headed by the reserved `BlobBytes` / `BlobHash` — WHEREVER it appears (any field), not a per-field
/// convention — to the named blob's content bytes / content hash, from the run's inline blobs. Blob-name
/// resolution is thus an explicit resolve CALL (its head names how to resolve), the cleaner and more general
/// shape (`design/cadenza-platform.md` §3/§8). Everything else is copied unchanged. An unknown name, a wrong
/// arity, or a non-literal name arg is a clean [`SpecError`] — reject rather than resolve to nothing.
fn resolve_references(
    arenas: &Arenas,
    load: impl FnMut(&str) -> Option<Bytes>,
) -> Result<Arenas, SpecError> {
    let table = blob_table(arenas, load);
    let mut b = Builder::new();
    let root = rewrite_references(arenas, arenas.root, &table, &mut b)?;
    Ok(b.finish(root))
}

/// Copy `id` from `old` into `b`, rewriting a `BlobBytes(<name>)` / `BlobHash(<name>)` application node to its
/// resolved bytes; recurse into every child so a reference nested in a list/record resolves too.
fn rewrite_references(
    old: &Arenas,
    id: StructId,
    table: &BTreeMap<String, Bytes>,
    b: &mut Builder,
) -> Result<StructId, SpecError> {
    match old.get(id) {
        Struct::Atom(leaf) => Ok(b.atom_leaf(old.leaf(*leaf).clone())),
        Struct::List(children) => {
            // `Value("<Type>", <value>)` — encode the given Cadenza value to the canonical binary form a guest
            // `Value.decode`s (a message/notification `payload` that is a STRUCTURED value, not opaque bytes),
            // so a reducer can decode + dispatch on it by schema (§3/§4). Resolves like the blob references
            // (WHEREVER it appears): the value is ascribed with `<Type>` at the encode boundary — the root
            // ascription `Value.decode` requires — and any nested blob reference in the value resolves first.
            // The `<value>` is written in ordinary value form: a single-constructor sum ELIDES its constructor
            // (write the payload directly — `Value("Effect", b"x")`, not `Value("Effect", Perform(b"x"))`, since
            // `Effect = | Perform(Bytes)`); a multi-constructor sum keeps its constructor head
            // (`Value("T", SomeCtor(...))`). A record/list payload is written in plain ML syntax
            // (`{ key = b"K", value = b"V" }`, `[b"a", b"b"]`) and canonicalized on encode (see
            // `rewrite_value_canonical`). The type token is the schema type name.
            if let Some((&head, args)) = children.split_first()
                && old.as_name(head) == Some("Value")
            {
                let [type_id, value_id] =
                    <[StructId; 2]>::try_from(args).map_err(|_| SpecError::WrongType {
                        field: "Value reference",
                        want: "a type name and a value: Value(\"<Type>\", <value>)",
                    })?;
                let ty = old.as_str(type_id).ok_or(SpecError::WrongType {
                    field: "Value reference",
                    want: "a type-name string as the first arg of Value(\"<Type>\", <value>)",
                })?;
                // Encode the value subtree standalone: copy+resolve it into a fresh arena, ascribe it with the
                // type, and encode — the bytes a guest `Value.decode`s type-directed against `<Type>`. The copy
                // is CANONICALIZING (`rewrite_value_canonical`, not the plain `rewrite_references`): the ML
                // surface a run is written in heads records/lists with the string constructors `("record" …)`/
                // `("list" …)` and keeps record fields in declaration order, but the compiler's `Value.encode`
                // emits the NAME-headed `(record …)`/`(list …)` with fields ascending by name — so an author can
                // write a structured payload in ordinary ML record/list syntax and it still decodes.
                let mut vb = Builder::new();
                let inner = rewrite_value_canonical(old, value_id, table, &mut vb)?;
                let ascribed = ascribe(&mut vb, inner, ty);
                let bytes = codec::encode(&vb.finish(ascribed));
                return Ok(bytes_leaf(b, &bytes));
            }
            if let Some((&head, args)) = children.split_first()
                && let Some(ctor @ ("BlobBytes" | "BlobHash")) = old.as_name(head)
            {
                let [name_id] =
                    <[StructId; 1]>::try_from(args).map_err(|_| SpecError::WrongType {
                        field: "blob reference",
                        want: "a single blob name in BlobBytes(<name>) / BlobHash(<name>)",
                    })?;
                let name = old.as_str(name_id).ok_or(SpecError::WrongType {
                    field: "blob reference",
                    want: "a blob-name string in BlobBytes(<name>) / BlobHash(<name>)",
                })?;
                let bytes = table.get(name).ok_or_else(|| SpecError::UnknownBlob {
                    referrer: format!("{ctor} reference"),
                    blob: name.to_string(),
                })?;
                let resolved = if ctor == "BlobHash" {
                    Bytes::copy_from_slice(ProgramHash::of(bytes).hash().as_bytes())
                } else {
                    bytes.clone()
                };
                return Ok(bytes_leaf(b, &resolved));
            }
            let children = children
                .iter()
                .map(|&c| rewrite_references(old, c, table, b))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(b.list(children))
        }
    }
}

/// Copy a `Value("<Type>", <value>)` payload subtree into `b` in the CANONICAL value form the codec produces —
/// the shape a guest `Value.decode`s type-directed. The ML surface a run is written in (`cdz convert`) heads
/// records/lists with the STRING constructors `("record" …)`/`("list" …)` and keeps record fields in
/// DECLARATION order, but the compiler's own `Value.encode` emits the NAME-headed `(record …)`/`(list …)` with
/// record fields ascending by NAME (see [`crate::contract_value::record`]); a guest decoding the string-headed
/// or unsorted form fails, because the runtime decoder — unlike our own liberal readers ([`record_field`]/
/// [`list_items`]) — reads only the canonical head/order. So records are rebuilt name-headed + sorted and lists
/// name-headed, recursively, while nested blob references still resolve exactly as in [`rewrite_references`] and
/// every other node (bare-name constructors, ascriptions, leaves) copies through unchanged. Idempotent on an
/// already-canonical subtree.
fn rewrite_value_canonical(
    old: &Arenas,
    id: StructId,
    table: &BTreeMap<String, Bytes>,
    b: &mut Builder,
) -> Result<StructId, SpecError> {
    match old.get(id) {
        Struct::Atom(leaf) => Ok(b.atom_leaf(old.leaf(*leaf).clone())),
        Struct::List(children) => {
            // A nested blob reference resolves to its bytes/hash, exactly as in the general resolve pass.
            if let Some((&head, args)) = children.split_first()
                && let Some(ctor @ ("BlobBytes" | "BlobHash")) = old.as_name(head)
            {
                let [name_id] =
                    <[StructId; 1]>::try_from(args).map_err(|_| SpecError::WrongType {
                        field: "blob reference",
                        want: "a single blob name in BlobBytes(<name>) / BlobHash(<name>)",
                    })?;
                let name = old.as_str(name_id).ok_or(SpecError::WrongType {
                    field: "blob reference",
                    want: "a blob-name string in BlobBytes(<name>) / BlobHash(<name>)",
                })?;
                let bytes = table.get(name).ok_or_else(|| SpecError::UnknownBlob {
                    referrer: format!("{ctor} reference"),
                    blob: name.to_string(),
                })?;
                let resolved = if ctor == "BlobHash" {
                    Bytes::copy_from_slice(ProgramHash::of(bytes).hash().as_bytes())
                } else {
                    bytes.clone()
                };
                return Ok(bytes_leaf(b, &resolved));
            }
            // A record → the canonical NAME-headed, ascending-name-sorted `(record …)`, each field value
            // recursively canonicalized. Accept either head (`is_record`) so it is idempotent.
            if is_record(old, id) {
                let fields = old
                    .compound_form_of(id, CompoundCtor::Record)
                    .expect("is_record just matched a record head");
                let mut out = Vec::with_capacity(fields.len());
                for &f in fields {
                    let kv = old.as_form(f, "=").ok_or(SpecError::WrongType {
                        field: "Value record field",
                        want: "a (= <name> <value>) field",
                    })?;
                    let [name_id, value_id] =
                        <[StructId; 2]>::try_from(kv).map_err(|_| SpecError::WrongType {
                            field: "Value record field",
                            want: "a (= <name> <value>) field",
                        })?;
                    let name = old.as_name(name_id).ok_or(SpecError::WrongType {
                        field: "Value record field",
                        want: "a field name",
                    })?;
                    let value = rewrite_value_canonical(old, value_id, table, b)?;
                    out.push((name, value));
                }
                return Ok(record(b, out));
            }
            // A list → the canonical NAME-headed `(list …)`, each element recursively canonicalized.
            if let Some(items) = list_items(old, id) {
                let elems = items
                    .iter()
                    .map(|&e| rewrite_value_canonical(old, e, table, b))
                    .collect::<Result<Vec<_>, _>>()?;
                let head = b.name("list");
                return Ok(b.list(std::iter::once(head).chain(elems).collect()));
            }
            // Everything else — a bare-name constructor `(Ctor …)`, an ascription `(: …)`, … — copies
            // structurally with its children canonicalized.
            let out = children
                .iter()
                .map(|&c| rewrite_value_canonical(old, c, table, b))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(b.list(out))
        }
    }
}

/// Read a required bytes-leaf field of a record, or a [`SpecError`] if absent or not a bytes literal.
fn read_required_bytes(
    arenas: &Arenas,
    record: StructId,
    field: &'static str,
) -> Result<Bytes, SpecError> {
    let id = record_field(arenas, record, field).ok_or(SpecError::MissingField(field))?;
    read_bytes(arenas, id).ok_or(SpecError::WrongType {
        field,
        want: "a bytes literal",
    })
}

/// The `("record" (= target …) (= message …)|(= notification …))` node for one delivery — the inverse of
/// [`read_delivery`]. An empty message token is the decode default, so it is omitted.
fn delivery_to_ast(b: &mut Builder, target: &str, event: &DeliveryEvent) -> StructId {
    let target_leaf = str_leaf(b, target);
    let (kind, inner) = match event {
        DeliveryEvent::Message {
            contract,
            payload,
            token,
            from,
        } => {
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let payload = bytes_leaf(b, payload);
            let mut fields = vec![("contract", contract), ("payload", payload)];
            if !token.is_empty() {
                let token = bytes_leaf(b, token);
                fields.push(("token", token));
            }
            // A default (absent) `from` is the synthetic external origin, so it is omitted on encode.
            if let Some(origin) = from {
                let from = origin_to_ast(b, origin);
                fields.push(("from", from));
            }
            ("message", record(b, fields))
        }
        DeliveryEvent::Notification { contract, payload } => {
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let payload = bytes_leaf(b, payload);
            (
                "notification",
                record(b, vec![("contract", contract), ("payload", payload)]),
            )
        }
        DeliveryEvent::Response {
            contract,
            token,
            answer,
        } => {
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let answer = answer_to_ast(b, answer);
            let mut fields = vec![("contract", contract), ("answer", answer)];
            if !token.is_empty() {
                let token = bytes_leaf(b, token);
                fields.push(("token", token));
            }
            ("response", record(b, fields))
        }
    };
    record(b, vec![("target", target_leaf), (kind, inner)])
}

/// The `(Ok <bytes>)` / `(Err <error-tag>)` node for a response's `answer` — the inverse of [`read_answer`].
fn answer_to_ast(b: &mut Builder, answer: &Result<Bytes, Error>) -> StructId {
    match answer {
        Ok(bytes) => {
            let v = bytes_leaf(b, bytes);
            bare_ctor(b, "Ok", vec![v])
        }
        Err(e) => {
            let tag = str_leaf(b, error_tag(*e));
            bare_ctor(b, "Err", vec![tag])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BlobSource, BlobSpec, DeliveryEvent, GraphEdge, HarnessSpec, PureRun, RegistrySpec,
        SpecError, read_bytes, record_field, resolve_references,
    };
    use crate::contract_value::bare_ctor;
    use crate::contract_value::{ascribe, bytes_leaf, record, uint_leaf};
    use crate::testing::SpawnSpec;
    use crate::{
        Bytes, ContractId, Error, HostId, Links, Origin, ProgramHash, ReducerId, ReducerKind,
    };
    use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
    use cadenza_ast::codec;
    use std::sync::Arc;
    use std::time::Duration;

    /// Build a value with `build`, finish the arena, and hand back the `Arenas` to read from.
    fn built(build: impl FnOnce(&mut Builder) -> StructId) -> Arenas {
        let mut b = Builder::new();
        let root = build(&mut b);
        b.finish(root)
    }

    #[test]
    fn reads_a_harness_description_whose_outer_record_is_the_native_ctor_leaf_form() {
        // §9 regression guard: rcdzc compiles a harness description to the M2 NATIVE compound form — the outer
        // record carries a `RecordCtor` ctor-LEAF head + native `FieldPair` entries, not the name-alias
        // `(record …)` the platform's own builder emits. Before the descent recognized the native leaves,
        // `is_record`/`record_field` returned "not a record" on the compiled harness (SpecError::NotARecord),
        // blocking cdz-platform-itest's harness-run (the §9 harness-reducer-dispatch tail). Build the outer
        // record NATIVELY (`b.compound(CompoundCtor::Record, [b.field_pair(name, val)…])`) and confirm it reads.
        let arenas = built(|b| {
            let default = b.atom_leaf(Leaf::Str(Arc::from("sys")));
            // A registry sub-record — also native, to exercise nested native records.
            let default_key = b.name("default");
            let reg_field = b.field_pair(default_key, default);
            let registry = b.compound(cadenza_ast::ast::CompoundCtor::Record, &[reg_field]);
            let reg_key = b.name("registry");
            let outer_field = b.field_pair(reg_key, registry);
            b.compound(cadenza_ast::ast::CompoundCtor::Record, &[outer_field])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root)
            .expect("a native-#record harness description reads (not SpecError::NotARecord)");
        assert_eq!(spec.registry.default, "sys");
    }

    /// A `#list(e…)` value — the M2 NATIVE list constructor (`Leaf::Ctor(CompoundCtor::List)` head); the
    /// M3 reader-flip (#6528) dropped the legacy STRING-head `("list" …)` form.
    fn list(b: &mut Builder, items: Vec<StructId>) -> StructId {
        b.compound(cadenza_ast::ast::CompoundCtor::List, &items)
    }

    /// A string leaf.
    fn s(b: &mut Builder, text: &str) -> StructId {
        b.atom_leaf(Leaf::Str(Arc::from(text)))
    }

    /// A minimal `("record" (= default <name>))` registry sub-record — just a default handler, no overrides.
    /// Every well-formed run carries a `registry`, so this is the smallest one a shape test needs.
    fn reg(b: &mut Builder, default: &str) -> StructId {
        let default = s(b, default);
        record(b, vec![("default", default)])
    }

    /// A full description: a registry (default handler `sys`), a run horizon, two blobs (one inline, one by
    /// path), and three spawns (a root, an event-kind reducer, and a child).
    fn full(b: &mut Builder) -> StructId {
        let registry = reg(b, "sys");
        let run_for = uint_leaf(b, 5_000_000_000);
        let inline = {
            let name = s(b, "greeter");
            let bytes = bytes_leaf(b, &[0x00, 0x61, 0x73, 0x6d]);
            record(b, vec![("name", name), ("bytes", bytes)])
        };
        let by_path = {
            let name = s(b, "worker");
            let path = s(b, "worker.wasm");
            record(b, vec![("name", name), ("path", path)])
        };
        let blobs = list(b, vec![inline, by_path]);
        let spawn_root = {
            let name = s(b, "root");
            let blob = s(b, "greeter");
            record(b, vec![("name", name), ("blob", blob)])
        };
        let spawn_event = {
            let name = s(b, "sysred");
            let blob = s(b, "greeter");
            let kind = s(b, "event");
            record(b, vec![("name", name), ("blob", blob), ("kind", kind)])
        };
        let spawn_child = {
            let name = s(b, "child");
            let blob = s(b, "worker");
            let parent = s(b, "root");
            record(b, vec![("name", name), ("blob", blob), ("parent", parent)])
        };
        let spawns = list(b, vec![spawn_root, spawn_event, spawn_child]);
        record(
            b,
            vec![
                ("registry", registry),
                ("run-for", run_for),
                ("blobs", blobs),
                ("spawns", spawns),
            ],
        )
    }

    #[test]
    fn reads_a_full_run_from_the_ast() {
        let arenas = built(full);
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("a well-formed description");
        assert_eq!(spec.registry.default, "sys");
        assert_eq!(spec.run_for, Some(Duration::from_nanos(5_000_000_000)));
        assert_eq!(
            spec.blobs,
            vec![
                BlobSpec {
                    name: "greeter".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(&[0x00, 0x61, 0x73, 0x6d])),
                },
                BlobSpec {
                    name: "worker".to_string(),
                    source: BlobSource::Path("worker.wasm".to_string()),
                },
            ]
        );
        assert_eq!(
            spec.spawns,
            vec![
                SpawnSpec::new("root", "greeter"),
                SpawnSpec::new("sysred", "greeter").kind(ReducerKind::Event),
                SpawnSpec::new("child", "worker").child_of("root"),
            ]
        );
    }

    #[test]
    fn survives_a_binary_ast_round_trip() {
        // The real path: encode to canonical bytes, then decode from bytes — the executable's input.
        let arenas = built(full);
        let bytes = codec::encode(&arenas);
        let spec = HarnessSpec::decode(&bytes).expect("decode the encoded description");
        assert_eq!(spec.registry.default, "sys");
        assert_eq!(spec.run_for, Some(Duration::from_nanos(5_000_000_000)));
        assert_eq!(spec.blobs.len(), 2);
        assert_eq!(spec.spawns.len(), 3);
    }

    #[test]
    fn optional_fields_default_and_a_bare_registry_is_enough() {
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            record(b, vec![("registry", registry)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("just a registry is valid");
        assert_eq!(spec.registry.default, "sys");
        assert_eq!(spec.run_for, None);
        assert!(spec.blobs.is_empty());
        assert!(spec.spawns.is_empty());
        assert_eq!(spec.checker, None, "no checker field ⇒ no end-of-run check");
    }

    #[test]
    fn reads_the_checker_blob_name() {
        // A run may name a checker program — a reducer the harness runs over the completed log (§9).
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let checker = s(b, "assert-echo");
            record(b, vec![("registry", registry), ("checker", checker)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("a registry + checker is valid");
        assert_eq!(spec.checker.as_deref(), Some("assert-echo"));
    }

    #[test]
    fn build_resolves_inline_and_path_blobs() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![
                BlobSpec {
                    name: "inline".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"here")),
                },
                BlobSpec {
                    name: "external".to_string(),
                    source: BlobSource::Path("p.wasm".to_string()),
                },
            ],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        // The loader is invoked for the path blob only, with the exact path string.
        let mut loaded = Vec::new();
        let harness = spec
            .build::<std::convert::Infallible>(|path| {
                loaded.push(path.to_string());
                Ok(Bytes::from(format!("bytes-of-{path}")))
            })
            .expect("the loader always succeeds here");
        drop(harness); // the Harness's fields are private; the loader call log is the observable effect.
        assert_eq!(
            loaded,
            vec!["p.wasm".to_string()],
            "only the path blob is loaded"
        );
    }

    #[test]
    fn build_propagates_a_loader_error() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![BlobSpec {
                name: "external".to_string(),
                source: BlobSource::Path("missing.wasm".to_string()),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        let result = spec.build(|_| Err("no such file"));
        assert_eq!(result.err(), Some("no such file"));
    }

    #[test]
    fn rejects_undecodable_bytes() {
        assert_eq!(
            HarnessSpec::decode(b"not an ast"),
            Err(SpecError::Undecodable)
        );
    }

    #[test]
    fn rejects_a_non_record_root() {
        let arenas = built(|b| bytes_leaf(b, b"scalar"));
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::NotARecord)
        );
    }

    #[test]
    fn rejects_a_missing_registry() {
        let arenas = built(|b| record(b, vec![]));
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::MissingField("registry"))
        );
    }

    #[test]
    fn rejects_a_run_for_that_is_not_an_integer() {
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let bad = s(b, "soon");
            record(b, vec![("registry", registry), ("run-for", bad)])
        });
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::WrongType {
                field: "run-for",
                want: "an unsigned integer of nanoseconds",
            })
        );
    }

    #[test]
    fn rejects_a_blob_with_both_or_neither_source() {
        // Both sources at once.
        let both = built(|b| {
            let registry = reg(b, "sys");
            let name = s(b, "b");
            let bytes = bytes_leaf(b, b"x");
            let path = s(b, "p");
            let blob = record(b, vec![("name", name), ("bytes", bytes), ("path", path)]);
            let blobs = list(b, vec![blob]);
            record(b, vec![("registry", registry), ("blobs", blobs)])
        });
        assert_eq!(
            HarnessSpec::read(&both, both.root),
            Err(SpecError::BlobSource {
                blob: "b".to_string()
            })
        );
        // Neither source.
        let neither = built(|b| {
            let registry = reg(b, "sys");
            let name = s(b, "b");
            let blob = record(b, vec![("name", name)]);
            let blobs = list(b, vec![blob]);
            record(b, vec![("registry", registry), ("blobs", blobs)])
        });
        assert_eq!(
            HarnessSpec::read(&neither, neither.root),
            Err(SpecError::BlobSource {
                blob: "b".to_string()
            })
        );
    }

    #[test]
    fn rejects_an_unknown_reducer_kind() {
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let name = s(b, "t");
            let blob = s(b, "greeter");
            let kind = s(b, "supervisor");
            let spawn = record(b, vec![("name", name), ("blob", blob), ("kind", kind)]);
            let spawns = list(b, vec![spawn]);
            record(b, vec![("registry", registry), ("spawns", spawns)])
        });
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::UnknownKind {
                kind: "supervisor".to_string()
            })
        );
    }

    #[test]
    fn encode_is_the_inverse_of_decode() {
        // A spec built in Rust encodes to a binary AST that decodes back to an equal spec — pinning the
        // round-trip so a future schema change cannot silently break the executable's input. The spec covers
        // every optional: an inline and a path blob, a root/child parent, an event-kind spawn, and a message
        // (with a token) and a notification delivery.
        let spec = HarnessSpec {
            run_for: Some(Duration::from_nanos(7_500_000_000)),
            blobs: vec![
                BlobSpec {
                    name: "greeter".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(&[0x00, 0x61, 0x73, 0x6d])),
                },
                BlobSpec {
                    name: "worker".to_string(),
                    source: BlobSource::Path("worker.wasm".to_string()),
                },
            ],
            spawns: vec![
                SpawnSpec::new("root", "greeter"),
                SpawnSpec::new("sysred", "greeter").kind(ReducerKind::Event),
                SpawnSpec::new("child", "worker").child_of("root"),
            ],
            deliveries: vec![
                (
                    "root".to_string(),
                    DeliveryEvent::Message {
                        contract: ContractId::of(b"temp.celsius"),
                        payload: Bytes::from_static(b"21"),
                        token: Bytes::from_static(b"tok-1"),
                        from: None,
                    },
                ),
                (
                    "child".to_string(),
                    DeliveryEvent::Notification {
                        contract: ContractId::of(b"lifecycle.spawned"),
                        payload: Bytes::from_static(b"hello"),
                    },
                ),
            ],
            checker: Some("check".to_string()),
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        let bytes = spec.encode();
        assert_eq!(HarnessSpec::decode(&bytes), Ok(spec));
    }

    #[test]
    fn encode_omits_defaults_so_a_minimal_spec_round_trips() {
        // The defaults (no run-for, no blobs, no spawns) are omitted on encode and restored on decode.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    #[test]
    fn reads_message_and_notification_deliveries() {
        let cid = ContractId::of(b"temp.celsius");
        let nid = ContractId::of(b"lifecycle.spawned");
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let msg = {
                let contract = bytes_leaf(b, cid.hash().as_bytes());
                let payload = bytes_leaf(b, b"21");
                let token = bytes_leaf(b, b"tok");
                record(
                    b,
                    vec![
                        ("contract", contract),
                        ("payload", payload),
                        ("token", token),
                    ],
                )
            };
            let d1 = {
                let target = s(b, "root");
                record(b, vec![("target", target), ("message", msg)])
            };
            let note = {
                let contract = bytes_leaf(b, nid.hash().as_bytes());
                let payload = bytes_leaf(b, b"hi");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let d2 = {
                let target = s(b, "root");
                record(b, vec![("target", target), ("notification", note)])
            };
            let deliver = list(b, vec![d1, d2]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("valid deliveries");
        assert_eq!(
            spec.deliveries,
            vec![
                (
                    "root".to_string(),
                    DeliveryEvent::Message {
                        contract: cid,
                        payload: Bytes::from_static(b"21"),
                        token: Bytes::from_static(b"tok"),
                        from: None,
                    },
                ),
                (
                    "root".to_string(),
                    DeliveryEvent::Notification {
                        contract: nid,
                        payload: Bytes::from_static(b"hi"),
                    },
                ),
            ]
        );
    }

    #[test]
    fn a_message_token_defaults_to_empty_when_omitted() {
        let cid = ContractId::of(b"c");
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let msg = {
                let contract = bytes_leaf(b, cid.hash().as_bytes());
                let payload = bytes_leaf(b, b"p");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("valid delivery");
        assert_eq!(
            spec.deliveries,
            vec![(
                "root".to_string(),
                DeliveryEvent::Message {
                    contract: cid,
                    payload: Bytes::from_static(b"p"),
                    token: Bytes::new(),
                    from: None,
                },
            )]
        );
    }

    #[test]
    fn rejects_a_delivery_with_both_or_neither_event() {
        // Neither message nor notification.
        let neither = built(|b| {
            let registry = reg(b, "sys");
            let target = s(b, "root");
            let d = record(b, vec![("target", target)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        assert_eq!(
            HarnessSpec::read(&neither, neither.root),
            Err(SpecError::DeliveryKind {
                target: "root".to_string()
            })
        );
    }

    #[test]
    fn rejects_a_contract_id_of_the_wrong_length() {
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let msg = {
                let contract = bytes_leaf(b, b"too short for a hash");
                let payload = bytes_leaf(b, b"p");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::WrongType {
                field: "contract",
                want: "a 33-byte contract-id",
            })
        );
    }

    #[test]
    fn a_delivery_contract_may_be_a_base62_string() {
        // A name→id rewrite (done outside the platform) substitutes a base62 contract-id string for a
        // contract name; decode parses it to the same id the raw bytes would give — so nix can rewrite
        // `contract = <name>` to `contract = <base62>` with a pure string substitution.
        let cid = ContractId::of(b"temp.celsius");
        let text = cid.hash().to_string(); // the base62 §8 text form
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let msg = {
                let contract = s(b, &text);
                let payload = bytes_leaf(b, b"21");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("a base62 contract is valid");
        match &spec.deliveries[0].1 {
            DeliveryEvent::Message { contract, .. } => assert_eq!(*contract, cid),
            other => panic!("expected a message delivery, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_contract_string_that_is_not_base62() {
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let msg = {
                let contract = s(b, "not base62!!");
                let payload = bytes_leaf(b, b"p");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::WrongType {
                field: "contract",
                want: "a base62 contract-id",
            })
        );
    }

    /// An inline blob named `name` — the smallest well-formed blob for cross-reference tests.
    fn blob(name: &str) -> BlobSpec {
        BlobSpec {
            name: name.to_string(),
            source: BlobSource::Inline(Bytes::from_static(b"x")),
        }
    }

    /// A delivery of an empty message to `target` — the event kind is immaterial to cross-reference checks.
    fn deliver_to(target: &str) -> (String, DeliveryEvent) {
        (
            target.to_string(),
            DeliveryEvent::Message {
                contract: ContractId::of(b"c"),
                payload: Bytes::new(),
                token: Bytes::new(),
                from: None,
            },
        )
    }

    #[test]
    fn validate_accepts_a_well_formed_run() {
        // Registry default declared, every spawn's blob declared, parent spawned earlier, delivery hits a task.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("sys"), blob("worker")],
            spawns: vec![
                SpawnSpec::new("root", "worker"),
                SpawnSpec::new("child", "worker").child_of("root"),
            ],
            deliveries: vec![deliver_to("root")],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(spec.validate(), Ok(()));
    }

    #[test]
    fn validate_rejects_an_unregistered_registry_default() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("other")],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::UnknownBlob {
                referrer: "registry default".to_string(),
                blob: "sys".to_string(),
            })
        );
    }

    #[test]
    fn validate_rejects_a_spawn_of_an_undeclared_blob() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("sys")],
            spawns: vec![SpawnSpec::new("t", "missing")],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::UnknownBlob {
                referrer: "t".to_string(),
                blob: "missing".to_string(),
            })
        );
    }

    #[test]
    fn validate_rejects_a_parent_not_spawned_earlier() {
        // The child names a parent that is spawned AFTER it (order matters), so it is unresolved.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("sys"), blob("w")],
            spawns: vec![
                SpawnSpec::new("child", "w").child_of("root"),
                SpawnSpec::new("root", "w"),
            ],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::UnknownParent {
                task: "child".to_string(),
                parent: "root".to_string(),
            })
        );
    }

    #[test]
    fn validate_rejects_a_duplicate_task_name() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("sys"), blob("w")],
            spawns: vec![SpawnSpec::new("dup", "w"), SpawnSpec::new("dup", "w")],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::DuplicateTask {
                task: "dup".to_string(),
            })
        );
    }

    #[test]
    fn validate_rejects_a_delivery_to_an_unspawned_target() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("sys"), blob("w")],
            spawns: vec![SpawnSpec::new("root", "w")],
            deliveries: vec![deliver_to("ghost")],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::UnknownTarget {
                target: "ghost".to_string(),
            })
        );
    }

    #[test]
    fn validate_rejects_an_edge_naming_an_unspawned_task() {
        // An edge seeds a transform chain between spawned tasks; a `to` transform that is not spawned is
        // unresolved, so validate rejects it before the run (rather than panicking in name resolution).
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![blob("sys"), blob("w")],
            spawns: vec![SpawnSpec::new("emitter", "w")],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![GraphEdge {
                from: "emitter".to_string(),
                contract: ContractId::of(b"cdz-platform.effect"),
                to: vec!["ghost".to_string()],
            }],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::UnknownEdgeTask {
                task: "ghost".to_string(),
            })
        );
    }

    #[test]
    fn an_edge_round_trips() {
        // A seeded transform chain survives encode→decode, so a run can express the graph forward arm.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![GraphEdge {
                from: "emitter".to_string(),
                contract: ContractId::of(b"cdz-platform.effect"),
                to: vec!["transform-a".to_string(), "transform-b".to_string()],
            }],
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    #[test]
    fn a_spawn_with_supervision_links_round_trips() {
        // A spawn's supervision links (§7) survive encode→decode. Only the true flag is written, so a
        // one-directional link recovers exactly (parent watches child, child does not watch parent), while a
        // spawn with no links keeps the SpawnSpec::new default of none.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![],
            spawns: vec![
                SpawnSpec::new("watcher", "watcher"),
                SpawnSpec::new("child", "child")
                    .child_of("watcher")
                    .links(Links {
                        parent_watches_child: true,
                        child_watches_parent: false,
                    }),
            ],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        let decoded = HarnessSpec::decode(&spec.encode()).expect("decode");
        assert_eq!(decoded, spec);
        assert_eq!(
            decoded.spawns[1].supervision(),
            Links {
                parent_watches_child: true,
                child_watches_parent: false,
            }
        );
        assert_eq!(decoded.spawns[0].supervision(), Links::NONE);
    }

    #[test]
    fn response_deliveries_round_trip_ok_and_err() {
        // A run can inject a Response to exercise on_response — with an Ok output value or an Err runtime
        // failure. Both survive encode→decode, so the framework can express either reply path.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![],
            spawns: vec![],
            deliveries: vec![
                (
                    "root".to_string(),
                    DeliveryEvent::Response {
                        contract: ContractId::of(b"http.get"),
                        token: Bytes::from_static(b"tok-1"),
                        answer: Ok(Bytes::from_static(b"200")),
                    },
                ),
                (
                    "root".to_string(),
                    DeliveryEvent::Response {
                        contract: ContractId::of(b"http.get"),
                        token: Bytes::new(),
                        answer: Err(Error::Timeout),
                    },
                ),
            ],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    #[test]
    fn reads_a_response_delivery_from_the_authoring_shape() {
        // Pin the wire shape a run author writes: (= response (record (= contract …) (= answer (Ok b"…"))
        // (= token b"…"))). Decodes to a Response with the Ok output.
        let cid = ContractId::of(b"temp.celsius");
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let resp = {
                let contract = bytes_leaf(b, cid.hash().as_bytes());
                let ok_bytes = bytes_leaf(b, b"21");
                let answer = bare_ctor(b, "Ok", vec![ok_bytes]);
                let token = bytes_leaf(b, b"t");
                record(
                    b,
                    vec![("contract", contract), ("answer", answer), ("token", token)],
                )
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("response", resp)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("a valid response delivery");
        assert_eq!(
            spec.deliveries,
            vec![(
                "root".to_string(),
                DeliveryEvent::Response {
                    contract: cid,
                    token: Bytes::from_static(b"t"),
                    answer: Ok(Bytes::from_static(b"21")),
                },
            )]
        );
    }

    #[test]
    fn rejects_a_response_with_an_unknown_error_tag() {
        let cid = ContractId::of(b"c");
        let arenas = built(|b| {
            let registry = reg(b, "sys");
            let resp = {
                let contract = bytes_leaf(b, cid.hash().as_bytes());
                let bad = s(b, "kaboom");
                let answer = bare_ctor(b, "Err", vec![bad]);
                record(b, vec![("contract", contract), ("answer", answer)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("response", resp)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("registry", registry), ("deliver", deliver)])
        });
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::WrongType {
                field: "answer",
                want: "a known error tag: timeout | missing-handler | schema-violation | faulted",
            })
        );
    }

    #[test]
    fn a_message_delivery_round_trips_an_explicit_from_sender() {
        // A run can stamp the delivered message's `from` (its sender Origin), so a case can exercise a
        // reducer that routes or validates on who sent the effect. Absent `from` defaults to the synthetic
        // external origin (covered elsewhere); here the explicit sender survives encode→decode.
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![],
            spawns: vec![],
            deliveries: vec![(
                "root".to_string(),
                DeliveryEvent::Message {
                    contract: ContractId::of(b"c"),
                    payload: Bytes::from_static(b"p"),
                    token: Bytes::new(),
                    from: Some(Origin {
                        reducer: ReducerId::of(b"peer"),
                        host: HostId::of(b"node"),
                    }),
                },
            )],
            checker: None,
            pure_run: None,
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    /// A spec carrying a `pure-run` directive round-trips: program name, contract-id, input, and expected
    /// output all read back exactly.
    #[test]
    fn a_pure_run_round_trips() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![BlobSpec {
                name: "prog".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"the-program")),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: Some(PureRun {
                program: "prog".to_string(),
                contract: ContractId::of(b"cdz-platform.deliver"),
                input: Bytes::from_static(b"X"),
                expect_output: Bytes::from_static(b"X"),
            }),
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    /// A run's unnamed dependency components (`deps`) round-trip: both an inline and a path source read back
    /// exactly, with no name (the CAS keys them by content hash).
    #[test]
    fn a_run_with_deps_round_trips() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![BlobSpec {
                name: "$system".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"placeholder")),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: vec![
                BlobSource::Inline(Bytes::from_static(b"\x00\x61\x73\x6d")),
                BlobSource::Path("cdz-store/runtime.wasm".to_string()),
            ],
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    /// A run's `registry` (default handler + per-contract overrides) round-trips, and `validate` rejects a
    /// registry naming an undeclared handler blob.
    #[test]
    fn a_registry_round_trips_and_validates_handler_blobs() {
        let base = |registry| HarnessSpec {
            run_for: None,
            blobs: vec![
                BlobSpec {
                    name: "default-handler".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"dh")),
                },
                BlobSpec {
                    name: "special-handler".to_string(),
                    source: BlobSource::Inline(Bytes::from_static(b"sh")),
                },
            ],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: None,
            deps: vec![],
            registry,
            edges: vec![],
        };
        let spec = base(RegistrySpec {
            default: "default-handler".to_string(),
            handlers: vec![(
                ContractId::of(b"special.contract"),
                "special-handler".to_string(),
            )],
        });
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec.clone()));
        assert_eq!(spec.validate(), Ok(()));
        // An override naming an undeclared handler is a clean UnknownBlob.
        let bad = base(RegistrySpec {
            default: "default-handler".to_string(),
            handlers: vec![(ContractId::of(b"c"), "missing-handler".to_string())],
        });
        assert_eq!(
            bad.validate(),
            Err(SpecError::UnknownBlob {
                referrer: "registry handler".to_string(),
                blob: "missing-handler".to_string(),
            })
        );
    }

    /// `validate` rejects a pure run whose program is not a declared blob — the same cross-reference check
    /// as the registry's default handler, so a typo'd program name is a clean [`SpecError`], not a run-time
    /// failure.
    #[test]
    fn validate_rejects_a_pure_run_naming_an_unknown_program() {
        let spec = HarnessSpec {
            run_for: None,
            blobs: vec![BlobSpec {
                name: "$system".to_string(),
                source: BlobSource::Inline(Bytes::from_static(b"placeholder")),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
            pure_run: Some(PureRun {
                program: "missing".to_string(),
                contract: ContractId::of(b"c"),
                input: Bytes::new(),
                expect_output: Bytes::new(),
            }),
            deps: Vec::new(),
            registry: RegistrySpec {
                default: "sys".to_string(),
                handlers: vec![],
            },
            edges: vec![],
        };
        assert_eq!(
            spec.validate(),
            Err(SpecError::UnknownBlob {
                referrer: "pure-run".to_string(),
                blob: "missing".to_string(),
            })
        );
    }

    /// The resolve pass rewrites a `BlobHash(<name>)` / `BlobBytes(<name>)` reference — even NESTED inside a
    /// list/record — to the named blob's content hash / bytes, and leaves a plain payload untouched.
    #[test]
    fn resolve_references_rewrites_blob_calls_anywhere() {
        let sub = b"sub-program-bytes";
        // A run with one inline blob "sub" and a delivery whose payload is BlobHash("sub"), nested in the
        // deliver list — the reference sits deep in the value, not at a top-level field.
        let arenas = built(|b| {
            let sub_name = s(b, "sub");
            let sub_bytes = bytes_leaf(b, sub);
            let blob = record(b, vec![("name", sub_name), ("bytes", sub_bytes)]);
            let blobs = list(b, vec![blob]);
            let ref_name = s(b, "sub");
            let bhash = bare_ctor(b, "BlobHash", vec![ref_name]);
            let contract = bytes_leaf(b, ContractId::of(b"c").hash().as_bytes());
            let msg = record(b, vec![("contract", contract), ("payload", bhash)]);
            let target = s(b, "root");
            let delivery = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![delivery]);
            let registry = reg(b, "sub");
            record(
                b,
                vec![
                    ("registry", registry),
                    ("blobs", blobs),
                    ("deliver", deliver),
                ],
            )
        });
        let resolved =
            resolve_references(&arenas, |_| None).expect("resolve the BlobHash reference");
        let spec = HarnessSpec::read(&resolved, resolved.root).expect("read the resolved spec");
        let want = Bytes::copy_from_slice(ProgramHash::of(sub).hash().as_bytes());
        match &spec.deliveries[0].1 {
            DeliveryEvent::Message { payload, .. } => assert_eq!(*payload, want),
            other => panic!("expected a message, got {other:?}"),
        }
    }

    /// A `Value("<Type>", <value>)` payload resolves to the canonical binary encoding a guest `Value.decode`s:
    /// the value ascribed with the type, `(: <value> <Type>)`, run through the codec — so a run can deliver a
    /// STRUCTURED payload a reducer decodes by schema, not opaque bytes.
    #[test]
    fn resolve_references_encodes_a_value_payload() {
        let arenas = built(|b| {
            let ty = s(b, "Effect");
            let deep = bytes_leaf(b, b"DEEP");
            let inner = bare_ctor(b, "Perform", vec![deep]);
            let vref = bare_ctor(b, "Value", vec![ty, inner]);
            let contract = bytes_leaf(b, ContractId::of(b"c").hash().as_bytes());
            let msg = record(b, vec![("contract", contract), ("payload", vref)]);
            let target = s(b, "root");
            let delivery = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![delivery]);
            let blobs = list(b, vec![]);
            let registry = reg(b, "sys");
            record(
                b,
                vec![
                    ("registry", registry),
                    ("blobs", blobs),
                    ("deliver", deliver),
                ],
            )
        });
        let resolved = resolve_references(&arenas, |_| None).expect("resolve the Value reference");
        let spec = HarnessSpec::read(&resolved, resolved.root).expect("read the resolved spec");
        // The bytes a guest gets: codec-encoding of the value ascribed with its type — `(: (Perform b"DEEP") Effect)`.
        let want = {
            let mut vb = Builder::new();
            let deep = bytes_leaf(&mut vb, b"DEEP");
            let inner = bare_ctor(&mut vb, "Perform", vec![deep]);
            let root = ascribe(&mut vb, inner, "Effect");
            Bytes::from(codec::encode(&vb.finish(root)))
        };
        match &spec.deliveries[0].1 {
            DeliveryEvent::Message { payload, .. } => assert_eq!(*payload, want),
            other => panic!("expected a message, got {other:?}"),
        }
    }

    /// A `Value("<Type>", { … })` payload written in ML RECORD syntax is CANONICALIZED on encode: the ML surface
    /// heads a record with the STRING constructor `("record" …)` and keeps fields in declaration order, but the
    /// compiler's `Value.encode` (what a guest `Value.decode`s) emits the NAME-headed `(record …)` with fields
    /// ascending by name. So the resolved payload bytes must match the canonical form even when the author wrote
    /// the fields OUT of order — the string head is rewritten and the fields are sorted. Without
    /// [`rewrite_value_canonical`] the string-headed / declaration-order bytes reach the guest and its
    /// `Value.decode` returns `None` (the multi-contract state dispatcher's set arm decode-failed exactly so).
    #[test]
    fn resolve_references_canonicalizes_a_record_value_payload() {
        // A NATIVE ctor-leaf record `#record((= value b"V") (= key b"K"))` — fields DELIBERATELY in
        // non-canonical (value-before-key) order to prove `resolve_references` canonicalizes (sorts) them.
        // (Was a legacy STRING-headed `("record" …)` proving the string→native rewrite; the M3 reader-flip
        // #6528 dropped string-head recognition, so the input is built native and the test now pins the
        // field-sort/canonicalization of a native record.)
        let arenas = built(|b| {
            let ty = s(b, "SetRequest");
            let f_value = {
                let eq = b.name("=");
                let name = b.name("value");
                let v = bytes_leaf(b, b"V");
                b.list(vec![eq, name, v])
            };
            let f_key = {
                let eq = b.name("=");
                let name = b.name("key");
                let v = bytes_leaf(b, b"K");
                b.list(vec![eq, name, v])
            };
            let ml_record = b.compound(cadenza_ast::ast::CompoundCtor::Record, &[f_value, f_key]);
            let vref = bare_ctor(b, "Value", vec![ty, ml_record]);
            let contract = bytes_leaf(b, ContractId::of(b"c").hash().as_bytes());
            let msg = record(b, vec![("contract", contract), ("payload", vref)]);
            let target = s(b, "root");
            let delivery = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![delivery]);
            let blobs = list(b, vec![]);
            let registry = reg(b, "sys");
            record(
                b,
                vec![
                    ("registry", registry),
                    ("blobs", blobs),
                    ("deliver", deliver),
                ],
            )
        });
        let resolved = resolve_references(&arenas, |_| None).expect("resolve the Value reference");
        let spec = HarnessSpec::read(&resolved, resolved.root).expect("read the resolved spec");
        // The bytes a guest gets: codec-encoding of the NAME-headed, name-SORTED record `(record (= key b"K")
        // (= value b"V"))` ascribed with its type — built via the canonical `record()` (sorts) + `ascribe`.
        let want = {
            let mut vb = Builder::new();
            let k = bytes_leaf(&mut vb, b"K");
            let v = bytes_leaf(&mut vb, b"V");
            let rec = record(&mut vb, vec![("key", k), ("value", v)]);
            let root = ascribe(&mut vb, rec, "SetRequest");
            Bytes::from(codec::encode(&vb.finish(root)))
        };
        match &spec.deliveries[0].1 {
            DeliveryEvent::Message { payload, .. } => assert_eq!(*payload, want),
            other => panic!("expected a message, got {other:?}"),
        }
    }

    /// The resolve pass rejects a reference to a blob the run does not declare — a clean [`SpecError`], not a
    /// silent resolution to empty bytes.
    #[test]
    fn resolve_references_rejects_an_unknown_blob() {
        let arenas = built(|b| {
            let ref_name = s(b, "missing");
            let bhash = bare_ctor(b, "BlobHash", vec![ref_name]);
            let msg = record(b, vec![("payload", bhash)]);
            record(b, vec![("x", msg)])
        });
        assert_eq!(
            resolve_references(&arenas, |_| None),
            Err(SpecError::UnknownBlob {
                referrer: "BlobHash reference".to_string(),
                blob: "missing".to_string(),
            })
        );
    }

    /// A `path`-form blob's bytes are materialized through the loader, so a `BlobHash` reference to a
    /// path-supplied program (how the harness supplies guests) resolves at build — the case #3341's
    /// decode-time inline-only pass got wrong.
    #[test]
    fn resolve_references_materializes_a_path_blob_via_the_loader() {
        let sub = b"path-supplied-program-bytes";
        let arenas = built(|b| {
            let name = s(b, "sub");
            let path = s(b, "/nix/store/sub.wasm");
            let blob = record(b, vec![("name", name), ("path", path)]);
            let blobs = list(b, vec![blob]);
            let ref_name = s(b, "sub");
            let bhash = bare_ctor(b, "BlobHash", vec![ref_name]);
            let msg = record(b, vec![("payload", bhash)]);
            let registry = reg(b, "sub");
            record(
                b,
                vec![("registry", registry), ("blobs", blobs), ("x", msg)],
            )
        });
        // A loader that materializes the path (as the itest binary does via fs::read).
        let resolved = resolve_references(&arenas, |path| {
            assert_eq!(path, "/nix/store/sub.wasm");
            Some(Bytes::copy_from_slice(sub))
        })
        .expect("resolve the path-blob BlobHash reference");
        // The `x.payload` node resolved to the path blob's content hash.
        let x = record_field(&resolved, resolved.root, "x").expect("x field");
        let payload = record_field(&resolved, x, "payload").expect("payload field");
        assert_eq!(
            read_bytes(&resolved, payload),
            Some(Bytes::copy_from_slice(
                ProgramHash::of(sub).hash().as_bytes()
            ))
        );
    }
}
