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
//! What is realized on an integer-width module: `max`/`min` (folding bound constants), `wrap` (the
//! truncating conversion). What remains an `unrealized` field, declining cleanly rather than reading
//! as unbound: `of` (the checked conversion — it returns `Option<T>`, so it waits on sum types) and
//! `checked-*`/`wrapping-*`. The binary arithmetic and comparison operators (`+ - * / …`, `< = …`)
//! are realized as top-level prelude operators, not module fields.

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

    // The unit VALUE, bound to the bare name `unit` — an alias for the empty list `()`, the other
    // spelling of the same value (core-semantics.md #Unit And The Empty Tuple Are The Same Value;
    // 01-literals "unit and the empty tuple are the same value"). An empty-list node resolves to
    // `Resolved::Unit` exactly as a source `()` does, so `unit` and `()` are interchangeable in value
    // position — this is what lets the pervasive nullary-variant idiom `(None unit)` / `(Sign.Pos unit)`
    // and the direct `(input unit)` case RUN, rather than declining "unbound name `unit`". (`Unit`,
    // capitalized, is the TYPE above; `unit` is the value.)
    names.insert("unit".to_string(), push_list(ast, vec![]));

    // Type constructors — a record whose META channel `(meta apply)` holds the native builder. `(Int
    // a)` / `(-> A B)` are ORDINARY applications: project `(meta apply)`, apply it. `Int`/`UInt` build
    // a width-specialized integer MODULE; `->` builds a function type-value.
    names.insert("Int".to_string(), ctor_record(ast, "Int"));
    names.insert("UInt".to_string(), ctor_record(ast, "UInt"));
    names.insert("->".to_string(), ctor_record(ast, "->"));
    // `Tuple` — the tuple-type constructor, VARIADIC over its element types: `(Tuple Int64 Bool)` builds
    // the tuple type-value. Same `(meta apply)` mechanism as `->`; only the builder differs.
    names.insert("Tuple".to_string(), ctor_record(ast, "Tuple"));
    // `Record` — the record-type constructor, VARIADIC over `(name type)` field pairs: `(Record (a
    // Int64) (b Bool))` builds the record type-value. Same `(meta apply)` mechanism as `Tuple`; the
    // builder reads each arg as a `(name type)` pair.
    names.insert("Record".to_string(), ctor_record(ast, "Record"));

    // The compound-VALUE constructors as SHADOWABLE aliases. The primitive is a symbol head (`(,)`
    // builds a tuple, `{}` builds a record — dispatched structurally in `resolve`), but the ordinary
    // names `tuple`/`record` are prelude records here so `(tuple 1 2)` / `(record (x 1))` written with
    // the NAME are ordinary applications: their `(meta apply)` holds the value-constructor intrinsic,
    // and being ordinary names they are SHADOWABLE (a local `(let ((tuple …)) …)` wins via the
    // scope-first lookup, never reaching this entry). This is what removes the head-vs-value resolution
    // split — the name is looked up, the symbol is the unspellable primitive.
    names.insert("tuple".to_string(), ctor_record(ast, "tuple-new"));
    names.insert("record".to_string(), ctor_record(ast, "record-new"));
    // `list` — the list-VALUE constructor alias (`(list 1 2 3)`), variadic + homogeneous → `Ty::List`.
    names.insert("list".to_string(), ctor_record(ast, "list-new"));
    // `List` — BOTH the list-TYPE constructor (`(List Int64)` in type position → `(meta apply)=List`) AND
    // the module of list OPERATIONS (its `len`/… fields, reached by member access `(. List len)`). One
    // record carries both roles: applying it builds the type, projecting a field gives an operation.
    names.insert("List".to_string(), list_module(ast));

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
        names.insert(
            op.to_string(),
            operator_record(ast, op, OpShape::Comparison),
        );
    }

    // The named fixed-width integer modules — `Int8`/`Int16`/`Int32`/`Int64` and
    // `UInt8`/`UInt16`/`UInt32`/`UInt64`. Each is an ALIAS for the module `(Int N)` / `(UInt N)`
    // reduces to: a record whose `(meta t)` is that width's concrete type-value and whose `max`/`min`
    // fields are that width's bounds (`UInt64.max = 2^64-1`, exact). Built by the SAME width-generic
    // builder the constructor uses, so a named width and `(Int N)` denote the same module — nothing is
    // special-cased per name. `(Int N)` for any other width (odd ones like `(UInt 7)`) is built on
    // demand by the constructor; these are just the commonly-written names pre-installed.
    for (name, signed, width) in [
        ("Int8", true, 8u32),
        ("Int16", true, 16),
        ("Int32", true, 32),
        ("Int64", true, 64),
        ("UInt8", false, 8),
        ("UInt16", false, 16),
        ("UInt32", false, 32),
        ("UInt64", false, 64),
    ] {
        names.insert(name.to_string(), int_module_record(ast, signed, width));
    }

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
/// `KEY`. This is how the reserved meta channel is written as ordinary record structure. `pub(crate)`
/// so the program-driven sum-record synthesis (`sum_synth`) writes its `(meta t)`/`(meta variant)`
/// channels the same way the prelude writes its built-in records.
pub(crate) fn meta_field(ast: &mut Arenas, key: &str, value: StructId) -> StructId {
    let meta_head = push_atom(ast, Leaf::Name("meta".to_string()));
    let key_name = push_atom(ast, Leaf::Name(key.to_string()));
    let meta_key = push_list(ast, vec![meta_head, key_name]);
    push_list(ast, vec![meta_key, value])
}

