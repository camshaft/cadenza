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
//! The log is a list of record values; each record is a record with the three questions it answers plus its
//! entry, and an entry is a `tag`-discriminated record:
//! ```text
//! ("list" <record>…)
//! <record>  = ("record" (= seq <u>) (= time <u>) (= source <origin>) (= entry <entry>))
//! <origin>  = ("record" (= reducer b"…") (= host b"…"))
//! <entry>   = ("record" (= tag "kv-put") (= key b"…") (= value b"…"))            ; and one shape per tag
//! ```
//! Every id/hash crosses as its raw bytes, every payload/key as opaque bytes, every flag as `0`/`1`, so no
//! fidelity is lost. Decoding is total: any malformation is a rejected log ([`deserialize`] returns `None`),
//! never a panic.

use super::observation::{BlobOp, Entry, EventKind, EventOp, KvOp, Record, SpawnInfo};
use crate::contract_value::{bytes_leaf, read_bytes, read_uint, record, record_field, uint_leaf};
use crate::{
    Bytes, ContractId, Error, Hash, HostId, Origin, ProgramHash, ReducerId, ReducerKind, Str,
};
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
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
    let root = list_value(&mut b, items);
    codec::encode(&b.finish(root))
}

/// Decode an observation log from a Cadenza binary AST. Total: any input that is not a well-formed log value
/// is a rejected log (`None`), never a panic — the same discipline as every value reader in the crate.
#[must_use]
pub fn deserialize(bytes: &[u8]) -> Option<Vec<Record>> {
    let arenas = codec::decode(bytes)?;
    let items = list_items(&arenas, arenas.root)?;
    items.iter().map(|&r| read_record(&arenas, r)).collect()
}

// --- record + envelope ---

fn record_value(b: &mut Builder, r: &Record) -> StructId {
    let seq = uint_leaf(b, r.seq);
    let time = uint_leaf(b, r.time_ns);
    let source = origin_value(b, &r.source);
    let entry = entry_value(b, &r.entry);
    record(
        b,
        vec![
            ("seq", seq),
            ("time", time),
            ("source", source),
            ("entry", entry),
        ],
    )
}

fn read_record(arenas: &Arenas, id: StructId) -> Option<Record> {
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
        Entry::Spawn(info) => spawn_value(b, info),
    }
}

fn read_entry(arenas: &Arenas, id: StructId) -> Option<Entry> {
    let tag = str_at(arenas, field(arenas, id, "tag")?)?;
    Some(match tag {
        "kv-get" | "kv-put" | "kv-delete" | "kv-scan" => Entry::Kv(read_kv(arenas, id, tag)?),
        "blob-put" | "blob-get" | "blob-has" => Entry::Blob(read_blob(arenas, id, tag)?),
        "delivered" | "emitted" | "closed" | "failed" => Entry::Event(read_event(arenas, id, tag)?),
        "spawn" => Entry::Spawn(read_spawn(arenas, id)?),
        _ => return None,
    })
}

// --- kv ---

fn kv_value(b: &mut Builder, op: &KvOp) -> StructId {
    match op {
        KvOp::Get { key, hit } => {
            let key = bytes_leaf(b, key);
            let hit = bool_value(b, *hit);
            tagged(b, "kv-get", vec![("key", key), ("hit", hit)])
        }
        KvOp::Put { key, value } => {
            let key = bytes_leaf(b, key);
            let value = bytes_leaf(b, value);
            tagged(b, "kv-put", vec![("key", key), ("value", value)])
        }
        KvOp::Delete { key, existed } => {
            let key = bytes_leaf(b, key);
            let existed = bool_value(b, *existed);
            tagged(b, "kv-delete", vec![("key", key), ("existed", existed)])
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
                "kv-scan",
                vec![("lower", lower), ("upper", upper), ("keys-only", keys_only)],
            )
        }
    }
}

