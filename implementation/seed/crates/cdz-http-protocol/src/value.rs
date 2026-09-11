//! The canonical binary-AST **value-form toolkit** — the small set of builders + readers that produce and
//! parse the value forms the compiler's own `Value.encode`/`Value.decode` produce. Exposed `pub` so any
//! harness crate that needs to build binary-AST frames (the mock control server's admin protocol, a future
//! driver frame, …) reuses ONE codec rather than re-implementing these primitives (operator: binary-AST is
//! THE data-exchange format — no JSON, and no drifting copies).
//!
//! The forms (pinned empirically against the compiler; see the crate root docs):
//! - a record → `#record((= <field> <value>)…)`, fields in ascending NAME order.
//! - a `List(T)` → `#list(<elem>…)`, elements in order.
//! - a constructor application `(<Ctor> <payload>…)`; a nullary variant carries the `unit` atom.
//! - `String` → a `Str` leaf; `Bytes` → a `Bytes` leaf; an integer → an `Int` leaf (decimal).
//! - the whole payload is wrapped at the encode boundary in a root ascription `(: <value> <Type>)`.

use bytes::Bytes;
use cadenza_ast::ast::{Builder, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use std::sync::Arc;

// Re-export the AST types a frame codec needs, so downstream crates build + read frames without depending
// on `cadenza_ast` directly. `Arenas` is used unaliased in this module's own fn signatures too.
pub use cadenza_ast::ast::{Arenas, Builder as ValueBuilder, StructId as ValueId};

// --- builders ------------------------------------------------------------------------------------------

/// Wrap `value` in the root ascription `(: value ty)`, finish the AST, and encode it to binary-AST bytes.
#[must_use]
pub fn finish(mut b: Builder, value: StructId, ty: &str) -> Bytes {
    let root = ascribe(&mut b, value, ty);
    let arenas = b.finish(root);
    Bytes::from(cadenza_ast::codec::encode(&arenas))
}

/// A root ascription `(: <value> <ty>)`.
pub fn ascribe(b: &mut Builder, value: StructId, ty: &str) -> StructId {
    let colon = b.name(":");
    let ty = b.name(ty);
    b.list(vec![colon, value, ty])
}

/// A constructor application `(<name> <payload>…)` — the ctor name as head, then its payload.
pub fn bare_ctor(b: &mut Builder, name: &str, payload: Vec<StructId>) -> StructId {
    let head = b.name(name);
    b.list(std::iter::once(head).chain(payload).collect())
}

/// The `unit` atom — the payload of a nullary variant (`Foo.Bar` → `(Bar unit)`).
pub fn unit(b: &mut Builder) -> StructId {
    b.name("unit")
}

/// A record value — `#record((= <field> <value>)…)`, fields emitted in ascending NAME order.
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

/// A `List(T)` value — `#list(<elem>…)`, elements in order (NOT sorted).
pub fn list_value(b: &mut Builder, elems: Vec<StructId>) -> StructId {
    b.compound(CompoundCtor::List, &elems)
}

/// A `Bytes` leaf.
pub fn bytes_leaf(b: &mut Builder, bytes: &[u8]) -> StructId {
    b.atom_leaf(Leaf::Bytes(Arc::from(bytes)))
}

/// A `String` leaf (`Leaf::Str` — text, distinct from a `Leaf::Name` bare identifier).
pub fn str_leaf(b: &mut Builder, s: &str) -> StructId {
    b.atom_leaf(Leaf::Str(Arc::from(s)))
}

/// An integer leaf carrying `value`, written in decimal.
pub fn uint_leaf(b: &mut Builder, value: u64) -> StructId {
    b.atom_leaf(Leaf::Int {
        value: IntValue::from_u128(u128::from(value)),
        radix: Radix::Dec,
    })
}

/// A boolean leaf (`Leaf::Bool` — the encoding a surface `true` / `false` produces, distinct from a
/// `Leaf::Name` bare identifier). The inverse of [`read_bool`].
pub fn bool_leaf(b: &mut Builder, value: bool) -> StructId {
    b.atom_leaf(Leaf::Bool(value))
}

// --- readers (exact inverses; total) -------------------------------------------------------------------

/// Decode binary-AST bytes into an [`Arenas`], or `None` if malformed.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Arenas> {
    cadenza_ast::codec::decode(bytes)
}

/// The value inside a root ascription `(: <value> <ty>)`, ignoring the type token.
fn as_ascribed(arenas: &Arenas, id: StructId) -> Option<StructId> {
    let inner = arenas.as_form(id, ":")?;
    (inner.len() == 2).then_some(inner[0])
}

/// Strip an optional root ascription `(: value ty)` AND any reader comment wrappers `(comment "…" form)` /
/// `(comment-after "…" form)`, returning the underlying value. A run-spec compiled from ML surface by
/// `cdz convert --to binary` carries a doc comment above its root value as a `(comment …)` wrapper (and
/// values may be ascribed); peeling both — to a fixpoint, since they can nest in any order — lets the
/// structural readers see the value regardless. A value built by [`ValueBuilder`] (no comment nodes) is
/// unaffected. Named `unascribe` for compatibility; it now also peels comments.
#[must_use]
pub fn unascribe(arenas: &Arenas, id: StructId) -> StructId {
    let mut id = id;
    loop {
        let peeled = arenas.peel_comments(id);
        let next = as_ascribed(arenas, peeled).unwrap_or(peeled);
        if next == id {
            return id;
        }
        id = next;
    }
}

