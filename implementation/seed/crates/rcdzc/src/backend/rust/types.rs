//! `Ty` → native Rust type — the Rust backend's value strategy for scalars.
//!
//! A backend that uses its target's native aggregates maps each Cadenza type to a target type
//! (`backends-and-targets.md` §A Compound Value's Representation Is The Backend's Choice). For the
//! scalar value language that is a total, obvious map: a Cadenza integer of an ALIASED width is the
//! Rust integer of that exact width and signedness (`Int8` → `i8`, `UInt32` → `u32`, `Int64` → `i64`,
//! `UInt64` → `u64`), so Cadenza's checked-overflow arithmetic becomes Rust's `checked_*` on the same
//! type — no representation gap. Bool → `bool`, Unit → `()`.
//!
//! A NON-ALIASED width (`UInt7`, `UInt24`, `UInt48`, …) has no native Rust type, so it maps to `None`
//! and the caller declines — the SAME boundary the wasm backend draws (`comp_valtype_of` returns
//! `None` for a non-standard width, so it cannot cross the component boundary either): a narrow
//! non-standard width is a fine INTERNAL type but has no wire/native form, so a value of it must be
//! converted (`.wrap`) to an aliased width before it crosses out. A compound, function, or type value
//! also maps to `None` in this scalar slice (compounds arrive with the native-aggregate strategy in a
//! later increment).

use crate::backend::Db;
use crate::ty::{IntTy, Sign, Ty, Width};

