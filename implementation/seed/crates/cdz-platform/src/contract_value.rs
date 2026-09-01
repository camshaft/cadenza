//! The canonical Cadenza value-model primitives shared by every generated contract value builder.
//!
//! A contract's schema is generated from its Cadenza source (`cargo xtask codegen`); so are the per-
//! constructor value builders and readers in `contracts/<name>.rs`. Those generated functions are thin —
//! they name a constructor and its fields and defer the actual value SHAPE to the primitives here, so the
//! canonical forms live in exactly one place and cannot drift between contracts or from the compiler's own
//! encoding.
//!
//! The forms match the compiler's own canonical `Value.encode`/`Value.decode` (empirically pinned), so a
//! contract value a Rust builder emits is decodable by a Cadenza guest and vice versa — the "one canonical
//! codec" (§12) actually holding across the Rust↔Cadenza boundary. A guest `Value.decode`s type-directed,
//! so the shape is:
//!  - a constructor `T.C` applied to a payload → the BARE-name form `(C <payload>…)` (the type `T` is not in
//!    the value — it comes from the root ascription / the target type); a SINGLE-constructor sum ELIDES the
//!    constructor entirely (the payload directly) — that elision is the generated code's call, not here;
//!  - a record value → the NAME-headed `(record (= <field> <value>)…)`, with fields in ascending NAME order
//!    (the decoder reads records in the compiler's canonical order — [`record`] sorts them);
//!  - a prelude (unqualified) constructor `C` → the same bare-name `(C <payload>…)` (e.g. `Ok`/`Err`);
//!  - the whole payload, at the encode boundary, is wrapped in a root ascription `(: <value> <Type>)` via
//!    [`ascribe`] (the decoder reads the type token but does not match it against the target — [`as_ascribed`]
//!    strips it on the way in).
//!
//! These are generic over the constructor/field names, so they carry no schema-specific knowledge — the
//! generated code supplies the names. Readers are the exact inverses and are total (`Option`), so decoding
//! a malformed value is a rejected value, never a panic; they are LIBERAL on the head (accept the canonical
//! name-headed form AND the string-headed ML-surface form) so the same reader serves a platform-built value
//! and a `cdz convert` surface value.

use crate::{Bytes, Hash};
use cadenza_ast::ast::{Builder, CompoundCtor, Leaf, Radix, Struct, StructId};
use std::sync::Arc;

// --- builders ---

/// A constructor application in the canonical BARE-name form `(<ctor> <payload>…)` — the constructor name is
/// the head, then its payload occurrences; nullary → `(<ctor>)`. `ty` is accepted for the generated caller's
/// convenience (it names the sum the constructor belongs to) but is NOT part of the value: the compiler's
/// value form carries no `(. ty ctor)` member node — the type is fixed by the root ascription / the decode
/// target. A single-constructor sum elides the constructor (the generated code emits the payload directly),
/// so this builds only the multi-constructor case.
#[must_use]
pub fn qctor(b: &mut Builder, ty: &str, ctor: &str, payload: Vec<StructId>) -> StructId {
    let _ = ty;
    bare_ctor(b, ctor, payload)
}

/// A constructor application `(<name> <payload>…)` — the constructor name as the head, then its payload. The
/// canonical sum form for both a schema constructor and a prelude one (`Ok`/`Err`/`Some`/`None`).
#[must_use]
pub fn bare_ctor(b: &mut Builder, name: &str, payload: Vec<StructId>) -> StructId {
    let head = b.name(name);
    b.list(std::iter::once(head).chain(payload).collect())
}

/// The `unit` atom — the canonical Value form of a Unit value. The compiler's `Value.encode` renders Unit as
/// the bare name atom `unit` (the same form the runtime's value renderer emits for a `Unit` payload), so a
/// **nullary single-constructor** sum elides its constructor to exactly this atom, framed only by the root
/// ascription: `(type Ack = | Ack)` encodes at the payload boundary as `(: unit Ack)`, not `(: (Ack) Ack)`.
/// (A *multi*-constructor nullary variant keeps its bare-name form `(Ctor)` — the elision is single-ctor
/// only.) The inverse is [`is_unit`].
#[must_use]
pub fn unit(b: &mut Builder) -> StructId {
    b.name("unit")
}