fn read_kv(arenas: &Arenas, id: StructId, tag: &str) -> Option<KvOp> {
    Some(match tag {
        "kv-get" => KvOp::Get {
            key: read_bytes(arenas, field(arenas, id, "key")?)?,
            hit: read_bool(arenas, field(arenas, id, "hit")?)?,
        },
        "kv-put" => KvOp::Put {
            key: read_bytes(arenas, field(arenas, id, "key")?)?,
            value: read_bytes(arenas, field(arenas, id, "value")?)?,
        },
        "kv-delete" => KvOp::Delete {
            key: read_bytes(arenas, field(arenas, id, "key")?)?,
            existed: read_bool(arenas, field(arenas, id, "existed")?)?,
        },
        "kv-scan" => KvOp::Scan {
            lower: read_bound(arenas, field(arenas, id, "lower")?)?,
            upper: read_bound(arenas, field(arenas, id, "upper")?)?,
            keys_only: read_bool(arenas, field(arenas, id, "keys-only")?)?,
        },
        _ => return None,
    })
}

fn bound_value(b: &mut Builder, bound: &Bound<Bytes>) -> StructId {
    match bound {
        Bound::Unbounded => {
            let kind = str_leaf(b, "unbounded");
            record(b, vec![("bound", kind)])
        }
        Bound::Included(key) => {
            let kind = str_leaf(b, "included");
            let key = bytes_leaf(b, key);
            record(b, vec![("bound", kind), ("key", key)])
        }
        Bound::Excluded(key) => {
            let kind = str_leaf(b, "excluded");
            let key = bytes_leaf(b, key);
            record(b, vec![("bound", kind), ("key", key)])
        }
    }
}

fn read_bound(arenas: &Arenas, id: StructId) -> Option<Bound<Bytes>> {
    let kind = str_at(arenas, field(arenas, id, "bound")?)?;
    Some(match kind {
        "unbounded" => Bound::Unbounded,
        "included" => Bound::Included(read_bytes(arenas, field(arenas, id, "key")?)?),
        "excluded" => Bound::Excluded(read_bytes(arenas, field(arenas, id, "key")?)?),
        _ => return None,
    })
}

// --- blob ---

fn blob_value(b: &mut Builder, op: &BlobOp) -> StructId {
    match op {
        BlobOp::Put { hash, len } => {
            let hash = bytes_leaf(b, hash.as_bytes());
            let len = uint_leaf(b, *len as u64);
            tagged(b, "blob-put", vec![("hash", hash), ("len", len)])
        }
        BlobOp::Get { hash, hit } => {
            let hash = bytes_leaf(b, hash.as_bytes());
            let hit = bool_value(b, *hit);
            tagged(b, "blob-get", vec![("hash", hash), ("hit", hit)])
        }
        BlobOp::Has { hash, present } => {
            let hash = bytes_leaf(b, hash.as_bytes());
            let present = bool_value(b, *present);
            tagged(b, "blob-has", vec![("hash", hash), ("present", present)])
        }
    }
}

fn read_blob(arenas: &Arenas, id: StructId, tag: &str) -> Option<BlobOp> {
    let hash = read_hash(arenas, field(arenas, id, "hash")?)?;
    Some(match tag {
        "blob-put" => BlobOp::Put {
            hash,
            len: usize::try_from(read_uint(arenas, field(arenas, id, "len")?)?).ok()?,
        },
        "blob-get" => BlobOp::Get {
            hash,
            hit: read_bool(arenas, field(arenas, id, "hit")?)?,
        },
        "blob-has" => BlobOp::Has {
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
            let mut fields = vec![
                ("event-kind", event_kind),
                ("contract", contract),
                ("token", token),
                ("payload", payload),
            ];
            if let Some(o) = from {
                let o = origin_value(b, o);
                fields.push(("from", o));
            }
            if let Some(e) = error {
                let e = str_leaf(b, error_str(*e));
                fields.push(("error", e));
            }
            tagged(b, "delivered", fields)
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
            let deadline = bool_value(b, *has_deadline);
            tagged(
                b,
                "emitted",
                vec![
                    ("contract", contract),
                    ("payload", payload),
                    ("token", token),
                    ("deadline", deadline),
                ],
            )
        }
        EventOp::Closed { schema, reason } => {
            let schema = bytes_leaf(b, schema.hash().as_bytes());
            let reason = bytes_leaf(b, reason);
            tagged(b, "closed", vec![("schema", schema), ("reason", reason)])
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
                "failed",
                vec![
                    ("during", during),
                    ("contract", contract),
                    ("reason", reason),
                ],
            )
        }
    }
}

