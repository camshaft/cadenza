//! The canonical Cadenza value-model primitives shared by every generated contract value builder.
//!
//! A contract's schema is generated from its Cadenza source (`cargo xtask codegen`); so are the per-
//! constructor value builders and readers in `contracts/<name>.rs`. Those generated functions are thin —
//! they name a constructor and its fields and defer the actual value SHAPE to the primitives here, so the
//! canonical forms live in exactly one place and cannot drift between contracts or from the compiler's own
//! encoding. The forms, as the compiler canonicalizes them:
//!
//!  - a qualified constructor `T.C` applied to a payload → `((. T C) <payload>…)`; nullary → `((. T C))`;
//!  - a record value → the string-headed `("record" (= <field> <value>)…)`;
//!  - a prelude (unqualified) constructor `C` → `(C <payload>…)` (e.g. `Ok`/`Err`).
//!
//! These are generic over the constructor/field names, so they carry no schema-specific knowledge — the
//! generated code supplies the names. Readers are the exact inverses and are total (`Option`), so decoding
//! a malformed value is a rejected value, never a panic.

use crate::{Bytes, Hash};
use cadenza_ast::ast::{Builder, Leaf, Radix, Struct, StructId};
use num_bigint::BigInt;
use std::sync::Arc;

// --- builders ---

/// A qualified constructor application `((. <ty> <ctor>) <payload>…)` — the member node `(. ty ctor)`
/// applied to its payload occurrences. An empty `payload` builds the nullary form `((. ty ctor))`.
#[must_use]
pub fn qctor(b: &mut Builder, ty: &str, ctor: &str, payload: Vec<StructId>) -> StructId {
    let head = member(b, ty, ctor);
    b.list(std::iter::once(head).chain(payload).collect())
}

/// The member node `(. <ty> <ctor>)` naming a qualified constructor — the head of a [`qctor`] application.
fn member(b: &mut Builder, ty: &str, ctor: &str) -> StructId {
    let dot = b.name(".");
    let ty = b.name(ty);
    let ctor = b.name(ctor);
    b.list(vec![dot, ty, ctor])
}

/// A prelude (unqualified) constructor application `(<name> <payload>…)` — e.g. `(Ok v)` / `(Err e)`, the
/// bare `Result` constructors. Distinct from [`qctor`]: a prelude constructor is a bare name, not a member.
#[must_use]
pub fn bare_ctor(b: &mut Builder, name: &str, payload: Vec<StructId>) -> StructId {
    let head = b.name(name);
    b.list(std::iter::once(head).chain(payload).collect())
}

/// A record value `("record" (= <field> <value>)…)` — the string-headed record constructor, then one
/// `(= <name> <value>)` field per entry, in the given order (fields are read back by name, so the order is
/// not load-bearing).
#[must_use]
pub fn record(b: &mut Builder, fields: Vec<(&str, StructId)>) -> StructId {
    let head = b.atom_leaf(Leaf::Str(Arc::from("record")));
    let mut children = Vec::with_capacity(1 + fields.len());
    children.push(head);
    for (name, value) in fields {
        let eq = b.name("=");
        let name = b.name(name);
        children.push(b.list(vec![eq, name, value]));
    }
    b.list(children)
}

/// A `Bytes` leaf carrying `bytes` — how every hash and opaque payload crosses in a contract value.
#[must_use]
pub fn bytes_leaf(b: &mut Builder, bytes: &[u8]) -> StructId {
    b.atom_leaf(Leaf::Bytes(Arc::from(bytes)))
}

/// An unsigned-integer leaf carrying `value` — how a native Cadenza `UInt64` (and the narrower unsigned
/// ints) field crosses in a contract value. The value model's integer is arbitrary-precision (`Leaf::Int`);
/// the schema's declared type is what fixes the width and signedness, so this builder is width-agnostic and
/// [`read_uint`] below enforces the non-negative range. Written in decimal (the base is display-only; it does
/// not change the value).
#[must_use]
pub fn uint_leaf(b: &mut Builder, value: u64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: BigInt::from(value),
        radix: Radix::Dec,
    })
}

// --- readers (the exact inverses; total) ---

