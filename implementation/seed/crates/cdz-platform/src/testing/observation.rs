//! The observation log — what happened during a platform run, recorded for a checker to assert over
//! (`design/cadenza-platform.md` §9).
//!
//! The design calls this an *observation log*: a deployment may record every event a reducer folds,
//! for inspection or analysis, and it does not back a reducer's state, so recording it changes only
//! what can be audited, never how a reducer behaves. This crate makes that log concrete for tests.
//!
//! The log is one ordered sequence of [`Record`]s. Each record answers three questions and nothing
//! else: **when** it happened ([`Record::time_ns`], the runtime clock — deterministic under the bach
//! simulator), **who** caused it ([`Record::source`], a reducer on a host — an [`Origin`]), and
//! **what** happened ([`Record::entry`]). The `what` is a platform-level fact observable of *any*
//! reducer — a key-value or blob store call today, a delivered or emitted event in a later slice —
//! never a program's internal shape. A checker reads only these facts, so it never assumes a program
//! is Cadenza, Rust, or anything: the log is language-neutral, which is the whole point of the harness.
//!
//! The log is a cheaply-clonable handle over shared, append-only state. Many recording stores (one per
//! reducer) share the one handle, so the log is a single global order across every reducer — the
//! [`Record::seq`] is that order, assigned as each record is appended. A checker takes a
//! [`snapshot`](ObservationLog::snapshot) of the records once the run is quiescent.

use crate::{
    Bytes, ContractId, Dir, EdgeKind, Error, Hash, Origin, ProgramHash, ReducerId, ReducerKind, Str,
};
use std::fmt::{self, Write as _};
use std::ops::Bound;
use std::sync::{Arc, Mutex};

/// One entry in the observation log: who did what, and when. Cheap to clone — every buffer it carries
/// is [`Bytes`] (an O(1) refcount bump), so a checker snapshots the whole log without deep-copying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// This record's position in the one global order of observations, assigned on append (the first
    /// record is `0`). It is the total order across every reducer sharing the log, so a checker can
    /// reason about "before" and "after" without comparing timestamps (which can tie).
    pub seq: u64,
    /// The runtime clock, in nanoseconds, at the moment the operation was recorded. Wall-clock under
    /// the production runtime; deterministic simulated time under the bach simulator, so a test folds
    /// the same times every run. Timestamps may tie (two ops at the same simulated instant); use
    /// [`seq`](Record::seq) for a strict order.
    pub time_ns: u64,
    /// Who caused this: the acting reducer and the host it ran on. For a store call it is the reducer
    /// whose store was touched; for an event (a later slice) it is the event's emitter.
    pub source: Origin,
    /// What happened.
    pub entry: Entry,
}

/// What a [`Record`] observed. A closed set of platform-level facts — the operations the platform
/// mediates for a reducer and the setup a run performs, observable without decoding a payload or knowing
/// the program's language: a key-value or blob store call, an event a reducer folded/emitted/closed with,
/// or the harness assigning a spawn its name and id at the start of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// A call to the reducer's key-value store (§7).
    Kv(KvOp),
    /// A call to the content-addressed blob store (§8).
    Blob(BlobOp),
    /// An event the reducer folded, emitted, or closed with (§3/§4/§10).
    Event(EventOp),
    /// A call to the reducer's node-side **provenance** capability — the privileged `program-of` read (§4).
    Provenance(ProvOp),
    /// A call to the reducer's node-side **graph** capability — a read or write of the reducer graph (§7).
    Graph(GraphOp),
    /// The harness spawned a reducer and assigned it a name — recorded at the start of a run so the log
    /// is self-describing: a reader derefs a name to the reducer id ([`Record::source`]) it was assigned,
    /// with no out-of-band metadata (§3).
    Spawn(SpawnInfo),
    /// A privileged host call whose ARGUMENTS did not parse to their typed form, so the host early-returned
    /// a default (`[]`/`false`) without reaching the recordable capability. Recorded regardless — at the host
    /// boundary, above the parse — so no host call a reducer performs is silently unobservable
    /// (`design/cadenza-platform.md` §9, log-every-host-call). Coarse and interface-general: it names the
    /// interface + operation and carries the raw argument bytes as received, since a malformed value has no
    /// typed shape to record. A well-formed call still records through its typed op (`Kv`/`Graph`/…).
    HostCallRejected(RejectedCall),
    /// A synchronous pure-`run` host call (§3): the sub-program and contract ids it ran, the input, and its
    /// outcome. A `run` is hosted (it instantiates + folds a pure sub-program) but leaves no `step.requests`
    /// entry, so without this a checker cannot observe that a reducer invoked `run` — an echo of the payload
    /// is satisfiable without ever calling it (`design/cadenza-platform.md` §9, log-every-host-call).
    Run(RunCall),
}