/// The native Rust type for a solved Cadenza type, or `None` if this backend has no native
/// representation for it (a non-aliased integer width, a not-yet-supported compound, or an
/// unresolved/erased type). The caller turns a `None` into a decline attributed to this target.
///
/// Returns an owned `String` because a compound type is a COMPOSED spelling (a tuple `(T0, T1)`), not a
/// fixed primitive name. A scalar's mapping is still one of a fixed set (`int_type`/`bool`/`()`).
pub fn rust_type(ncx: &crate::ty::NameCtx, ty: &Ty) -> Option<String> {
    match ty {
        Ty::Int(it) => int_type(*it).map(String::from),
        // A float maps to Rust's native `f64`/`f32` by its width — the admitted {32,64} are exactly
        // Rust's two IEEE float types. (A non-admitted width never reaches here — it is CDZ0302.)
        Ty::Float(ft) => Some(
            if ft.ground_width() == 32 {
                "f32"
            } else {
                "f64"
            }
            .to_string(),
        ),
        Ty::Bool => Some("bool".to_string()),
        Ty::Unit => Some("()".to_string()),
        // An arbitrary-precision integer maps to `cdz_num::Big` — the SAME bignum the wasm runtime uses,
        // shared by SOURCE (`cdz-num` `#[path]`-includes the runtime's `bigint.rs`; see that crate). A
        // `Big` is a value type (`Clone + Eq`, total `cmp`), so it composes as a tuple/record element, a
        // sum payload, a `Vec`/`BTreeSet`/`BTreeMap` element/key. Non-Copy (owns a limb `Vec`) → the
        // clone-on-read discipline covers a shared BigInt (`ty_is_non_copy` includes it). The gate links
        // the `cdz-num` rlib via `--extern cdz_num`.
        Ty::BigInt => Some("cdz_num::Big".to_string()),
        // An exact rational maps to `cdz_num::Rational` (a `Big` num/den pair in the runtime's canonical
        // normalized form). Its ops mirror the wasm runtime's `rational-*` byte-for-byte, so a rust program
        // computes the same rational value. Non-Copy (owns two limb `Vec`s) → clone-on-read.
        Ty::Rational => Some("cdz_num::Rational".to_string()),
        // A QUANTITY is a COMPILE-TIME-only dimension over an inner numeric magnitude — `lower` erases the
        // `Ty::Qty` wrapper, so the emitted VALUE is just the inner magnitude (stored RAW at the SOURCE
        // unit). Map to `rust_type(inner)`; the unit is recovered for the boundary render from the backend's
        // `// cdz-unit[…]` note (`Unit::render_value_form`, at the REFERENCE unit — scale dropped) and any
        // non-1 scale from the `// cdz-scale[…]` note, so the gate harness renders `((. Qty of) <scaled
        // magnitude> <reference-unit>)`.
        //   - SCALE-1 (a reference unit): the stored magnitude IS the displayed one — any inner type.
        //   - NON-SCALE-1 (`5 kilometer`, `5 foot`): the display SCALES the magnitude to the reference
        //     (`5 km` → `5000 m`), applied in the harness in the inner numeric type. Supported for FLOAT /
        //     fixed-INT (`× num/den` — Float rounds, Int truncates), RATIONAL (exact `num/den`), and a
        //     BigInt at a WHOLE-ratio scale (`den == 1`, e.g. kilo — `Big.mul(num)` exact). A NON-whole
        //     BigInt ratio (`5 mile` → 201168/125 m) still DECLINES (see `qty_scale_supported`).
        Ty::Qty { inner, unit }
            if unit.scale() == (1, 1) || qty_scale_supported(inner, unit.scale()) =>
        {
            rust_type(ncx, inner)
        }
        // A CHAR is a single Unicode scalar value — Rust's native `char` (which IS a Unicode scalar,
        // exactly the Cadenza model). Copy, so no clone-on-read needed. Lets a `Char` cross as a sum
        // payload / tuple element (a `(Tok (Ch Char))` enum) and a `ConstChar` emit as a `'…'` literal.
        Ty::Char => Some("char".to_string()),
        // A tuple is Rust's native tuple: `(T0, T1, …)` — each element mapped recursively (so a nested
        // tuple / a tuple of scalars composes). A 1-tuple is written `(T,)` (Rust needs the trailing
        // comma to distinguish it from a parenthesized type). An element with no native mapping declines
        // the whole tuple. (The empty tuple `Ty::Tuple([])` is distinct from `Unit` upstream, but has no
        // element to map — render it as `()`, Rust's unit/empty-tuple type.)
        Ty::Tuple(elems) => tuple_type(ncx, elems.iter()),
        // A RECORD is structural (anonymous) in Cadenza and at run time IS a positional array in
        // sorted-field-name order (a record field read is a `Core::Proj` at the field's sorted index —
        // the SAME machinery a tuple uses). So it maps to the SAME Rust tuple as a tuple of its fields'
        // types IN SORTED KEY ORDER: `(record (b Bool) (a Int64))` → `(i64, bool)` (a before b). The
        // `BTreeMap` already iterates sorted, so this is just the tuple mapping over its values. Field
        // NAMES are compile-time-only (they became sorted positions) — the emitted `.rs` reads fields
        // positionally (`r.0`); the names re-appear only in the boundary render (`(record (a …) …)`).
        // (When Cadenza gains NOMINAL records, THAT is when a named Rust struct becomes the right
        // emission — the name will come from the language, not be synthesized.)
        Ty::Record(fields) => tuple_type(ncx, fields.values()),
        // A SUM is a NOMINAL type — unlike a record it HAS a name (the declared sum name), so it maps to
        // a Rust ENUM of that name (the backend emits the `enum <Name> { … }` declaration separately).
        // A generic sum instantiation carries its type ARGS (`Option Int64` → args `[Int64]`), which
        // become the Rust type parameters: `Option<i64>`. A monomorphic sum (`Sign`, no args) is the
        // bare name. The enum name is sanitized (a `-` in a sum name → `_`), matching the declaration.
        Ty::Sum { decl, args } => {
            let ident = sum_ident(ncx.name_of(*decl)?);
            if args.is_empty() {
                Some(ident)
            } else {
                let mut params = Vec::with_capacity(args.len());
                for a in args.iter() {
                    params.push(rust_type(ncx, a)?);
                }
                Some(format!("{ident}<{}>", params.join(", ")))
            }
        }
        // A NOMINAL newtype erases at run time to its underlying structural value (the tag "adds nothing
        // to the value's runtime representation", `type-system.md §156`), so it maps to the SAME Rust type
        // as its underlying type — a transparent alias. `(type UserId (Mk Int64))` → `i64`, `(type Point
        // (Mk Int64 Int64))` → `(i64, i64)`. (A named Rust newtype struct is a possible future
        // refinement; the erased mapping is correct and matches the wasm backend's read-through.)
        Ty::Nominal { inner, .. } => rust_type(ncx, inner),
        // A FUNCTION value is a first-class closure — `Rc<dyn Fn(A, …) -> R>`. `Rc` (not `Box`) so it is
        // CLONE-able: a closure bound/used in more than one position is cloned on read (the tick-5
        // clone-on-read discipline), and `Box<dyn Fn>` is not Clone.
        //
        // The arrow SPINE is FLATTENED: `(-> A (-> B C))` → `Rc<dyn Fn(A, B) -> C>`, NOT a nested
        // `Rc<dyn Fn(A) -> Rc<dyn Fn(B) -> C>>`. A runtime closure is lambda-LIFTED to a FLAT function of
        // all its parameters and applied at FULL arity by a `Core::CallClosure` carrying every arg (a
        // PARTIAL application of a runtime multi-param closure declines at the application site — the wasm
        // backend draws the same line), so the runtime representation is flat and the type must match it.
        // Each parameter and the final (non-arrow) result maps recursively; any with no native mapping
        // declines the whole function type.
        Ty::Fn(_, _) => {
            let mut params = Vec::new();
            let mut cur = ty;
            while let Ty::Fn(p, r) = cur {
                params.push(rust_type(ncx, p)?);
                cur = r;
            }
            let ret = rust_type(ncx, cur)?;
            Some(format!(
                "std::rc::Rc<dyn Fn({}) -> {ret}>",
                params.join(", ")
            ))
        }
        // A LIST is a homogeneous sequence — Rust's native growable sequence `Vec<T>` (the wasm backend
        // builds it on the persistent `vec-*` heap; here it is an owned `Vec`). The element type maps
        // recursively, so a `List Int64` → `Vec<i64>` and a nested `List (List Int64)` → `Vec<Vec<i64>>`
        // compose; an element with no native mapping declines the whole list. (Cadenza lists are
        // PERSISTENT/immutable; a `Vec` is owned+`Clone`, and every list op emitted for this backend
        // produces a NEW `Vec` — no in-place mutation — so the value semantics agree.)
        Ty::List(elem) => Some(format!("Vec<{}>", rust_type(ncx, elem)?)),
        // A MAP is a persistent key→value association — Rust's ordered `BTreeMap<K, V>`. `BTree` (not
        // `HashMap`) because it ITERATES IN SORTED KEY ORDER, which is exactly the CANONICAL order Cadenza's
        // `Map.to-list` yields — so enumeration matches the runtime for free (a `HashMap` would need an
        // explicit sort). Keys compare BY VALUE (`BTreeMap` uses `Ord`), matching the language's by-value
        // key semantics. `K` must be `Ord` — but a FLOAT (or a float-containing tuple/record/SUM) is only
        // `PartialOrd`, so a float-keyed `BTreeMap` fails to compile (E0277). The Ord CHECK is NOT here:
        // `rust_type` is pure (no `Db`), so it cannot resolve whether a `Ty::Sum` KEY's enum derives `Ord`
        // (that needs the sum's payloads). The decline is enforced by the `Db`-aware `enums::ty_ord_key_ok`
        // gate at the emit boundary (`sum_representable` for a param/result Map/Set TYPE; the construction
        // ops `MapNew`/`MapInsert`/`SetOf`/`SetInsert` in `expr.rs` for a VALUE). So this arm just maps the
        // shape; a non-Ord key/element is caught before it can emit an uncompilable `BTreeMap`/`BTreeSet`.
        Ty::Map(k, v) => Some(format!(
            "std::collections::BTreeMap<{}, {}>",
            ord_key_type(ncx, k)?,
            rust_type(ncx, v)?
        )),
        // A SET is a persistent collection of unique elements — Rust's ordered `BTreeSet<T>` (sorted
        // iteration = the canonical `Set.to-list` order; `Ord` element compares by value, dedup at insert).
        // The Ord-element decline is enforced by the `Db`-aware gates (see the `Ty::Map` note), not here.
        Ty::Set(elem) => Some(format!(
            "std::collections::BTreeSet<{}>",
            ord_key_type(ncx, elem)?
        )),
        // A BYTES value is a raw byte sequence — Rust's owned `Vec<u8>`. Non-Copy (owned heap buffer) →
        // clone-on-read covers a shared bytes value. (Cadenza's `Bytes` is a persistent rope at run time;
        // the native rep is a flat `Vec<u8>`, and every emitted bytes op produces a NEW `Vec` — the
        // rope-vs-flat distinction is invisible at the value level, so `Bytes.compact` is a no-op here.)
        Ty::Bytes => Some("Vec<u8>".to_string()),
        // A STRING is a UTF-8 text value — Rust's owned `String`. A Cadenza string counts UNICODE SCALAR
        // VALUES (not bytes), which Rust's `String`/`.chars()` model directly. Non-Copy (owned heap
        // buffer) → clone-on-read covers a shared string. (`String` is `Ord`, so it can also key a
        // `BTreeMap` — unblocking String-keyed maps that declined while `String` had no rep.)
        Ty::String => Some("String".to_string()),
        // A SYMBOL is a canonical TEXT leaf whose identity IS its content (an interned name) — the SAME
        // tagless byte-leaf rep as a String on the wasm side. On the native rep it maps to Rust's owned
        // `String`: a Symbol value IS its text, compared/keyed/rendered by content exactly as a String
        // (`String` is `Eq`+`Ord`, so a Symbol can key a `BTreeMap` and compare with `==` — the content
        // equality the wasm byte-leaf compare gives). The Symbol↔String retag (`Symbol.of`/`.to-string`)
        // is then the identity on the `String` (a `String` is already a flat canonical leaf — the wasm
        // `bytes-compact` rope-flatten has no analogue), handled in `Core::StrToBytes`.
        Ty::Symbol => Some("String".to_string()),
        // Functions and type/erased values have no native mapping.
        _ => None,
    }
}