/// If `id` is a qualified-constructor application `((. ty ctor) tail…)`, its `tail` (the payload
/// occurrences). `None` if the shape or the constructor name does not match.
#[must_use]
pub fn as_qctor<'a>(
    arenas: &'a cadenza_ast::ast::Arenas,
    id: StructId,
    ty: &str,
    ctor: &str,
) -> Option<&'a [StructId]> {
    let Struct::List(items) = arenas.get(id) else {
        return None;
    };
    let (&head, tail) = items.split_first()?;
    let m = arenas.as_form(head, ".")?;
    if m.len() == 2 && arenas.as_name(m[0]) == Some(ty) && arenas.as_name(m[1]) == Some(ctor) {
        Some(tail)
    } else {
        None
    }
}

/// If `id` is a prelude-constructor application `(name tail…)`, its `tail`. `None` otherwise.
#[must_use]
pub fn as_bare_ctor<'a>(
    arenas: &'a cadenza_ast::ast::Arenas,
    id: StructId,
    name: &str,
) -> Option<&'a [StructId]> {
    arenas.as_form(id, name)
}

/// The value of a record's field named `name` — the `<value>` of the `(= <name> <value>)` field inside a
/// `("record" …)` value. `None` if `id` is not a record or has no such field.
#[must_use]
pub fn record_field(
    arenas: &cadenza_ast::ast::Arenas,
    id: StructId,
    name: &str,
) -> Option<StructId> {
    let Struct::List(items) = arenas.get(id) else {
        return None;
    };
    if arenas.head_ctor(id) != Some("record") {
        return None;
    }
    items.iter().skip(1).find_map(|&f| {
        let kv = arenas.as_form(f, "=")?;
        (kv.len() == 2 && arenas.as_name(kv[0]) == Some(name)).then_some(kv[1])
    })
}