/// A host call rejected at the argument-parse boundary (`design/cadenza-platform.md` §9): its interface and
/// operation names, and the raw argument byte slices as received (before the parse that failed). Coarse and
/// interface-general because a malformed argument has no typed form — the point is only that the call was
/// *performed* and *observed*, so a checker can assert it happened rather than it vanishing silently.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCall {
    /// The host interface the operation belongs to (e.g. `graph`).
    pub iface: Str,
    /// The operation name within the interface (e.g. `neighbors`).
    pub op: Str,
    /// The raw argument byte slices as received, before the (failed) parse.
    pub raw_args: Vec<Bytes>,
}

/// A pure-`run` host call (`design/cadenza-platform.md` §3): the `program` and `contract` id hash bytes, the
/// `input` bytes, and its outcome. On success `output` is the returned bytes and `error` is `None`; on failure
/// `output` is `None` and `error` names the category (`missing-handler`/`faulted`), mirroring the WIT
/// `result<payload, error>` a guest sees. A `run` is hosted but leaves no request in the step, so recording it
/// here is what makes the run act observable to a checker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunCall {
    /// The sub-program's id hash bytes.
    pub program: Bytes,
    /// The contract id hash bytes the run was over.
    pub contract: Bytes,
    /// The input bytes handed to the run.
    pub input: Bytes,
    /// The returned output bytes on a successful run, else `None`.
    pub output: Option<Bytes>,
    /// The failure category (`missing-handler`/`faulted`) on a failed run, else `None`.
    pub error: Option<Str>,
}

/// A reducer the harness spawned at the start of a run, and the name the run gave it — the log's record
/// of the name→id assignment (`design/cadenza-platform.md` §3). The assigned reducer id is the enclosing
/// [`Record::source`]'s reducer (the id derived from this genesis); this carries the human name and the
/// rest of the genesis, so anything reading the log can map a name to its id and see what was spawned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnInfo {
    /// The name the run gave this spawn — the handle a delivery, another spawn's parent, or a checker
    /// refers to it by.
    pub name: Str,
    /// The program it runs, by content hash.
    pub program: ProgramHash,
    /// Its parent reducer (its own id for a root).
    pub parent: ReducerId,
    /// Its privilege — ordinary, or a privileged event reducer.
    pub kind: ReducerKind,
}

/// A key-value store operation, with enough of its outcome to assert over — whether a `get` hit, what
/// a `put` wrote, whether a `delete` found anything (`design/cadenza-platform.md` §7). Keys and values
/// are the raw [`Bytes`] the reducer used; the checker interprets them, the log does not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KvOp {
    /// `get(key)` — `hit` is whether the store held an entry for `key`.
    Get { key: Bytes, hit: bool },
    /// `put(key, value)` — the value written, last-write-wins.
    Put { key: Bytes, value: Bytes },
    /// `delete(key)` — `existed` is whether an entry was present and is now gone.
    Delete { key: Bytes, existed: bool },
    /// A range or prefix scan — its inclusive/exclusive bounds, and whether it was the keys-only
    /// variant. The scan *invocation* is recorded (who scanned what range, when), not each streamed
    /// pair: a scan is a lazy stream a reducer may drain partially, so the range is the observable
    /// fact; a checker that needs the contents reads the store's own state.
    Scan {
        lower: Bound<Bytes>,
        upper: Bound<Bytes>,
        keys_only: bool,
    },
}

/// A content-addressed blob store operation, with its outcome (`design/cadenza-platform.md` §8). The
/// hash is the content address; `put` records the hash it minted and the byte length stored (not the
/// bytes again — they are addressed by the hash), `get`/`has` record whether the hash was present.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlobOp {
    /// `put(bytes)` — the content hash returned and the number of bytes stored under it.
    Put { hash: Hash, len: usize },
    /// `get(hash)` — `hit` is whether the store held bytes for `hash`.
    Get { hash: Hash, hit: bool },
    /// `has(hash)` — `present` is the answer.
    Has { hash: Hash, present: bool },
}

