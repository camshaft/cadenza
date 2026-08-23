//! Decode a whole harness run from a **Cadenza binary AST** (`design/cadenza-platform.md` §9).
//!
//! The integration-test executable's input is not an argv convention — it is a single Cadenza value that
//! *describes the entire run*: the program blobs, the tasks to spawn, and the system reducer. A value, not a
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
//!   (= system  "sys")                         ; the blob NAME of the system reducer (§4)
//!   (= run-for 3600000000000)                 ; optional; virtual-time horizon in NANOSECONDS (default 1h)
//!   (= blobs   ("list" <blob>…))              ; the program blobs, by name
//!   (= spawns  ("list" <spawn>…))             ; the tasks to spawn, in order
//!   (= deliver ("list" <delivery>…))          ; optional; the initial events to inject, in order
//!   (= checker "check")))                      ; optional; the blob name of the checker reducer (§9)
//! ```
//!
//! The `checker`, if present, names a program blob the harness runs over the completed observation log to
//! decide pass/fail: it is delivered the whole log and emits a verdict. The harness just executes it as a
//! wasm reducer — it knows nothing of how the checker was authored (a declarative set of checks compiled to
//! a Cadenza reducer, or hand-written); that transform is separate, upstream, and never seen here.
//!
//! A `<delivery>` names a `target` task and carries exactly one event to inject into it — a `message` (an
//! effect folded through `on_message`) or a `notification` (a control-plane event folded through
//! `on_notification`). Both carry a `contract` (a contract-id) and a `payload` (opaque bytes); a message also
//! takes an optional `token` (the caller's continuation token, default empty). A `contract` is written either
//! as its raw 33 tagged bytes, or as a **base64url** string (the §8 text form) — the string form is what a
//! name→id rewrite substitutes for a contract name (contract-name resolution is done outside the platform and
//! rewritten into the spec, so a resolved name arrives as its base64url id):
//! ```text
//! ("record" (= target "root") (= message ("record" (= contract b"…33 bytes") (= payload b"…") (= token b"…"))))
//! ("record" (= target "root") (= notification ("record" (= contract "AbC…base64url") (= payload b"…"))))
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
//!   (= kind   "event"))      ; optional — "ordinary" (default) or "event" (a privileged system reducer)
//! ```
//!
//! Both the list head `("list" …)` and the bare name head `(list …)` are accepted, as they denote the same
//! construct. Every field is read by name, so order is not load-bearing. A malformed description is a
//! [`SpecError`], never a panic.

use super::harness::{Harness, Parent, SpawnSpec};
use crate::contract_value::{bytes_leaf, read_bytes, read_uint, record, record_field, uint_leaf};
use crate::{
    Bytes, ContractId, Delivered, Hash, HostId, Message, Notification, Origin, ReducerId,
    ReducerKind,
};
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
use cadenza_ast::codec;
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

/// A whole harness run decoded from a Cadenza binary AST: the system reducer's blob name, the optional
/// virtual-time horizon, the program blobs, and the tasks to spawn. [`build`](HarnessSpec::build) turns it
/// into a runnable [`Harness`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessSpec {
    /// The blob name of the system reducer every effect routes to by default (§4).
    pub system: String,
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
}

/// An event the harness injects into a task's mailbox as a run's initial stimulus (§4). The schema's
/// delivery vocabulary — its own type rather than the platform's full [`Delivered`], so it names exactly
/// what a harness description can express: a [`Message`] or a [`Notification`]. (A `Response` — a reply to a
/// request the reducer itself made — is not a sensible *initial* stimulus, so it is not in the vocabulary.)
/// [`build`](HarnessSpec::build) turns each into a [`Delivered`], stamping a delivered message with the
/// harness's synthetic external [`Origin`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryEvent {
    /// An effect to fold through the target's `on_message`.
    Message {
        /// The contract-id of the effect.
        contract: ContractId,
        /// The effect's input value, opaque bytes.
        payload: Bytes,
        /// The caller's continuation token (the reducer's reply is correlated back by it); empty by default.
        token: Bytes,
    },
    /// A control-plane event to fold through the target's `on_notification`.
    Notification {
        /// The contract-id of the notification's schema.
        contract: ContractId,
        /// The notification value, opaque bytes.
        payload: Bytes,
    },
}

