//! The prelude — ONE cached `HashMap<String, Hir>` holding EVERY built-in binding: the prelude sum
//! constructors (`Some`/`None`/`Ok`/`Err` bare + `Option`/`Result`/`Sign`/`Ordering`/`Ast` as records
//! of their constructor values) AND the built-in modules (`Int64`/`Int`/`Bytes`/`List`/`String`/`Map`/
//! `Set` as records of their operations). Resolve consults exactly this one map — a bare name and a
//! `(. obj field)` member access both just look `obj`/`name` up in it. Nothing built-in is named
//! anywhere else; a new built-in is one entry here.
//!
//! The map is built ONCE (cached in a `OnceLock`) from the process-global `SumDef` singletons in
//! [`crate::ty`] (so a constructor's `SumRef` identity is `Arc::ptr_eq` everywhere) and the intrinsic
//! table. Resolve CLONES it per program and layers the program's own `(type …)` declarations on top
//! (an Arc clone is a refcount bump — identity preserved). The `SumDef` allocations live in
//! [`crate::ty`] because intrinsic signatures need the `SumRef`s too; this is the resolve-facing view.

use crate::ir::{HeapIntrinsic, Hir, Intrinsic};
use crate::ty::SumRef;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The prelude scope: a name → the `Hir` it resolves to (a bare constructor value, a sum type's
/// constructor record, or a built-in module's operation record). Extended per program with its
/// `(type …)` declarations (resolve does that on a clone of [`all`]).
pub type Prelude = HashMap<String, Hir>;

static PRELUDE: OnceLock<Prelude> = OnceLock::new();

/// The cached built-in prelude map. Built once; resolve clones it and adds the program's own types.
pub fn all() -> &'static Prelude {
    PRELUDE.get_or_init(build)
}

/// The `SumRef` a prelude/user type NAME denotes, recovered from its constructor RECORD in `p`: a sum
/// type binds to a `Hir::Record` whose fields are `Hir::Ctor` values, each carrying the shared
/// `SumRef` — so the def is read off any one of them. `None` if `name` is unbound or not a sum (a
/// built-in MODULE record holds `Hir::Intrinsic`s, not `Ctor`s). Used by the payload-type parser to
/// resolve a type-expression naming a sum (prelude OR user — both are ctor records in the one map).
pub fn sum_ref(p: &Prelude, name: &str) -> Option<SumRef> {
    match p.get(name) {
        Some(Hir::Record(fields)) => fields.iter().find_map(|(_, v)| match v {
            Hir::Ctor { def, .. } => Some(def.clone()),
            _ => None,
        }),
        _ => None,
    }
}