/// The ASYNC-mode Rust type for a solved Cadenza type — identical to [`rust_type`] EXCEPT a function type
/// `Ty::Fn` spells the UNIFORM async closure ABI `std::rc::Rc<dyn cdz_rt::EnvClosure<A, R>>` instead of the
/// sync `Rc<dyn Fn(A…)->R>`. Under `--target rust-async` a lifted closure VALUE is `Rc<dyn EnvClosure<A,R>>`
/// (its `call` future borrows the `&mut dyn DynCdzEnv` passed AT the call — a `dyn Fn` cannot express that;
/// see `cdz_rt::EnvClosure`). So every SIGNATURE position that spells a closure type in async mode — a def
/// param/result, a lifted-fn param/result, the `Core::Closure` value cast — must use THIS spelling, or the
/// closure value (built as `Rc<dyn EnvClosure>`) mismatches its slot (E0308). `A`/`R` follow the
/// lifted-lambda calling convention: `R` is the (non-arrow) result; `A` is the SINGLE arg for a 1-param
/// closure, a TUPLE of the args for a multi-param closure (the flat lifted params tupled into `A`), and `()`
/// for a 0-param closure. A closure NESTED inside a compound (a `(List (-> Int Int))`, a tuple element) is
/// spelled async too — the walk recurses with `async_closure_type` through every compound arm, delegating a
/// scalar/leaf to [`rust_type`] (which is mode-agnostic for non-function types). `None` on any
/// non-representable component, exactly like `rust_type`.
pub(super) fn async_closure_type(ncx: &crate::ty::NameCtx, ty: &Ty) -> Option<String> {
    match ty {
        // A FUNCTION → the uniform async closure ABI. Peel the arrow SPINE (flat, like `rust_type`): the
        // args are the spine's params, the result is the final non-arrow tail. Each arg + the result is
        // itself spelled ASYNC (a higher-order closure arg/result is also `EnvClosure`).
        Ty::Fn(_, _) => {
            let mut args = Vec::new();
            let mut cur = ty;
            while let Ty::Fn(p, r) = cur {
                args.push(async_closure_type(ncx, p)?);
                cur = r;
            }
            let ret = async_closure_type(ncx, cur)?;
            // `A` = () for 0 args, the single type for 1, a tuple for ≥2 (the lifted convention tuples a
            // multi-arg closure's args into one `A`; `EnvClosure::call` destructures it).
            let a = match args.len() {
                0 => "()".to_string(),
                1 => args.into_iter().next().unwrap(),
                _ => format!("({})", args.join(", ")),
            };
            Some(format!("std::rc::Rc<dyn cdz_rt::EnvClosure<{a}, {ret}>>"))
        }
        // Compounds that can CONTAIN a closure recurse with `async_closure_type`; the spelling is otherwise
        // identical to `rust_type`'s (same wrapper types), so a closure-free compound maps byte-identically.
        Ty::Tuple(elems) => {
            let mut parts = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                parts.push(async_closure_type(ncx, e)?);
            }
            Some(if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            })
        }
        Ty::Record(fields) => {
            let mut parts = Vec::with_capacity(fields.len());
            for v in fields.values() {
                parts.push(async_closure_type(ncx, v)?);
            }
            Some(if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            })
        }
        Ty::Sum { decl, args } => {
            let ident = sum_ident(ncx.name_of(*decl)?);
            if args.is_empty() {
                Some(ident)
            } else {
                let mut params = Vec::with_capacity(args.len());
                for a in args.iter() {
                    params.push(async_closure_type(ncx, a)?);
                }
                Some(format!("{ident}<{}>", params.join(", ")))
            }
        }
        Ty::Nominal { inner, .. } => async_closure_type(ncx, inner),
        Ty::List(elem) => Some(format!("Vec<{}>", async_closure_type(ncx, elem)?)),
        // A Map/Set KEY cannot be a closure (not `Ord`), so the key keeps `ord_key_type`; only the Map VALUE
        // can carry a closure, so it recurses. (A closure-keyed collection would have declined upstream.)
        Ty::Map(k, v) => Some(format!(
            "std::collections::BTreeMap<{}, {}>",
            ord_key_type(ncx, k)?,
            async_closure_type(ncx, v)?
        )),
        Ty::Set(elem) => Some(format!(
            "std::collections::BTreeSet<{}>",
            ord_key_type(ncx, elem)?
        )),
        Ty::Qty { inner, unit }
            if unit.scale() == (1, 1) || qty_scale_supported(inner, unit.scale()) =>
        {
            async_closure_type(ncx, inner)
        }
        // Every other type (scalars, Bytes, String, Symbol, …) has no closure inside — delegate to the
        // mode-agnostic `rust_type` (identical spelling in both modes).
        _ => rust_type(ncx, ty),
    }
}

/// The `(A, R)` type-argument spelling for a lifted lambda's `EnvClosure<A, R>` impl, in ASYNC mode: `R` is
/// the lambda's result type, `A` is the single param type (arity 1), a tuple of the param types (arity ≥2),
/// or `()` (arity 0) — the SAME convention [`async_closure_type`]'s `Ty::Fn` arm produces, so a closure
/// VALUE's `Rc<dyn EnvClosure<A,R>>` and its TYPE-position spelling agree. `None` if any param/result has no
/// async representation.
pub(super) fn env_closure_args(
    ncx: &crate::ty::NameCtx,
    params: &[(crate::ast::StructId, Ty)],
    ret: &Ty,
) -> Option<(String, String)> {
    let mut arg_tys = Vec::with_capacity(params.len());
    for (_, t) in params {
        arg_tys.push(async_closure_type(ncx, t)?);
    }
    let a = match arg_tys.len() {
        0 => "()".to_string(),
        1 => arg_tys.into_iter().next().unwrap(),
        _ => format!("({})", arg_tys.join(", ")),
    };
    let r = async_closure_type(ncx, ret)?;
    Some((a, r))
}

