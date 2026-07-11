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

    // Ground types — a record whose META channel `(meta t)` holds the type-value. Using `Bool` in
    // type position projects `(meta t)`; it is not applyable (no `(meta apply)`).
    names.insert("Bool".to_string(), ground_type_record(ast, "Bool"));
    names.insert("Unit".to_string(), ground_type_record(ast, "Unit"));

    // Type constructors — a record whose META channel `(meta apply)` holds the native builder. `(Int
    // a)` / `(-> A B)` are ORDINARY applications: project `(meta apply)`, apply it. `Int`/`UInt` build
    // a width-specialized integer MODULE; `->` builds a function type-value.
    names.insert("Int".to_string(), ctor_record(ast, "Int"));
    names.insert("UInt".to_string(), ctor_record(ast, "UInt"));
    names.insert("->".to_string(), ctor_record(ast, "->"));

    // The binary INTEGER operators — records whose META channel carries their type (`(meta t)`, a
    // compile-time type-lambda) and their reduction (`(meta apply)`, the intrinsic). `(+ a b)` is the
    // application of the value `+` resolves to — the SAME mechanism every application uses, dispatched
    // by the head's meta channel, never by an operator name the resolver special-cases. Arithmetic,
    // division, shift, and bitwise all share the width-generic `∀a. (Int a) → (Int a) → (Int a)` type.
    for op in ["+", "-", "*", "/", "%", "<<", ">>", "&", "|", "^"] {
        names.insert(op.to_string(), operator_record(ast, op, OpShape::IntBinary));
    }

    // The relational comparisons — `∀a. a → a → Bool`. The operand is a BARE type variable (it relates
    // `Bool` and structurally any value, not only integers) and the result is `Bool`. Same operator-
    // record mechanism; only the `(meta t)` type-lambda differs.
    for op in ["<", ">", "<=", ">=", "="] {
        names.insert(op.to_string(), operator_record(ast, op, OpShape::Comparison));
    }

    // `Int64` — the pre-installed width-64 integer module (the module `(Int 64)` reduces to). Its
    // fields (`max`/`min`/…) are the width-64 specialization; reached by the ordinary projection.
    names.insert("Int64".to_string(), int64_record(ast));

    names
}

/// An `(intrinsic NAME)` node — the arena form a native primitive value takes. `resolve` turns it
/// into a `Resolved::Prim`; the name selects which primitive.
fn intrinsic_node(ast: &mut Arenas, name: &str) -> StructId {
    let head = push_atom(ast, Leaf::Name("intrinsic".to_string()));
    let who = push_atom(ast, Leaf::Name(name.to_string()));
    push_list(ast, vec![head, who])
}

/// A meta field `((meta KEY) VALUE)` — a record field whose key is the `meta`-namespaced symbol
/// `KEY`. This is how the reserved meta channel is written as ordinary record structure.
fn meta_field(ast: &mut Arenas, key: &str, value: StructId) -> StructId {
    let meta_head = push_atom(ast, Leaf::Name("meta".to_string()));
    let key_name = push_atom(ast, Leaf::Name(key.to_string()));
    let meta_key = push_list(ast, vec![meta_head, key_name]);
    push_list(ast, vec![meta_key, value])
}

/// A ground-type record `(record ((meta t) (intrinsic PRIM)))` — `Bool`/`Unit`. Its `(meta t)` holds
/// the ground type-value; it carries no `(meta apply)`, so it is not applyable.
fn ground_type_record(ast: &mut Arenas, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Name("record".to_string()));
    let ty_val = intrinsic_node(ast, prim);
    let t_field = meta_field(ast, "t", ty_val);
    push_list(ast, vec![head, t_field])
}

/// A type-constructor record `(record ((meta apply) (intrinsic PRIM)))` — `Int`/`UInt`/`->`. Applying
/// it (`(Int a)`) projects `(meta apply)` and applies the native builder.
fn ctor_record(ast: &mut Arenas, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Name("record".to_string()));
    let builder = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", builder);
    push_list(ast, vec![head, apply_field])
}

