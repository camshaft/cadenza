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

/// Strip an optional ascription, returning the inner value (or `id` unchanged if not ascribed).
#[must_use]
pub fn unascribe(arenas: &Arenas, id: StructId) -> StructId {
    as_ascribed(arenas, id).unwrap_or(id)
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

/// The members of a `#list(…)` value, or `None` if `id` is not a list.
#[must_use]
pub fn read_list(arenas: &Arenas, id: StructId) -> Option<&[StructId]> {
    arenas.compound_form_of(id, CompoundCtor::List)
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

/// A `String` leaf's text.
#[must_use]
pub fn read_str(arenas: &Arenas, id: StructId) -> Option<String> {
    arenas.as_str(id).map(str::to_string)
}

/// A `Bytes` leaf's bytes.
#[must_use]
pub fn read_bytes(arenas: &Arenas, id: StructId) -> Option<Bytes> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Bytes(bytes) => Some(Bytes::copy_from_slice(bytes)),
            _ => None,
        },
        Struct::List(_) => None,
    }
}

/// An integer leaf's value as a `u64`, or `None` if not an integer / negative / too large.
#[must_use]
pub fn read_uint(arenas: &Arenas, id: StructId) -> Option<u64> {
    match arenas.get(id) {
        Struct::Atom(leaf) => match arenas.leaf(*leaf) {
            Leaf::Int { value, .. } => value.to_u128().and_then(|u| u64::try_from(u).ok()),
            _ => None,
        },
        Struct::List(_) => None,
    }
}