/// A root ascription `(: <value> <ty>)` — the top-level wrapper the compiler's `Value.decode` requires at the
/// payload-encode boundary. The type token is recorded but not matched against the decode target (the decoder
/// is type-directed by the caller's annotation), so any name — conventionally the contract's declared input
/// type — is accepted; [`as_ascribed`] strips it on read.
#[must_use]
pub fn ascribe(b: &mut Builder, value: StructId, ty: &str) -> StructId {
    let colon = b.name(":");
    let ty = b.name(ty);
    b.list(vec![colon, value, ty])
}

/// Encode a self-built value in the canonical binary form ([`cadenza_ast::codec`]): run `build` into a
/// fresh [`Builder`], [`ascribe`] its root to schema type `ty`, finish, and codec-encode. Every contract
/// event/envelope's `encode` is this exact `build → ascribe → finish → encode` wrapper, so it lives here
/// once — each type supplies only its `build` closure and ascription type.
#[must_use]
pub fn encode_ascribed(build: impl FnOnce(&mut Builder) -> StructId, ty: &str) -> Bytes {
    let mut b = Builder::new();
    let value = build(&mut b);
    let root = ascribe(&mut b, value, ty);
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// A record value — the **M2 NATIVE** record constructor: a `Leaf::Ctor(CompoundCtor::Record)` head
/// (recognized by leaf-KIND, `KIND_RECORD_CTOR`, not head text) followed by one native `field_pair`
/// `(= <name> <value>)` (a `Leaf::FieldPair` marker, not a `Name("=")`) per entry, emitted in
/// **name-sorted** order. The compiler's `Value.decode` reads a record's fields **canonically ordered**
/// (that is what `Value.encode` produces), so a value a Cadenza guest decodes must present its fields
/// sorted — declaration order does not decode. Our own readers ([`record_field`]) read by name and are
/// order-independent, so the sort is transparent to them.
///
/// NATIVE, not the legacy name-headed `(record (= f v)…)` list: the guest runtime's `decode_value` Record
/// arm REQUIRES a native `Ctor(Record)` struct head and returns `None` on a name/string head — so a
/// name-headed record silently fails every guest `Value.decode` (the §9 checker-close: the check `Envelope`
/// this helper builds would not decode). Mirrors the native emit `cdz-contract` and rcdzc already use.
#[must_use]
pub fn record(b: &mut Builder, fields: Vec<(&str, StructId)>) -> StructId {
    let mut fields = fields;
    fields.sort_by_key(|&(name, _)| name);
    let pairs: Vec<StructId> = fields
        .into_iter()
        .map(|(name, value)| {
            let key = b.name(name);
            b.field_pair(key, value)
        })
        .collect();
    b.compound(CompoundCtor::Record, &pairs)
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
        value: cadenza_ast::ast::IntValue::from_u128(value as u128),
        radix: Radix::Dec,
    })
}

// --- readers (the exact inverses; total) ---

/// If `id` is a constructor application `(ctor tail…)` in the canonical bare-name form, its `tail` (the
/// payload occurrences). `None` if the shape or the constructor name does not match. `ty` is accepted for the
/// generated caller's symmetry with [`qctor`] but is not part of the value (the compiler form has no member
/// node), so it is not checked — the constructor name alone identifies the case within its type, which the
/// decode target / root ascription fixes.
#[must_use]
pub fn as_qctor<'a>(
    arenas: &'a cadenza_ast::ast::Arenas,
    id: StructId,
    ty: &str,
    ctor: &str,
) -> Option<&'a [StructId]> {
    // LIBERAL: the canonical Value form is the bare-name `(ctor tail…)`, but a value crossing from the ML
    // SURFACE (`cdz convert`) or an older member form may be `((. ty ctor) tail…)`. Accept both; the builders
    // emit only the bare form.
    as_bare_ctor(arenas, id, ctor).or_else(|| {
        let Struct::List(items) = arenas.get(id) else {
            return None;
        };
        let (&head, tail) = items.split_first()?;
        let m = arenas.as_form(head, ".")?;
        (m.len() == 2 && arenas.as_name(m[0]) == Some(ty) && arenas.as_name(m[1]) == Some(ctor))
            .then_some(tail)
    })
}