/// The Rust type for a Set ELEMENT / Map KEY position — like [`rust_type`], except a bare `Float` maps to a
/// WIDTH-SPECIFIC total-order wrapper (a `BTreeSet`/`BTreeMap` needs `Ord`, which `f32`/`f64` lack). The
/// wrapper MUST match the float's width: a `Float64` → `__CdzF64` (over `u64` bits), a `Float32` → `__CdzF32`
/// (over `u32` bits) — a single `__CdzF64` around an `f32` would not type-check (and an `as f64` widen would
/// collapse distinct f32 keys). This is the ONLY place a float becomes a wrapper: a float in a NON-key
/// position (a value, a tuple element, a Map VALUE) stays a bare `f32`/`f64`. A float NESTED inside a
/// compound KEY (a `(Tuple Float Int64)` key) is NOT handled here — that still declines via `ty_is_ord` (the
/// wrapper would have to be threaded through the tuple, a later increment); this covers the bare-`Float`
/// key/element the corpus exercises. Any other type falls through to `rust_type` unchanged.
pub(super) fn ord_key_type(ncx: &crate::ty::NameCtx, ty: &Ty) -> Option<String> {
    // A `Qty` erases to its inner numeric, so a Qty-over-Float KEY TYPE is the total-order wrapper
    // `__CdzF{N}` — the same as a bare float — NOT the raw `f64` the `_ => rust_type` fallback would spell
    // (which is not `Ord` → `f64: Ord` E0277 at the `BTreeMap`/`BTreeSet` key, qkm1/qkm3). Peel `Qty`
    // (possibly under a nominal) and recurse; the matching VALUE wrap is `expr::wrap_ord_key`'s Qty peel.
    // A Qty inner is always numeric (Float/Int/Rational), so this only exposes a numeric leaf — a Qty-over-
    // Int key recurses to `i64` (unchanged), a nominal (non-Qty) key keeps its existing verbatim handling.
    if let Ty::Qty { inner, .. } = ty.strip_nominal() {
        return ord_key_type(ncx, inner);
    }
    match ty {
        Ty::Float(ft) => Some(if ft.ground_width() == 32 {
            "__CdzF32".to_string()
        } else {
            "__CdzF64".to_string()
        }),
        // A TUPLE key threads the wrapper through EACH element (a float element becomes `__CdzF{N}`, a
        // non-float element stays its `rust_type`), so `(Tuple Float64 Int64)` keys as `(__CdzF64, i64)` —
        // which IS `Ord` (Rust derives Ord structurally over Ord fields). This lifts the float-in-a-tuple-key
        // decline (v-runtime differential: a `(Tuple Float Int)` Map key computed on wasm but declined on
        // rust). The matching VALUE-side rebuild is `expr::wrap_ord_key` (wraps each float element by
        // position); the two MUST agree on which elements wrap + the width. A 1-tuple keeps the trailing
        // comma (`(T,)`). An element with no ord-key mapping declines the whole tuple key.
        Ty::Tuple(elems) => {
            let mut parts = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                parts.push(ord_key_type(ncx, e)?);
            }
            Some(if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            })
        }
        // A RECORD key threads the wrapper through each FIELD in sorted-field order (a record erases to a
        // tuple in that order), so `(Record (f Float64) (n Int64))` keys as `(__CdzF64, i64)` — the record
        // twin of the tuple key. The value-side rebuild is `expr::wrap_ord_key`'s Record arm (same sorted
        // `.i` positions). A field with no ord-key mapping declines the whole record key.
        Ty::Record(fields) => {
            let mut parts = Vec::with_capacity(fields.len());
            for t in fields.values() {
                parts.push(ord_key_type(ncx, t)?);
            }
            Some(if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            })
        }
        // A built-in `Option`-KEY maps to the declared-order wrapper `__CdzOpt<inner_ord_key>` — NOT the bare
        // std `Option<T>`, whose derived Ord (`None < Some`) is the REVERSE of Cadenza's `Some < None` (#42
        // witness 2). The inner payload uses its own `ord_key_type` (so `Option Float64` → `__CdzOpt<__CdzF64>`
        // — the wrapper composes). A NON-Option sum, or an Option over a payload with no ord-key mapping,
        // falls through to `rust_type`/decline as before. `is_flip_order_option_key` recognizes the built-in
        // Option (not a user `(type Option …)`, which emits its own decl-order enum with correct native Ord).
        Ty::Sum { args, .. } if is_flip_order_option_key_shallow(ncx, ty) => {
            // `Option a` has exactly one type arg = the `Some` payload.
            let inner = args.first()?;
            let inner_key = ord_key_type(ncx, inner)?;
            Some(format!("__CdzOpt<{inner_key}>"))
        }
        _ => rust_type(ncx, ty),
    }
}

/// Whether `ty` is an `Option` sum at a single-payload instantiation — the type shape whose std-`Option`
/// mapping has the flipped derived Ord that `__CdzOpt` corrects for a key/element position (#42 witness 2).
/// SHALLOW + Db-free (name + arity only): `ord_key_type`/`wrap_ord_key` are `Db`-free, so they can only key
/// on this NAME-level test. It CANNOT distinguish a user `(type Option …)` shadow from the built-in — so it
/// is NOT the authority on whether to wrap. The AUTHORITY is the Db-aware admission gate
/// [`ty_is_ord_key`] / [`is_builtin_option`], which DECLINES a name-`Option` key that is not the built-in
/// (PR#894): a non-built-in never reaches the wrap path, so by the time this shallow test runs on an admitted
/// key it IS the built-in. (Correcting the earlier doc, which wrongly claimed `wrap_ord_key` has a `Db` +
/// re-checks `is_builtin_std_sum` — it does not; the gate is `ty_is_ord_key`.) Nominal is peeled first.
pub(super) fn is_flip_order_option_key_shallow(ncx: &crate::ty::NameCtx, ty: &Ty) -> bool {
    matches!(ty.strip_nominal(), Ty::Sum { decl, args } if args.len() == 1 && ncx.name_of(*decl) == Some("Option"))
}

/// GROUND the still-unsolved type VARIABLES in `ty` to the default `Int64`, recursively — the type-level
/// analogue of [`IntTy::ground_width`] grounding an unresolved WIDTH at selection. Used to spell a Rust
/// annotation for a construction whose type inference left open, specifically an EMPTY collection literal
/// (`Map.empty` / `Set.of (list)`) whose element/key/value types are fixed only by LATER use — e.g. an
/// empty-Map handler state whose `K`/`V` are pinned through the `get`/`put` effect ops downstream, not at
/// the construction site. Without a spelled annotation the emitted `BTreeMap::new()` is uninferrable and
/// rustc raises E0282 ("type annotations needed"); grounding an open `Ty::Var`/`Ty::Any` to `Int64` gives
/// a concrete annotation for the common integer-typed accumulator/handler-state shape.
///
/// A `Var`/`Any` in the KEY or VALUE position becomes `Int64`; a concrete leaf is unchanged; a compound
/// recurses. This can only ever produce a WRONG type if the collection is genuinely used at a NON-default
/// element type reachable only through unsolved vars — in which case rustc errors LOUDLY at the annotated
/// `new()` (a build failure the gate records as `todo`), never a silent miscompile. So grounding is
/// strictly safer than the bare `new()` (which E0282s for EVERY open case) and correct for the int-typed
/// majority. Only the OPEN vars are grounded — a partially-solved `(Map Int64 Var)` grounds just the value.
/// Render `ty` as a Rust type spelling exactly like [`rust_type`], EXCEPT a free type variable
/// (`Ty::Var`/`Ty::Any`) renders as the inference HOLE `_` (instead of failing, as `rust_type` does).
/// Used to annotate an empty collection whose OUTER shape is solved but whose INTERIOR element type is
/// fixed only by later use — e.g. a get-only `Map.empty` whose value type the downstream match-join
/// pins to `List <var>` (breaker ms9-family ms13/ns1): the outer `Vec` satisfies rustc's method
/// resolution (so `.push` / `.clone` on the value resolve) while the `_` element lets rustc solve the
/// interior from the actual use — strictly better than grounding the interior to the DEFAULT `i64`,
/// which under-approximates a nested value (`List (List i64)` → wrongly `Vec<i64>` → E0308 at `.push`).
/// A concrete leaf renders as itself; a compound recurses. `None` only if a NON-var component has no rep
/// (a float Map key, a closure in sync-map position) — the same decline surface as `rust_type`.
pub(super) fn rust_type_holes(ncx: &crate::ty::NameCtx, ty: &Ty) -> Option<String> {
    match ty {
        Ty::Var(_) | Ty::Any => Some("_".to_string()),
        Ty::List(e) => Some(format!("Vec<{}>", rust_type_holes(ncx, e)?)),
        Ty::Set(e) => Some(format!(
            "std::collections::BTreeSet<{}>",
            ord_key_type_holes(ncx, e)?
        )),
        Ty::Map(k, v) => Some(format!(
            "std::collections::BTreeMap<{}, {}>",
            ord_key_type_holes(ncx, k)?,
            rust_type_holes(ncx, v)?
        )),
        Ty::Tuple(elems) => {
            let mut parts = Vec::with_capacity(elems.len());
            for e in elems.iter() {
                parts.push(rust_type_holes(ncx, e)?);
            }
            Some(if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            })
        }
        // A non-compound (scalar/text/sum/nominal/…) has no interior collection var to hole out — spell
        // it exactly as `rust_type` (a free var in a sum ARG is rare here and rust_type declines it, the
        // same fail-loud floor).
        _ => rust_type(ncx, ty),
    }
}

