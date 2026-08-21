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
use cadenza_ast::ast::{Builder, Leaf, Struct, StructId};
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

/// A [`Hash`] read from a `Bytes` leaf of exactly `Hash::LEN` bytes, or `None`.
#[must_use]
pub fn read_hash(arenas: &cadenza_ast::ast::Arenas, id: StructId) -> Option<Hash> {
    let bytes = read_bytes(arenas, id)?;
    Some(Hash::from_bytes(
        <[u8; Hash::LEN]>::try_from(bytes.as_ref()).ok()?,
    ))
}
