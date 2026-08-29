//! The observation log as a **Cadenza value** — the language-neutral, full-fidelity log a checker reads
//! (`design/cadenza-platform.md` §9).
//!
//! The human [`render`](super::render) is lossy on purpose (ids and payloads elide for scanning), so a
//! checker that must assert on *exact* bytes — a contract-id, a continuation token, a payload — cannot read
//! the rendered text. [`serialize`] encodes the whole log to a Cadenza binary AST (via
//! [`cadenza_ast::codec`]), the same encoding every value on the wire uses; [`deserialize`] is its total
//! inverse. A checker (a native `Fn(&[Record])`, or later a wasm component reading the serialized bytes)
//! asserts over the decoded records with nothing elided. This is symmetric with the harness *input*, which
//! is itself a Cadenza binary AST ([`HarnessSpec`](super::HarnessSpec)) — the run and its log are both
//! Cadenza values, so the whole harness is language-neutral end to end.
//!
//! ## The value shape
//! The log is a canonical Cadenza value (the form `Value.encode`/`Value.decode` uses) so a guest checker
//! decodes the whole log into a `List(LogRecord)` from `guests/log-schema.cdz`: a **name-headed** list wrapped
//! at the root in the ascription `Value.decode` requires; each element is a single-constructor `LogRecord`, so
//! its value is the bare **name-headed** record carrying its own `(: … LogRecord)` ascription; and its `entry`
//! is an `Entry` **sum** in the bare-constructor form `(<Ctor> <record>)`. `Value.decode` matches constructor
//! and field names EXACTLY (no kebab/camel normalization) and reads a record's fields **canonically ordered**,
//! so the constructors are the exact schema spellings (`Emitted`, `KvGet`, …), the field names the exact schema
//! spellings (`eventKind`, `keysOnly`, …), and fields are emitted name-sorted (by `contract_value::record`):
//! ```text
//! (: (list (: <record> LogRecord)…) List)
//! <record>  = (record (= entry <entry>) (= seq <u>) (= source <origin>) (= time <u>))   ; fields name-sorted
//! <origin>  = (record (= host b"…") (= reducer b"…"))
//! <entry>   = (KvPut (record (= key b"…") (= value b"…")))                               ; one schema ctor per kind
//! ```
//! Encoding the entry as a sum (not a `tag`-discriminated record) is what lets a Cadenza **checker**
//! `Value.decode` the whole heterogeneous log into a `List(Entry)`: a value-form sum decodes into a Cadenza
//! sum by its exact constructor name, whereas a record with a `tag` field plus per-kind fields has no single
//! fixed record type. Every id/hash crosses as its raw bytes, every payload/key as opaque bytes, every flag as
//! `0`/`1`, so no fidelity is lost. Decoding is total: any malformation is a rejected log ([`deserialize`]
//! returns `None`), never a panic.

use super::observation::{
    ArgProbeCall, BlobOp, Entry, EventKind, EventOp, GraphOp, KvOp, ProvOp, Record, RejectedCall,
    RunCall, SpawnInfo,
};
use crate::contract_value::{
    as_ascribed, ascribe, bare_ctor, bytes_leaf, qctor, read_bytes, read_uint, record,
    record_field, uint_leaf,
};
use crate::{
    Bytes, ContractId, Dir, EdgeKind, Error, Hash, HostId, Origin, ProgramHash, ReducerId,
    ReducerKind, Str,
};
use cadenza_ast::ast::Struct;
use cadenza_ast::ast::{Arenas, Builder, CompoundCtor, Leaf, StructId};
use cadenza_ast::codec;
use std::ops::Bound;
use std::sync::Arc;

/// Serialize the observation log to a Cadenza binary AST — the full-fidelity, language-neutral log a checker
/// reads (§9). The exact inverse of [`deserialize`]: `deserialize(&serialize(r)) == Some(r)`. Re-exported as
/// `testing::serialize_log`.
#[must_use]
pub fn serialize(records: &[Record]) -> Vec<u8> {
    let mut b = Builder::new();
    let items: Vec<StructId> = records.iter().map(|r| record_value(&mut b, r)).collect();
    let list = list_value(&mut b, items);
    // The whole log crosses as one canonical Cadenza value a guest `Value.decode`s into a `List(LogRecord)`,
    // so the root carries the ascription `Value.decode` requires (`(: <list> List)`); the type token is
    // name-agnostic (the decoder is target-directed), so the bare `List` suffices.
    let root = ascribe(&mut b, list, "List");
    codec::encode(&b.finish(root))
}

/// Decode an observation log from a Cadenza binary AST. Total: any input that is not a well-formed log value
/// is a rejected log (`None`), never a panic — the same discipline as every value reader in the crate.
#[must_use]
pub fn deserialize(bytes: &[u8]) -> Option<Vec<Record>> {
    let arenas = codec::decode(bytes)?;
    // Strip the root ascription `(: <list> List)` if present (`serialize` writes it so a guest can
    // `Value.decode` the log); tolerate a bare list too, so the reader is liberal about the wrapper.
    let root = as_ascribed(&arenas, arenas.root).unwrap_or(arenas.root);
    let items = list_items(&arenas, root)?;
    items.iter().map(|&r| read_record(&arenas, r)).collect()
}

// --- record + envelope ---

fn record_value(b: &mut Builder, r: &Record) -> StructId {
    let seq = uint_leaf(b, r.seq);
    let time = uint_leaf(b, r.time_ns);
    let source = origin_value(b, &r.source);
    let entry = entry_value(b, &r.entry);
    let rec = record(
        b,
        vec![
            ("seq", seq),
            ("time", time),
            ("source", source),
            ("entry", entry),
        ],
    );
    // `LogRecord` is a SINGLE-constructor sum, so its value is the bare record with the constructor elided —
    // but a bare record is ambiguous, so each single-constructor sum value carries its own `(: … LogRecord)`
    // ascription (this is what `Value.encode` emits for each element of a `List(LogRecord)`). Without it the
    // list elements do not decode as `LogRecord`s.
    ascribe(b, rec, "LogRecord")
}

fn read_record(arenas: &Arenas, id: StructId) -> Option<Record> {
    // Strip the per-element `(: <record> LogRecord)` ascription `serialize` writes (liberal — a bare record
    // is tolerated too).
    let id = as_ascribed(arenas, id).unwrap_or(id);
    Some(Record {
        seq: read_uint(arenas, field(arenas, id, "seq")?)?,
        time_ns: read_uint(arenas, field(arenas, id, "time")?)?,
        source: read_origin(arenas, field(arenas, id, "source")?)?,
        entry: read_entry(arenas, field(arenas, id, "entry")?)?,
    })
}