/// The value of a record's field named `name` (ascription-tolerant on `id`).
#[must_use]
pub fn record_field(arenas: &Arenas, id: StructId, name: &str) -> Option<StructId> {
    let fields = arenas.compound_form_of(unascribe(arenas, id), CompoundCtor::Record)?;
    fields.iter().find_map(|&f| {
        let kv = arenas.as_form(f, "=")?;
        (kv.len() == 2 && arenas.as_name(kv[0]) == Some(name)).then_some(kv[1])
    })
}

/// The members of a `#list(…)` value, or `None` if `id` is not a list (ascription/comment-tolerant).
#[must_use]
pub fn read_list(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    arenas.compound_form_of(unascribe(arenas, id), CompoundCtor::List)
}

/// The head constructor name of a `(<Ctor> …)` value (ascription-tolerant), or `None` if not a ctor form.
#[must_use]
pub fn read_ctor(arenas: &Arenas, id: StructId) -> Option<&str> {
    match arenas.get(unascribe(arenas, id)) {
        Struct::List(items) => arenas.as_name(*items.first()?),
        Struct::Atom(_) => None,
    }
}

/// The payload of a constructor application `(<Ctor> <payload>…)` — the elements after the head
/// (ascription-tolerant), or `None` if not a ctor form.
#[must_use]
pub fn ctor_payload(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    match arenas.get(unascribe(arenas, id)) {
        Struct::List(items) => items.get(1..),
        Struct::Atom(_) => None,
    }
}

/// A `String` leaf's text (ascription/comment-tolerant).
#[must_use]
pub fn read_str(arenas: &Arenas, id: StructId) -> Option<String> {
    arenas.as_str(unascribe(arenas, id)).map(str::to_string)
}

/// A `Bytes` leaf's bytes (ascription/comment-tolerant).
#[must_use]
pub fn read_bytes(arenas: &Arenas, id: StructId) -> Option<Bytes> {
    match arenas.get(unascribe(arenas, id)) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bytes(bytes) => Some(Bytes::copy_from_slice(bytes)),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// An integer leaf's value as a `u64`, or `None` if not an integer / negative / too large
/// (ascription/comment-tolerant).
#[must_use]
pub fn read_uint(arenas: &Arenas, id: StructId) -> Option<u64> {
    match arenas.get(unascribe(arenas, id)) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Int { value, .. } => value.to_u128().and_then(|u| u64::try_from(u).ok()),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// A boolean leaf's value (ascription/comment-tolerant). A surface `true` / `false` encodes as a
/// `Leaf::Bool` — NOT a `Leaf::Name` — so it must be read through [`Arenas::as_bool`], not `as_name`.
#[must_use]
pub fn read_bool(arenas: &Arenas, id: StructId) -> Option<bool> {
    arenas.as_bool(unascribe(arenas, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A run-spec compiled from ML surface by `cdz convert --to binary` carries doc comments as reader
    /// `(comment "…" form)` wrappers around the values they annotate. The structural readers must see
    /// through them — a comment on the root record AND on a field's value are both transparent here.
    #[test]
    fn readers_see_through_reader_comment_wrappers() {
        let mut b = Builder::new();
        // A field value wrapped in a comment: (comment "field doc" "hello").
        let hello = str_leaf(&mut b, "hello");
        let c1 = str_leaf(&mut b, "field doc");
        let commented_value = bare_ctor(&mut b, "comment", vec![c1, hello]);
        let n = uint_leaf(&mut b, 200);
        let rec = record(&mut b, vec![("field", commented_value), ("n", n)]);
        // The whole record wrapped in a comment: (comment "scenario doc" #record(...)) — like cdz's output.
        let c2 = str_leaf(&mut b, "scenario doc");
        let commented_root = bare_ctor(&mut b, "comment", vec![c2, rec]);
        let bytes = cadenza_ast::codec::encode(&b.finish(commented_root));

        let arenas = decode(&bytes).expect("decodes");
        // record_field sees through the comment on the root; read_str through the comment on the value.
        let field =
            record_field(&arenas, arenas.root, "field").expect("finds field past root comment");
        assert_eq!(read_str(&arenas, field).as_deref(), Some("hello"));
        let n = record_field(&arenas, arenas.root, "n").expect("finds n");
        assert_eq!(read_uint(&arenas, n), Some(200));
    }

    #[test]
    fn bool_leaf_round_trips_and_is_not_a_name() {
        // A `Leaf::Bool` reads back via read_bool; as_name does NOT see it (the trap the harness parser hit).
        let mut b = Builder::new();
        let t = bool_leaf(&mut b, true);
        let f = bool_leaf(&mut b, false);
        let rec = record(&mut b, vec![("t", t), ("f", f)]);
        let bytes = cadenza_ast::codec::encode(&b.finish(rec));
        let arenas = decode(&bytes).expect("decodes");
        let tf = record_field(&arenas, arenas.root, "t").unwrap();
        let ff = record_field(&arenas, arenas.root, "f").unwrap();
        assert_eq!(read_bool(&arenas, tf), Some(true));
        assert_eq!(read_bool(&arenas, ff), Some(false));
        // A bool is not a name, and a non-bool (a str "true") is not a bool.
        assert_eq!(arenas.as_name(unascribe(&arenas, tf)), None);
        let mut b2 = Builder::new();
        let s = str_leaf(&mut b2, "true");
        let rec2 = record(&mut b2, vec![("s", s)]);
        let arenas2 = decode(&cadenza_ast::codec::encode(&b2.finish(rec2))).unwrap();
        let sf = record_field(&arenas2, arenas2.root, "s").unwrap();
        assert_eq!(read_bool(&arenas2, sf), None, "a string is not a bool");
    }
}