/// The type shape of a binary operator — which `(meta t)` type-lambda it carries. Both are ordinary
/// AST written entirely in terms of the grammar (`fn`/`->`/`Int`), reduced to a `Scheme` by the one
/// evaluator; the shape only selects which lambda body is built.
#[derive(Clone, Copy)]
enum OpShape {
    /// `∀a. (Int a) → (Int a) → (Int a)` — the width-generic integer binary operators.
    IntBinary,
    /// `∀a. a → a → Bool` — the relational comparisons (bare operand var, `Bool` result).
    Comparison,
}

/// An operator record `(record ((meta t) TYPE-LAMBDA) ((meta apply) (intrinsic PRIM)))`. `(meta t)`
/// is the operator's type — a compile-time type-lambda read generically by `infer`; `(meta apply)` is
/// the reduction, read by `lower`. `shape` selects the type-lambda (integer-binary vs comparison).
fn operator_record(ast: &mut Arenas, op: &str, shape: OpShape) -> StructId {
    let head = push_atom(ast, Leaf::Name("record".to_string()));
    let lambda = match shape {
        OpShape::IntBinary => binop_type_lambda(ast),
        OpShape::Comparison => comparison_type_lambda(ast),
    };
    let t_field = meta_field(ast, "t", lambda);
    let prim = intrinsic_node(ast, op);
    let apply_field = meta_field(ast, "apply", prim);
    push_list(ast, vec![head, t_field, apply_field])
}

/// The type-lambda `(fn (a) (-> (Int a) (-> (Int a) (Int a))))` shared by the binary arithmetic
/// operators — generic over the integer type: a lambda over the width `a`, whose body is the curried
/// function type built from the `Int` constructor applied to that same `a` in each position. Written
/// entirely as ordinary AST (lambda + applications) so `infer` reduces it through the one evaluator to
/// a `Scheme`, with `a` an ordinary lambda parameter.
fn binop_type_lambda(ast: &mut Arenas) -> StructId {
    // `(Int a)` — reused shape; each occurrence references the same parameter name `a`.
    let int_a = |ast: &mut Arenas| -> StructId {
        let int = push_atom(ast, Leaf::Name("Int".to_string()));
        let a = push_atom(ast, Leaf::Name("a".to_string()));
        push_list(ast, vec![int, a])
    };
    // `(-> (Int a) (-> (Int a) (Int a)))` — curried binary.
    let arrow = |ast: &mut Arenas, l: StructId, r: StructId| -> StructId {
        let arr = push_atom(ast, Leaf::Name("->".to_string()));
        push_list(ast, vec![arr, l, r])
    };
    let ia1 = int_a(ast);
    let ia2 = int_a(ast);
    let ia3 = int_a(ast);
    let inner = arrow(ast, ia2, ia3);
    let body = arrow(ast, ia1, inner);
    // `(fn (a) BODY)`.
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
    let a_param = push_atom(ast, Leaf::Name("a".to_string()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
}

/// The type-lambda `(fn (a) (-> a (-> a Bool)))` shared by the relational comparisons — generic over
/// the operand type `a` (a BARE parameter, so it unifies with `Bool` or an integer or, structurally,
/// any value), with a `Bool` result. `Bool` here is the ground-type prelude name the evaluator reduces
/// to `Ty::Bool`. Written as ordinary AST so `infer` reduces it to a `Scheme` `∀a. a → a → Bool`, with
/// `a` an ordinary lambda parameter — the same generic mechanism as the arithmetic lambda, differing
/// only in that the operand is the bare variable rather than `(Int a)`.
fn comparison_type_lambda(ast: &mut Arenas) -> StructId {
    // A bare reference to the parameter `a`.
    let a_ref = |ast: &mut Arenas| -> StructId { push_atom(ast, Leaf::Name("a".to_string())) };
    let arrow = |ast: &mut Arenas, l: StructId, r: StructId| -> StructId {
        let arr = push_atom(ast, Leaf::Name("->".to_string()));
        push_list(ast, vec![arr, l, r])
    };
    let a1 = a_ref(ast);
    let a2 = a_ref(ast);
    let bool_res = push_atom(ast, Leaf::Name("Bool".to_string()));
    let inner = arrow(ast, a2, bool_res); // (-> a Bool)
    let body = arrow(ast, a1, inner); // (-> a (-> a Bool))
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
    let a_param = push_atom(ast, Leaf::Name("a".to_string()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
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