fn origin_value(b: &mut Builder, o: &Origin) -> StructId {
    let reducer = bytes_leaf(b, o.reducer.hash().as_bytes());
    let host = bytes_leaf(b, o.host.hash().as_bytes());
    record(b, vec![("reducer", reducer), ("host", host)])
}

fn read_origin(arenas: &Arenas, id: StructId) -> Option<Origin> {
    Some(Origin {
        reducer: read_reducer(arenas, field(arenas, id, "reducer")?)?,
        host: read_host(arenas, field(arenas, id, "host")?)?,
    })
}

// --- entry ---

fn entry_value(b: &mut Builder, e: &Entry) -> StructId {
    match e {
        Entry::Kv(op) => kv_value(b, op),
        Entry::Blob(op) => blob_value(b, op),
        Entry::Event(op) => event_value(b, op),
        Entry::Provenance(op) => prov_value(b, op),
        Entry::Graph(op) => graph_value(b, op),
        Entry::Spawn(info) => spawn_value(b, info),
        Entry::HostCallRejected(c) => rejected_value(b, c),
        Entry::Run(r) => run_value(b, r),
        Entry::ArgProbe(c) => arg_probe_value(b, c),
    }
}

fn read_entry(arenas: &Arenas, id: StructId) -> Option<Entry> {
    let (tag, inner) = entry_ctor(arenas, id)?;
    Some(match tag {
        "KvGet" | "KvPut" | "KvDelete" | "KvScan" => Entry::Kv(read_kv(arenas, inner, tag)?),
        "BlobPut" | "BlobGet" | "BlobHas" => Entry::Blob(read_blob(arenas, inner, tag)?),
        "Delivered" | "Emitted" | "Closed" | "Failed" | "Routed" => {
            Entry::Event(read_event(arenas, inner, tag)?)
        }
        "ProgramOf" => Entry::Provenance(read_prov(arenas, inner, tag)?),
        "GraphInsert" | "GraphContains" | "GraphRemove" | "GraphLink" | "GraphSetEdges"
        | "GraphNeighbors" | "GraphInKinds" | "GraphReach" => {
            Entry::Graph(read_graph(arenas, inner, tag)?)
        }
        "Spawn" => Entry::Spawn(read_spawn(arenas, inner)?),
        "HostCallRejected" => Entry::HostCallRejected(read_rejected(arenas, inner)?),
        "Run" => Entry::Run(read_run(arenas, inner)?),
        "ArgProbe" => Entry::ArgProbe(read_arg_probe(arenas, inner)?),
        _ => return None,
    })
}

// --- kv ---

fn kv_value(b: &mut Builder, op: &KvOp) -> StructId {
    match op {
        KvOp::Get { key, hit } => {
            let key = bytes_leaf(b, key);
            let hit = bool_value(b, *hit);
            tagged(b, "KvGet", vec![("key", key), ("hit", hit)])
        }
        KvOp::Put { key, value } => {
            let key = bytes_leaf(b, key);
            let value = bytes_leaf(b, value);
            tagged(b, "KvPut", vec![("key", key), ("value", value)])
        }
        KvOp::Delete { key, existed } => {
            let key = bytes_leaf(b, key);
            let existed = bool_value(b, *existed);
            tagged(b, "KvDelete", vec![("key", key), ("existed", existed)])
        }
        KvOp::Scan {
            lower,
            upper,
            keys_only,
        } => {
            let lower = bound_value(b, lower);
            let upper = bound_value(b, upper);
            let keys_only = bool_value(b, *keys_only);
            tagged(
                b,
                "KvScan",
                vec![("lower", lower), ("upper", upper), ("keysOnly", keys_only)],
            )
        }
    }
}

fn read_kv(arenas: &Arenas, id: StructId, tag: &str) -> Option<KvOp> {
    Some(match tag {
        "KvGet" => KvOp::Get {
            key: read_bytes(arenas, field(arenas, id, "key")?)?,
            hit: read_bool(arenas, field(arenas, id, "hit")?)?,
        },
        "KvPut" => KvOp::Put {
            key: read_bytes(arenas, field(arenas, id, "key")?)?,
            value: read_bytes(arenas, field(arenas, id, "value")?)?,
        },
        "KvDelete" => KvOp::Delete {
            key: read_bytes(arenas, field(arenas, id, "key")?)?,
            existed: read_bool(arenas, field(arenas, id, "existed")?)?,
        },
        "KvScan" => KvOp::Scan {
            lower: read_bound(arenas, field(arenas, id, "lower")?)?,
            upper: read_bound(arenas, field(arenas, id, "upper")?)?,
            keys_only: read_bool(arenas, field(arenas, id, "keysOnly")?)?,
        },
        _ => return None,
    })
}

/// A scan `Bound` as a Cadenza **sum** — the canonical bare-constructor form `(unbounded)`,
/// `(included <key>)`, or `(excluded <key>)`. A sum with a fixed per-constructor shape (not a `bound`-tagged record whose
/// `key` field is present only for included/excluded) is what lets a Cadenza checker `Value.decode` a log
/// containing kv-scan entries — the same reason the entry itself is a sum (§9 checker decodability).
fn bound_value(b: &mut Builder, bound: &Bound<Bytes>) -> StructId {
    match bound {
        Bound::Unbounded => qctor(b, "Bound", "Unbounded", vec![]),
        Bound::Included(key) => {
            let key = bytes_leaf(b, key);
            qctor(b, "Bound", "Included", vec![key])
        }
        Bound::Excluded(key) => {
            let key = bytes_leaf(b, key);
            qctor(b, "Bound", "Excluded", vec![key])
        }
    }
}

fn read_bound(arenas: &Arenas, id: StructId) -> Option<Bound<Bytes>> {
    let Struct::List(items) = arenas.get(id) else {
        return None;
    };
    let (&head, tail) = items.split_first()?;
    Some(match arenas.as_name(head)? {
        "Unbounded" => {
            if !tail.is_empty() {
                return None;
            }
            Bound::Unbounded
        }
        "Included" => {
            let [key] = <[StructId; 1]>::try_from(tail).ok()?;
            Bound::Included(read_bytes(arenas, key)?)
        }
        "Excluded" => {
            let [key] = <[StructId; 1]>::try_from(tail).ok()?;
            Bound::Excluded(read_bytes(arenas, key)?)
        }
        _ => return None,
    })
}

// --- blob ---

fn blob_value(b: &mut Builder, op: &BlobOp) -> StructId {
    match op {
        BlobOp::Put { hash, len } => {
            let hash = bytes_leaf(b, hash.as_bytes());
            let len = uint_leaf(b, *len as u64);
            tagged(b, "BlobPut", vec![("hash", hash), ("len", len)])
        }
        BlobOp::Get { hash, hit } => {
            let hash = bytes_leaf(b, hash.as_bytes());
            let hit = bool_value(b, *hit);
            tagged(b, "BlobGet", vec![("hash", hash), ("hit", hit)])
        }
        BlobOp::Has { hash, present } => {
            let hash = bytes_leaf(b, hash.as_bytes());
            let present = bool_value(b, *present);
            tagged(b, "BlobHas", vec![("hash", hash), ("present", present)])
        }
    }
}