/// The value inside a root ascription `(: <value> <ty>)`, ignoring the type token (the decoder is
/// type-directed). `None` if `id` is not a `(:  …)` ascription of exactly `(value ty)`. The inverse of
/// [`ascribe`].
#[must_use]
pub fn as_ascribed(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<StructId> {
    let inner = arenas.as_form(id, ":")?;
    (inner.len() == 2).then_some(inner[0])
}

/// Whether `id` is the `unit` atom — the inverse of [`unit`]. A nullary single-constructor sum's elided
/// payload, once the root ascription is stripped, is exactly this atom.
#[must_use]
pub fn is_unit(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> bool {
    arenas.as_name(id) == Some("unit")
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
    // LIBERAL on the head, across ALL THREE record spellings: the M2 NATIVE ctor-leaf head (what rcdzc now
    // compiles a record VALUE to — e.g. a harness description / contract value emitted by the guest), the
    // NAME-headed `(record …)` alias, and the legacy string-headed `("record" …)` — via `compound_form_of`.
    // (Before this, the native ctor-leaf head was missed, so a guest-compiled native record read as
    // "not a record".) The `(= name value)` field head is read through the `as_name` FieldPair→"=" bridge,
    // so it accepts the native FieldPair leaf too.
    let fields = arenas.compound_form_of(id, CompoundCtor::Record)?;
    fields.iter().find_map(|&f| {
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
            Leaf::Int { value, .. } => value.to_u128().and_then(|u| u64::try_from(u).ok()),
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
        as_ascribed, as_bare_ctor, as_qctor, ascribe, bare_ctor, bytes_leaf, is_unit, qctor,
        read_bytes, read_hash, read_uint, record, record_field, uint_leaf, unit,
    };
    use crate::{Hash, HashTag};
    use cadenza_ast::ast::{Builder, CompoundCtor, Leaf, Radix};

    // Build a value with `build`, finish the arena, and hand back `(arenas, root)` to read from.
    fn built(build: impl FnOnce(&mut Builder) -> super::StructId) -> cadenza_ast::ast::Arenas {
        let mut b = Builder::new();
        let root = build(&mut b);
        b.finish(root)
    }

    #[test]
    fn a_constructor_round_trips_in_the_bare_name_form_and_rejects_the_wrong_ctor() {
        // An applied constructor is the canonical BARE-name form `(Message payload)`; its payload reads back
        // as the tail, byte-for-byte. The `ty` argument is not part of the value (no `(. ty ctor)` member) —
        // the constructor name alone identifies the case — so `as_qctor` reads it regardless of `ty`, and it
        // is the SAME form a prelude `as_bare_ctor` reads.
        let arenas = built(|b| {
            let x = bytes_leaf(b, b"payload");
            qctor(b, "Event", "Message", vec![x])
        });
        let tail = as_qctor(&arenas, arenas.root, "Event", "Message").expect("a Message(..)");
        assert_eq!(tail.len(), 1);
        assert_eq!(
            read_bytes(&arenas, tail[0]).as_deref(),
            Some(b"payload".as_slice())
        );
        // The constructor NAME must match; `ty` is not checked (it is not in the value).
        assert!(as_qctor(&arenas, arenas.root, "Event", "Response").is_none());
        assert!(as_qctor(&arenas, arenas.root, "AnyType", "Message").is_some());
        // Bare-name form: a schema constructor and a prelude constructor are one form.
        assert!(as_bare_ctor(&arenas, arenas.root, "Message").is_some());
        assert!(as_bare_ctor(&arenas, arenas.root, "Event").is_none());
    }

    #[test]
    fn a_nullary_constructor_has_an_empty_tail() {
        let arenas = built(|b| qctor(b, "Outcome", "Delivered", vec![]));
        let tail = as_qctor(&arenas, arenas.root, "Outcome", "Delivered").expect("a Delivered");
        assert!(tail.is_empty());
    }

    #[test]
    fn the_unit_atom_round_trips_and_is_the_single_ctor_nullary_elided_form() {
        // A nullary SINGLE-constructor sum (`type Ack = | Ack`) elides its constructor to the bare `unit`
        // atom — the compiler's `Value.encode` of the erased Unit payload — NOT the bare-name `(Ack)` a
        // multi-constructor nullary variant keeps. `unit` round-trips through `is_unit`, and the two forms
        // are distinct: the elided value is the `unit` atom, not a `(…)` list.
        let arenas = built(unit);
        assert!(is_unit(&arenas, arenas.root));
        // The multi-ctor nullary form `(Delivered)` is NOT the unit atom (it is a list, not the name `unit`).
        let other = built(|b| qctor(b, "Outcome", "Delivered", vec![]));
        assert!(!is_unit(&other, other.root));
        // Ascribed at the root, the single-ctor-nullary form is `(: unit Ack)`: strip the ascription, the
        // inner value is the unit atom (what a generated `is_ack_ack` checks after `as_ascribed`).
        let ascribed = built(|b| {
            let u = unit(b);
            ascribe(b, u, "Ack")
        });
        let inner = as_ascribed(&ascribed, ascribed.root).expect("a root ascription");
        assert!(is_unit(&ascribed, inner));
    }

    #[test]
    fn a_prelude_constructor_round_trips() {
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
    }

    #[test]
    fn a_root_ascription_round_trips_and_ignores_its_type_token() {
        // The encode-boundary wrapper `(: value Ty)`: `as_ascribed` returns the inner value; the type token
        // is not matched (the decoder is type-directed), so any name wraps and strips the same.
        let arenas = built(|b| {
            let v = bytes_leaf(b, b"inner");
            ascribe(b, v, "Envelope")
        });
        let inner = as_ascribed(&arenas, arenas.root).expect("an ascription");
        assert_eq!(
            read_bytes(&arenas, inner).as_deref(),
            Some(b"inner".as_slice())
        );
        // A non-ascription (a bare bytes leaf) is not an ascription.
        let bare = built(|b| bytes_leaf(b, b"x"));
        assert!(as_ascribed(&bare, bare.root).is_none());
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
    fn a_record_emits_its_fields_in_name_sorted_physical_order() {
        // FIX B invariant: the compiler's `Value.decode` reads a record's fields in the canonical
        // (ascending name) order that `Value.encode` produces, so a Rust-built value must present them
        // sorted — declaration order does NOT decode. This pins the PHYSICAL order of the emitted tree, which
        // `a_record_reads_its_fields_by_name_regardless_of_order` cannot: that test reads by name, so it
        // stays green even if the sort is removed — but a guest `Value.decode` would then fail. Build the
        // fields in DELIBERATELY unsorted input order and assert the encoded children come out sorted.
        let arenas = built(|b| {
            let zebra = bytes_leaf(b, b"z");
            let alpha = bytes_leaf(b, b"a");
            let mango = bytes_leaf(b, b"m");
            record(
                b,
                vec![("zebra", zebra), ("alpha", alpha), ("mango", mango)],
            )
        });
        // The raw record fields, in the physical order they were emitted (after the native `Ctor(Record)`
        // head). Read the NATIVE M2 form: a `compound_form_of(Record)` head + `field_pair` entries — the
        // same readers `cdz-contract` and the guest `decode_value` use, not the legacy name-head `(record …)`.
        let fields = arenas
            .compound_form_of(arenas.root, CompoundCtor::Record)
            .expect("a native record value");
        let names: Vec<&str> = fields
            .iter()
            .map(|&f| {
                let (key, _value) = arenas
                    .field_pair_parts(f)
                    .expect("a native `(= name value)` field pair");
                arenas.as_name(key).expect("a field name")
            })
            .collect();
        assert_eq!(
            names,
            ["alpha", "mango", "zebra"],
            "record fields must be emitted in ascending NAME order (canonical form), not declaration order"
        );
    }

    #[test]
    fn a_root_ascription_wraps_the_value_as_the_outermost_colon_form() {
        // FIX B invariant: the encode boundary wraps the payload in `(: <value> <ty>)` as the OUTERMOST node
        // — the top-level form the compiler's `Value.decode` requires. Pin the physical shape: the root is a
        // `:`-headed list of exactly `(value ty)`, with the value first and the type token second.
        let arenas = built(|b| {
            let v = record(b, vec![]);
            ascribe(b, v, "Envelope")
        });
        let inner = arenas.as_form(arenas.root, ":").expect("a `:`-headed root");
        assert_eq!(inner.len(), 2, "an ascription is exactly `(: value ty)`");
        // The type token is the SECOND child (the value is first); `as_ascribed` returns the first.
        assert_eq!(arenas.as_name(inner[1]), Some("Envelope"));
        assert_eq!(as_ascribed(&arenas, arenas.root), Some(inner[0]));
    }

    #[test]
    fn readers_reject_a_mismatched_shape() {
        // A constructor `(C)` has no record fields, so `record_field` is `None` (its head is `C`, not
        // `record`). Each reader is total and rejects the wrong shape rather than panicking.
        let qc = built(|b| qctor(b, "T", "C", vec![]));
        assert!(record_field(&qc, qc.root, "id").is_none());
        // A record is `(record …)`; reading it as the constructor `C` is `None` (head is `record`, not `C`).
        let rec = built(|b| record(b, vec![]));
        assert!(as_qctor(&rec, rec.root, "T", "C").is_none());
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
                value: cadenza_ast::ast::IntValue::from_i64(-1),
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
