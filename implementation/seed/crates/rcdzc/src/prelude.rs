//! The prelude — the ONE map of built-in bindings the resolver consults by name.
//!
//! A built-in is NOT a special value world. A built-in module is *just a record*, and a record is
//! already an ordinary node in the AST arena — so the prelude is installed as real AST nodes appended
//! to the program's arena, and a built-in binding is simply a `StructId` like any program node. There
//! is no `Module` type and no parallel "builtin value" enum: `Int64` binds to a `(record (max …)
//! (min …))` node, and `Int64.max` reaches its field through the EXACT same member-access-and-fold
//! path as a program's `p.x` (`resolve` returns a `Ref` to the prelude record; `type_of`/`core_of`
//! already follow a `Ref` into a record). This is "records everywhere" taken to the representation:
//! nothing is privileged by name (`reference-compiler.md` §Nothing Is Privileged By Name) OR by shape.
//!
//! Resolving a name is one ordered lookup — the lexical scope, then this map (`prelude-and-
//! resolution.md` §The Prelude Is A Single Map The Resolver Consults By Name Alone). A program binding
//! shadows a built-in of the same name because `resolve` searches the scope FIRST — no special case.
//!
//! No open-vs-closed rule. A built-in module carries EVERY field it will ever have; a field for an
//! operation not yet realized is filled with an `(unrealized …)` node that resolves to a DECLINE. So
//! projecting an unrealized field declines through the ordinary member-access-then-poison path — the
//! same CLOSED record projection every user record takes — and there is no "the module is open"
//! branch anywhere. An unimplemented built-in is a capability the compiler lacks (a decline), reached
//! by exactly the mechanism a program record uses.
//!
//! Stage-1 scope: `Int64`'s `max`/`min` are realized as folding constants (the most-witnessed scalar
//! built-in); its arithmetic/conversion operations — which need the checked-arith machinery — are
//! present as `unrealized` fields, so referencing one declines cleanly rather than reading as unbound.

use crate::ast::{Arenas, IntValue, Leaf, LeafId, Radix, Struct, StructId};
use std::collections::BTreeMap;

/// Install the prelude into `ast`, appending its built-in bindings as ordinary AST nodes and
/// returning the `name → node` map. The appended nodes take `StructId`s AFTER the program's, so no
/// program id shifts (byte identity of program-derived facts is preserved). Deterministic: the
/// prelude is a fixed function of nothing.
pub fn install(ast: &mut Arenas) -> BTreeMap<String, StructId> {
    let mut names = BTreeMap::new();
    // `Int64` — a record of the type's bounds. Reached by `(. Int64 max)` = the ordinary projection.
    names.insert("Int64".to_string(), int64_record(ast));
    // The arithmetic operators — each a built-in OPERATION value, installed as an `(intrinsic name)`
    // arena node. `(+ a b)` is the application of the value `+` resolves to (the same mechanism a
    // user function application uses) — NOT an operator name the resolver special-cases. Each is
    // generic over the integer type (one width variable shared by operands and result).
    for op in ["+", "-", "*"] {
        names.insert(op.to_string(), intrinsic_node(ast, op));
    }
    names
}

/// Append an `(intrinsic NAME)` node and return it — the arena form a built-in operation value takes,
/// mirroring the `(unrealized …)` and `(record …)` prelude nodes. `resolve` turns it into a
/// `Resolved::Intrinsic`; the name says which operation.
fn intrinsic_node(ast: &mut Arenas, name: &str) -> StructId {
    let head = push_atom(ast, Leaf::Name("intrinsic".to_string()));
    let who = push_atom(ast, Leaf::Name(name.to_string()));
    push_list(ast, vec![head, who])
}

/// Append the `Int64` module as a genuine `(record …)` form and return its root occurrence, so
/// `resolve` classifies it via the same `resolve_record` path a program record takes. It carries
/// EVERY witnessed field: `max`/`min` as realized constants, and the arithmetic/conversion operations
/// as `unrealized` fields (each declines when projected). No field is absent, so there is no
/// open-module case.
fn int64_record(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Name("record".to_string()));
    let mut fields = vec![
        int_field(ast, "max", i64::MAX),
        int_field(ast, "min", i64::MIN),
    ];
    // Operations not yet realized — present, but their value declines when projected.
    for op in ["of", "checked-add", "checked-mul", "wrapping-add", "wrapping-mul"] {
        fields.push(unrealized_field(ast, op));
    }
    let mut children = vec![head];
    children.append(&mut fields);
    push_list(ast, children)
}

/// A `(name value)` record field whose value is an integer constant.
fn int_field(ast: &mut Arenas, name: &str, value: i64) -> StructId {
    let k = push_atom(ast, Leaf::Name(name.to_string()));
    let v = push_atom(ast, Leaf::Int { value: IntValue::from_i64(value), radix: Radix::Dec });
    push_list(ast, vec![k, v])
}

/// A `(name (unrealized name))` record field: the field exists, but its value is an `unrealized`
/// form that `resolve` turns into a decline — so projecting it declines by the ordinary path, no
/// open-module special case. The op name rides along so the decline can say which operation it is.
fn unrealized_field(ast: &mut Arenas, name: &str) -> StructId {
    let k = push_atom(ast, Leaf::Name(name.to_string()));
    let head = push_atom(ast, Leaf::Name("unrealized".to_string()));
    let who = push_atom(ast, Leaf::Name(name.to_string()));
    let v = push_list(ast, vec![head, who]);
    push_list(ast, vec![k, v])
}

/// Append a leaf and an `Atom` occurrence of it, returning the occurrence's id. (No dedup — the
/// prelude is small and its leaves need not be interned against the program's.)
fn push_atom(ast: &mut Arenas, leaf: Leaf) -> StructId {
    let lid = LeafId(ast.leaves.len() as u32);
    ast.leaves.push(leaf);
    let sid = StructId(ast.structure.len() as u32);
    ast.structure.push(Struct::Atom(lid));
    sid
}

/// Append a `List` occurrence, returning its id.
fn push_list(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let sid = StructId(ast.structure.len() as u32);
    ast.structure.push(Struct::List(children));
    sid
}