fn read_blob(arenas: &Arenas, id: StructId, tag: &str) -> Option<BlobOp> {
    let hash = read_hash(arenas, field(arenas, id, "hash")?)?;
    Some(match tag {
        "BlobPut" => BlobOp::Put {
            hash,
            len: usize::try_from(read_uint(arenas, field(arenas, id, "len")?)?).ok()?,
        },
        "BlobGet" => BlobOp::Get {
            hash,
            hit: read_bool(arenas, field(arenas, id, "hit")?)?,
        },
        "BlobHas" => BlobOp::Has {
            hash,
            present: read_bool(arenas, field(arenas, id, "present")?)?,
        },
        _ => return None,
    })
}

// --- event ---

fn event_value(b: &mut Builder, op: &EventOp) -> StructId {
    match op {
        EventOp::Delivered {
            kind,
            contract,
            from,
            continuation_token,
            payload,
            error,
        } => {
            let event_kind = str_leaf(b, event_kind_str(*kind));
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let token = bytes_leaf(b, continuation_token);
            let payload = bytes_leaf(b, payload);
            // `from` and `error` are ALWAYS-PRESENT Option fields (`(Some …)` / `(None unit)`), never
            // conditionally-omitted, so the delivered record has one fixed shape a Cadenza checker
            // `Value.decode`s into a single record type (§9 checker decodability).
            let from_inner = from.as_ref().map(|o| origin_value(b, o));
            let from = option_value(b, from_inner);
            let error_inner = error.as_ref().map(|e| str_leaf(b, error_str(*e)));
            let error = option_value(b, error_inner);
            tagged(
                b,
                "Delivered",
                vec![
                    ("eventKind", event_kind),
                    ("contract", contract),
                    ("from", from),
                    ("token", token),
                    ("payload", payload),
                    ("error", error),
                ],
            )
        }
        EventOp::Emitted {
            contract,
            payload,
            continuation_token,
            has_deadline,
        } => {
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let payload = bytes_leaf(b, payload);
            let token = bytes_leaf(b, continuation_token);
            let has_deadline_leaf = bool_value(b, *has_deadline);
            tagged(
                b,
                "Emitted",
                vec![
                    ("contract", contract),
                    ("payload", payload),
                    ("token", token),
                    ("hasDeadline", has_deadline_leaf),
                ],
            )
        }
        EventOp::Closed { schema, reason } => {
            let schema = bytes_leaf(b, schema.hash().as_bytes());
            let reason = bytes_leaf(b, reason);
            tagged(b, "Closed", vec![("schema", schema), ("reason", reason)])
        }
        EventOp::Failed {
            during,
            contract,
            reason,
        } => {
            let during = str_leaf(b, event_kind_str(*during));
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let reason = str_leaf(b, reason.as_str());
            tagged(
                b,
                "Failed",
                vec![
                    ("during", during),
                    ("contract", contract),
                    ("reason", reason),
                ],
            )
        }
        EventOp::Routed {
            kind,
            target,
            contract,
            continuation_token,
            payload,
            error,
        } => {
            let event_kind = str_leaf(b, event_kind_str(*kind));
            let target = bytes_leaf(b, target.hash().as_bytes());
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let token = bytes_leaf(b, continuation_token);
            let payload = bytes_leaf(b, payload);
            // `error` is an always-present Option (`(Some …)` / `(None unit)`), never conditionally omitted,
            // so the routed record has one fixed shape a checker `Value.decode`s (§9), mirroring `Delivered`.
            let error_inner = error.as_ref().map(|e| str_leaf(b, error_str(*e)));
            let error = option_value(b, error_inner);
            tagged(
                b,
                "Routed",
                vec![
                    ("eventKind", event_kind),
                    ("target", target),
                    ("contract", contract),
                    ("token", token),
                    ("payload", payload),
                    ("error", error),
                ],
            )
        }
    }
}

fn read_event(arenas: &Arenas, id: StructId, tag: &str) -> Option<EventOp> {
    Some(match tag {
        "Delivered" => EventOp::Delivered {
            kind: read_event_kind(str_at(arenas, field(arenas, id, "eventKind")?)?)?,
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            from: read_option(arenas, field(arenas, id, "from")?, read_origin)?,
            continuation_token: read_bytes(arenas, field(arenas, id, "token")?)?,
            payload: read_bytes(arenas, field(arenas, id, "payload")?)?,
            error: read_option(arenas, field(arenas, id, "error")?, |a, i| {
                read_error(str_at(a, i)?)
            })?,
        },
        "Emitted" => EventOp::Emitted {
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            payload: read_bytes(arenas, field(arenas, id, "payload")?)?,
            continuation_token: read_bytes(arenas, field(arenas, id, "token")?)?,
            has_deadline: read_bool(arenas, field(arenas, id, "hasDeadline")?)?,
        },
        "Closed" => EventOp::Closed {
            schema: read_contract(arenas, field(arenas, id, "schema")?)?,
            reason: read_bytes(arenas, field(arenas, id, "reason")?)?,
        },
        "Failed" => EventOp::Failed {
            during: read_event_kind(str_at(arenas, field(arenas, id, "during")?)?)?,
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            reason: Str::from(str_at(arenas, field(arenas, id, "reason")?)?),
        },
        "Routed" => EventOp::Routed {
            kind: read_event_kind(str_at(arenas, field(arenas, id, "eventKind")?)?)?,
            target: read_reducer(arenas, field(arenas, id, "target")?)?,
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            continuation_token: read_bytes(arenas, field(arenas, id, "token")?)?,
            payload: read_bytes(arenas, field(arenas, id, "payload")?)?,
            error: read_option(arenas, field(arenas, id, "error")?, |a, i| {
                read_error(str_at(a, i)?)
            })?,
        },
        _ => return None,
    })
}

// --- provenance ---

fn prov_value(b: &mut Builder, op: &ProvOp) -> StructId {
    match op {
        ProvOp::ProgramOf { reducer, program } => {
            let reducer = bytes_leaf(b, reducer.hash().as_bytes());
            // `program` is an always-present Option (`(Some …)` / `(None unit)`), a fixed shape a checker
            // `Value.decode`s — the same discipline as a delivered event's `from`/`error`.
            let program_inner = program.as_ref().map(|p| bytes_leaf(b, p.hash().as_bytes()));
            let program = option_value(b, program_inner);
            tagged(
                b,
                "ProgramOf",
                vec![("reducer", reducer), ("program", program)],
            )
        }
    }
}