/// A ground-type record `(record ((meta t) (intrinsic PRIM)))` — `Bool`/`Unit`. Its `(meta t)` holds
/// the ground type-value; it carries no `(meta apply)`, so it is not applyable.
fn ground_type_record(ast: &mut Arenas, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let ty_val = intrinsic_node(ast, prim);
    let t_field = meta_field(ast, "t", ty_val);
    push_list(ast, vec![head, t_field])
}

/// A type-constructor record `(record ((meta apply) (intrinsic PRIM)))` — `Int`/`UInt`/`->`. Applying
/// it (`(Int a)`) projects `(meta apply)` and applies the native builder.
fn ctor_record(ast: &mut Arenas, prim: &str) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let builder = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", builder);
    push_list(ast, vec![head, apply_field])
}

/// The `List` module record — a record carrying BOTH `(meta apply)` = the `List` type constructor (so
/// `(List Int64)` in type position builds `Ty::List`) AND a field per list OPERATION (reached by member
/// access `(. List len)`). Each operation is an operator record: its `(meta t)` is a type-lambda over
/// the element type, its `(meta apply)` the runtime op. This increment realizes `len : ∀a. (List a) →
/// Int64`; push/concat/at arrive in the next increment (a projected-but-unrealized field DECLINES, the
/// same closed-module rule every prelude module follows).
fn list_module(ast: &mut Arenas) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    // `(meta apply)` = the `List` TYPE constructor (`(List Int64)` reduces to `Ty::List(Int64)`).
    let builder = intrinsic_node(ast, "List");
    let apply_field = meta_field(ast, "apply", builder);
    // One field per realized operation — each an operator record `(name <op-record>)`. `len : ∀a. (List
    // a) → Int64`; `push : ∀a. (List a) → a → (List a)`; `concat : ∀a. (List a) → (List a) → (List a)`.
    // Each lambda is built first (a `&mut ast` borrow) then handed to `list_op_record`.
    let len_lambda = list_len_type_lambda(ast);
    let push_lambda = list_push_type_lambda(ast);
    let concat_lambda = list_concat_type_lambda(ast);
    let mut children = vec![head, apply_field];
    for (name, prim, lambda) in [
        ("len", "list-len", len_lambda),
        ("push", "list-push", push_lambda),
        ("concat", "list-concat", concat_lambda),
    ] {
        let op = list_op_record(ast, prim, lambda);
        let k = push_atom(ast, Leaf::Name(name.to_string()));
        children.push(push_list(ast, vec![k, op]));
    }
    push_list(ast, children)
}

/// An operation record for a `List` module field: `(record ((meta t) TYPE-LAMBDA) ((meta apply)
/// (intrinsic PRIM)))` — the same shape as `operator_record`, but the type-lambda is supplied (a list
/// operation's signature varies per op, unlike the shared arithmetic/comparison shapes).
fn list_op_record(ast: &mut Arenas, prim: &str, type_lambda: StructId) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    let t_field = meta_field(ast, "t", type_lambda);
    let apply = intrinsic_node(ast, prim);
    let apply_field = meta_field(ast, "apply", apply);
    push_list(ast, vec![head, t_field, apply_field])
}

/// The type-lambda `(fn (a) (-> (List a) Int64))` for `List.len` — generic over the element type `a`,
/// taking a list of it and returning `Int64`. Written as ordinary AST so `infer` reduces it to the
/// scheme `∀a. (List a) → Int64` through the one evaluator (`(List a)` reduces via `Prim::ListCtor`).
fn list_len_type_lambda(ast: &mut Arenas) -> StructId {
    let list_a = list_a_type(ast);
    let int64 = push_atom(ast, Leaf::Name("Int64".to_string()));
    let body = arrow_type(ast, list_a, int64);
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> (List a) (-> a (List a))))` for `List.push` — `∀a. (List a) → a →
/// (List a)`: take a list and an element of its type, return the new list.
fn list_push_type_lambda(ast: &mut Arenas) -> StructId {
    let list_r = list_a_type(ast);
    let elem = push_atom(ast, Leaf::Name("a".to_string()));
    let inner = arrow_type(ast, elem, list_r); // (-> a (List a))
    let list_l = list_a_type(ast);
    let body = arrow_type(ast, list_l, inner); // (-> (List a) (-> a (List a)))
    list_type_lambda(ast, body)
}