/// The ord-key twin of [`rust_type_holes`]: an ord-key position (a Set element / Map key) renders a free
/// var as `_` and otherwise defers to [`ord_key_type`] (the `__CdzF*`/`__CdzOpt` wrapper spellings). A
/// key is almost always solved (it IS the lookup key), but a NESTED key inside a holed value (a
/// `Map String (Set <var>)`) recurses here for the inner set element.
fn ord_key_type_holes(ncx: &crate::ty::NameCtx, ty: &Ty) -> Option<String> {
    match ty {
        Ty::Var(_) | Ty::Any => Some("_".to_string()),
        Ty::List(e) => Some(format!("Vec<{}>", rust_type_holes(ncx, e)?)),
        Ty::Set(e) => Some(format!(
            "std::collections::BTreeSet<{}>",
            ord_key_type_holes(ncx, e)?
        )),
        Ty::Map(k, v) => Some(format!(
            "std::collections::BTreeMap<{}, {}>",
            ord_key_type_holes(ncx, k)?,
            rust_type_holes(ncx, v)?
        )),
        _ => ord_key_type(ncx, ty),
    }
}

pub(super) fn ground_open_vars(ty: &Ty) -> Ty {
    match ty {
        Ty::Var(_) | Ty::Any => Ty::int64(),
        Ty::List(e) => Ty::List(Box::new(ground_open_vars(e))),
        Ty::Set(e) => Ty::Set(Box::new(ground_open_vars(e))),
        Ty::Map(k, v) => Ty::Map(Box::new(ground_open_vars(k)), Box::new(ground_open_vars(v))),
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(ground_open_vars)
                .collect::<std::rc::Rc<[Ty]>>(),
        ),
        // A GENERIC sum / nominal carries its instantiation in `args` — an unconstrained arg (a bare `(Ok x)`
        // whose Err the checker left free, or a `Box` over an unsolved element) is a `Ty::Var` NESTED in
        // `args`, which the old (no-Sum/Nominal) walk left ungrounded — so a `Result<i64, _>` in a discarded
        // value emitted an ambiguous type (E0282). Recurse into `args` (and a nominal's derived `inner`) so
        // the grounded type is fully concrete. `decl` (the identity) is unchanged.
        Ty::Sum { decl, args } => Ty::Sum {
            decl: *decl,
            args: args.iter().map(ground_open_vars).collect::<std::rc::Rc<[Ty]>>(),
        },
        Ty::Nominal { decl, args, inner } => Ty::Nominal {
            decl: *decl,
            args: args.iter().map(ground_open_vars).collect::<std::rc::Rc<[Ty]>>(),
            inner: std::rc::Rc::new(ground_open_vars(inner)),
        },
        // A record's field types can likewise nest a free var (a `(Ok x)` field of a discarded record).
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), ground_open_vars(t)))
                .collect(),
        )),
        // A Qty erases to its inner magnitude — ground the inner, keep the unit.
        Ty::Qty { inner, unit } => Ty::Qty {
            inner: Box::new(ground_open_vars(inner)),
            unit: unit.clone(),
        },
        _ => ty.clone(),
    }
}