fn read_prov(arenas: &Arenas, id: StructId, tag: &str) -> Option<ProvOp> {
    Some(match tag {
        "ProgramOf" => ProvOp::ProgramOf {
            reducer: read_reducer(arenas, field(arenas, id, "reducer")?)?,
            program: read_option(arenas, field(arenas, id, "program")?, read_program)?,
        },
        _ => return None,
    })
}

// --- graph ---

/// A reducer-id list as a name-headed list of raw-hash byte leaves, preserving order.
fn reducer_list(b: &mut Builder, ids: &[ReducerId]) -> StructId {
    let items = ids
        .iter()
        .map(|r| bytes_leaf(b, r.hash().as_bytes()))
        .collect();
    list_value(b, items)
}

fn read_reducer_list(arenas: &Arenas, id: StructId) -> Option<Vec<ReducerId>> {
    list_items(arenas, id)?
        .iter()
        .map(|&r| read_reducer(arenas, r))
        .collect()
}

/// An edge-kind as its raw hash bytes (a `for_contract(C)` kind is `C`'s hash, matchable relationally).
fn edge_kind_leaf(b: &mut Builder, kind: &EdgeKind) -> StructId {
    bytes_leaf(b, kind.hash().as_bytes())
}

fn read_edge_kind(arenas: &Arenas, id: StructId) -> Option<EdgeKind> {
    Some(EdgeKind::from_hash(read_hash(arenas, id)?))
}

fn edge_kind_list(b: &mut Builder, kinds: &[EdgeKind]) -> StructId {
    let items = kinds.iter().map(|k| edge_kind_leaf(b, k)).collect();
    list_value(b, items)
}

fn read_edge_kind_list(arenas: &Arenas, id: StructId) -> Option<Vec<EdgeKind>> {
    list_items(arenas, id)?
        .iter()
        .map(|&k| read_edge_kind(arenas, k))
        .collect()
}

fn dir_str(dir: Dir) -> &'static str {
    match dir {
        Dir::Out => "out",
        Dir::In => "in",
    }
}

fn read_dir(s: &str) -> Option<Dir> {
    match s {
        "out" => Some(Dir::Out),
        "in" => Some(Dir::In),
        _ => None,
    }
}

fn graph_value(b: &mut Builder, op: &GraphOp) -> StructId {
    match op {
        GraphOp::Insert { node, added } => {
            let node = bytes_leaf(b, node.hash().as_bytes());
            let added = bool_value(b, *added);
            tagged(b, "GraphInsert", vec![("node", node), ("added", added)])
        }
        GraphOp::Contains { node, present } => {
            let node = bytes_leaf(b, node.hash().as_bytes());
            let present = bool_value(b, *present);
            tagged(
                b,
                "GraphContains",
                vec![("node", node), ("present", present)],
            )
        }
        GraphOp::Remove { node, existed } => {
            let node = bytes_leaf(b, node.hash().as_bytes());
            let existed = bool_value(b, *existed);
            tagged(b, "GraphRemove", vec![("node", node), ("existed", existed)])
        }
        GraphOp::Link {
            from,
            to,
            kind,
            added,
        } => {
            let from = bytes_leaf(b, from.hash().as_bytes());
            let to = bytes_leaf(b, to.hash().as_bytes());
            let kind = edge_kind_leaf(b, kind);
            let added = bool_value(b, *added);
            tagged(
                b,
                "GraphLink",
                vec![("from", from), ("to", to), ("kind", kind), ("added", added)],
            )
        }
        GraphOp::SetEdges {
            from,
            kind,
            targets,
            prior,
        } => {
            let from = bytes_leaf(b, from.hash().as_bytes());
            let kind = edge_kind_leaf(b, kind);
            let targets = reducer_list(b, targets);
            let prior = reducer_list(b, prior);
            tagged(
                b,
                "GraphSetEdges",
                vec![
                    ("from", from),
                    ("kind", kind),
                    ("targets", targets),
                    ("prior", prior),
                ],
            )
        }
        GraphOp::Neighbors {
            node,
            kind,
            dir,
            result,
        } => {
            let node = bytes_leaf(b, node.hash().as_bytes());
            let kind = edge_kind_leaf(b, kind);
            let dir = str_leaf(b, dir_str(*dir));
            let result = reducer_list(b, result);
            tagged(
                b,
                "GraphNeighbors",
                vec![
                    ("node", node),
                    ("kind", kind),
                    ("dir", dir),
                    ("result", result),
                ],
            )
        }
        GraphOp::InKinds { node, result } => {
            let node = bytes_leaf(b, node.hash().as_bytes());
            let result = edge_kind_list(b, result);
            tagged(b, "GraphInKinds", vec![("node", node), ("result", result)])
        }
        GraphOp::Reach {
            node,
            kind,
            dir,
            result,
        } => {
            let node = bytes_leaf(b, node.hash().as_bytes());
            let kind = edge_kind_leaf(b, kind);
            let dir = str_leaf(b, dir_str(*dir));
            let result = reducer_list(b, result);
            tagged(
                b,
                "GraphReach",
                vec![
                    ("node", node),
                    ("kind", kind),
                    ("dir", dir),
                    ("result", result),
                ],
            )
        }
    }
}

fn read_graph(arenas: &Arenas, id: StructId, tag: &str) -> Option<GraphOp> {
    Some(match tag {
        "GraphInsert" => GraphOp::Insert {
            node: read_reducer(arenas, field(arenas, id, "node")?)?,
            added: read_bool(arenas, field(arenas, id, "added")?)?,
        },
        "GraphContains" => GraphOp::Contains {
            node: read_reducer(arenas, field(arenas, id, "node")?)?,
            present: read_bool(arenas, field(arenas, id, "present")?)?,
        },
        "GraphRemove" => GraphOp::Remove {
            node: read_reducer(arenas, field(arenas, id, "node")?)?,
            existed: read_bool(arenas, field(arenas, id, "existed")?)?,
        },
        "GraphLink" => GraphOp::Link {
            from: read_reducer(arenas, field(arenas, id, "from")?)?,
            to: read_reducer(arenas, field(arenas, id, "to")?)?,
            kind: read_edge_kind(arenas, field(arenas, id, "kind")?)?,
            added: read_bool(arenas, field(arenas, id, "added")?)?,
        },
        "GraphSetEdges" => GraphOp::SetEdges {
            from: read_reducer(arenas, field(arenas, id, "from")?)?,
            kind: read_edge_kind(arenas, field(arenas, id, "kind")?)?,
            targets: read_reducer_list(arenas, field(arenas, id, "targets")?)?,
            prior: read_reducer_list(arenas, field(arenas, id, "prior")?)?,
        },
        "GraphNeighbors" => GraphOp::Neighbors {
            node: read_reducer(arenas, field(arenas, id, "node")?)?,
            kind: read_edge_kind(arenas, field(arenas, id, "kind")?)?,
            dir: read_dir(str_at(arenas, field(arenas, id, "dir")?)?)?,
            result: read_reducer_list(arenas, field(arenas, id, "result")?)?,
        },
        "GraphInKinds" => GraphOp::InKinds {
            node: read_reducer(arenas, field(arenas, id, "node")?)?,
            result: read_edge_kind_list(arenas, field(arenas, id, "result")?)?,
        },
        "GraphReach" => GraphOp::Reach {
            node: read_reducer(arenas, field(arenas, id, "node")?)?,
            kind: read_edge_kind(arenas, field(arenas, id, "kind")?)?,
            dir: read_dir(str_at(arenas, field(arenas, id, "dir")?)?)?,
            result: read_reducer_list(arenas, field(arenas, id, "result")?)?,
        },
        _ => return None,
    })
}