/// The type-lambda `(fn (a) (-> (List a) (-> (List a) (List a))))` for `List.concat` — `∀a. (List a) →
/// (List a) → (List a)`: concatenate two lists of the same element type.
fn list_concat_type_lambda(ast: &mut Arenas) -> StructId {
    let list_r = list_a_type(ast);
    let list_2 = list_a_type(ast);
    let inner = arrow_type(ast, list_2, list_r); // (-> (List a) (List a))
    let list_1 = list_a_type(ast);
    let body = arrow_type(ast, list_1, inner); // (-> (List a) (-> (List a) (List a)))
    list_type_lambda(ast, body)
}

/// Build `(List a)` — the list type applied to the element parameter `a`, a shared shape in the `List`
/// operation type-lambdas (each occurrence is a fresh `(List a)` referencing the same parameter name).
fn list_a_type(ast: &mut Arenas) -> StructId {
    let list = push_atom(ast, Leaf::Name("List".to_string()));
    let a = push_atom(ast, Leaf::Name("a".to_string()));
    push_list(ast, vec![list, a])
}

/// Build `(-> l r)` — a function type from `l` to `r`.
fn arrow_type(ast: &mut Arenas, l: StructId, r: StructId) -> StructId {
    let arrow = push_atom(ast, Leaf::Name("->".to_string()));
    push_list(ast, vec![arrow, l, r])
}