/// A call to the node-side **provenance** capability (`design/cadenza-platform.md` §4): the privileged
/// `program-of` read, which resolves the program a running reducer was spawned from. A narrow capability —
/// one op today — so a checker can assert *who asked which provenance question, and what the platform
/// answered*, observed at the host boundary like a store call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvOp {
    /// `program-of(reducer)` — the queried reducer, and the program it resolved to (`None` if no reducer is
    /// running under that id). The answer is recorded so a checker sees what the platform returned, not just
    /// that the reducer asked.
    ProgramOf {
        reducer: ReducerId,
        program: Option<ProgramHash>,
    },
}

/// A call to the node-side **reducer-graph** capability (`design/cadenza-platform.md` §7) — the routing
/// substrate a reducer reads and writes: the active set, the spawn tree, supervision edges, and the
/// per-contract handler chains. One variant per host-wired method (the `ReducerGraph` conveniences decompose
/// to these). Each records the call's arguments **and its result**, so a checker asserts what the reducer
/// asked of the graph and what it got back — e.g. an event reducer's `neighbors(kind = for_contract(C))`
/// read is exactly how it routes `C`. An [`EdgeKind`] is recorded as its raw hash [`Bytes`] (like a
/// contract-id: `for_contract(C)` is `C`'s hash, matchable relationally); a [`Dir`] as a string; every
/// reducer-id list preserves its returned order (neighbours come back in **weight-then-id** = chain order,
/// which is routing order).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphOp {
    /// `insert(node)` — `added` is whether the node was newly added (`false` if already present).
    Insert { node: ReducerId, added: bool },
    /// `contains(node)` — `present` is whether the node is in the active set.
    Contains { node: ReducerId, present: bool },
    /// `remove(node)` — `existed` is whether the node was present and is now gone.
    Remove { node: ReducerId, existed: bool },
    /// `link(from, to, kind)` — `added` is whether the edge was newly added.
    Link {
        from: ReducerId,
        to: ReducerId,
        kind: EdgeKind,
        added: bool,
    },
    /// `set_edges(from, kind, targets)` — the whole-chain replace; `prior` is the ordered targets it
    /// replaced (empty if none). Both `targets` and `prior` keep their order (weight order).
    SetEdges {
        from: ReducerId,
        kind: EdgeKind,
        targets: Vec<ReducerId>,
        prior: Vec<ReducerId>,
    },
    /// `neighbors(node, kind, dir)` — the direct neighbours, in weight-then-id (chain) order.
    Neighbors {
        node: ReducerId,
        kind: EdgeKind,
        dir: Dir,
        result: Vec<ReducerId>,
    },
    /// `in_kinds(node)` — the distinct kinds of the node's in-edges, ascending.
    InKinds {
        node: ReducerId,
        result: Vec<EdgeKind>,
    },
    /// `reach(node, kind, dir)` — the nodes transitively reachable along `kind` edges, nearest first.
    Reach {
        node: ReducerId,
        kind: EdgeKind,
        dir: Dir,
        result: Vec<ReducerId>,
    },
}

/// Which entry point folded a delivered event (`design/cadenza-platform.md` §3) — the observable
/// distinction the runtime routes on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// `on_message` — an effect performed on the reducer, carrying its source `Origin`.
    Message,
    /// `on_response` — a reply to a request the reducer performed.
    Response,
    /// `on_notification` — an unsolicited control-plane event (birth, a lifecycle event, a new handler).
    Notification,
}