fn event_kind_str(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Message => "message",
        EventKind::Response => "response",
        EventKind::Notification => "notification",
    }
}

fn read_event_kind(s: &str) -> Option<EventKind> {
    Some(match s {
        "message" => EventKind::Message,
        "response" => EventKind::Response,
        "notification" => EventKind::Notification,
        _ => return None,
    })
}

fn error_str(e: Error) -> &'static str {
    match e {
        Error::Timeout => "timeout",
        Error::MissingHandler => "missing-handler",
        Error::SchemaViolation => "schema-violation",
        Error::Faulted => "faulted",
    }
}

fn read_error(s: &str) -> Option<Error> {
    Some(match s {
        "timeout" => Error::Timeout,
        "missing-handler" => Error::MissingHandler,
        "schema-violation" => Error::SchemaViolation,
        "faulted" => Error::Faulted,
        _ => return None,
    })
}

// --- spawn ---

fn spawn_value(b: &mut Builder, info: &SpawnInfo) -> StructId {
    let name = str_leaf(b, info.name.as_str());
    let program = bytes_leaf(b, info.program.hash().as_bytes());
    let parent = bytes_leaf(b, info.parent.hash().as_bytes());
    let kind = str_leaf(b, reducer_kind_str(info.kind));
    tagged(
        b,
        "Spawn",
        vec![
            ("name", name),
            ("program", program),
            ("parent", parent),
            ("reducerKind", kind),
        ],
    )
}

fn read_spawn(arenas: &Arenas, id: StructId) -> Option<SpawnInfo> {
    Some(SpawnInfo {
        name: Str::from(str_at(arenas, field(arenas, id, "name")?)?),
        program: read_program(arenas, field(arenas, id, "program")?)?,
        parent: read_reducer(arenas, field(arenas, id, "parent")?)?,
        kind: read_reducer_kind(str_at(arenas, field(arenas, id, "reducerKind")?)?)?,
    })
}

// --- host-call-rejected (the arg-parse-guard observation, §9) ---

/// A `HostCallRejected` as a record `{iface, op, rawArgs}` — the interface/op strings and the raw argument
/// byte leaves as received. `rawArgs` is a name-headed list of raw-bytes leaves, order-preserving.
fn rejected_value(b: &mut Builder, c: &RejectedCall) -> StructId {
    let iface = str_leaf(b, c.iface.as_str());
    let op = str_leaf(b, c.op.as_str());
    let items = c.raw_args.iter().map(|a| bytes_leaf(b, a)).collect();
    let raw_args = list_value(b, items);
    tagged(
        b,
        "HostCallRejected",
        vec![("iface", iface), ("op", op), ("rawArgs", raw_args)],
    )
}

fn read_rejected(arenas: &Arenas, id: StructId) -> Option<RejectedCall> {
    let raw_args = list_items(arenas, field(arenas, id, "rawArgs")?)?
        .iter()
        .map(|&a| read_bytes(arenas, a))
        .collect::<Option<Vec<_>>>()?;
    Some(RejectedCall {
        iface: Str::from(str_at(arenas, field(arenas, id, "iface")?)?),
        op: Str::from(str_at(arenas, field(arenas, id, "op")?)?),
        raw_args,
    })
}

/// A `Run` as a record `{program, contract, input, output, error}` — the program/contract id hash bytes and
/// the input bytes, plus the outcome as two ALWAYS-PRESENT Option fields (`output` the returned bytes on
/// success, `error` the failure-category string on failure), so the record has one fixed shape a Cadenza
/// checker `Value.decode`s (§9), mirroring a delivered entry's `from`/`error`.
fn run_value(b: &mut Builder, r: &RunCall) -> StructId {
    let program = bytes_leaf(b, &r.program);
    let contract = bytes_leaf(b, &r.contract);
    let input = bytes_leaf(b, &r.input);
    let output_inner = r.output.as_ref().map(|o| bytes_leaf(b, o));
    let output = option_value(b, output_inner);
    let error_inner = r.error.as_ref().map(|e| str_leaf(b, e.as_str()));
    let error = option_value(b, error_inner);
    tagged(
        b,
        "Run",
        vec![
            ("program", program),
            ("contract", contract),
            ("input", input),
            ("output", output),
            ("error", error),
        ],
    )
}

fn read_run(arenas: &Arenas, id: StructId) -> Option<RunCall> {
    Some(RunCall {
        program: read_bytes(arenas, field(arenas, id, "program")?)?,
        contract: read_bytes(arenas, field(arenas, id, "contract")?)?,
        input: read_bytes(arenas, field(arenas, id, "input")?)?,
        output: read_option(arenas, field(arenas, id, "output")?, read_bytes)?,
        error: read_option(arenas, field(arenas, id, "error")?, |a, i| {
            str_at(a, i).map(Str::from)
        })?,
    })
}

/// An `ArgProbe` as a record `{record, items}` — the two received `probe` arguments each already in their
/// canonical Cadenza value encoding, carried verbatim as opaque bytes (the checker `Value.decode`s the log and
/// asserts these against its own `Value.encode` of the expected argument literals).
fn arg_probe_value(b: &mut Builder, c: &ArgProbeCall) -> StructId {
    let record = bytes_leaf(b, &c.record);
    let items = bytes_leaf(b, &c.items);
    tagged(b, "ArgProbe", vec![("record", record), ("items", items)])
}

fn read_arg_probe(arenas: &Arenas, id: StructId) -> Option<ArgProbeCall> {
    Some(ArgProbeCall {
        record: read_bytes(arenas, field(arenas, id, "record")?)?,
        items: read_bytes(arenas, field(arenas, id, "items")?)?,
    })
}