/// Whether `ty` maps to a Rust type that implements `Ord` — the bound `BTreeSet<T>`/`BTreeMap<K,_>`
/// requires of its element/key. Every scalar/text/compound the backend represents IS `Ord` EXCEPT a
/// FLOAT (and anything CONTAINING one): Rust's `f64`/`f32` are `PartialOrd` but NOT `Ord` (NaN breaks
/// totality), so a float in an ordered position (the Set element or Map key, or nested inside a
/// tuple/record/list/set/map/SUM used there) makes the `BTree*` uninstantiable (E0277). The runtime
/// orders a float by its canonical bytes (so wasm supports a float key/element), but the Rust backend
/// has no total float order, so it DECLINES rather than emit an uncompilable `BTreeSet<f64>`.
///
/// A `Ty::Sum`/`Ty::Nominal` is orderable iff its emitted enum derives `Ord` — which is iff it derives
/// `Eq` (see `enums::emit_one_enum`: both gate on `sum_derives_eq`, since Rust's `Ord` derive composes
/// over the SAME payload fields `Eq` does). So the SUM case delegates to the `Db`-aware
/// `enums::ty_derives_eq`. This CLOSES the float-carrying-sum hole (Copilot PR#455): the old version
/// returned `true` for EVERY `Ty::Sum`, so a `Set`/`Map` keyed by a sum whose enum canNOT derive `Ord`
/// (a float / non-`Eq` payload) slipped past and the backend emitted an uncompilable
/// `BTreeSet<Enum>`/`BTreeMap<Enum,_>` — the exact failure the float-key decline prevents, one shape over.
///
/// NOT a blanket delegation to `ty_derives_eq`, because Ord ≠ current-Eq for the NUMERIC types: a
/// `BigInt`/`Rational` has a total order (usable as a `BTree*` key — the CHAMP orders it by canonical
/// bytes on wasm; a rust `cdz_num::Big` has a total `cmp`) even though it is not yet native-`Eq`-derivable
/// on the rust backend. Treating those as non-Ord would REGRESS the passing BigInt-keyed set/map cases.
/// So keep the structural walk (with the sum case now Db-aware) rather than reusing the Eq predicate.
pub(super) fn ty_is_ord(db: &mut Db, ty: &Ty) -> bool {
    match ty {
        // A float is `PartialOrd` but NOT `Ord` — the one scalar that cannot key a `BTree*`.
        Ty::Float(_) => false,
        // `Bytes` HAS a blessed total order (§order): lexicographic over its UNSIGNED byte values. That is
        // EXACTLY the derived `Ord` on `Vec<u8>` (Bytes' Rust rep), so an ordering op / `Set`/`Map` over
        // `Bytes` maps directly to `BTreeSet<Vec<u8>>`/`BTreeMap<Vec<u8>, _>` with NO wrapper — and this order
        // agrees byte-for-byte with the wasm `value_cmp_shaped` Bytes arm (both compare the raw byte slices).
        // Bytes EQUALITY was already blessed (byte-canonical, `enums::ty_derives_eq`); this adds the ORDER.
        Ty::Bytes => true,
        // Compounds are `Ord` iff every ordered component is. Recurse over BORROWS directly — `db: &mut Db`
        // and the element borrows (which come from `ty`, not `db`) don't conflict, so no clone is needed
        // (the earlier `.clone()`s were over-defensive; this mirrors `enums::ty_derives_eq`'s
        // allocation-free walk over the same shapes). PR#460 (Copilot) cleanup.
        Ty::Tuple(elems) => elems.iter().all(|e| ty_is_ord(db, e)),
        Ty::Record(fields) => fields.values().all(|t| ty_is_ord(db, t)),
        Ty::Nominal { inner, .. } => ty_is_ord(db, inner),
        Ty::List(e) | Ty::Set(e) => ty_is_ord(db, e),
        Ty::Map(k, v) => ty_is_ord(db, k) && ty_is_ord(db, v),
        // A SUM is orderable if EITHER (a) its enum derives `Ord` (= derives `Eq`; the native path), OR
        // (b) it is a float-carrying MONOMORPHIC sum we give a hand-written `impl Ord` via a `__ord_<Ident>`
        // walk (`sum_is_custom_ord` — e.g. `Ast`, a `Float`+`List Ast` sum used as a Set element / Map key).
        // The custom-impl branch orders the float leaf by canonical bits (matching wasm's value-cmp order),
        // so the `BTreeSet<Ast>`/`BTreeMap<Ast, _>` is instantiable and its order agrees cross-backend. NOTE:
        // `sum_is_custom_ord` re-checks `ty_supports_eq` (native path) FIRST and returns false there, so the
        // two branches are disjoint and there is no double-count; a float-carrying sum with a flip-Option or
        // generic args still declines (neither branch admits it).
        Ty::Sum { .. } => {
            crate::backend::rust::enums::ty_derives_eq(
                db,
                ty,
                &mut std::collections::HashSet::new(),
            ) || crate::backend::rust::expr::sum_is_custom_ord(db, ty)
        }
        // Every other representable type (Int/Bool/Unit/Char/String/Bytes and the NUMERIC BigInt/Rational,
        // which have a total order) maps to an `Ord` Rust type. A non-representable type declines earlier.
        _ => true,
    }
}

/// Whether `ty` can occupy a Set ELEMENT / Map KEY position directly — the gate the construction ops and
/// the boundary use. Like [`ty_is_ord`], EXCEPT a `Float` is OK because it maps to the total-order wrapper
/// `__CdzF{64,32}` (see [`ord_key_type`]), which IS `Ord` — AND a TUPLE that contains floats is OK too,
/// because `ord_key_type` now threads the wrapper through the tuple's elements (each float element becomes
/// `__CdzF{N}`, so the whole tuple keys as an `Ord` `(__CdzF64, i64)`). A float nested in a SUM
/// payload is still NOT threaded (a later increment), so that delegates to the strict `ty_is_ord`. This gate,
/// the type spelling, and the value wrap (`expr::wrap_ord_key`) MUST agree on exactly which shapes admit a
/// contained float, or an emitted key value would not match the collection's key type.
pub(super) fn ty_is_ord_key(db: &mut Db, ty: &Ty) -> bool {
    match ty {
        // A bare float keys/elements via the `__CdzF{N}` wrapper — representable and totally ordered.
        Ty::Float(_) => true,
        // A bare BUILT-IN `Option` key/element keys via the `__CdzOpt` declared-order wrapper (#42 witness 2)
        // — admitted. KEYSTONE: A USER `(type Option …)` shadow (name `Option`, 1 arg, but a source-node decl) must
        // NOT reach the wrapper: `ord_key_type`/`wrap_ord_key` are Db-free and key on the NAME-only
        // `is_flip_order_option_key_shallow`, so they'd wrap it as `__CdzOpt<..>` while its VALUE is the user
        // enum → rustc mismatch (PR#894). Since the Db-free wrap path can't tell them apart, DECLINE a
        // name-`Option` sum that is NOT the built-in here (the Db-aware gate) → it's not admitted as an ord
        // key → not emitted → no mismatch. (A user-Option ord key is vanishingly rare — the prelude owns
        // `Option` — and declining is sound: correct-wrap-or-honest-decline. A user sum NOT named `Option`
        // takes the strict `ty_is_ord` path unaffected.)
        s @ Ty::Sum { decl, args }
            if args.len() == 1
                && db
                    .type_decl_by_occ(*decl)
                    .is_some_and(|d| d.name == "Option") =>
        {
            is_builtin_option(db, s)
        }
        // A TUPLE key is ord iff each element is ord-KEY-able (a float element is OK via `__CdzF`) — matching
        // `ord_key_type`'s per-element threading. EXCEPT: a tuple/record key that CONTAINS a built-in Option
        // ANYWHERE (a direct element OR nested) DECLINES: the `__CdzOpt` wrapper threads through nested
        // tuples/records only for FLOAT today (`wrap_ord_key`'s Tuple/Record arms gate on the float-only
        // `key_ty_has_wrappable_float`), NOT Option — so an Option-in-a-compound key emits `__CdzOpt<..>` in
        // the KEY TYPE (`ord_key_type` recurses into Option) but a bare `Option<..>` VALUE (the wrap arm
        // skips it) → rustc E0308 + a missed `<(__CdzOpt` struct injection (PR#894). Declining is the
        // correct-wrap-or-honest-decline bound; a BARE Option key (not under a tuple/record) still wraps fine
        // via the `is_builtin_option` arm above. Threading `__CdzOpt` through compounds is a later increment.
        Ty::Tuple(elems) => {
            !elems.iter().any(|e| ty_contains_builtin_option(db, e))
                && elems.iter().all(|e| ty_is_ord_key(db, e))
        }
        // A RECORD key is ord iff each FIELD is ord-key-able (a float/bare-Option field OK via the wrapper) —
        // matching `ord_key_type`'s per-field threading; same Option-in-compound decline as the tuple arm.
        Ty::Record(fields) => {
            !fields.values().any(|t| ty_contains_builtin_option(db, t))
                && fields.values().all(|t| ty_is_ord_key(db, t))
        }
        // Any other shape uses the strict predicate (a float nested in a SUM payload still declines).
        _ => ty_is_ord(db, ty),
    }
}