fn read_event(arenas: &Arenas, id: StructId, tag: &str) -> Option<EventOp> {
    Some(match tag {
        "delivered" => EventOp::Delivered {
            kind: read_event_kind(str_at(arenas, field(arenas, id, "event-kind")?)?)?,
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            from: match field(arenas, id, "from") {
                None => None,
                Some(f) => Some(read_origin(arenas, f)?),
            },
            continuation_token: read_bytes(arenas, field(arenas, id, "token")?)?,
            payload: read_bytes(arenas, field(arenas, id, "payload")?)?,
            error: match field(arenas, id, "error") {
                None => None,
                Some(e) => Some(read_error(str_at(arenas, e)?)?),
            },
        },
        "emitted" => EventOp::Emitted {
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            payload: read_bytes(arenas, field(arenas, id, "payload")?)?,
            continuation_token: read_bytes(arenas, field(arenas, id, "token")?)?,
            has_deadline: read_bool(arenas, field(arenas, id, "deadline")?)?,
        },
        "closed" => EventOp::Closed {
            schema: read_contract(arenas, field(arenas, id, "schema")?)?,
            reason: read_bytes(arenas, field(arenas, id, "reason")?)?,
        },
        "failed" => EventOp::Failed {
            during: read_event_kind(str_at(arenas, field(arenas, id, "during")?)?)?,
            contract: read_contract(arenas, field(arenas, id, "contract")?)?,
            reason: Str::from(str_at(arenas, field(arenas, id, "reason")?)?),
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
    }
}

fn read_error(s: &str) -> Option<Error> {
    Some(match s {
        "timeout" => Error::Timeout,
        "missing-handler" => Error::MissingHandler,
        "schema-violation" => Error::SchemaViolation,
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
        "spawn",
        vec![
            ("name", name),
            ("program", program),
            ("parent", parent),
            ("reducer-kind", kind),
        ],
    )
}

fn read_spawn(arenas: &Arenas, id: StructId) -> Option<SpawnInfo> {
    Some(SpawnInfo {
        name: Str::from(str_at(arenas, field(arenas, id, "name")?)?),
        program: read_program(arenas, field(arenas, id, "program")?)?,
        parent: read_reducer(arenas, field(arenas, id, "parent")?)?,
        kind: read_reducer_kind(str_at(arenas, field(arenas, id, "reducer-kind")?)?)?,
    })
}

fn reducer_kind_str(kind: ReducerKind) -> &'static str {
    match kind {
        ReducerKind::Ordinary => "ordinary",
        ReducerKind::Event => "event",
    }
}

fn read_reducer_kind(s: &str) -> Option<ReducerKind> {
    Some(match s {
        "ordinary" => ReducerKind::Ordinary,
        "event" => ReducerKind::Event,
        _ => return None,
    })
}

// --- primitive helpers ---

/// A `("record" (= tag <tag>) …fields)` — a tag-discriminated entry record.
fn tagged(b: &mut Builder, tag: &str, mut fields: Vec<(&'static str, StructId)>) -> StructId {
    let tag = str_leaf(b, tag);
    fields.insert(0, ("tag", tag));
    record(b, fields)
}

fn str_leaf(b: &mut Builder, text: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(text)))
}

fn list_value(b: &mut Builder, items: Vec<StructId>) -> StructId {
    let head = b.atom_leaf(Leaf::Str(Arc::from("list")));
    b.list(std::iter::once(head).chain(items).collect())
}

/// A boolean as `1` / `0` — the fidelity-preserving flag encoding.
fn bool_value(b: &mut Builder, v: bool) -> StructId {
    uint_leaf(b, u64::from(v))
}

fn list_items(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    arenas
        .as_ctor_form(id, "list")
        .or_else(|| arenas.as_form(id, "list"))
}

fn field(arenas: &Arenas, record: StructId, name: &str) -> Option<StructId> {
    record_field(arenas, record, name)
}

fn str_at(arenas: &Arenas, id: StructId) -> Option<&str> {
    arenas.as_str(id)
}

fn read_bool(arenas: &Arenas, id: StructId) -> Option<bool> {
    match read_uint(arenas, id)? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
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
    use crate::testing::observation::{BlobOp, Entry, EventKind, EventOp, KvOp, Record, SpawnInfo};
    use crate::{
        Bytes, ContractId, Error, Hash, HashTag, HostId, Origin, ProgramHash, ReducerId,
        ReducerKind, Str,
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
}