impl DeliveryEvent {
    /// Realize this as a platform [`Delivered`], stamping a message with the harness's synthetic external
    /// origin (an injected event has no real sending reducer).
    fn into_delivered(self) -> Delivered {
        match self {
            DeliveryEvent::Message {
                contract,
                payload,
                token,
            } => Delivered::Message(Message {
                id: contract,
                payload,
                from: external_origin(),
                continuation_token: token,
            }),
            DeliveryEvent::Notification { contract, payload } => {
                Delivered::Notification(Notification {
                    id: contract,
                    payload,
                })
            }
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
        }
    }
}

impl std::error::Error for SpecError {}

impl HarnessSpec {
    /// Decode a harness description from a Cadenza binary AST. Total: any malformation is a [`SpecError`].
    pub fn decode(bytes: &[u8]) -> Result<Self, SpecError> {
        let arenas = codec::decode(bytes).ok_or(SpecError::Undecodable)?;
        Self::read(&arenas, arenas.root)
    }

    /// Interpret an already-decoded AST node as a harness description — the shape-reading core, split out so
    /// it is testable against an in-memory `Arenas` without a full encode/decode round-trip.
    pub fn read(arenas: &Arenas, root: StructId) -> Result<Self, SpecError> {
        if arenas.head_ctor(root) != Some("record") {
            return Err(SpecError::NotARecord);
        }

        let system = str_field(arenas, root, "system")?
            .ok_or(SpecError::MissingField("system"))?
            .to_string();

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

        Ok(HarnessSpec {
            system,
            run_for,
            blobs,
            spawns,
            deliveries,
            checker,
        })
    }

    /// Turn the description into a runnable [`Harness`], resolving each `path`-sourced blob through
    /// `load_path` (an inline blob passes straight through, so `load_path` is only ever called for a path).
    /// The loader's error type `E` is threaded out unchanged — the binary passes a filesystem read; a test
    /// passing only inline blobs can use any `E` since the loader is never invoked.
    pub fn build<E>(
        self,
        mut load_path: impl FnMut(&str) -> Result<Bytes, E>,
    ) -> Result<Harness, E> {
        let mut harness = Harness::new(self.system);
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
        for (target, event) in self.deliveries {
            harness = harness.deliver(target, event.into_delivered());
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
        let system = str_leaf(b, &self.system);
        fields.push(("system", system));
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
        record(b, fields)
    }
}

/// A string leaf `"text"`.
fn str_leaf(b: &mut Builder, text: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(text)))
}

/// A `("list" e…)` value (string head, the canonical list constructor).
fn list_value(b: &mut Builder, items: Vec<StructId>) -> StructId {
    let head = b.atom_leaf(Leaf::Str(Arc::from("list")));
    b.list(std::iter::once(head).chain(items).collect())
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

/// The `("record" (= name …) (= blob …) (= parent …)? (= kind …)?)` node for one spawn — the inverse of
/// [`read_spawn`]. A root parent and an ordinary kind are the decode defaults, so they are omitted.
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
    record(b, fields)
}