/// Wrap `body` in `(fn (a) body)` — the one-parameter type-lambda over the element type `a`, shared by
/// the `List` operation schemes.
fn list_type_lambda(ast: &mut Arenas, body: StructId) -> StructId {
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
    let a_param = push_atom(ast, Leaf::Name("a".to_string()));
    let params = push_list(ast, vec![a_param]);
    push_list(ast, vec![fn_head, params, body])
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
    let head = push_atom(ast, Leaf::Str("record".to_string()));
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

/// Append a fixed-width integer MODULE record for `(signed, width)` and return its occurrence — the
/// value a named width (`Int64`, `UInt8`, …) binds to. Built the same way the `(Int N)`/`(UInt N)`
/// constructor builds its module (see `eval::build_int_module`), so a named width and the constructor
/// application denote the same thing. It carries a `(meta t)` — its TYPE-VALUE — so the name works IN
/// TYPE POSITION (`(: e UInt8)` reduces to `Ty::Int` via the ordinary `(meta t)` projection); its
/// `max`/`min` are that width's bounds (from the shared `eval::int_bounds`, arbitrary precision so
/// `UInt64.max = 2^64-1` is exact); its arithmetic/conversion ops are `unrealized` (decline when
/// projected). Nothing is special-cased per name — only the `(signed, width)` differs.
fn int_module_record(ast: &mut Arenas, signed: bool, width: u32) -> StructId {
    let head = push_atom(ast, Leaf::Str("record".to_string()));
    // `(meta t)` = the type expression `(Int width)` / `(UInt width)`, reduced to the concrete
    // type-value by `typeval_of`. This is what makes the name usable as a TYPE.
    let ty_expr = {
        let ctor = push_atom(
            ast,
            Leaf::Name(if signed { "Int" } else { "UInt" }.to_string()),
        );
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let mut fields = vec![meta_field(ast, "t", ty_expr)];
    // `max`/`min` — that width's bounds, at arbitrary precision (shared with the constructor's builder),
    // each ANNOTATED with the module's own width so projecting the field carries its type.
    match crate::eval::int_bounds(signed, width) {
        Some((max, min)) => {
            fields.push(int_field(ast, "max", max, signed, width));
            fields.push(int_field(ast, "min", min, signed, width));
        }
        None => {
            fields.push(unrealized_field(ast, "max"));
            fields.push(unrealized_field(ast, "min"));
        }
    }
    // `wrap` — the TRUNCATING conversion INTO this width: `∀(w,s). Int^s_w → THIS`. An operator record
    // whose `(meta t)` is `(fn (a) (-> (Int a) TARGET))` — the source `(Int a)` fully polymorphic in
    // width AND sign (the paired sign-variable), the target `TARGET` this module's own concrete width —
    // and whose `(meta apply)` is the `wrap` intrinsic. ONE such field per module (no per-source-type
    // explosion): the target is fixed by the module, the source by unification at the call site.
    fields.push(wrap_field(ast, signed, width));
    // Operations not yet realized — present, but their value declines when projected. `of` (the CHECKED
    // conversion) returns `Option<T>`; sum types now exist, so what remains is wiring `.of` to build a
    // `(Some v)` in range / `(None)` out (task #59) — until that lands it stays an unrealized field.
    for op in [
        "of",
        "checked-add",
        "checked-mul",
        "wrapping-add",
        "wrapping-mul",
    ] {
        fields.push(unrealized_field(ast, op));
    }
    let mut children = vec![head];
    children.append(&mut fields);
    push_list(ast, children)
}

/// A `(wrap (record ((meta t) TYPE-LAMBDA) ((meta apply) (intrinsic wrap))))` field — the module's
/// truncating conversion. `TYPE-LAMBDA` is `(fn (a) (-> (Int a) TARGET))`: the source is the generic
/// integer `(Int a)` (width `a` + its paired sign variable, so it accepts ANY integer), the result is
/// `TARGET` = `(Int width)` / `(UInt width)`, this module's own concrete type. `(meta apply)` is the
/// shared `wrap` intrinsic — one prim, the target read off the application's solved type at lowering.
fn wrap_field(ast: &mut Arenas, signed: bool, width: u32) -> StructId {
    // `(fn (a) (-> (Int a) TARGET))`.
    let int_a = {
        let int = push_atom(ast, Leaf::Name("Int".to_string()));
        let a = push_atom(ast, Leaf::Name("a".to_string()));
        push_list(ast, vec![int, a])
    };
    let target = {
        let ctor = push_atom(
            ast,
            Leaf::Name(if signed { "Int" } else { "UInt" }.to_string()),
        );
        let w = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(width as i64),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![ctor, w])
    };
    let arr = push_atom(ast, Leaf::Name("->".to_string()));
    let body = push_list(ast, vec![arr, int_a, target]);
    let fn_head = push_atom(ast, Leaf::Name("fn".to_string()));
    let a_param = push_atom(ast, Leaf::Name("a".to_string()));
    let params = push_list(ast, vec![a_param]);
    let lambda = push_list(ast, vec![fn_head, params, body]);
    // `(record ((meta t) lambda) ((meta apply) (intrinsic wrap)))`.
    let rec_head = push_atom(ast, Leaf::Str("record".to_string()));
    let t_field = meta_field(ast, "t", lambda);
    let prim = intrinsic_node(ast, "wrap");
    let apply_field = meta_field(ast, "apply", prim);
    let record = push_list(ast, vec![rec_head, t_field, apply_field]);
    // `(wrap record)`.
    let k = push_atom(ast, Leaf::Name("wrap".to_string()));
    push_list(ast, vec![k, record])
}

/// A `(name (: value (Int/UInt width)))` record field — an arbitrary-precision integer constant
/// ANNOTATED with the module's own width, so projecting the field yields a value typed at that width
/// (mirrors `eval::named_int_field`; the two builders must annotate identically so a named width and
/// `(Int N)` project the same typed bound).
fn int_field(ast: &mut Arenas, name: &str, value: IntValue, signed: bool, width: u32) -> StructId {
    let k = push_atom(ast, Leaf::Name(name.to_string()));
    let lit = push_atom(
        ast,
        Leaf::Int {
            value,
            radix: Radix::Dec,
        },
    );
    let ctor = push_atom(
        ast,
        Leaf::Name(if signed { "Int" } else { "UInt" }.to_string()),
    );
    let w = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(width as i64),
            radix: Radix::Dec,
        },
    );
    let ty_expr = push_list(ast, vec![ctor, w]);
    let colon = push_atom(ast, Leaf::Name(":".to_string()));
    let annot = push_list(ast, vec![colon, lit, ty_expr]);
    push_list(ast, vec![k, annot])
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
/// prelude is small and its leaves need not be interned against the program's.) `pub(crate)` so the
/// program-driven sum-record synthesis appends its atoms through the same helper.
pub(crate) fn push_atom(ast: &mut Arenas, leaf: Leaf) -> StructId {
    let lid = LeafId(ast.leaves.len() as u32);
    ast.leaves.push(leaf);
    let sid = StructId(ast.structure.len() as u32);
    ast.structure.push(Struct::Atom(lid));
    sid
}

/// Append a `List` occurrence, returning its id. `pub(crate)` so the program-driven sum-record
/// synthesis builds its lists through the same helper.
pub(crate) fn push_list(ast: &mut Arenas, children: Vec<StructId>) -> StructId {
    let sid = StructId(ast.structure.len() as u32);
    ast.structure.push(Struct::List(children));
    sid
}
