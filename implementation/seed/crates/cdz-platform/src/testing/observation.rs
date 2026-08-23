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

use crate::{Bytes, ContractId, Error, Hash, Origin, ProgramHash, ReducerId, ReducerKind, Str};
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
    /// The harness spawned a reducer and assigned it a name — recorded at the start of a run so the log
    /// is self-describing: a reader derefs a name to the reducer id ([`Record::source`]) it was assigned,
    /// with no out-of-band metadata (§3).
    Spawn(SpawnInfo),
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
    /// Its privilege — ordinary, or a privileged event/system reducer.
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
    /// A request the reducer emitted from a fold — an effect it performs, a timer it arms, or (for an
    /// event reducer) a deliver it routes. `contract` is the request's contract-id, `payload` its input
    /// value, `continuation_token` the token it will correlate the response by, and `has_deadline`
    /// whether it set a per-request deadline.
    Emitted {
        contract: ContractId,
        payload: Bytes,
        continuation_token: Bytes,
        has_deadline: bool,
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
#[derive(Clone, Default)]
pub struct ObservationLog {
    records: Arc<Mutex<Vec<Record>>>,
}

impl ObservationLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append an observation: stamp it with the next sequence number and store it. Returns the `seq`
    /// assigned. Called by the recording stores; a checker never appends.
    pub fn record(&self, time_ns: u64, source: Origin, entry: Entry) -> u64 {
        let mut records = self.lock();
        let seq = records.len() as u64;
        records.push(Record {
            seq,
            time_ns,
            source,
            entry,
        });
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
                    ..
                } => {
                    let verb = match kind {
                        EventKind::Message => "recv msg",
                        EventKind::Response => "recv response",
                        EventKind::Notification => "recv notification",
                    };
                    match from {
                        Some(o) => {
                            write!(f, "{verb} {} from {}", short(*contract), short(o.reducer))
                        }
                        None => write!(f, "{verb} {}", short(*contract)),
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
                EventOp::Closed { schema, .. } => write!(f, "close {}", short(*schema)),
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
            },
            Entry::Spawn(info) => write!(
                f,
                "spawn {:?} program={} kind={:?}",
                info.name.as_str(),
                short(info.program),
                info.kind
            ),
        }
    }
}

/// A short prefix of a hash/id's textual (base64url, §8) form — enough to recognize it when scanning,
/// not the full value.
fn short(id: impl fmt::Display) -> String {
    let s = id.to_string();
    // base64url is ASCII, so a byte slice is a char boundary; take a prefix and mark it elided.
    match s.get(..10) {
        Some(prefix) if prefix.len() < s.len() => format!("{prefix}…"),
        _ => s,
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
    use super::{Entry, EventOp, KvOp, Record, SpawnInfo, render};
    use crate::{Bytes, ContractId, HostId, Origin, ProgramHash, ReducerId, ReducerKind, Str};

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
        ];
        let out = render(&records);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 3, "one line per record");
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
        // An empty log renders to an empty string.
        assert!(render(&[]).is_empty());
    }
}