fn reducer_kind_str(kind: ReducerKind) -> &'static str {
    match kind {
        ReducerKind::Ordinary => "ordinary",
        ReducerKind::Event => "event",
        // A pure `run` reducer bypasses the System, so it is never spawned into an observation log; the arm
        // exists for exhaustiveness and round-trips with `read_reducer_kind`.
        ReducerKind::Pure => "pure",
    }
}

fn read_reducer_kind(s: &str) -> Option<ReducerKind> {
    Some(match s {
        "ordinary" => ReducerKind::Ordinary,
        "event" => ReducerKind::Event,
        "pure" => ReducerKind::Pure,
        _ => return None,
    })
}

// --- primitive helpers ---

/// An `Entry` sum value `(<tag> (record (= <field> <value>)…))` — the bare constructor named by `tag`
/// wrapping a record of the tag-specific fields. Encoding the entry as a CADENZA SUM (not a tag-discriminated
/// record) is what lets a Cadenza checker `Value.decode` the whole heterogeneous log into a `List(Entry)`: a
/// value-form sum decodes into a Cadenza sum by constructor name (kebab-matched), whereas a record with a
/// `tag` field plus per-tag fields has no single fixed record type. The native `deserialize` round-trips it
/// via [`entry_ctor`].
fn tagged(b: &mut Builder, tag: &str, fields: Vec<(&'static str, StructId)>) -> StructId {
    let rec = record(b, fields.into_iter().map(|(n, v)| (n as &str, v)).collect());
    qctor(b, "Entry", tag, vec![rec])
}

/// Read an `Entry` sum value back: its constructor name (the former tag) and the payload record the
/// tag-specific readers pull their fields from. `None` if `id` is not a `(<ctor> <record>)` value. The
/// constructor is the canonical BARE-name head (the sum's type is fixed by the enclosing record's `entry`
/// field, so the value carries no `(. Entry ctor)` member node).
fn entry_ctor(arenas: &Arenas, id: StructId) -> Option<(&str, StructId)> {
    let Struct::List(items) = arenas.get(id) else {
        return None;
    };
    let (&head, tail) = items.split_first()?;
    let ctor = arenas.as_name(head)?;
    let [inner] = <[StructId; 1]>::try_from(tail).ok()?;
    Some((ctor, inner))
}

/// Encode an `Option<T>` as its canonical Cadenza value — `(Some <v>)`, or the nullary `(None unit)` — the
/// form R2 `Value.encode` produces. Used for a delivered entry's `from`/`error` so they are ALWAYS-PRESENT
/// fields of one fixed record shape (a Cadenza checker `Value.decode`s them into an `Option`), never fields
/// that appear only for some deliveries.
fn option_value(b: &mut Builder, inner: Option<StructId>) -> StructId {
    match inner {
        Some(v) => bare_ctor(b, "Some", vec![v]),
        None => {
            let unit = b.name("unit");
            bare_ctor(b, "None", vec![unit])
        }
    }
}

/// Read an `Option<T>` value `(Some <v>)` / `(None unit)`, applying `read_inner` to the `Some` payload.
/// `None` (the outer `Option`, i.e. a rejected log) if `id` is neither constructor.
fn read_option<T>(
    arenas: &Arenas,
    id: StructId,
    read_inner: impl FnOnce(&Arenas, StructId) -> Option<T>,
) -> Option<Option<T>> {
    let Struct::List(items) = arenas.get(id) else {
        return None;
    };
    let (&head, tail) = items.split_first()?;
    match arenas.as_name(head)? {
        "Some" => {
            let [v] = <[StructId; 1]>::try_from(tail).ok()?;
            Some(Some(read_inner(arenas, v)?))
        }
        // The nullary `(None unit)` — the `unit` payload is a placeholder, ignored.
        "None" => Some(None),
        _ => None,
    }
}

fn str_leaf(b: &mut Builder, text: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(text)))
}

fn list_value(b: &mut Builder, items: Vec<StructId>) -> StructId {
    // The NAME-headed `(list …)` — the canonical Cadenza list form a guest `Value.decode`s (a string-headed
    // `("list" …)` is a surface literal the value decoder rejects).
    let head = b.name("list");
    b.list(std::iter::once(head).chain(items).collect())
}

/// A boolean as the canonical Cadenza `Bool` value — a `Leaf::Bool` atom (the codec's payloadless
/// `KIND_BOOL_TRUE`/`KIND_BOOL_FALSE` tag), so a checker's field is a real `Bool`, not an integer `0`/`1`. This
/// mirrors [`checker_protocol::encode_verdict`], which already writes the verdict's `pass` as `Leaf::Bool`.
fn bool_value(b: &mut Builder, v: bool) -> StructId {
    b.atom_leaf(Leaf::Bool(v))
}

fn list_items(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    // All three list spellings incl. the M2 native ctor-leaf head (rcdzc-compiled log values).
    arenas.compound_form_of(id, CompoundCtor::List)
}

fn field(arenas: &Arenas, record: StructId, name: &str) -> Option<StructId> {
    record_field(arenas, record, name)
}

fn str_at(arenas: &Arenas, id: StructId) -> Option<&str> {
    arenas.as_str(id)
}

fn read_bool(arenas: &Arenas, id: StructId) -> Option<bool> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bool(v) => Some(*v),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

fn read_contract(arenas: &Arenas, id: StructId) -> Option<ContractId> {
    ContractId::try_from(read_bytes(arenas, id)?.as_ref()).ok()
}

fn read_reducer(arenas: &Arenas, id: StructId) -> Option<ReducerId> {
    ReducerId::try_from(read_bytes(arenas, id)?.as_ref()).ok()
}

fn read_host(arenas: &Arenas, id: StructId) -> Option<HostId> {
    HostId::try_from(read_bytes(arenas, id)?.as_ref()).ok()
}

fn read_program(arenas: &Arenas, id: StructId) -> Option<ProgramHash> {
    ProgramHash::try_from(read_bytes(arenas, id)?.as_ref()).ok()
}

fn read_hash(arenas: &Arenas, id: StructId) -> Option<Hash> {
    Hash::try_from(read_bytes(arenas, id)?.as_ref()).ok()
}

#[cfg(test)]
mod tests {
    use super::{deserialize, serialize};
    use crate::testing::observation::{
        ArgProbeCall, BlobOp, Entry, EventKind, EventOp, GraphOp, KvOp, ProvOp, Record,
        RejectedCall, RunCall, SpawnInfo,
    };
    use crate::{
        Bytes, ContractId, Dir, EdgeKind, Error, Hash, HashTag, HostId, Origin, ProgramHash,
        ReducerId, ReducerKind, Str,
    };
    use std::ops::Bound;