/// The items of a `("list" e…)` value — accepting both the string head `"list"` and the bare name head
/// `list`, which denote the same construct.
fn list_items(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    arenas
        .as_ctor_form(id, "list")
        .or_else(|| arenas.as_form(id, "list"))
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

/// Read one blob record: a `name` and exactly one of `bytes` (inline) or `path`.
fn read_blob(arenas: &Arenas, id: StructId) -> Result<BlobSpec, SpecError> {
    if arenas.head_ctor(id) != Some("record") {
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
    if arenas.head_ctor(id) != Some("record") {
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
    Ok(spec)
}

/// Read one delivery record: a `target` task name and exactly one event — a `message` or a `notification`
/// sub-record. Returns the target and the [`DeliveryEvent`].
fn read_delivery(arenas: &Arenas, id: StructId) -> Result<(String, DeliveryEvent), SpecError> {
    if arenas.head_ctor(id) != Some("record") {
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
    let event = match (message, notification) {
        (Some(m), None) => {
            let token = match record_field(arenas, m, "token") {
                None => Bytes::new(),
                Some(t) => read_bytes(arenas, t).ok_or(SpecError::WrongType {
                    field: "token",
                    want: "a bytes literal",
                })?,
            };
            DeliveryEvent::Message {
                contract: read_contract_id(arenas, m)?,
                payload: read_required_bytes(arenas, m, "payload")?,
                token,
            }
        }
        (None, Some(n)) => DeliveryEvent::Notification {
            contract: read_contract_id(arenas, n)?,
            payload: read_required_bytes(arenas, n, "payload")?,
        },
        _ => return Err(SpecError::DeliveryKind { target }),
    };
    Ok((target, event))
}

/// Read the required `contract` field of an event record as a [`ContractId`] (its 33 raw hash bytes).
fn read_contract_id(arenas: &Arenas, event: StructId) -> Result<ContractId, SpecError> {
    let id = record_field(arenas, event, "contract").ok_or(SpecError::MissingField("contract"))?;
    // A contract-id crosses either as its raw 33 tagged bytes, or as a base64url string (§8) — the text form
    // a name→id rewrite substitutes for a contract name (the platform does no name resolution; that mapping
    // is produced outside and rewritten into the spec, so a resolved name arrives here as its base64url id).
    if let Some(text) = arenas.as_str(id) {
        let hash = text.parse::<Hash>().map_err(|_| SpecError::WrongType {
            field: "contract",
            want: "a base64url contract-id",
        })?;
        Ok(ContractId::from_hash(hash))
    } else {
        let bytes = read_bytes(arenas, id).ok_or(SpecError::WrongType {
            field: "contract",
            want: "a base64url string or a 33-byte contract-id",
        })?;
        ContractId::try_from(bytes.as_ref()).map_err(|_| SpecError::WrongType {
            field: "contract",
            want: "a 33-byte contract-id",
        })
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
        } => {
            let contract = bytes_leaf(b, contract.hash().as_bytes());
            let payload = bytes_leaf(b, payload);
            let mut fields = vec![("contract", contract), ("payload", payload)];
            if !token.is_empty() {
                let token = bytes_leaf(b, token);
                fields.push(("token", token));
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
    };
    record(b, vec![("target", target_leaf), (kind, inner)])
}

#[cfg(test)]
mod tests {
    use super::{BlobSource, BlobSpec, DeliveryEvent, HarnessSpec, SpecError};
    use crate::contract_value::{bytes_leaf, record, uint_leaf};
    use crate::testing::SpawnSpec;
    use crate::{Bytes, ContractId, ReducerKind};
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

    /// A `("list" e…)` value (string head, the canonical list constructor).
    fn list(b: &mut Builder, items: Vec<StructId>) -> StructId {
        let head = b.atom_leaf(Leaf::Str(Arc::from("list")));
        b.list(std::iter::once(head).chain(items).collect())
    }

    /// A string leaf.
    fn s(b: &mut Builder, text: &str) -> StructId {
        b.atom_leaf(Leaf::Str(Arc::from(text)))
    }

    /// A full description: a system reducer, a run horizon, two blobs (one inline, one by path), and three
    /// spawns (a root, an event-kind reducer, and a child).
    fn full(b: &mut Builder) -> StructId {
        let system = s(b, "sys");
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
                ("system", system),
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
        assert_eq!(spec.system, "sys");
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
        assert_eq!(spec.system, "sys");
        assert_eq!(spec.run_for, Some(Duration::from_nanos(5_000_000_000)));
        assert_eq!(spec.blobs.len(), 2);
        assert_eq!(spec.spawns.len(), 3);
    }

    #[test]
    fn optional_fields_default_and_a_bare_system_is_enough() {
        let arenas = built(|b| {
            let system = s(b, "sys");
            record(b, vec![("system", system)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("just a system is valid");
        assert_eq!(spec.system, "sys");
        assert_eq!(spec.run_for, None);
        assert!(spec.blobs.is_empty());
        assert!(spec.spawns.is_empty());
        assert_eq!(spec.checker, None, "no checker field ⇒ no end-of-run check");
    }

    #[test]
    fn reads_the_checker_blob_name() {
        // A run may name a checker program — a reducer the harness runs over the completed log (§9).
        let arenas = built(|b| {
            let system = s(b, "sys");
            let checker = s(b, "assert-echo");
            record(b, vec![("system", system), ("checker", checker)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("a system + checker is valid");
        assert_eq!(spec.checker.as_deref(), Some("assert-echo"));
    }

    #[test]
    fn build_resolves_inline_and_path_blobs() {
        let spec = HarnessSpec {
            system: "sys".to_string(),
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
            system: "sys".to_string(),
            run_for: None,
            blobs: vec![BlobSpec {
                name: "external".to_string(),
                source: BlobSource::Path("missing.wasm".to_string()),
            }],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
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
    fn rejects_a_missing_system() {
        let arenas = built(|b| record(b, vec![]));
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::MissingField("system"))
        );
    }

    #[test]
    fn rejects_a_run_for_that_is_not_an_integer() {
        let arenas = built(|b| {
            let system = s(b, "sys");
            let bad = s(b, "soon");
            record(b, vec![("system", system), ("run-for", bad)])
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
            let system = s(b, "sys");
            let name = s(b, "b");
            let bytes = bytes_leaf(b, b"x");
            let path = s(b, "p");
            let blob = record(b, vec![("name", name), ("bytes", bytes), ("path", path)]);
            let blobs = list(b, vec![blob]);
            record(b, vec![("system", system), ("blobs", blobs)])
        });
        assert_eq!(
            HarnessSpec::read(&both, both.root),
            Err(SpecError::BlobSource {
                blob: "b".to_string()
            })
        );
        // Neither source.
        let neither = built(|b| {
            let system = s(b, "sys");
            let name = s(b, "b");
            let blob = record(b, vec![("name", name)]);
            let blobs = list(b, vec![blob]);
            record(b, vec![("system", system), ("blobs", blobs)])
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
            let system = s(b, "sys");
            let name = s(b, "t");
            let blob = s(b, "greeter");
            let kind = s(b, "supervisor");
            let spawn = record(b, vec![("name", name), ("blob", blob), ("kind", kind)]);
            let spawns = list(b, vec![spawn]);
            record(b, vec![("system", system), ("spawns", spawns)])
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
            system: "sys".to_string(),
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
        };
        let bytes = spec.encode();
        assert_eq!(HarnessSpec::decode(&bytes), Ok(spec));
    }

    #[test]
    fn encode_omits_defaults_so_a_minimal_spec_round_trips() {
        // The defaults (no run-for, no blobs, no spawns) are omitted on encode and restored on decode.
        let spec = HarnessSpec {
            system: "only-system".to_string(),
            run_for: None,
            blobs: vec![],
            spawns: vec![],
            deliveries: vec![],
            checker: None,
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }

    #[test]
    fn reads_message_and_notification_deliveries() {
        let cid = ContractId::of(b"temp.celsius");
        let nid = ContractId::of(b"lifecycle.spawned");
        let arenas = built(|b| {
            let system = s(b, "sys");
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
            record(b, vec![("system", system), ("deliver", deliver)])
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
            let system = s(b, "sys");
            let msg = {
                let contract = bytes_leaf(b, cid.hash().as_bytes());
                let payload = bytes_leaf(b, b"p");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("system", system), ("deliver", deliver)])
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
                },
            )]
        );
    }

    #[test]
    fn rejects_a_delivery_with_both_or_neither_event() {
        // Neither message nor notification.
        let neither = built(|b| {
            let system = s(b, "sys");
            let target = s(b, "root");
            let d = record(b, vec![("target", target)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("system", system), ("deliver", deliver)])
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
            let system = s(b, "sys");
            let msg = {
                let contract = bytes_leaf(b, b"too short for a hash");
                let payload = bytes_leaf(b, b"p");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("system", system), ("deliver", deliver)])
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
    fn a_delivery_contract_may_be_a_base64url_string() {
        // A name→id rewrite (done outside the platform) substitutes a base64url contract-id string for a
        // contract name; decode parses it to the same id the raw bytes would give — so nix can rewrite
        // `contract = <name>` to `contract = <base64url>` with a pure string substitution.
        let cid = ContractId::of(b"temp.celsius");
        let text = cid.hash().to_string(); // the base64url §8 text form
        let arenas = built(|b| {
            let system = s(b, "sys");
            let msg = {
                let contract = s(b, &text);
                let payload = bytes_leaf(b, b"21");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("system", system), ("deliver", deliver)])
        });
        let spec = HarnessSpec::read(&arenas, arenas.root).expect("a base64url contract is valid");
        match &spec.deliveries[0].1 {
            DeliveryEvent::Message { contract, .. } => assert_eq!(*contract, cid),
            other => panic!("expected a message delivery, got {other:?}"),
        }
    }

    #[test]
    fn rejects_a_contract_string_that_is_not_base64url() {
        let arenas = built(|b| {
            let system = s(b, "sys");
            let msg = {
                let contract = s(b, "not base64url!!");
                let payload = bytes_leaf(b, b"p");
                record(b, vec![("contract", contract), ("payload", payload)])
            };
            let target = s(b, "root");
            let d = record(b, vec![("target", target), ("message", msg)]);
            let deliver = list(b, vec![d]);
            record(b, vec![("system", system), ("deliver", deliver)])
        });
        assert_eq!(
            HarnessSpec::read(&arenas, arenas.root),
            Err(SpecError::WrongType {
                field: "contract",
                want: "a base64url contract-id",
            })
        );
    }
}