/// Whether `ty` is the BUILT-IN `Option` sum (Db-aware — distinguishes a user `(type Option …)` shadow,
/// which emits its own decl-order enum and must NOT be `__CdzOpt`-wrapped). Peels a nominal. This is the
/// authority the key-wrap path needs but `is_flip_order_option_key_shallow` (name-only, Db-free) cannot be.
pub(super) fn is_builtin_option(db: &mut Db, ty: &Ty) -> bool {
    if let Ty::Sum { decl, args } = ty.strip_nominal()
        && args.len() == 1
    {
        let decl_occ = *decl;
        return db
            .type_decl_by_occ(decl_occ)
            .filter(|d| d.name == "Option")
            .map(|d| {
                let d = d.clone();
                crate::backend::rust::enums::is_builtin_std_sum(db, &d)
            })
            .unwrap_or(false);
    }
    false
}

/// Whether `ty` contains a built-in `Option` ANYWHERE (as the bare type OR under a tuple/record) — the
/// tuple/record `ty_is_ord_key` arms decline a key CONTAINING one (the `__CdzOpt` wrapper isn't threaded
/// through compounds yet, PR#894). A bare-Option key is handled by the `is_builtin_option` arm ABOVE this
/// (admitted+wrapped), so this only fires for an Option UNDER a tuple/record layer.
fn ty_contains_builtin_option(db: &mut Db, ty: &Ty) -> bool {
    let s = ty.strip_nominal();
    if is_builtin_option(db, s) {
        return true;
    }
    match s {
        Ty::Tuple(elems) => elems.iter().any(|e| ty_contains_builtin_option(db, e)),
        Ty::Record(fields) => fields.values().any(|t| ty_contains_builtin_option(db, t)),
        _ => false,
    }
}

/// The Rust identifier for a sum type / variant name — sanitized to a valid identifier the same way a
/// def name is (`super::sanitize_ident`: a `-` and any non-ident char → `_`, a Rust keyword → `r#kw`), so
/// the emitted `enum` declaration, every `Name::Variant(...)` construction, and every `match` pattern agree
/// on the spelling.
///
/// ADDITIONALLY escapes a name that collides with a Rust PRIMITIVE TYPE (`i64`, `bool`, `str`, `usize`, …).
/// Unlike a `fn`/binder name (a local `let i64 = …` harmlessly shadows the primitive in value position), an
/// ENUM name appears in TYPE position — `enum i64 { A(i64), … }` makes the field `A(i64)` refer to the ENUM
/// (infinitely sized, rustc E0072) instead of the primitive. A primitive name is not a Rust keyword, so
/// `sanitize_ident` passes it through unescaped; prefix it here (`cdz_ty_i64`) so a user sum named after a
/// primitive is namespace-isolated from the primitive it would otherwise shadow. The prefix is applied to
/// the WHOLE sum/variant-name space uniformly (decl, construct, match all route through here), so they agree.
pub fn sum_ident(name: &str) -> String {
    // INJECTIVITY (a sum name is an emitted TYPE name — two distinct sums that collapse to one enum ident
    // would be a duplicate `enum` / conflated construct+match, rustc E0428). `sanitize_ident` is LOSSY: it
    // maps every non-`[A-Za-z0-9_]` char to `_` and prefixes a leading digit, so `Foo-Bar` and `Foo_Bar`
    // both become `Foo_Bar`. For the sum/variant namespace we need an INJECTIVE map. A "clean" name — a
    // valid Rust identifier start + body (`[A-Za-z_][A-Za-z0-9_]*`) that does NOT begin with the mangle
    // MARKER — passes through `sanitize_ident` LOSSLESSLY (it changes nothing), so keep it (readable, and
    // the common case). Any OTHER name (a lossy char, a leading digit, or a literal MARKER prefix) is
    // HEX-mangled: `MARKER + hex(utf8)`. Two distinct originals give distinct hex; a clean name never
    // begins with MARKER (excluded); a name that literally begins with MARKER is itself mangled — so the
    // clean space and the mangled space are disjoint and each is injective. Result: no two distinct sum
    // names ever share an emitted ident.
    const MARKER: &str = "cdzsum_";
    if is_clean_ident(name) && !name.starts_with(MARKER) {
        let s = super::sanitize_ident(name);
        // A clean name may still be a Rust PRIMITIVE (`i64`, `bool`, …) — valid char-wise but ruinous as an
        // emitted enum name (`enum i64 { A(i64) }` makes the field refer to the enum, E0072). Escape it.
        // (`sanitize_ident` already handled keywords → `r#kw` / `cdz_kw_…`.)
        if is_rust_primitive_type(&s) {
            format!("cdz_ty_{s}")
        } else {
            s
        }
    } else {
        // Lossy / leading-digit / MARKER-prefixed → hex-mangle the whole ORIGINAL name (injective).
        let mut hex = String::with_capacity(name.len() * 2 + MARKER.len());
        hex.push_str(MARKER);
        for b in name.bytes() {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }
}

/// Whether `name` is already a valid Rust identifier — starts with `[A-Za-z_]`, and every char is
/// `[A-Za-z0-9_]`. Such a name passes through `sanitize_ident` UNCHANGED (the lossy `→ _` map fires on no
/// char), so it is safe to emit verbatim; any other name must be hex-mangled to stay injective. An empty
/// name is not a valid ident. (A keyword IS a clean ident here — `sanitize_ident` escapes it separately to
/// `r#kw`, which is still injective: distinct keywords give distinct `r#kw`.)
fn is_clean_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// Whether `s` names a Rust PRIMITIVE / built-in scalar type — the set an emitted `enum <name>` would
/// shadow in type position. Covers the integer/float widths, `bool`, `char`, `str`, the machine-size ints,
/// and the unit-like `()` is not spellable as an ident so is excluded. NOT the keyword set (that is
/// [`super::is_rust_keyword`], handled by `sanitize_ident`); these are contextual type names, valid as a
/// binder but ruinous as an emitted type name.
fn is_rust_primitive_type(s: &str) -> bool {
    matches!(
        s,
        "i8" | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "isize"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "usize"
            | "f32"
            | "f64"
            | "bool"
            | "char"
            | "str"
    )
}

/// The Rust tuple type for a sequence of element types — `(T0, T1, …)`, each mapped recursively; a
/// 1-element tuple is `(T,)` (Rust needs the trailing comma to distinguish it from a parenthesized
/// type); an empty one is `()`. `None` if any element has no native mapping. Shared by `Ty::Tuple` and
/// `Ty::Record` (a record IS a tuple of its fields' types in sorted-key order — the `BTreeMap`'s
/// `.values()` iterate sorted, so passing them here gives the right positional order).
fn tuple_type<'a>(ncx: &crate::ty::NameCtx, elems: impl Iterator<Item = &'a Ty>) -> Option<String> {
    let mut parts = Vec::new();
    for e in elems {
        parts.push(rust_type(ncx, e)?);
    }
    if parts.is_empty() {
        return Some("()".to_string());
    }
    let trailing = if parts.len() == 1 { "," } else { "" };
    Some(format!("({}{trailing})", parts.join(", ")))
}