    fn origin(tag: &[u8]) -> Origin {
        Origin {
            reducer: ReducerId::of(tag),
            host: HostId::of(b"node"),
        }
    }

    fn rec(seq: u64, entry: Entry) -> Record {
        Record {
            seq,
            time_ns: seq * 1000,
            source: origin(b"r"),
            entry,
        }
    }

    /// A log holding at least one of every entry variant — serialized and decoded back equal, so the
    /// round-trip pins the full-fidelity encoding for every shape a checker may read.
    #[test]
    fn every_entry_variant_round_trips() {
        let records = vec![
            rec(
                0,
                Entry::Spawn(SpawnInfo {
                    name: Str::from("agent"),
                    program: ProgramHash::of(b"prog"),
                    parent: ReducerId::of(b"agent"),
                    kind: ReducerKind::Event,
                }),
            ),
            rec(
                1,
                Entry::Kv(KvOp::Get {
                    key: Bytes::from_static(b"k"),
                    hit: true,
                }),
            ),
            rec(
                2,
                Entry::Kv(KvOp::Put {
                    key: Bytes::from_static(b"k"),
                    value: Bytes::from_static(b"v"),
                }),
            ),
            rec(
                3,
                Entry::Kv(KvOp::Delete {
                    key: Bytes::from_static(b"k"),
                    existed: false,
                }),
            ),
            rec(
                4,
                Entry::Kv(KvOp::Scan {
                    lower: Bound::Included(Bytes::from_static(b"a")),
                    upper: Bound::Excluded(Bytes::from_static(b"z")),
                    keys_only: true,
                }),
            ),
            rec(
                5,
                Entry::Kv(KvOp::Scan {
                    lower: Bound::Unbounded,
                    upper: Bound::Unbounded,
                    keys_only: false,
                }),
            ),
            rec(
                6,
                Entry::Blob(BlobOp::Put {
                    hash: Hash::of(HashTag::Blob, b"bytes"),
                    len: 5,
                }),
            ),
            rec(
                7,
                Entry::Blob(BlobOp::Get {
                    hash: Hash::of(HashTag::Blob, b"bytes"),
                    hit: false,
                }),
            ),
            rec(
                8,
                Entry::Blob(BlobOp::Has {
                    hash: Hash::of(HashTag::Blob, b"bytes"),
                    present: true,
                }),
            ),
            rec(
                9,
                Entry::Event(EventOp::Delivered {
                    kind: EventKind::Message,
                    contract: ContractId::of(b"c"),
                    from: Some(origin(b"caller")),
                    continuation_token: Bytes::from_static(b"tok"),
                    payload: Bytes::from_static(b"p"),
                    error: None,
                }),
            ),
            rec(
                10,
                Entry::Event(EventOp::Delivered {
                    kind: EventKind::Response,
                    contract: ContractId::of(b"c"),
                    from: None,
                    continuation_token: Bytes::from_static(b"tok"),
                    payload: Bytes::new(),
                    error: Some(Error::Timeout),
                }),
            ),
            rec(
                11,
                Entry::Event(EventOp::Delivered {
                    kind: EventKind::Notification,
                    contract: ContractId::of(b"c"),
                    from: None,
                    continuation_token: Bytes::new(),
                    payload: Bytes::from_static(b"n"),
                    error: Some(Error::MissingHandler),
                }),
            ),
            rec(
                12,
                Entry::Event(EventOp::Emitted {
                    contract: ContractId::of(b"http.get"),
                    payload: Bytes::from_static(b"url"),
                    continuation_token: Bytes::from_static(b"c1"),
                    has_deadline: true,
                }),
            ),
            rec(
                13,
                Entry::Event(EventOp::Closed {
                    schema: ContractId::of(b"done"),
                    reason: Bytes::from_static(b"ok"),
                }),
            ),
            rec(
                14,
                Entry::Event(EventOp::Failed {
                    during: EventKind::Message,
                    contract: ContractId::of(b"go"),
                    reason: Str::from("boom"),
                }),
            ),
            rec(
                15,
                Entry::Event(EventOp::Routed {
                    kind: EventKind::Message,
                    target: ReducerId::of(b"handler"),
                    contract: ContractId::of(b"c"),
                    continuation_token: Bytes::from_static(b"tok"),
                    payload: Bytes::from_static(b"p"),
                    error: None,
                }),
            ),
            rec(
                16,
                Entry::Event(EventOp::Routed {
                    kind: EventKind::Response,
                    target: ReducerId::of(b"caller"),
                    contract: ContractId::of(b"c"),
                    continuation_token: Bytes::from_static(b"tok"),
                    payload: Bytes::from_static(b"200"),
                    error: None,
                }),
            ),
            rec(
                17,
                Entry::Event(EventOp::Routed {
                    kind: EventKind::Response,
                    target: ReducerId::of(b"caller"),
                    contract: ContractId::of(b"c"),
                    continuation_token: Bytes::from_static(b"tok"),
                    payload: Bytes::new(),
                    error: Some(Error::Timeout),
                }),
            ),
            rec(
                18,
                Entry::Event(EventOp::Routed {
                    kind: EventKind::Notification,
                    target: ReducerId::of(b"watcher"),
                    contract: ContractId::of(b"lifecycle.spawned"),
                    continuation_token: Bytes::new(),
                    payload: Bytes::from_static(b"n"),
                    error: None,
                }),
            ),
            rec(
                19,
                Entry::Provenance(ProvOp::ProgramOf {
                    reducer: ReducerId::of(b"peer"),
                    program: Some(ProgramHash::of(b"peer-prog")),
                }),
            ),
            rec(
                20,
                Entry::Provenance(ProvOp::ProgramOf {
                    reducer: ReducerId::of(b"gone"),
                    program: None,
                }),
            ),
            rec(
                21,
                Entry::Graph(GraphOp::Insert {
                    node: ReducerId::of(b"n"),
                    added: true,
                }),
            ),
            rec(
                22,
                Entry::Graph(GraphOp::Contains {
                    node: ReducerId::of(b"n"),
                    present: false,
                }),
            ),
            rec(
                23,
                Entry::Graph(GraphOp::Remove {
                    node: ReducerId::of(b"n"),
                    existed: true,
                }),
            ),
            rec(
                24,
                Entry::Graph(GraphOp::Link {
                    from: ReducerId::of(b"a"),
                    to: ReducerId::of(b"b"),
                    kind: EdgeKind::spawn(),
                    added: true,
                }),
            ),
            rec(
                25,
                Entry::Graph(GraphOp::SetEdges {
                    from: ReducerId::of(b"owner"),
                    kind: EdgeKind::for_contract(ContractId::of(b"http.get")),
                    targets: vec![ReducerId::of(b"h1"), ReducerId::of(b"h2")],
                    prior: vec![],
                }),
            ),
            rec(
                26,
                Entry::Graph(GraphOp::Neighbors {
                    node: ReducerId::of(b"owner"),
                    kind: EdgeKind::for_contract(ContractId::of(b"http.get")),
                    dir: Dir::Out,
                    result: vec![ReducerId::of(b"h1"), ReducerId::of(b"h2")],
                }),
            ),
            rec(
                27,
                Entry::Graph(GraphOp::InKinds {
                    node: ReducerId::of(b"n"),
                    result: vec![EdgeKind::spawn(), EdgeKind::watch_exit()],
                }),
            ),
            rec(
                28,
                Entry::Graph(GraphOp::Reach {
                    node: ReducerId::of(b"leaf"),
                    kind: EdgeKind::spawn(),
                    dir: Dir::Out,
                    result: vec![ReducerId::of(b"parent"), ReducerId::of(b"root")],
                }),
            ),
            rec(
                18,
                Entry::HostCallRejected(RejectedCall {
                    iface: Str::from("graph"),
                    op: Str::from("neighbors"),
                    raw_args: vec![
                        Bytes::from_static(b"not-a-reducer-id"),
                        Bytes::from_static(b"kind"),
                    ],
                }),
            ),
            rec(
                19,
                Entry::Run(RunCall {
                    program: Bytes::from_static(b"sub-program-hash"),
                    contract: Bytes::from_static(b"contract-hash"),
                    input: Bytes::from_static(b"the-input"),
                    output: Some(Bytes::from_static(b"the-output")),
                    error: None,
                }),
            ),
            rec(
                20,
                Entry::Run(RunCall {
                    program: Bytes::from_static(b"missing-program"),
                    contract: Bytes::from_static(b"contract-hash"),
                    input: Bytes::from_static(b"the-input"),
                    output: None,
                    error: Some(Str::from("missing-handler")),
                }),
            ),
            rec(
                21,
                Entry::ArgProbe(ArgProbeCall {
                    record: Bytes::from_static(b"encoded-probe-record"),
                    items: Bytes::from_static(b"encoded-list-of-narrow"),
                }),
            ),
        ];
        let bytes = serialize(&records);
        assert_eq!(deserialize(&bytes), Some(records));
    }