/// Build the one prelude map: the sum constructors + the built-in module records.
fn build() -> Prelude {
    let mut p: Prelude = HashMap::new();

    // `unit` — the empty-product value. A named built-in like any other, so it lives here (resolve just
    // looks it up); `()` is its literal spelling (an empty list, handled structurally in resolve).
    p.insert("unit".to_string(), Hir::Unit);

    // ── Layer 1 first-class types: pure scalar type names (those without operations) as compile-time
    //    values. A bare `Bool` or `Unit` resolves to a `TypeVal` (typed as `Ty::Type`). Module types
    //    (`Int64`, `Bytes`, `String`, etc.) are handled specially in resolve.rs (bare → TypeVal, but
    //    `(. Int64 wrapping-add)` still accesses the module record). ──
    p.insert("Bool".to_string(), Hir::TypeVal(crate::ty::Ty::Bool));
    p.insert("Unit".to_string(), Hir::TypeVal(crate::ty::Ty::Unit));

    // ── Layer 2: parametric type constructors as first-class compile-time values. Each parametric
    //    type name (`List`, `Map`, `Set`, `Tuple`, `Option`, `Result`) is a `TypeCtor` value that, when
    //    applied to type-value arguments, β-reduces to a TypeVal of the constructed type. `(List Int64)`
    //    → `TypeVal(Ty::List(Int))`. The fold handles the β-reduction. ──
    use crate::ir::TypeCtorKind;
    p.insert("List".to_string(), Hir::TypeCtor(TypeCtorKind::List));
    p.insert("Map".to_string(), Hir::TypeCtor(TypeCtorKind::Map));
    p.insert("Set".to_string(), Hir::TypeCtor(TypeCtorKind::Set));
    p.insert("Tuple".to_string(), Hir::TypeCtor(TypeCtorKind::Tuple2)); // 2-arity for now
    // Option and Result are already in the prelude as sum constructors; we need to also bind them as
    // type constructors for use in type position. For now, skip them (they're handled via parse_type_expr
    // which we'll delete after this works).

    // ── The prelude SUM types. Declaration order fixes discriminants (Option Some=0/None=1, …). A type
    //    NAME binds to a record of its constructor values (`(. Sign Pos)` = record projection); an
    //    UNQUALIFIED sum ALSO binds each variant bare (`Some`/`None`/`Ok`/`Err`). Built from the
    //    process-global singletons so identity is `Arc::ptr_eq` everywhere. ──
    for def in [
        crate::ty::prelude_option(),
        crate::ty::prelude_result(),
        crate::ty::prelude_sign(),
        crate::ty::prelude_ordering(),
        crate::ty::prelude_ast(),
    ] {
        let ctor = |i| Hir::Ctor {
            def: def.clone(),
            index: i,
        };
        let fields: Vec<(String, Hir)> = def
            .variants()
            .iter()
            .enumerate()
            .map(|(i, v)| (v.name.clone(), ctor(i)))
            .collect();
        if !def.qualified {
            for (i, v) in def.variants().iter().enumerate() {
                p.insert(v.name.clone(), ctor(i));
            }
        }
        p.insert(def.name.clone(), Hir::Record(fields));
    }

    // ── The built-in MODULES, each a record of its operations (core-semantics.md §A Built-In Module Is
    //    A Record Of Its Operations). A bare `Int64` resolves to its record; `(. Int64 wrapping-add)` is
    //    ordinary projection. Each is PARTIAL — it lists only the realized ops, so an unlisted field
    //    DECLINES (a later phase), never a wrong emit. A `Heap` op boxes a scalar arg (its lowering
    //    threads the solved type via `Mir::HeapOp`); a plain `Intrinsic` does not. ──
    let op = |i: Intrinsic| Hir::Intrinsic(i);
    let heap = |h: HeapIntrinsic| Hir::Intrinsic(Intrinsic::Heap(h));
    let module = |fields: Vec<(&str, Hir)>| {
        Hir::Record(
            fields
                .into_iter()
                .map(|(n, v)| (n.to_string(), v))
                .collect(),
        )
    };

    // `Int64`: value constants + wrapping arithmetic. `checked-*` / trapping operators land later.
    p.insert(
        "Int64".to_string(),
        module(vec![
            ("max", Hir::Int(i64::MAX)),
            ("min", Hir::Int(i64::MIN)),
            ("wrapping-add", op(Intrinsic::WrappingAdd)),
            ("wrapping-sub", op(Intrinsic::WrappingSub)),
            ("wrapping-mul", op(Intrinsic::WrappingMul)),
            ("to-byte", op(Intrinsic::IntToByte)),
        ]),
    );
    // `Bytes`: build/measure/append + the fallible reads (`at`/`slice` → Option) + `compact` (re-base
    // a slice to release its parent buffer).
    p.insert(
        "Bytes".to_string(),
        module(vec![
            ("of", op(Intrinsic::BytesOf)),
            ("len", op(Intrinsic::BytesLen)),
            ("concat", op(Intrinsic::BytesConcat)),
            ("at", op(Intrinsic::BytesAt)),
            ("slice", op(Intrinsic::BytesSlice)),
            ("compact", op(Intrinsic::BytesCompact)),
        ]),
    );
    // `List`: `len`/`push`/`concat`/`at`. `push` is a `Heap` op (boxes a scalar element by its solved type).
    p.insert(
        "List".to_string(),
        module(vec![
            ("len", op(Intrinsic::ListLen)),
            ("push", heap(HeapIntrinsic::ListPush)),
            ("concat", op(Intrinsic::ListConcat)),
            ("at", op(Intrinsic::ListAt)),
        ]),
    );
    // `String`: the TOTAL UTF-8 decode `from-bytes` (→ Option String). Still PARTIAL (scalar-indexed ops later).
    p.insert(
        "String".to_string(),
        module(vec![("from-bytes", op(Intrinsic::StrFromBytes))]),
    );
    // `Map K V`: the CHAMP key→value map. `Map.empty` is the empty-map VALUE — the same `(map)` literal
    // node (an empty `Hir::Map`), NOT a nullary op, so it needs no function-value handling and shares the
    // literal's lowering/render. `size` is plain; `insert`/`lookup`(→Option V)/`remove` are `Heap` ops
    // (box a scalar key/value). A map's key SET is runtime data, NOT part of its type.
    p.insert(
        "Map".to_string(),
        module(vec![
            ("empty", Hir::Map(vec![])),
            ("insert", heap(HeapIntrinsic::MapInsert)),
            ("lookup", heap(HeapIntrinsic::MapLookup)),
            ("remove", heap(HeapIntrinsic::MapRemove)),
            ("size", op(Intrinsic::MapSize)),
        ]),
    );
    // `Set E`: the CHAMP set. `Set.empty` is the empty-set VALUE (the `(set)` literal node). `of`(from a
    // List)/`insert`/`contains`(→Bool)/`remove` are `Heap` ops (box a scalar element); `size`/`len` + the
    // algebra are plain. `len` is an alias of `size`.
    p.insert(
        "Set".to_string(),
        module(vec![
            ("empty", Hir::Set(vec![])),
            ("of", heap(HeapIntrinsic::SetOf)),
            ("insert", heap(HeapIntrinsic::SetInsert)),
            ("contains", heap(HeapIntrinsic::SetContains)),
            ("remove", heap(HeapIntrinsic::SetRemove)),
            ("size", op(Intrinsic::SetSize)),
            ("len", op(Intrinsic::SetSize)),
            ("union", op(Intrinsic::SetUnion)),
            ("intersection", op(Intrinsic::SetIntersection)),
            ("difference", op(Intrinsic::SetDifference)),
        ]),
    );

    p
}