/// The bytes of a `Bytes` leaf, or `None` if `id` is not one.
#[must_use]
pub fn read_bytes(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Bytes> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bytes(bytes) => Some(Bytes::copy_from_slice(bytes)),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// The value of an integer leaf as a `u64`, or `None` if `id` is not an integer leaf or the value is
/// negative or too large to fit `u64`. The inverse of [`uint_leaf`].
#[must_use]
pub fn read_uint(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<u64> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Int { value, .. } => u64::try_from(value).ok(),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// A [`Hash`] read from a `Bytes` leaf of exactly `Hash::LEN` bytes, or `None`.
#[must_use]
pub fn read_hash(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Hash> {
    let bytes = read_bytes(arenas, id)?;
    Some(Hash::from_bytes(
        <[u8; Hash::LEN]>::try_from(bytes.as_ref()).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        as_bare_ctor, as_qctor, bare_ctor, bytes_leaf, qctor, read_bytes, read_hash, read_uint,
        record, record_field, uint_leaf,
    };
    use crate::{Hash, HashTag};
    use cadenza_ast::ast::{Builder, Leaf, Radix};

    // Build a value with `build`, finish the arena, and hand back `(arenas, root)` to read from.
    fn built(build: impl FnOnce(&mut Builder) -> super::StructId) -> cadenza_ast::ast::Arenas {
        let mut b = Builder::new();
        let root = build(&mut b);
        b.finish(root)
    }

    #[test]
    fn a_qualified_constructor_round_trips_and_rejects_the_wrong_name() {
        // An applied `T.C` payload reads back as its tail; the payload leaf reads back byte-for-byte.
        let arenas = built(|b| {
            let x = bytes_leaf(b, b"payload");
            qctor(b, "Event", "Message", vec![x])
        });
        let tail = as_qctor(&arenas, arenas.root, "Event", "Message").expect("an Event.Message");
        assert_eq!(tail.len(), 1);
        assert_eq!(
            read_bytes(&arenas, tail[0]).as_deref(),
            Some(b"payload".as_slice())
        );
        // Neither the type nor the constructor name may differ.
        assert!(as_qctor(&arenas, arenas.root, "Event", "Response").is_none());
        assert!(as_qctor(&arenas, arenas.root, "Other", "Message").is_none());
        // A qualified constructor is not a prelude (bare) constructor.
        assert!(as_bare_ctor(&arenas, arenas.root, "Event").is_none());
    }

    #[test]
    fn a_nullary_qualified_constructor_has_an_empty_tail() {
        let arenas = built(|b| qctor(b, "Outcome", "Delivered", vec![]));
        let tail =
            as_qctor(&arenas, arenas.root, "Outcome", "Delivered").expect("an Outcome.Delivered");
        assert!(tail.is_empty());
    }

    #[test]
    fn a_prelude_constructor_round_trips_and_does_not_cross_with_qualified() {
        let arenas = built(|b| {
            let v = bytes_leaf(b, b"answer");
            bare_ctor(b, "Ok", vec![v])
        });
        let tail = as_bare_ctor(&arenas, arenas.root, "Ok").expect("an Ok(..)");
        assert_eq!(tail.len(), 1);
        assert_eq!(
            read_bytes(&arenas, tail[0]).as_deref(),
            Some(b"answer".as_slice())
        );
        assert!(as_bare_ctor(&arenas, arenas.root, "Err").is_none());
        // A prelude constructor is not a qualified one.
        assert!(as_qctor(&arenas, arenas.root, "Ok", "Ok").is_none());
    }

    #[test]
    fn a_record_reads_its_fields_by_name_regardless_of_order() {
        let arenas = built(|b| {
            let id = bytes_leaf(b, b"the-id");
            let parent = bytes_leaf(b, b"the-parent");
            record(b, vec![("id", id), ("parent", parent)])
        });
        // Fields are addressed by name, not position — read them in the opposite order.
        let parent = record_field(&arenas, arenas.root, "parent").expect("a parent field");
        let id = record_field(&arenas, arenas.root, "id").expect("an id field");
        assert_eq!(
            read_bytes(&arenas, id).as_deref(),
            Some(b"the-id".as_slice())
        );
        assert_eq!(
            read_bytes(&arenas, parent).as_deref(),
            Some(b"the-parent".as_slice())
        );
        // A missing field, and a record read as a non-record, are both `None`.
        assert!(record_field(&arenas, arenas.root, "absent").is_none());
    }

    #[test]
    fn readers_reject_a_mismatched_shape() {
        // A qualified constructor is not a record, and its `record_field` is `None`; a record is not a
        // qualified constructor. Each reader is total and rejects the wrong shape rather than panicking.
        let qc = built(|b| qctor(b, "T", "C", vec![]));
        assert!(record_field(&qc, qc.root, "id").is_none());
        let rec = built(|b| record(b, vec![]));
        assert!(as_qctor(&rec, rec.root, "record", "record").is_none());
        // `read_bytes` on a list (not an atom) is `None`.
        assert!(read_bytes(&rec, rec.root).is_none());
    }

    #[test]
    fn bytes_leaves_round_trip_including_empty() {
        let arenas = built(|b| bytes_leaf(b, b""));
        assert_eq!(
            read_bytes(&arenas, arenas.root).as_deref(),
            Some(b"".as_slice())
        );
        let arenas = built(|b| bytes_leaf(b, &[0x00, 0xFF, 0x13, 0x37]));
        assert_eq!(
            read_bytes(&arenas, arenas.root).as_deref(),
            Some([0x00, 0xFF, 0x13, 0x37].as_slice())
        );
    }

    #[test]
    fn unsigned_integers_round_trip_and_the_reader_enforces_range() {
        // Unsigned values round-trip, including the full range up to u64::MAX.
        for v in [0u64, 42, u64::MAX] {
            let arenas = built(|b| uint_leaf(b, v));
            assert_eq!(read_uint(&arenas, arenas.root), Some(v));
        }
        // The reader enforces the declared non-negative range: a negative value is not a valid u64.
        let neg = built(|b| {
            b.atom_leaf(Leaf::Int {
                value: (-1).into(),
                radix: Radix::Dec,
            })
        });
        assert_eq!(read_uint(&neg, neg.root), None, "a negative is not a u64");
        // A non-integer leaf (bytes) and a list are not a uint.
        let bytes = built(|b| bytes_leaf(b, b"x"));
        assert!(read_uint(&bytes, bytes.root).is_none());
        let list = built(|b| qctor(b, "T", "C", vec![]));
        assert!(read_uint(&list, list.root).is_none());
    }

    #[test]
    fn read_hash_round_trips_a_tagged_hash_and_rejects_a_wrong_length_leaf() {
        // A hash crosses as its 33 raw bytes (tag + digest) and reads back equal, tag preserved.
        let h = Hash::of(HashTag::Contract, b"a contract declaration");
        let arenas = built(|b| bytes_leaf(b, h.as_bytes()));
        let read = read_hash(&arenas, arenas.root).expect("a hash");
        assert_eq!(read, h);
        assert_eq!(read.tag(), Some(HashTag::Contract));
        // A leaf that is not exactly `Hash::LEN` bytes is not a hash.
        let short = built(|b| bytes_leaf(b, b"too short"));
        assert!(read_hash(&short, short.root).is_none());
    }
}