/// An event observed at a reducer (`design/cadenza-platform.md` §3/§4/§10). The three things a fold does
/// that are observable at the reducer boundary: an event is delivered into it, it emits requests, and it
/// may close itself. These are exactly what the design's observation log records — the event vocabulary
/// of §10, seen without decoding a payload or knowing the program's language. The reducer this happened
/// at is the enclosing [`Record::source`]; every buffer is [`Bytes`] (O(1) clone).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EventOp {
    /// An event delivered into the reducer's mailbox and folded by it. `kind` is the entry point;
    /// `contract` is the event's contract-id. `from` is the emitter's `Origin` — present only for a
    /// message (a response's or notification's source is the runtime). `continuation_token` correlates a
    /// message/response (empty for a notification). `payload` is the delivered value, or a response's
    /// `Ok` bytes; `error` is `Some` for a response that delivered a runtime failure (`Err`) instead.
    Delivered {
        kind: EventKind,
        contract: ContractId,
        from: Option<Origin>,
        continuation_token: Bytes,
        payload: Bytes,
        error: Option<Error>,
    },
    /// A request the reducer emitted from a fold — an effect it performs or a timer it arms. `contract` is
    /// the request's contract-id, `payload` its input value, `continuation_token` the token it will
    /// correlate the response by, and `has_deadline` whether it set a per-request deadline. A privileged
    /// **deliver** an event reducer routes is a distinct act, recorded as [`Routed`](Self::Routed).
    Emitted {
        contract: ContractId,
        payload: Bytes,
        continuation_token: Bytes,
        has_deadline: bool,
    },
    /// A privileged **deliver** an event reducer performed — the routing act itself (§4), recorded at the
    /// `deliver` host boundary independent of whether a target was running to receive it (so a §4 dispatch
    /// run asserts what the reducer routed without a live listener). The send-side counterpart to
    /// [`Delivered`](Self::Delivered): `kind` is which deliver primitive (`on_message`/`on_response`/
    /// `on_notification` at the target), `target` the reducer routed to, `contract` the routed event's
    /// contract-id, `continuation_token` the correlation token (empty for a notification), `payload` the
    /// routed value (a message's payload or a response's `Ok` bytes), and `error` `Some` for a response
    /// that routed a runtime failure (`Err`) instead.
    Routed {
        kind: EventKind,
        target: ReducerId,
        contract: ContractId,
        continuation_token: Bytes,
        payload: Bytes,
        error: Option<Error>,
    },
    /// The reducer terminated itself, returning `Break(schema, reason)` — the typed reason for its
    /// closure (§3). A clean completion and a failure are both closes, distinguished only by the reason.
    Closed { schema: ContractId, reason: Bytes },
    /// The reducer's fold **failed uncontrolled** — it panicked (or exhausted fuel) rather than
    /// returning — so it could not describe its own exit (`design/cadenza-platform.md` §3/§10:
    /// fold-failed). `during` and `contract` name the event whose fold failed; `reason` is the panic
    /// message. This is the reducer's terminal event, distinct from the [`Closed`](EventOp::Closed) of a
    /// controlled exit; the runtime separately delivers a `crashed` lifecycle event to any watcher.
    Failed {
        during: EventKind,
        contract: ContractId,
        reason: Str,
    },
}

/// The observation log: a cheaply-clonable handle over shared, append-only records. Clone it to hand
/// the same log to several recording stores (each clone shares the one underlying buffer), then
/// [`snapshot`](ObservationLog::snapshot) it once the run is quiescent to read what happened.
///
/// The shared buffer is behind a [`std::sync::Mutex`], not a runtime-specific lock, so the log is
/// runtime-agnostic: it records identically whether the platform runs on tokio or under the bach
/// simulator. A record append is a short critical section (assign a sequence number, push), never
/// held across an await, so it does not stall the async runtime.
/// A live sink over the observation log: called with each [`Record`] the moment it is appended (see
/// [`ObservationLog::on_record`]). Shared behind an `Arc` so the log stays cheaply clonable.
type RecordSink = Arc<dyn Fn(&Record) + Send + Sync>;

#[derive(Clone, Default)]
pub struct ObservationLog {
    records: Arc<Mutex<Vec<Record>>>,
    /// An optional live sink called with each [`Record`] as it is appended (in `seq` order), set with
    /// [`on_record`](ObservationLog::on_record). The log always accumulates records for a later
    /// [`snapshot`](ObservationLog::snapshot); a sink additionally *streams* them as they happen, so a run
    /// that hangs or crashes before its log is rendered still shows its progress. `None` (the default)
    /// streams nothing.
    sink: Option<RecordSink>,
}

