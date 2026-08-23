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
//!   (= blobs  ("list" <blob>…))               ; the program blobs, by name
//!   (= spawns ("list" <spawn>…)))             ; the tasks to spawn, in order
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
use crate::{Bytes, ReducerKind};
use cadenza_ast::ast::{Arenas, Builder, Leaf, StructId};
use cadenza_ast::codec;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

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

        Ok(HarnessSpec {
            system,
            run_for,
            blobs,
            spawns,
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
        Ok(harness)
    }

    /// Encode this description to a Cadenza binary AST — the exact inverse of [`decode`](HarnessSpec::decode),
    /// producing the bytes the executable reads. A field at its default (an absent `run-for`, an ordinary
    /// spawn kind, a root parent) is omitted, so `decode(spec.encode())` recovers an equal `HarnessSpec`.
    /// This is how a test or a build step produces a `harness.ast` programmatically rather than by hand.
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

#[cfg(test)]
mod tests {
    use super::{BlobSource, BlobSpec, HarnessSpec, SpecError};
    use crate::contract_value::{bytes_leaf, record, uint_leaf};
    use crate::testing::SpawnSpec;
    use crate::{Bytes, ReducerKind};
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
        // every optional: an inline and a path blob, a root/child parent, and an event-kind spawn.
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
        };
        assert_eq!(HarnessSpec::decode(&spec.encode()), Ok(spec));
    }
}