/// The native Rust integer type for an integer of a given signedness and (grounded) width, or `None`
/// for a width Rust has no primitive for. Rust primitives exist only for 8/16/32/64/128 bits; Cadenza
/// exposes 8/16/32/64 as its aliased boundary widths, so those four (each signed and unsigned) map,
/// and any other width (`UInt7`, `UInt24`, `UInt48`, an odd width) is `None` — no native form, decline.
/// The width is GROUNDED (a still-deferred/variable width takes the default, `Int64`), exactly as the
/// wasm backend grounds an unresolved width at selection.
fn int_type(it: IntTy) -> Option<&'static str> {
    // Only a fixed width has a native primitive; a deferred/variable width grounds to the default (64),
    // matching `IntTy::ground_width`. Read the fixed axes directly so a non-aliased FIXED width (e.g.
    // `UInt 24`) is rejected rather than silently grounded to something wider.
    let width = match it.width {
        Width::Fixed(w) => w,
        Width::Deferred | Width::Var(_) => crate::ty::DEFAULT_INT_WIDTH,
    };
    let signed = match it.sign {
        Sign::Fixed(s) => s,
        Sign::Deferred | Sign::Var(_) => true,
    };
    Some(match (signed, width) {
        (true, 8) => "i8",
        (true, 16) => "i16",
        (true, 32) => "i32",
        (true, 64) => "i64",
        (false, 8) => "u8",
        (false, 16) => "u16",
        (false, 32) => "u32",
        (false, 64) => "u64",
        // An UNUSUAL in-range width (`UInt48`, `UInt12`, `Int24` — 1..=64 but not an aliased boundary) has
        // no exact Rust primitive, so it is STORED in the next-larger machine width (`UInt48`→`u64`,
        // `UInt12`→`u16`, `Int24`→`i32`). A value of the unusual width always fits its storage width, so a
        // const/wrap value + a boundary render are exact. WARNING: RUNTIME ARITHMETIC on an unusual width would
        // need the overflow check at `2^N` (not the storage width's `2^machine`), so `emit_arith`/shift/
        // convert on an unusual width must DECLINE (defense-in-depth — no corpus case runs arith on an
        // unusual width today: the only `(+ (UInt48) (UInt48))` case is a compile-time CDZ0304 reject). The
        // storage-width map here is safe for the value/wrap/render surface; the arith guard prevents a
        // silent wrong-overflow miscompile if a runtime unusual-width arith ever reaches emit.
        (_, w) if (1..=64).contains(&w) => storage_width_type(signed, w),
        // Out of the 1..=64 admitted range (a compiler bug — CDZ0302 rejects earlier) — decline.
        _ => return None,
    })
}

/// The next-larger native Rust integer primitive that STORES an unusual width `w` (1..=64) of the given
/// signedness — the smallest of 8/16/32/64 that is `>= w`. A value of the unusual width always fits, so the
/// storage is lossless for a const/wrap value + a boundary render. (Arithmetic must still range-check at
/// `2^w`, which the emit guards by declining unusual-width arith — see `int_type`.)
fn storage_width_type(signed: bool, w: u32) -> &'static str {
    match (signed, w) {
        (true, w) if w <= 8 => "i8",
        (true, w) if w <= 16 => "i16",
        (true, w) if w <= 32 => "i32",
        (true, _) => "i64",
        (false, w) if w <= 8 => "u8",
        (false, w) if w <= 16 => "u16",
        (false, w) if w <= 32 => "u32",
        (false, _) => "u64",
    }
}

/// Whether a NON-scale-1 quantity over inner type `inner` can DISPLAY-SCALE its magnitude on the Rust
/// backend. The display multiplies the stored magnitude by the unit's `num/den` scale in the inner numeric
/// type; the harness scales the boundary value directly:
///   - FLOAT — `× num/den` as f64 (IEEE rounds),
///   - fixed-width INT — `× num / den` (truncates toward zero),
///   - RATIONAL — EXACT: multiply by the scale as a `Rational` `num/den` (`Rational::mul` normalizes, no
///     rounding — `5 mile` → `201168/25 meter`).
///   - BIGINT — ONLY a WHOLE-ratio scale (`den == 1`, e.g. a prefix `kilo` = ×1000/1): `Big.mul(num)`
///     exactly (`5 km` → `5000 m`). A NON-whole BigInt ratio (`mile` = 201168/125) would TRUNCATE — a
///     bignum scaled by a non-integer ratio is not a BigInt (no Rational result the `Qty BigInt` type
///     allows) — so it still DECLINES. Hence the SCALE (not just `inner`) is needed to make that split.
///
/// Mirrors wasm `const_value_ast_scaled`.
pub(super) fn qty_scale_supported(inner: &Ty, scale: (i128, i128)) -> bool {
    match inner {
        Ty::Int(_) | Ty::Float(_) | Ty::Rational => true,
        // A BigInt scales EXACTLY only when the ratio is whole (den == 1); a non-whole ratio truncates.
        Ty::BigInt => scale.1 == 1,
        _ => false,
    }
}

/// Whether an integer type is SIGNED — a fixed unsigned sign is `false`; a fixed signed, or a
/// deferred/variable sign (which grounds to signed, matching `int_type`'s default), is `true`. Used by the
/// division emit to decide whether to guard the `MIN / -1` overflow trap (a signed-only case — an unsigned
/// `/` never overflows, and a `-1` divisor would not even type-check for an unsigned operand).
pub(super) fn int_type_is_signed(it: IntTy) -> bool {
    match it.sign {
        Sign::Fixed(s) => s,
        Sign::Deferred | Sign::Var(_) => true,
    }
}

/// The UNSIGNED Rust integer type whose bit width matches an integer type's slot — the type a constant
/// bit-pattern literal is written in before casting to the signed/target type (mirroring the wasm
/// backend's `to_i64_bits`/`to_i32_bits`, which emit the two's-complement bit pattern). Used by the
/// expression emitter to write `<bits>u64 as i64` etc., so a negative signed value and an unsigned
/// value at/above the signed max share ONE spelling. `None` for a non-aliased width (which declines
/// upstream before a constant of it is emitted).
pub fn unsigned_bits_type(it: IntTy) -> Option<&'static str> {
    let width = match it.width {
        Width::Fixed(w) => w,
        Width::Deferred | Width::Var(_) => crate::ty::DEFAULT_INT_WIDTH,
    };
    Some(match width {
        8 => "u8",
        16 => "u16",
        32 => "u32",
        64 => "u64",
        // An UNUSUAL width stores in the next-larger UNSIGNED machine type (`UInt48`/`Int48`→`u64` bits);
        // `emit_const_int_at` writes the low-`width`-bit magnitude (`wrap_to(false, width)`) in this type,
        // which fits exactly, then casts to the signed target if needed — mirroring the aliased path.
        w if (1..=64).contains(&w) => storage_width_type(false, w),
        _ => return None,
    })
}