impl ObservationLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stream each record live: `sink` is called with every [`Record`] the moment it is appended, in `seq`
    /// order, in addition to the log accumulating it for a later [`snapshot`](ObservationLog::snapshot). The
    /// integration-test executable uses this to emit each observation line as it happens (behind an env
    /// toggle), so a run that gets stuck in a loop or crashes *before* its log is rendered still shows the
    /// progress up to the hang/crash, rather than losing the whole log (which is only rendered once the run
    /// completes). The sink must be cheap and must not call back into the log — it runs inside the append's
    /// short critical section, which is what keeps the streamed lines in the one global order.
    #[must_use]
    pub fn on_record(mut self, sink: impl Fn(&Record) + Send + Sync + 'static) -> Self {
        self.sink = Some(Arc::new(sink));
        self
    }

    /// Append an observation: stamp it with the next sequence number and store it. Returns the `seq`
    /// assigned. Called by the recording stores; a checker never appends. If a live sink is installed
    /// (see [`on_record`](ObservationLog::on_record)) it is called with the just-appended record.
    pub fn record(&self, time_ns: u64, source: Origin, entry: Entry) -> u64 {
        let mut records = self.lock();
        let seq = records.len() as u64;
        records.push(Record {
            seq,
            time_ns,
            source,
            entry,
        });
        if let Some(sink) = &self.sink {
            // Stream the just-appended record. Held inside the critical section on purpose: it serializes the
            // emit with the push, so streamed lines never interleave out of `seq` order under concurrent
            // recorders. The sink is a short, non-reentrant write.
            sink(records.last().expect("a record was just pushed"));
        }
        seq
    }

    /// A snapshot of every record so far, in order — what a checker reads. Each record clones cheaply
    /// (its buffers are `Bytes`), and the snapshot is detached from the log, so the run may continue
    /// (or the log drop) without affecting it.
    #[must_use]
    pub fn snapshot(&self) -> Vec<Record> {
        self.lock().clone()
    }

    /// The number of records logged so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// Whether nothing has been recorded yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Lock the shared buffer. A poisoned lock means a recorder panicked mid-append, which a test run
    /// treats as a failed run — recover the buffer rather than cascading the poison, since the records
    /// already appended are still the truth of what happened.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Record>> {
        self.records
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Render an observation log as human-readable lines — one per [`Record`], in order — for inspection
/// (`design/cadenza-platform.md` §9: where a log is kept, human inspection should be able to render the
/// raw events). This is a rendering *of* the log, not a program's projection of it: every record shows
/// its seq, time, acting reducer, and what happened. Useful in a checker's failure message or when
/// debugging a run. For programmatic use read the [`Record`]s directly; this is for eyes.
#[must_use]
pub fn render(records: &[Record]) -> String {
    let mut out = String::new();
    for r in records {
        // writeln! into a String is infallible.
        let _ = writeln!(out, "{r}");
    }
    out
}