    #[test]
    fn an_empty_log_round_trips() {
        assert_eq!(deserialize(&serialize(&[])), Some(Vec::new()));
    }

    #[test]
    fn undecodable_bytes_are_rejected() {
        assert_eq!(deserialize(b"not an ast"), None);
    }

    #[test]
    fn a_full_fidelity_id_survives_where_the_render_would_elide_it() {
        // The point of the structured log: a contract-id and a token survive byte-exact, so a checker can
        // assert `token == <a reducer id>` — which the lossy human render cannot express.
        let token = ReducerId::of(b"reducer-echo").hash().as_bytes().to_vec();
        let records = vec![rec(
            0,
            Entry::Event(EventOp::Emitted {
                contract: ContractId::of(b"echo"),
                payload: Bytes::from_static(b"hello"),
                continuation_token: Bytes::copy_from_slice(&token),
                has_deadline: false,
            }),
        )];
        let back = deserialize(&serialize(&records)).expect("round-trips");
        let Entry::Event(EventOp::Emitted {
            contract,
            continuation_token,
            ..
        }) = &back[0].entry
        else {
            panic!("expected an emitted event");
        };
        assert_eq!(*contract, ContractId::of(b"echo"));
        assert_eq!(continuation_token.as_ref(), token.as_slice());
    }

    #[test]
    fn a_delivered_entry_always_carries_its_from_and_error_option_fields() {
        // The fixed-shape contract: `from`/`error` are ALWAYS-PRESENT Option fields, so the delivered record
        // has one shape a Cadenza checker `Value.decode`s into a single record type — even when both are
        // absent. Round-trip alone would not catch a regression to conditionally-omitted fields; this pins
        // that the fields (and the nullary `(None unit)` value) are always emitted.
        let log = vec![rec(
            0,
            Entry::Event(EventOp::Delivered {
                kind: EventKind::Message,
                contract: ContractId::of(b"c"),
                from: None,
                continuation_token: Bytes::new(),
                payload: Bytes::new(),
                error: None,
            }),
        )];
        let bytes = serialize(&log);
        let arenas = cadenza_ast::codec::decode(&bytes).expect("a decodable log");
        let root = super::as_ascribed(&arenas, arenas.root).expect("the log root is ascribed");
        let items = super::list_items(&arenas, root).expect("the log is a list");
        let record =
            super::as_ascribed(&arenas, items[0]).expect("each LogRecord element is ascribed");
        let entry = super::field(&arenas, record, "entry").expect("the record has an entry");
        let (ctor, inner) = super::entry_ctor(&arenas, entry).expect("the entry is an Entry sum");
        assert_eq!(ctor, "Delivered");
        assert!(
            super::field(&arenas, inner, "from").is_some(),
            "`from` is present even when None"
        );
        assert!(
            super::field(&arenas, inner, "error").is_some(),
            "`error` is present even when None"
        );
    }

    #[test]
    fn a_scan_bound_is_a_cadenza_sum_not_a_tagged_record() {
        // Bound is encoded as a sum `(<ctor> <key>?)` — a fixed per-constructor shape a Cadenza checker
        // `Value.decode`s — not a `bound`-tagged record whose `key` varies. Round-trip alone would
        // pass for either encoding, so pin the sum form: the `lower` value is a Bound sum (decodes back) and
        // carries no `bound` record field.
        let log = vec![rec(
            0,
            Entry::Kv(KvOp::Scan {
                lower: Bound::Included(Bytes::from_static(b"a")),
                upper: Bound::Unbounded,
                keys_only: false,
            }),
        )];
        let bytes = serialize(&log);
        let arenas = cadenza_ast::codec::decode(&bytes).expect("a decodable log");
        let root = super::as_ascribed(&arenas, arenas.root).expect("the log root is ascribed");
        let items = super::list_items(&arenas, root).expect("the log is a list");
        let record =
            super::as_ascribed(&arenas, items[0]).expect("each LogRecord element is ascribed");
        let entry = super::field(&arenas, record, "entry").expect("the record has an entry");
        let (ctor, inner) = super::entry_ctor(&arenas, entry).expect("the entry is an Entry sum");
        assert_eq!(ctor, "KvScan");
        let lower = super::field(&arenas, inner, "lower").expect("the scan has a lower bound");
        assert!(
            super::field(&arenas, lower, "bound").is_none(),
            "a Bound is a sum, not a `bound`-tagged record"
        );
        assert_eq!(
            super::read_bound(&arenas, lower),
            Some(Bound::Included(Bytes::from_static(b"a")))
        );
    }
}