impl fmt::Display for Record {
    /// One line: `#<seq> @<time>ns <reducer>  <what>`. Ids and hashes are shown short (a prefix) since a
    /// human is scanning; the full values live in the record for programmatic use. Byte buffers show a
    /// quoted preview when short valid UTF-8, else a byte count.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:<3} @{:>13}ns  {:<10}  ",
            self.seq,
            self.time_ns,
            short(self.source.reducer)
        )?;
        match &self.entry {
            Entry::Kv(op) => match op {
                KvOp::Get { key, hit } => {
                    write!(f, "kv get {} -> {}", preview(key), hit_miss(*hit))
                }
                KvOp::Put { key, value } => {
                    write!(f, "kv put {} = {}", preview(key), preview(value))
                }
                KvOp::Delete { key, existed } => write!(
                    f,
                    "kv delete {} -> {}",
                    preview(key),
                    if *existed { "existed" } else { "absent" }
                ),
                KvOp::Scan {
                    lower,
                    upper,
                    keys_only,
                } => write!(
                    f,
                    "kv scan{} {}..{}",
                    if *keys_only { " (keys)" } else { "" },
                    bound(lower),
                    bound(upper)
                ),
            },
            Entry::Blob(op) => match op {
                BlobOp::Put { hash, len } => write!(f, "blob put {} ({len} bytes)", short(*hash)),
                BlobOp::Get { hash, hit } => {
                    write!(f, "blob get {} -> {}", short(*hash), hit_miss(*hit))
                }
                BlobOp::Has { hash, present } => write!(
                    f,
                    "blob has {} -> {}",
                    short(*hash),
                    if *present { "present" } else { "absent" }
                ),
            },
            Entry::Event(op) => match op {
                EventOp::Delivered {
                    kind,
                    contract,
                    from,
                    payload,
                    ..
                } => {
                    let verb = match kind {
                        EventKind::Message => "recv msg",
                        EventKind::Response => "recv response",
                        EventKind::Notification => "recv notification",
                    };
                    // Show the delivered payload (preview) at parity with the emit line, so a scan of the log
                    // shows *what* was delivered — the counterpart to what was emitted.
                    match from {
                        Some(o) => write!(
                            f,
                            "{verb} {} {} from {}",
                            short(*contract),
                            preview(payload),
                            short(o.reducer)
                        ),
                        None => write!(f, "{verb} {} {}", short(*contract), preview(payload)),
                    }
                }
                EventOp::Emitted {
                    contract,
                    payload,
                    continuation_token,
                    has_deadline,
                } => write!(
                    f,
                    "emit {} {} token {}{}",
                    short(*contract),
                    preview(payload),
                    preview(continuation_token),
                    if *has_deadline { " (deadline)" } else { "" }
                ),
                // Show the close REASON (preview), not just the schema — the reason is why the reducer
                // closed, the first thing a reader needs when a run ends in a close (e.g. a checker that
                // closed without emitting a verdict).
                EventOp::Closed { schema, reason } => {
                    write!(f, "close {} {}", short(*schema), preview(reason))
                }
                EventOp::Failed {
                    during,
                    contract,
                    reason,
                } => write!(
                    f,
                    "fold-failed ({during:?} {}): {:?}",
                    short(*contract),
                    reason.as_str()
                ),
                // The send side: show what the event reducer routed (deliver primitive, contract, payload)
                // and to whom — the counterpart to the `emit`/`recv` lines, so a §4 dispatch run's routed
                // event is visible in a log scan.
                EventOp::Routed {
                    kind,
                    target,
                    contract,
                    payload,
                    ..
                } => {
                    let verb = match kind {
                        EventKind::Message => "deliver msg",
                        EventKind::Response => "deliver response",
                        EventKind::Notification => "deliver notification",
                    };
                    write!(
                        f,
                        "{verb} {} {} to {}",
                        short(*contract),
                        preview(payload),
                        short(*target)
                    )
                }
            },
            Entry::Provenance(op) => match op {
                ProvOp::ProgramOf { reducer, program } => match program {
                    Some(p) => write!(f, "program-of {} -> {}", short(*reducer), short(*p)),
                    None => write!(f, "program-of {} -> none", short(*reducer)),
                },
            },
            Entry::Graph(op) => match op {
                GraphOp::Insert { node, added } => write!(
                    f,
                    "graph insert {} -> {}",
                    short(*node),
                    if *added { "added" } else { "present" }
                ),
                GraphOp::Contains { node, present } => write!(
                    f,
                    "graph contains {} -> {}",
                    short(*node),
                    if *present { "yes" } else { "no" }
                ),
                GraphOp::Remove { node, existed } => write!(
                    f,
                    "graph remove {} -> {}",
                    short(*node),
                    if *existed { "existed" } else { "absent" }
                ),
                GraphOp::Link {
                    from,
                    to,
                    kind,
                    added,
                } => write!(
                    f,
                    "graph link {} -> {} kind {} -> {}",
                    short(*from),
                    short(*to),
                    short(kind.hash()),
                    if *added { "added" } else { "exists" }
                ),
                GraphOp::SetEdges {
                    from,
                    kind,
                    targets,
                    prior,
                } => write!(
                    f,
                    "graph set-edges {} kind {} = [{}] (was [{}])",
                    short(*from),
                    short(kind.hash()),
                    short_ids(targets),
                    short_ids(prior)
                ),
                GraphOp::Neighbors {
                    node,
                    kind,
                    dir,
                    result,
                } => write!(
                    f,
                    "graph neighbors {} kind {} {} -> [{}]",
                    short(*node),
                    short(kind.hash()),
                    dir_str(*dir),
                    short_ids(result)
                ),
                GraphOp::InKinds { node, result } => write!(
                    f,
                    "graph in-kinds {} -> [{}]",
                    short(*node),
                    short_kinds(result)
                ),
                GraphOp::Reach {
                    node,
                    kind,
                    dir,
                    result,
                } => write!(
                    f,
                    "graph reach {} kind {} {} -> [{}]",
                    short(*node),
                    short(kind.hash()),
                    dir_str(*dir),
                    short_ids(result)
                ),
            },
            Entry::Spawn(info) => write!(
                f,
                "spawn {:?} program={} kind={:?}",
                info.name.as_str(),
                short(info.program),
                info.kind
            ),
            Entry::HostCallRejected(c) => write!(
                f,
                "host-call-rejected {}.{} ({} arg(s), unparseable)",
                c.iface.as_str(),
                c.op.as_str(),
                c.raw_args.len()
            ),
            Entry::Run(r) => write!(
                f,
                "run program=({} byte(s)) contract=({} byte(s)) input=({} byte(s)) -> {}",
                r.program.len(),
                r.contract.len(),
                r.input.len(),
                match &r.output {
                    Some(o) => format!("ok ({} byte(s))", o.len()),
                    None => format!("err {}", r.error.as_ref().map_or("?", Str::as_str)),
                }
            ),
        }
    }
}

/// A short prefix of a hash/id's textual (base62, §8) form — enough to recognize it when scanning,
/// not the full value.
fn short(id: impl fmt::Display) -> String {
    let s = id.to_string();
    // base62 is ASCII, so a byte slice is a char boundary; take a prefix and mark it elided.
    match s.get(..10) {
        Some(prefix) if prefix.len() < s.len() => format!("{prefix}…"),
        _ => s,
    }
}

/// A comma-joined preview of a reducer-id list, in order — for a graph op's neighbour/chain result.
fn short_ids(ids: &[ReducerId]) -> String {
    ids.iter()
        .map(|id| short(*id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A comma-joined preview of an edge-kind list, in order — for a graph `in-kinds` result.
fn short_kinds(kinds: &[EdgeKind]) -> String {
    kinds
        .iter()
        .map(|k| short(k.hash()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// A [`Dir`] as its wire string — matches [`log_value`](super::log_value)'s encoding.
fn dir_str(dir: Dir) -> &'static str {
    match dir {
        Dir::Out => "out",
        Dir::In => "in",
    }
}

/// A byte buffer preview for the eye: a quoted string when it is short, valid UTF-8, else its length.
fn preview(bytes: &Bytes) -> String {
    if bytes.len() <= 32
        && let Ok(s) = std::str::from_utf8(bytes)
    {
        return format!("{s:?}");
    }
    format!("<{} bytes>", bytes.len())
}

/// A scan bound for the render: the value's preview, or `*` for unbounded (inclusive/exclusive elided —
/// this is for human scanning; the record holds the exact bound).
fn bound(b: &Bound<Bytes>) -> String {
    match b {
        Bound::Unbounded => "*".to_string(),
        Bound::Included(v) | Bound::Excluded(v) => preview(v),
    }
}

fn hit_miss(hit: bool) -> &'static str {
    if hit { "hit" } else { "miss" }
}

#[cfg(test)]
mod render_tests {
    use super::{Entry, EventKind, EventOp, GraphOp, KvOp, ProvOp, Record, SpawnInfo, render};
    use crate::{
        Bytes, ContractId, Dir, EdgeKind, HostId, Origin, ProgramHash, ReducerId, ReducerKind, Str,
    };

    fn rec(seq: u64, reducer: &[u8], entry: Entry) -> Record {
        Record {
            seq,
            time_ns: seq * 1000,
            source: Origin {
                reducer: ReducerId::of(reducer),
                host: HostId::of(b"node"),
            },
            entry,
        }
    }

    #[test]
    fn render_shows_one_readable_line_per_record_covering_every_entry_kind() {
        let records = vec![
            rec(
                0,
                b"agent",
                Entry::Spawn(SpawnInfo {
                    name: Str::from("agent"),
                    program: ProgramHash::of(b"agent-prog"),
                    parent: ReducerId::of(b"agent"),
                    kind: ReducerKind::Ordinary,
                }),
            ),
            rec(
                1,
                b"agent",
                Entry::Kv(KvOp::Put {
                    key: Bytes::from_static(b"seen/x"),
                    value: Bytes::from_static(b"1"),
                }),
            ),
            rec(
                2,
                b"agent",
                Entry::Event(EventOp::Emitted {
                    contract: ContractId::of(b"http.get"),
                    payload: Bytes::from_static(b"url=/x"),
                    continuation_token: Bytes::from_static(b"c1"),
                    has_deadline: true,
                }),
            ),
            rec(
                3,
                b"agent",
                Entry::Event(EventOp::Delivered {
                    kind: EventKind::Notification,
                    contract: ContractId::of(b"http.get"),
                    from: None,
                    continuation_token: Bytes::new(),
                    payload: Bytes::from_static(b"body=hi"),
                    error: None,
                }),
            ),
            rec(
                4,
                b"agent",
                Entry::Event(EventOp::Closed {
                    schema: ContractId::of(b"done"),
                    reason: Bytes::from_static(b"quiescent"),
                }),
            ),
            rec(
                5,
                b"agent",
                Entry::Event(EventOp::Routed {
                    kind: EventKind::Message,
                    target: ReducerId::of(b"handler"),
                    contract: ContractId::of(b"http.get"),
                    continuation_token: Bytes::from_static(b"c1"),
                    payload: Bytes::from_static(b"routed=on"),
                    error: None,
                }),
            ),
            rec(
                6,
                b"agent",
                Entry::Provenance(ProvOp::ProgramOf {
                    reducer: ReducerId::of(b"peer"),
                    program: Some(ProgramHash::of(b"peer-prog")),
                }),
            ),
            rec(
                7,
                b"agent",
                Entry::Graph(GraphOp::Neighbors {
                    node: ReducerId::of(b"owner"),
                    kind: EdgeKind::for_contract(ContractId::of(b"http.get")),
                    dir: Dir::Out,
                    result: vec![ReducerId::of(b"h1"), ReducerId::of(b"h2")],
                }),
            ),
        ];
        let out = render(&records);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 8, "one line per record");
        // Each line leads with its seq and carries a readable description of the entry.
        assert!(lines[0].starts_with("#0"));
        assert!(lines[0].contains("spawn \"agent\""), "line: {}", lines[0]);
        assert!(
            lines[1].contains("kv put \"seen/x\" = \"1\""),
            "line: {}",
            lines[1]
        );
        // The emit line shows the contract, the payload it emitted, the correlation token, and the deadline
        // marker — at parity with the recv/kv lines, so a reader (or a coarse grep) sees what was emitted.
        assert!(lines[2].contains("emit "), "line: {}", lines[2]);
        assert!(lines[2].contains("\"url=/x\""), "line: {}", lines[2]);
        assert!(lines[2].contains("token \"c1\""), "line: {}", lines[2]);
        assert!(lines[2].contains("(deadline)"), "line: {}", lines[2]);
        // The delivered line shows what was delivered (the payload), at parity with the emit line.
        assert!(lines[3].contains("recv notification"), "line: {}", lines[3]);
        assert!(lines[3].contains("\"body=hi\""), "line: {}", lines[3]);
        // The close line shows WHY it closed (the reason), not just the schema.
        assert!(lines[4].contains("close "), "line: {}", lines[4]);
        assert!(lines[4].contains("\"quiescent\""), "line: {}", lines[4]);
        // The routed line shows the deliver primitive, the payload, and the target — the send-side counterpart
        // to the recv/emit lines, so a §4 dispatch run's routed event is visible in a log scan.
        assert!(lines[5].contains("deliver msg"), "line: {}", lines[5]);
        assert!(lines[5].contains("\"routed=on\""), "line: {}", lines[5]);
        assert!(lines[5].contains(" to "), "line: {}", lines[5]);
        // The provenance line shows the queried reducer and the program it resolved to.
        assert!(lines[6].contains("program-of "), "line: {}", lines[6]);
        assert!(lines[6].contains(" -> "), "line: {}", lines[6]);
        // The graph line shows the op, the node, the direction, and the ordered neighbour result.
        assert!(lines[7].contains("graph neighbors "), "line: {}", lines[7]);
        assert!(lines[7].contains(" out -> ["), "line: {}", lines[7]);
        // An empty log renders to an empty string.
        assert!(render(&[]).is_empty());
    }
}

#[cfg(test)]
mod log_tests {
    use super::{Entry, KvOp, ObservationLog};
    use crate::{Bytes, HostId, Origin, ReducerId};
    use std::sync::{Arc, Mutex};

    fn origin(reducer: &[u8]) -> Origin {
        Origin {
            reducer: ReducerId::of(reducer),
            host: HostId::of(b"node"),
        }
    }

    fn a_get() -> Entry {
        Entry::Kv(KvOp::Get {
            key: Bytes::from_static(b"k"),
            hit: false,
        })
    }

    #[test]
    fn on_record_streams_each_record_live_in_seq_order() {
        // A live sink observes every record the moment it is appended, in seq order — the streaming an
        // executable uses to surface a run's progress before its log is rendered.
        let seen: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let log = ObservationLog::new().on_record(move |r| sink_seen.lock().unwrap().push(r.seq));
        for i in 0..3u64 {
            log.record(i * 100, origin(b"r"), a_get());
        }
        // The sink saw each append live, in order, and the log still accumulated them for a snapshot.
        assert_eq!(*seen.lock().unwrap(), vec![0, 1, 2]);
        assert_eq!(log.snapshot().len(), 3);
    }

    #[test]
    fn a_log_without_a_sink_records_normally() {
        // The default (no sink) accumulates without streaming — the unchanged behavior.
        let log = ObservationLog::new();
        assert_eq!(log.record(0, origin(b"r"), a_get()), 0);
        assert_eq!(log.len(), 1);
    }
}
