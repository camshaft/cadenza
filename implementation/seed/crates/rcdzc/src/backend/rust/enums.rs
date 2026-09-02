//! Emitting a Cadenza SUM type as a Rust `enum` declaration.
//!
//! A sum is NOMINAL — unlike a record it has a declared name — so it maps to a Rust `enum` of that name,
//! with one Rust variant per Cadenza variant: a 0-payload variant is a unit variant (`None`), a
//! 1-payload variant carries its payload type (`Some(T)`), a multi-payload variant carries each
//! positionally (`Cons(T0, T1)`). A GENERIC sum (`Option`, with type parameters) emits a generic enum
//! `enum Option<T0> { Some(T0), None }`; a payload that IS a type parameter renders as that `T{i}`, and
//! a use site `Option Int64` becomes `Option<i64>` (via `types::rust_type`).
//!
//! One enum is emitted per DECLARATION (keyed by its `TypeDecl.occ` — the sum's nominal identity), NOT
//! per instantiation, so a generic sum yields one generic enum however many types it is used at. A
//! declaration a program never uses still emits (harmless, `#[allow(dead_code)]`); collecting exactly
//! the reachable ones would be a walk the small declaration count does not warrant.
//!
//! A RECURSIVE sum (`(type IntList (Cons (Tuple Int64 IntList)) Nil)` — a payload mentions the sum
//! itself) would need `Box` indirection in Rust (an enum containing itself by value is infinitely
//! sized); that is deferred, so a self-referential declaration DECLINES.

use super::types;
use crate::db::Db;
use crate::diag::Reject;

/// Emit the `enum` declarations for every sum `TypeDecl` in the program that has a native Rust form.
/// Returns the concatenated declarations (each ends with a newline), to be placed before the functions.
/// A declaration whose variant payloads have no native mapping (or that is recursive) is SKIPPED here —
/// a use of such a sum declines at `rust_type`/selection, attributed to this target, so skipping its
/// declaration is consistent (no enum is emitted that no compilable code names).
pub fn emit_enum_decls(db: &mut Db, mode: super::Mode) -> String {
    let mut out = String::new();
    let n = db.type_decls.len();
    // DEDUP by emitted enum IDENTIFIER: a LINKED multi-module program can carry two declarations that emit
    // the SAME Rust enum name — e.g. a library and the entry each `(type Box (W a) (E))`. In Cadenza these
    // are DISTINCT nominal types (identity = fully-qualified name, per-module namespace), but the rust
    // backend names both `Box`, so emitting both is a duplicate `enum Box` (rustc E0428). When the two
    // emit BYTE-IDENTICAL source (same variants, same payloads — the composing case, where a value of one
    // is matched as the other), keep the FIRST and skip the rest: one `enum Box` serves both. Guard on
    // byte-identity so a genuine same-name/different-shape collision (unsupported) still emits twice and
    // fails loudly rather than silently dropping a distinct type.
    let mut emitted: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for i in 0..n {
        if let Ok(decl_src) = emit_one_enum(db, i, mode) {
            let ident = types::sum_ident(&db.type_decls[i].name);
            match emitted.get(&ident) {
                // A same-ident decl already emitted with identical source — skip (dedup the linked twin).
                Some(prev) if *prev == decl_src => continue,
                _ => {}
            }
            emitted.insert(ident, decl_src.clone());
            out.push('\n');
            out.push_str(&decl_src);
        }
    }
    out
}

/// Emit a machine-readable DESCRIPTOR note per USER sum whose enum emits — the variant structure a
/// boundary render needs but the plain type name (`Opt`) erases. Format, one line per sum:
///   `// cdz-sum[<EnumIdent>]: (<VariantName> <payload-render-name>) (<NullaryVariant>) …`
/// one paren-group per variant IN DISCRIMINANT ORDER: the Cadenza variant name, then (for a payload
/// variant) the payload type's `render_name` (a one-payload variant's payload type; a MULTI-payload
/// variant's single `(Tuple …)`), or nothing for a nullary variant. The gate driver parses this to build
/// a `match` that renders a user-sum value to cdz-run's bare form (`(Sm 42)`, `(Nn unit)`), keyed by the
/// enum ident so it composes with the `cdz-return` type note.
///
/// A built-in `Option`/`Result` renders via the driver's head-type path (it maps to std's, not an emitted
/// enum). A GENERIC user sum's payload is a type PARAMETER, rendered here as the parameter placeholder
/// `T{k}` (via `render_payload_ty` at the sentinel instantiation); the gate driver substitutes the result
/// type's concrete args (`(Box Int64)` → `T0 = Int64`) when it renders, so a generic-sum escape renders
/// like a monomorphic one — a `T{k}`-parameterized descriptor plus a `// cdz-sum-params[Ident]: N` note
/// giving the parameter count so the driver knows how many args to bind.
pub fn emit_sum_descriptors(db: &mut Db) -> String {
    let mut out = String::new();
    let n = db.type_decls.len();
    for i in 0..n {
        let decl = db.type_decls[i].clone();
        // Only a sum whose enum actually emits (non-built-in, non-recursive, native payloads). A GENERIC
        // sum now gets a descriptor too (payloads as `T{k}` placeholders); a monomorphic one has no params.
        // Mode-INVARIANT check: a payload is representable in both modes (only a closure's SPELLING differs —
        // `Rc<dyn Fn>` vs `Rc<dyn EnvClosure>`, both native), and this descriptor path emits no closure type
        // into the module. `Sync` suffices to decide "does the enum emit at all".
        if emit_one_enum(db, i, super::Mode::Sync).is_err() {
            continue;
        }
        // Key the descriptor by the CADENZA name (`decl.name`) — the SAME string `cdz-return` emits (a
        // type's `render_name`), so the driver's return-type→descriptor lookup matches. The driver
        // re-escapes it (its `sum_rust_ident`, mirroring the backend) when building the `prog::<Enum>` path,
        // so a lossy/primitive/keyword name resolves to the ACTUAL emitted enum ident. (For a clean name the
        // Cadenza name and the enum ident coincide; the distinction only matters for an escaped name.)
        let ident = &decl.name;
        let mut groups = Vec::with_capacity(decl.variants.len());
        for variant in &decl.variants {
            let payloads = if decl.params.is_empty() {
                variant_payload_renders(db, variant)
            } else {
                // A generic sum: render each payload at the sentinel instantiation so a type parameter
                // (bare or nested, `(Option a)`) shows as `T{k}` — the placeholder the driver substitutes.
                variant_payload_renders_generic(db, &decl, variant)
            };
            // One token per payload: `(Name)` nullary, `(Name T)` single, `(Name T0 T1 …)` multi-payload
            // (the token COUNT is the arity, so the harness spreads a multi-payload variant flat).
            if payloads.is_empty() {
                groups.push(format!("({})", variant.name));
            } else {
                groups.push(format!("({} {})", variant.name, payloads.join(" ")));
            }
        }
        out.push_str(&format!("// cdz-sum[{ident}]: {}\n", groups.join(" ")));
        // Whether this sum renders its variant heads QUALIFIED (`((. Ast Str) …)`) rather than bare
        // (`(Str …)`) at the value boundary — a PER-SUM property: true iff ANY variant name is bound in the
        // prelude to a NON-variant-ctor (a type ctor / module / value), so a bare head would resolve to that
        // OTHER binding. Computed by the SAME `lower::sum_needs_qualified_heads` the WASM backend uses
        // (both-backend parity — one predicate, so a prelude change updates one place). This is what makes
        // the built-in reflection `Ast` render qualified (`Int`/`Float`/`Bool` are type ctors, `List` the
        // list module → the whole sum qualifies, so `Str`/`Name` qualify too as the per-sum consequence)
        // WHILE a user sum with non-colliding variant names renders bare — the render crate keyed on the
        // type NAME `Ast`, wrongly qualifying any sum so named (the boundary-render divergence this fixes).
        // Emit a bare per-sum marker note the dependency-free render crate consults (it lacks the prelude).
        if crate::lower::sum_needs_qualified_heads(db, decl.occ) {
            out.push_str(&format!("// cdz-sum-qualified-heads[{ident}]\n"));
        }
        // A generic sum records its parameter COUNT so the driver knows how many `T{k}` placeholders to
        // substitute from the result type's args. A monomorphic sum (no params) needs no such note.
        if !decl.params.is_empty() {
            out.push_str(&format!(
                "// cdz-sum-params[{ident}]: {}\n",
                decl.params.len()
            ));
        }
    }
    out
}

/// The payload render tokens of a GENERIC sum's variant IN CADENZA TYPE SYNTAX, with each type PARAMETER
/// shown as its placeholder `T{k}`. A payload is rendered at the sum's SENTINEL instantiation (so a
/// parameter appearing anywhere becomes `Ty::Var(PARAM_SENTINEL_BASE+k)`), then `render_name` (the SAME
/// Cadenza-syntax render the monomorphic descriptors use — `(Option Int64)`, not Rust `Option<T0>`) is
/// post-processed to rewrite each sentinel var `?{BASE+k}` to `T{k}`. So `(W a)` → `T0`, `(W (Option a))` →
/// `(Option T0)` — placeholders the gate driver parses with `parse_head_type` and substitutes with the
/// result type's concrete args. A payload that does not resolve is dropped, matching the monomorphic path.
fn variant_payload_renders_generic(
    db: &mut Db,
    decl: &crate::db::TypeDecl,
    variant: &crate::db::Variant,
) -> Vec<String> {
    // Resolve every payload type FIRST (needs `&mut db`), then render (needs `&db` via `NameCtx`) — the two
    // borrows cannot overlap, so collect the types before taking the render context.
    let tys: Vec<crate::ty::Ty> = variant
        .payloads
        .iter()
        .filter_map(|&occ| sentinel_payload_ty(db, decl, occ))
        .collect();
    let ncx = db.name_ctx();
    tys.iter()
        .map(|ty| render_sentinel_payload(ty, &ncx))
        .collect()
}

/// Render a sentinel-instantiated payload type in Cadenza TYPE SYNTAX (the SAME surface `render_name`
/// produces — `(Option Int64)`, not Rust `Option<T0>`), EXCEPT each sentinel param var
/// `Ty::Var(PARAM_SENTINEL_BASE+k)` renders as its placeholder `T{k}` — the token the gate driver parses
/// with `parse_head_type` and substitutes with the result type's concrete args. So `(W a)` → `T0`, `(W
/// (Option a))` → `(Option T0)`. This mirrors `Ty::render_name`'s structural arms but overrides only the
/// sentinel-var leaf (which `render_name` would print as the diagnostic hole `_`); every other leaf/compound
/// delegates to `render_name` so the two stay in lock-step. (Was a fake nullary `Ty::Sum` named `T{k}`
/// rendered through `render_name` — the type no longer carries a `name`, so the placeholder is spelled here.)
fn render_sentinel_payload(ty: &crate::ty::Ty, ncx: &crate::ty::NameCtx) -> String {
    use crate::ty::Ty;
    match ty {
        Ty::Var(n) if *n >= PARAM_SENTINEL_BASE => format!("T{}", n - PARAM_SENTINEL_BASE),
        Ty::List(e) => format!("(List {})", render_sentinel_payload(e, ncx)),
        Ty::Set(e) => format!("(Set {})", render_sentinel_payload(e, ncx)),
        Ty::Map(k, v) => format!(
            "(Map {} {})",
            render_sentinel_payload(k, ncx),
            render_sentinel_payload(v, ncx)
        ),
        Ty::Tuple(elems) => {
            let mut s = String::from("(Tuple");
            for e in elems.iter() {
                s.push(' ');
                s.push_str(&render_sentinel_payload(e, ncx));
            }
            s.push(')');
            s
        }
        Ty::Sum { decl, args } => {
            let name = ncx.name_of(*decl).unwrap_or("<sum>");
            if args.is_empty() {
                name.to_string()
            } else {
                let mut s = format!("({name}");
                for a in args.iter() {
                    s.push(' ');
                    s.push_str(&render_sentinel_payload(a, ncx));
                }
                s.push(')');
                s
            }
        }
        Ty::Nominal { inner, .. } => render_sentinel_payload(inner, ncx),
        // No sentinel var inside — spell exactly as `render_name` (scalars/text/etc.).
        _ => ty.render_name(ncx),
    }
}

/// Emit a machine-readable DESCRIPTOR note per erased NEWTYPE — the inner type its name erases to, so the
/// gate's value renderer can render a `Pt`-typed boundary value structurally. Format, one line per newtype:
///   `// cdz-newtype[<Ident>]: <inner-render-name>`
/// A newtype `(type Pt (Mk (Tuple Int64 Int64)))` ERASES (`db.newtype_inner`): its runtime value IS the
/// inner tuple, and `Ty::Nominal`'s `render_name` is just the bare name `Pt`. Without this note the gate
/// renderer meets the return type `Pt`, finds no record/tuple/sum match, and falls to a scalar `Display`
/// of the erased Rust tuple `(i64, i64)` → rustc E0277. The note lets it resolve `Pt` to its inner type
/// `(Tuple Int64 Int64)` and render the VALUE structurally as `(tuple 5 5)` (the tag-erased bare compound,
/// what the wasm gate produces). Only a MONOMORPHIC newtype gets one — a generic newtype's inner mentions a
/// type parameter with no concrete render form (the same restriction `emit_sum_descriptors` draws).
pub fn emit_newtype_descriptors(db: &mut Db) -> String {
    let mut out = String::new();
    let n = db.type_decls.len();
    for i in 0..n {
        let decl = db.type_decls[i].clone();
        if !decl.params.is_empty() {
            continue; // generic newtype — inner mentions a type param, no concrete render form.
        }
        let Some(inner) = db.newtype_inner.get(&decl.occ).cloned() else {
            continue; // not an erased newtype.
        };
        let ident = types::sum_ident(&decl.name);
        out.push_str(&format!(
            "// cdz-newtype[{ident}]: {}\n",
            inner.render_name(&db.name_ctx())
        ));
    }
    out
}

/// The `render_name` of EACH of a variant's payload types, in order — an EMPTY vec for a nullary variant,
/// ONE entry for a single-payload variant, N entries for a MULTI-payload variant. The descriptor emits one
/// token per entry, so the token COUNT carries the variant's ARITY: a multi-payload variant `(P Int64
/// (Option Int64))` (two tokens) renders its payloads SPREAD FLAT under the variant name — `(P 5 (Some 5))`
/// — matching the wasm value form, whereas a single-payload variant carrying a TUPLE `(Q (Tuple Int64
/// Int64))` (one token) keeps the nested `(Q (tuple …))`. Collapsing a multi-payload variant to one `(Tuple
/// …)` token (as before) made the two INDISTINGUISHABLE, so the rust gate rendered `(P (tuple …))` where
/// wasm flattens. Used only for monomorphic sums, so `typeval_of` yields a concrete type per payload.
fn variant_payload_renders(db: &mut Db, variant: &crate::db::Variant) -> Vec<String> {
    variant
        .payloads
        .iter()
        .filter_map(|&occ| {
            crate::eval::typeval_of(db, occ).map(|ty| ty.render_name(&db.name_ctx()))
        })
        .collect()
}

/// Emit one sum declaration `db.type_decls[i]` as a Rust `enum`, or decline (a variant payload with no
/// native mapping, or a recursive sum). `Err` means "skip this declaration" — the caller drops it.
fn emit_one_enum(db: &mut Db, i: usize, mode: super::Mode) -> Result<String, Reject> {
    let decl = db.type_decls[i].clone();
    // The BUILT-IN `Option`/`Result` map to RUST'S OWN `Option`/`Result` (the operator's ask — idiomatic
    // + trivially usable from Rust), so DON'T emit a synthetic `enum Option { … }` that would shadow
    // `std`'s. A use `Ty::Sum{Option, [T]}` renders `Option<T>` and a ctor `Some(x)`/`Option::None`
    // resolves to std's — the SAME `Enum::Variant` path syntax. A USER `(type Option …)` (a source-node
    // declaration) still emits its own enum (it shadows the prelude in Cadenza; here it needs a real
    // type). `is_builtin_std_sum` is the recognizer.
    if is_builtin_std_sum(db, &decl) {
        return Err(Reject::decline("built-in Option/Result maps to Rust's own"));
    }
    // A PRELUDE sum (`Sign`, `Ordering` — non-user nodes) whose emitted enum name is SHADOWED by a USER
    // declaration of the same sanitized name is SKIPPED: the user `(type Sign …)` shadows the prelude in
    // Cadenza (a source `Sign` resolves to the user's), so the prelude enum is unreferenceable — emitting
    // both would be a duplicate `enum Sign` (rustc E0428). The user decl emits the one `enum Sign`. (The
    // prelude sums are emitted unconditionally otherwise; only a same-name user decl suppresses one.)
    if !db.is_user_node(decl.occ) && user_decl_shadows_name(db, &decl) {
        return Err(Reject::decline(
            "a user declaration shadows this prelude sum's name",
        ));
    }
    // An erasable NEWTYPE emits NO enum — its runtime value IS the underlying payload (the tag adds
    // nothing), so `types::rust_type` maps a `Ty::Nominal` THROUGH to its `inner` Rust type and no boxed
    // enum is needed. (Both monomorphic and generic newtypes: a use `(: b UserId)` → the inner type
    // directly. Without this skip a dead `enum UserId { Mk(i64) }` was emitted — harmless `#[allow(dead_code)]`
    // clutter for the monomorphic case, but the value never uses it, so drop it uniformly.)
    if db.newtype_inner.contains_key(&decl.occ) {
        return Err(Reject::decline("an erased newtype has no boxed enum"));
    }
    let name = types::sum_ident(&decl.name);
    // The type parameters, `<T0, T1, …>` for a generic sum (empty for a monomorphic one). Their order is
    // `decl.params`' first-appearance order — the SAME order `Ty::Sum::args` and `types::rust_type`'s
    // `Option<i64>` use, so a payload param `params[k]` renders as `T{k}`.
    let generics = if decl.params.is_empty() {
        String::new()
    } else {
        let ps: Vec<String> = (0..decl.params.len()).map(|k| format!("T{k}")).collect();
        format!("<{}>", ps.join(", "))
    };

    let mut variants = Vec::with_capacity(decl.variants.len());
    for variant in &decl.variants {
        let vname = types::sum_ident(&variant.name);
        // Each payload type-expression → its Rust type, with a type-PARAMETER payload rendered as its
        // `T{k}`. A payload with no native mapping declines the whole enum.
        let mut payloads = Vec::with_capacity(variant.payloads.len());
        for &pty_occ in &variant.payloads {
            payloads.push(payload_rust_type(db, pty_occ, &decl, mode)?);
        }
        // A RECURSIVE variant — one whose payload mentions THIS sum (`(Cons Int64 L)`, `(Node L L)`) —
        // BOXES its whole payload field: a Rust enum containing itself by value is infinitely sized, so
        // the recursion goes behind one `Box<…>` indirection at the variant field. The box wraps the WHOLE
        // payload (`Cons(Box<(i64, L)>)`, `Node(Box<(L, L)>)`), a uniform ONE-box-per-recursive-variant
        // scheme the construct (`Box::new(payload)`) and match (`*__pay`) sites agree on — simpler than
        // boxing each recursive sub-position (which would need the box/deref threaded through every tuple
        // element). A non-recursive variant is unboxed as before.
        let recursive = variant_payloads_mention(db, variant, &decl);
        let field = |ty: String| {
            // Fully-qualify the indirection `Box` (`::std::boxed::Box`, not the prelude's) so a USER sum
            // NAMED `Box` — `(type Box (W a) (E))` → `enum Box<T0>` — cannot shadow the heap pointer the
            // recursion depends on. A bare `Box<…>` here would resolve to the user enum, making a recursive
            // `Tree` infinitely sized (E0072); the qualified path is immune to any sum-name collision.
            if recursive {
                format!("::std::boxed::Box<{ty}>")
            } else {
                ty
            }
        };
        match payloads.len() {
            // A nullary variant is a unit variant (`None`).
            0 => variants.push(vname),
            // A one-payload variant carries its payload directly (`Some(T)`), boxed if recursive.
            1 => variants.push(format!("{vname}({})", field(payloads[0].clone()))),
            // A MULTI-payload variant's payload is ONE TUPLE, not several positional fields — the core
            // models a multi-payload variant's payload as a single `Ty::Tuple` (`core.rs` SumNew: "a tuple
            // handle built from the payloads"; the front-end types it `Ty::Tuple`), and the match side reads
            // it as one bound value indexed `.0`/`.1`. So the enum field is that tuple: `Cons((T0, T1))` —
            // the SAME shape construction (`SumNew`) and matching (`SumPayload`) agree on. (A native
            // `Cons(T0, T1)` would disagree with the single-tuple payload the match binds → non-compiling.)
            _ => variants.push(format!(
                "{vname}({})",
                field(format!("({})", payloads.join(", ")))
            )),
        }
    }

    // The `=` intrinsic lowers (rust backend) to a native `x == y`, which needs `PartialEq`/`Eq`; the wasm
    // backend does a value-heap equality walk, so the derive is what makes the two backends AGREE on
    // realizability of a runtime sum `=`. Derive `PartialEq, Eq` whenever every payload type is itself
    // equality-derivable (an all-nullary enum trivially is; a payload of Int/Bool/nested comparable sum/
    // tuple/record is too) — a `derive(PartialEq, Eq)` over such fields compiles, so a runtime `(= a b)`
    // over the sum emits `a == b`. A payload that is NOT `Eq`-derivable (a float — `PartialEq` but not
    // `Eq`; a fn/closure; a `List`/`Map`/`Set`; a recursive `Box` field IS fine — `Box<T: Eq>: Eq`) keeps
    // `Clone` only, so its `ValueEq` still declines (decline-don't-miscompile), as before.
    // When every payload is `Eq`-derivable it is ALSO `Ord`-derivable (Rust's `PartialOrd`/`Ord` derives
    // compose over the same fields `Eq` does — an Int/Bool/nested-comparable payload is `Ord`), so derive
    // `PartialOrd, Ord` too. That lets a user sum be a `BTreeMap`/`BTreeSet` KEY/element (which needs `Ord`)
    // — a `(Map C V)` keyed by a nullary/comparable sum. The derived order is lexicographic (variant
    // declaration order, then payloads) — a valid canonical key order, and the map/set are compared only
    // for lookup identity, not an observable ordering, so any total order is sound. A non-`Eq` payload (a
    // float/closure/collection) keeps `Clone` only and such a sum cannot be a map key (it declines).
    let derives = if sum_derives_eq(db, &decl) {
        "Clone, PartialEq, Eq, PartialOrd, Ord"
    } else {
        "Clone"
    };
    // `#[derive(Clone)]` so a matched-and-rebuilt value (a payload read then re-wrapped) and a value used
    // more than once compose without move-out errors — the emitted code treats a sum value like any other
    // Cadenza value (freely copyable, pure). `#[allow(dead_code)]` because a declared-but-unused variant
    // (or the whole enum) is normal in generated code.
    // `pub` so the enum crosses the module boundary with the `pub fn`s that construct/consume it — a
    // consumer of the emitted `.rs` (or the gate's driver) can name the type and its variants. `Clone`
    // so a matched-and-rebuilt value composes; `#[allow(dead_code)]` since a declared-but-unused variant
    // is normal in generated code. Variants are `pub` implicitly (an enum's variants share its visibility).
    let enum_decl = format!(
        "#[derive({derives})]\n#[allow(dead_code)]\npub enum {name}{generics} {{ {} }}\n",
        variants.join(", ")
    );
    // A float-carrying MONOMORPHIC sum (`Ast` — a `Float`+`List Ast` sum) cannot `#[derive(Eq/Ord)]` (f64 is
    // not `Eq`/`Ord`), so it emits `#[derive(Clone)]` only above. But if it is used as a `BTreeSet`/`BTreeMap`
    // key/element the collection needs `Ord` (+ `Eq`). For such a sum we emit HAND-WRITTEN
    // `impl PartialEq/Eq/PartialOrd/Ord` that delegate to `__eq_<Ident>`/`__ord_<Ident>` walk helpers
    // (float leaf by canonical bits, recursion via the helper's call-indirection — the SAME walks a runtime
    // `=`/`compare` uses). `sum_is_custom_ord` gates this (monomorphic, float-walkable, no flip-Option). The
    // helpers are emitted alongside (deduped by name). This makes `BTreeSet<Ast>` instantiable with an order
    // that agrees byte-for-byte with the wasm value-cmp walk.
    let sum_ty = sentinel_sum_of(&decl); // monomorphic → args empty
    if crate::backend::rust::expr::sum_is_custom_ord(db, &sum_ty) {
        let mut helpers: Vec<String> = Vec::new();
        // Delegating bodies (also populate `helpers` with the recursive `__eq_`/`__ord_` fns).
        let eq_body = crate::backend::rust::expr::emit_value_eq_walk(
            db,
            &sum_ty,
            "self",
            "other",
            &mut helpers,
        );
        let ord_body = crate::backend::rust::expr::emit_value_ord_walk(
            db,
            &sum_ty,
            "self",
            "other",
            &mut helpers,
        );
        if let (Ok(eq_body), Ok(ord_body)) = (eq_body, ord_body) {
            let impls = format!(
                "\n{helpers}\nimpl PartialEq for {name} {{ fn eq(&self, other: &Self) -> bool {{ {eq_body} }} }}\n\
                 impl Eq for {name} {{}}\n\
                 impl PartialOrd for {name} {{ fn partial_cmp(&self, other: &Self) -> core::option::Option<core::cmp::Ordering> {{ core::option::Option::Some(self.cmp(other)) }} }}\n\
                 impl Ord for {name} {{ fn cmp(&self, other: &Self) -> core::cmp::Ordering {{ {ord_body} }} }}\n",
                helpers = helpers.join("\n"),
            );
            return Ok(format!("{enum_decl}{impls}"));
        }
        // A walk declined (an unexpected payload shape) — fall back to the plain enum (Clone only); a use as
        // a key then declines at the construction site, as before (reject-don't-miscompile).
    }
    Ok(enum_decl)
}

/// The monomorphic-or-sentinel `Ty::Sum` for a declaration — `args` are the sentinel params (empty for a
/// monomorphic sum). Used to query `sum_is_custom_ord` / drive the eq/ord walk generators for the enum's
/// hand-written impls.
fn sentinel_sum_of(decl: &crate::db::TypeDecl) -> crate::ty::Ty {
    crate::ty::Ty::Sum {
        decl: decl.occ,
        args: (0..decl.params.len())
            .map(|k| crate::ty::Ty::Var(PARAM_SENTINEL_BASE + k as u32))
            .collect(),
    }
}

/// Whether some USER (source-node) declaration emits the SAME Rust enum ident as `decl` — i.e. a user
/// `(type Sign …)` shadows the prelude `Sign`, or (more generally) two declarations whose names sanitize to
/// the same ident collide. Used to suppress a PRELUDE sum whose name a user re-declared, so only one
/// `enum <ident>` is emitted (rustc rejects a duplicate, E0428). Compares the emitted `sum_ident`, so a
/// `-`/`_` sanitization collision (`foo-bar` vs `foo_bar`) is caught too — though for two USER decls that
/// is a genuine ambiguity the front-end should reject; here we only use it to let a user decl win over a
/// prelude one.
fn user_decl_shadows_name(db: &Db, decl: &crate::db::TypeDecl) -> bool {
    let ident = types::sum_ident(&decl.name);
    db.type_decls.iter().any(|other| {
        other.occ != decl.occ
            && db.is_user_node(other.occ)
            && types::sum_ident(&other.name) == ident
    })
}

/// Whether `decl` is the BUILT-IN `Option`/`Result` that maps to Rust's own `Option`/`Result` — a
/// PRELUDE (non-user) declaration named `Option` or `Result` with the matching variant shape. A user
/// `(type Option …)` (a source-node declaration — `is_user_node(occ)`) is NOT built-in: it shadows the
/// prelude in Cadenza and gets its own emitted enum. Checked by occurrence provenance + name + variants,
/// not name alone, so a user's differently-shaped `Option` is not silently mapped to std's.
pub(super) fn is_builtin_std_sum(db: &Db, decl: &crate::db::TypeDecl) -> bool {
    if db.is_user_node(decl.occ) {
        return false; // a user declaration shadows — emit its own enum
    }
    let vnames: Vec<&str> = decl.variants.iter().map(|v| v.name.as_str()).collect();
    match decl.name.as_str() {
        "Option" => vnames == ["Some", "None"],
        "Result" => vnames == ["Ok", "Err"],
        _ => false,
    }
}

/// The Rust type of a variant payload declared at type-expression occurrence `pty_occ`, within
/// declaration `decl`. A payload that IS one of the sum's type PARAMETERS renders as that param's `T{k}`;
/// otherwise it is a concrete type resolved via the type machinery and mapped by `types::rust_type`.
/// Declines (so the enum is skipped) for a payload with no native mapping OR a RECURSIVE payload (one
/// whose resolved type mentions THIS declaration — a self-referential enum needs `Box`, deferred).
fn payload_rust_type(
    db: &mut Db,
    pty_occ: crate::ast::StructId,
    decl: &crate::db::TypeDecl,
    mode: super::Mode,
) -> Result<String, Reject> {
    // A bare type-parameter payload — the payload type-expr is a lowercase name that IS one of the
    // declaration's params. Render it as the corresponding `T{k}` (the enum's type parameter).
    if let Some(pname) = db.ast.as_name(pty_occ)
        && let Some(k) = decl.params.iter().position(|p| p == pname)
    {
        return Ok(format!("T{k}"));
    }
    // Otherwise resolve the payload type-expression to a solved `Ty` and map it. A payload that mentions
    // THIS sum (a recursive sum) is rendered NORMALLY here — the self-reference maps to the enum name, made
    // finite by the `Box<…>` the caller (`emit_one_enum`) wraps the recursive variant's field in. For a
    // GENERIC sum the self-reference must carry the decl's OWN type parameters (`Tree<T0>`, not a bare
    // `Tree` — a bare mention is E0107 "missing generics"), so `render_payload_ty` renders a self-`Ty::Sum`
    // at the decl's params; a `(Tuple Tree Tree)` payload → `(Tree<T0>, Tree<T0>)`, boxed by the caller.
    // For a GENERIC sum, render off the payload type at a SENTINEL instantiation `Sum<Var(BASE+0),
    // Var(BASE+1), …>` — a distinct sentinel var per param, in declaration order — so a param appearing
    // ANYWHERE in the payload (including NESTED, `(W (Option a))` → `Option<Var(BASE+0)>`) carries a
    // sentinel var that `render_payload_ty` renders as `T{k}`. Without this, a nested param reached
    // `types::rust_type(Ty::Var)` = None and the whole generic enum declined ("no native representation").
    // For a MONOMORPHIC sum (no params) render `typeval_of` directly — no sentinel needed.
    let ty = if decl.params.is_empty() {
        crate::eval::typeval_of(db, pty_occ)
            .ok_or_else(|| Reject::decline("a variant payload type does not resolve"))?
    } else {
        sentinel_payload_ty(db, decl, pty_occ)
            .ok_or_else(|| Reject::decline("a variant payload type does not resolve"))?
    };
    render_payload_ty(&db.name_ctx(), &ty, decl, mode).ok_or_else(|| {
        Reject::decline(format!(
            "a variant payload type {} has no native Rust representation",
            ty.render_name(&db.name_ctx())
        ))
    })
}

/// The base for SENTINEL param vars — a value far above any real inference var, so a sentinel never
/// collides with a genuine free var in a payload type. A payload var `PARAM_SENTINEL_BASE + k` is the
/// sum's `k`-th type parameter, rendered `T{k}`.
const PARAM_SENTINEL_BASE: u32 = 1 << 24;

/// The type of the payload at occurrence `pty_occ`, computed at the SENTINEL instantiation of `decl`
/// (`Sum<Var(BASE), Var(BASE+1), …>`) — so each param position `k`, wherever it appears (nested inside
/// `Option`/`Tuple`/…), carries `Var(BASE+k)`, which `render_payload_ty` renders as `T{k}`. Finds the
/// owning variant's ctor (the scheme relating payload → params) and peels it at the sentinel sum via
/// `payload_ty_at_instantiation`. `None` if the ctor/scheme is unavailable (the enum then declines, as
/// before). This is what lets a generic sum whose param is NESTED in a variant payload emit its enum.
fn sentinel_payload_ty(
    db: &mut Db,
    decl: &crate::db::TypeDecl,
    pty_occ: crate::ast::StructId,
) -> Option<crate::ty::Ty> {
    use crate::ty::Ty;
    let ctor = decl
        .variants
        .iter()
        .find(|v| v.payloads.contains(&pty_occ))
        .and_then(|v| v.ctor)?;
    let sentinel_sum = Ty::Sum {
        decl: decl.occ,
        args: (0..decl.params.len())
            .map(|k| Ty::Var(PARAM_SENTINEL_BASE + k as u32))
            .collect(),
    };
    let payload = crate::infer::payload_ty_at_instantiation(db, ctor, &sentinel_sum)?;
    // A MULTI-payload variant's `payload_ty_at_instantiation` returns the whole `Ty::Tuple` of all its
    // payloads; but `payload_rust_type` is called PER payload occurrence. Select this occurrence's element
    // by its position in the variant, so a multi-payload generic variant renders each field's own type.
    let variant = decl
        .variants
        .iter()
        .find(|v| v.payloads.contains(&pty_occ))?;
    if variant.payloads.len() > 1 {
        let idx = variant.payloads.iter().position(|&p| p == pty_occ)?;
        if let Ty::Tuple(elems) = &payload {
            return elems.get(idx).cloned();
        }
    }
    Some(payload)
}

/// Render a payload type to its Rust form WITHIN declaration `decl` — like [`types::rust_type`], but a
/// SELF-REFERENCE to `decl` (a `Ty::Sum`/`Ty::Nominal` of the same `occ`) is rendered with the decl's OWN
/// type parameters (`Tree<T0, T1>` for a generic decl, bare `Tree` for a monomorphic one) rather than
/// whatever args the bare self-mention carries (none). This is what makes a generic recursive sum's
/// self-referential payload name the enum correctly (E0107 otherwise). Non-self sums/compounds delegate to
/// `types::rust_type` (their args are concrete). `None` for a type with no native Rust form.
fn render_payload_ty(
    ncx: &crate::ty::NameCtx,
    ty: &crate::ty::Ty,
    decl: &crate::db::TypeDecl,
    mode: super::Mode,
) -> Option<String> {
    use crate::ty::Ty;
    // A SENTINEL param var (`PARAM_SENTINEL_BASE + k`) is the sum's k-th type parameter — render `T{k}`.
    // This is what lets a param appearing ANYWHERE in a payload (nested in `Option`/`Tuple`/a self-ref's
    // args) render as the enum's type parameter, not decline as an unmappable `Ty::Var`.
    if let Ty::Var(n) = ty
        && *n >= PARAM_SENTINEL_BASE
    {
        return Some(format!("T{}", n - PARAM_SENTINEL_BASE));
    }
    // A self-reference — the recursive mention of THIS declaration. Render the enum name + its args
    // rendered recursively (a sentinel-var arg → `T{k}`, a concrete arg → its type; the recursion is
    // closed by the enclosing `Box`). A monomorphic self-ref is the bare name.
    if let Ty::Sum { decl: d, args, .. } | Ty::Nominal { decl: d, args, .. } = ty
        && *d == decl.occ
    {
        let name = types::sum_ident(&decl.name);
        if decl.params.is_empty() {
            return Some(name);
        }
        // Render the self-ref's args (they carry the sentinel vars for a generic recursive sum). Fall back
        // to the positional `T{k}` if an arg is missing (a bare self-mention with no args).
        let ps: Vec<String> = if args.len() == decl.params.len() {
            args.iter()
                .map(|a| render_payload_ty(ncx, a, decl, mode))
                .collect::<Option<Vec<_>>>()?
        } else {
            (0..decl.params.len()).map(|k| format!("T{k}")).collect()
        };
        return Some(format!("{name}<{}>", ps.join(", ")));
    }
    // A compound that may CONTAIN a self-reference or a param — recurse so a nested `(Tuple Tree Tree)` /
    // `(List Tree)` / `(Option a)` renders its inner self-refs + params. Non-self concrete leaves delegate
    // to `types::rust_type`.
    match ty {
        Ty::Tuple(elems) => {
            let parts: Option<Vec<String>> = elems
                .iter()
                .map(|e| render_payload_ty(ncx, e, decl, mode))
                .collect();
            let parts = parts?;
            match parts.len() {
                0 => Some("()".to_string()),
                1 => Some(format!("({},)", parts[0])),
                _ => Some(format!("({})", parts.join(", "))),
            }
        }
        // A NON-self sum/nominal with args (`(Option a)`, `(Result a b)`) — render its head + each arg
        // recursively so a param arg becomes `T{k}`. A no-arg sum delegates to `types::rust_type` (a
        // concrete monomorphic sum name). The head name uses the built-in std mapping for Option/Result
        // via `types::rust_type` on the ARGS-STRIPPED shape is not needed — render the name + args here.
        Ty::Sum { decl: d, args } if !args.is_empty() => {
            let parts: Option<Vec<String>> = args
                .iter()
                .map(|a| render_payload_ty(ncx, a, decl, mode))
                .collect();
            let ident = types::sum_ident(ncx.name_of(*d)?);
            Some(format!("{ident}<{}>", parts?.join(", ")))
        }
        // A `List`/`Map`/`Set` element is NOT rendered (collections-as-values are unrealized on the Rust
        // backend — `types::rust_type` has no arm; a `Vec<…>` field would be an enum the construct/match
        // paths can't handle). A closure payload (`Ty::Fn`) delegates to the MODE-AWARE spelling: in async
        // mode a closure crosses as `Rc<dyn EnvClosure<A,R>>` (the enum FIELD must match the `Rc<dyn
        // EnvClosure>` VALUE a `Core::SumNew` constructs — else E0308), in sync mode the plain `Rc<dyn Fn>`.
        // A closure-free leaf is byte-identical (`async_closure_type` == `rust_type` off `Ty::Fn`).
        _ => super::async_or_rust_type(ncx, ty, mode),
    }
}

/// Whether declaration `decl`'s emitted enum can `#[derive(PartialEq, Eq)]` — i.e. every payload type of
/// every variant is itself `Eq`-derivable ([`ty_derives_eq`]). An all-nullary enum trivially qualifies
/// (no fields). This is the rust-backend condition under which a runtime `(= a b)` over the sum emits a
/// native `a == b`; a sum with a non-`Eq` payload (a float, a fn/closure, a collection) does not derive
/// it and its `ValueEq` declines. Recursion terminates via a `visited` set (a self-referential payload
/// `Box<Self>` is `Eq` iff `Self` is — the cycle is assumed `Eq` and confirmed by the other variants).
pub(super) fn sum_derives_eq(db: &mut Db, decl: &crate::db::TypeDecl) -> bool {
    let mut visited = std::collections::HashSet::new();
    visited.insert(decl.occ);
    // Evaluate each payload at the SENTINEL instantiation, so a type PARAMETER (bare or nested) appears as
    // a sentinel var. A generic enum's `#[derive(PartialEq, Eq)]` adds a `T{k}: Eq` bound automatically, so
    // a param payload IS Eq-derivable — `ty_derives_eq` treats a sentinel var as Eq. (For a monomorphic sum
    // the sentinel instantiation is empty and `typeval_of` would do; using the sentinel path uniformly is
    // simplest.) A payload whose ctor/scheme is unavailable falls back to `typeval_of` (the raw payload).
    let occs: Vec<(crate::ast::StructId, crate::ast::StructId)> = decl
        .variants
        .iter()
        .filter_map(|v| v.ctor.map(|c| (c, v.payloads.clone())))
        .flat_map(|(c, ps)| ps.into_iter().map(move |p| (c, p)))
        .collect();
    occs.iter().all(|&(_ctor, pty)| {
        let ty = sentinel_payload_ty(db, decl, pty).or_else(|| crate::eval::typeval_of(db, pty));
        ty.is_some_and(|ty| ty_derives_eq(db, &ty, &mut visited))
    })
}

/// Public entry: whether a runtime `(= a b)` over `ty` can emit a native Rust `==` — i.e. `ty` maps to a
/// Rust type that derives `Eq`/`PartialEq`. Wraps [`ty_derives_eq`] with a fresh `visited` set. Used by the
/// expr backend's `Core::ValueEq` to decide `==` vs decline; mirrors the enum-emit derive condition so the
/// `==` type-checks against the emitted derives.
pub(super) fn ty_supports_eq(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    ty_derives_eq(db, ty, &mut std::collections::HashSet::new())
}

/// Ground a comparison operand type whose ONLY obstacle to a native `==` is a genuinely-free (non-sentinel)
/// `Ty::Var`/`Any` — a PHANTOM type position no value ever flows through. Such a var appears when a variant
/// is never constructed (`Result Int64 ?e` where no `Err` is built leaves Err's `?e` free), so the `==`
/// never actually compares a value of that type. Rust's `#[derive(Eq)]` for the emitted enum bounds each
/// arg `Eq`, and rustc's inference is GLOBAL — a bare `Result::Ok(5) == Result::Ok(k)` leaves `E`
/// un-inferable ("type annotations needed"). Grounding the phantom var to `()` (zero-size, `Eq`, has a
/// nameable Rust type) lets the caller pin `E` via a typed `let` so rustc can instantiate the enum. Sound
/// for the SAME reason the wasm layout grounds a free-var heap slot to the i64 cell: a free var at codegen
/// means dead/phantom — a LIVE value never has a free-var type (inference would have solved it), so the
/// choice of `()` can never disagree with a value that flows there.
///
/// Returns `Some(grounded)` only when the grounded type genuinely supports native eq (so a type that
/// declines for a REAL reason — a `String` leaf, a float, a collection — still returns `None` and the
/// caller declines unchanged). Leaves a SENTINEL var (a generic sum's own param) untouched — it is already
/// `Eq` via the derive's bound and is never present in a solved `type_of`.
pub(super) fn ground_free_for_eq(db: &mut Db, ty: &crate::ty::Ty) -> Option<crate::ty::Ty> {
    let grounded = ground_free_vars(ty);
    if ty_supports_eq(db, &grounded) {
        Some(grounded)
    } else {
        None
    }
}

/// Replace every genuinely-free (non-sentinel) `Ty::Var`/`Any` in `ty` with `Ty::Unit`, recursing through
/// the compound structure (sum args, tuple/record elements, a nominal's args + inner). A sentinel var (the
/// generic-sum descriptor placeholder, `>= PARAM_SENTINEL_BASE`) is preserved. See [`ground_free_for_eq`].
fn ground_free_vars(ty: &crate::ty::Ty) -> crate::ty::Ty {
    use crate::ty::Ty;
    match ty {
        Ty::Var(n) if *n < PARAM_SENTINEL_BASE => Ty::Unit,
        Ty::Any => Ty::Unit,
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(ground_free_vars).collect()),
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), ground_free_vars(v)))
                .collect(),
        )),
        Ty::Sum { decl, args } => Ty::Sum {
            decl: *decl,
            args: args.iter().map(ground_free_vars).collect(),
        },
        Ty::Nominal { decl, args, inner } => Ty::Nominal {
            decl: *decl,
            args: args.iter().map(ground_free_vars).collect(),
            inner: std::rc::Rc::new(ground_free_vars(inner)),
        },
        other => other.clone(),
    }
}

/// Whether the solved type `ty` maps to a Rust type that derives `Eq` (hence `PartialEq`) — the condition
/// for a native `==`. Int/Bool/Unit/enum-disc + a nominal newtype over such + a tuple/record/Option/Result/
/// user-sum of such all qualify; a FLOAT (`PartialEq` but not `Eq`), a FUNCTION, a `List`/`Map`/`Set`, or a
/// `Ty::Var`/`Any` (unknown) do NOT. Recurses with a `visited` set of sum decls so a recursive sum
/// (`Box<Self>`) terminates: a back-edge to an in-progress decl is treated as `Eq` (its own variants are
/// checked at the top level), so `enum L { Cons(Box<(i64, L)>), Nil }` derives `Eq` iff `i64` does.
pub(super) fn ty_derives_eq(
    db: &mut Db,
    ty: &crate::ty::Ty,
    visited: &mut std::collections::HashSet<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Int(_) | Ty::Bool | Ty::Unit => true,
        // A SENTINEL var is the sum's OWN type PARAMETER (`T{k}`). A generic enum's `#[derive(PartialEq,
        // Eq)]` adds a `T{k}: PartialEq + Eq` bound automatically, so a param payload IS Eq-derivable — the
        // derive compiles, and a runtime `=` at a concrete instantiation (`Box Int64`) type-checks because
        // `i64: Eq`. Treat it as Eq. (A NON-sentinel free `Ty::Var` is a genuinely-unknown type → the
        // catch-all below rejects it.)
        Ty::Var(n) if *n >= PARAM_SENTINEL_BASE => true,
        // A float is `PartialEq` but NOT `Eq` — exclude it (a sum carrying a float can't `#[derive(Eq)]`).
        Ty::Float(_) => false,
        Ty::Tuple(elems) => elems.iter().all(|e| ty_derives_eq(db, e, visited)),
        Ty::Record(fields) => fields.values().all(|t| ty_derives_eq(db, t, visited)),
        Ty::Nominal { inner, args, .. } => {
            ty_derives_eq(db, inner, visited) && args.iter().all(|a| ty_derives_eq(db, a, visited))
        }
        Ty::Sum { decl, args, .. } => {
            // The type args must be Eq (`Option<f64>` is not Eq); check them first.
            if !args.iter().all(|a| ty_derives_eq(db, a, visited)) {
                return false;
            }
            // A built-in Option/Result maps to std's `Option`/`Result`, which derive `Eq` iff their type
            // args do — already checked above. Don't walk their (parametric) decl payloads (a `Some`
            // payload resolves to the unsubstituted param `a` = a `Ty::Var`, which would spuriously fail).
            if let Some(d) = db.type_decl_by_occ(*decl).cloned()
                && is_builtin_std_sum(db, &d)
            {
                return true;
            }
            // A USER sum is Eq iff its declaration's own payloads are (at the sentinel instantiation, so a
            // type PARAMETER payload counts as Eq — the generic enum's derive bounds it `T: Eq`, and the
            // args checked above are the concrete instantiation). Recurse once (visited-guarded); a
            // back-edge to an in-progress decl is assumed Eq (confirmed at top level). Delegate to
            // `sum_derives_eq`, which walks the decl's payloads at the sentinel instantiation.
            if !visited.insert(*decl) {
                return true;
            }
            match db.type_decl_by_occ(*decl).cloned() {
                Some(d) => {
                    // Inline `sum_derives_eq`'s payload walk sharing THIS `visited` set (so the cycle guard
                    // spans the whole recursion), evaluating each payload at the sentinel instantiation.
                    let occs: Vec<crate::ast::StructId> =
                        d.variants.iter().flat_map(|v| v.payloads.clone()).collect();
                    occs.iter().all(|&pty| {
                        let pt = sentinel_payload_ty(db, &d, pty)
                            .or_else(|| crate::eval::typeval_of(db, pty));
                        pt.is_some_and(|pt| ty_derives_eq(db, &pt, visited))
                    })
                }
                None => false,
            }
        }
        // `String`/`Char`/`Bytes` map to `String`/`char`/`Vec<u8>` — all `Eq` — so a runtime `=` over them
        // (and over a compound CONTAINING them) emits a native `==`. A `Vec<u8>`/`String` `==` compares
        // CONTENT, so a rope and its flat twin (same bytes) compare EQUAL — matching the canonical-byte-form
        // value equality the wasm heap walk gives. (`String` eq already worked via a different path; this
        // adds `Bytes` + `Char` + the compound-containing-them cases — see v-core-opt's Bytes-eq note.)
        Ty::String | Ty::Char | Ty::Bytes => true,
        // A `Symbol` maps to Rust's `String` (a canonical text leaf, identity = content), which is `Eq` —
        // so a runtime `=` over a Symbol (and over a compound CONTAINING one) emits a native `==` that
        // compares by content, matching the wasm byte-leaf compare (`Symbol.of "x" == #"x"`).
        Ty::Symbol => true,
        // `BigInt`/`Rational` map to `cdz_num::Big`/`cdz_num::Rational`, both of which `#[derive(PartialEq,
        // Eq)]` — so a runtime `=` over them (and over a compound CONTAINING them: a `(Tuple Rational Int64)`,
        // a Rational in a SUM payload) emits a native `==`. CRUCIALLY this is the CANONICAL-FORM equality the
        // value semantics require: `Big` is stored sign-magnitude with no leading-zero limbs, and `Rational`
        // is stored NORMALIZED (lowest terms, sign on the numerator, positive denominator) — so a derived
        // field-wise `==` compares by value (`1/2 == 2/4`, `0/2 == 0/3`), matching the wasm heap walk's
        // canonical-byte comparison. (Bare BigInt/Rational eq already worked via the `BigIntCmp`/`RationalCmp`
        // op path; this adds the COMPOUND-containing-them cases, which route through `Core::ValueEq`.)
        Ty::BigInt | Ty::Rational => true,
        // A `List`/`Set` of an `Eq` element → `Vec<T>`/`BTreeSet<T>` is `Eq` (elementwise); a `Map` is `Eq`
        // when both key and value are. Recurse into the element/key/value type (a `List Float64` is NOT Eq,
        // caught by the recursion). `BTreeMap`/`BTreeSet` `==` compares by content, matching value equality.
        // A FREE-VAR element is a special case: an EMPTY collection (`(Set.of (list))`) never constrains its
        // element type, so it stays `Set(Var _)` — but it emits with a concrete default rep (`BTreeSet<i64>`)
        // on BOTH sides of a `=`, which unifies and is `Eq`, so treat a free-var leaf as Eq (the drained-set
        // value-equality case). `ty_leaf_eq_or_free` recurses the same way but admits a bare free var.
        // A `List` element / `Map` VALUE is a RAW `Vec<T>` slot (a float there is `Vec<f64>`, NOT `Eq`); a
        // `Set` ELEMENT / `Map` KEY is stored in the `Eq`-deriving `__CdzF{N}` ord-wrapper, so a float there
        // makes the `BTreeSet`/`BTreeMap` key `Eq` — use `ty_ord_key_eq_or_free` for those. (See its docs.)
        Ty::List(e) => ty_leaf_eq_or_free(db, e, visited),
        Ty::Set(e) => ty_ord_key_eq_or_free(db, e, visited),
        Ty::Map(k, v) => {
            ty_ord_key_eq_or_free(db, k, visited) && ty_leaf_eq_or_free(db, v, visited)
        }
        // A `Qty` erases to its inner magnitude in `lower` (the unit is compile-time), so a runtime `=` over
        // a quantity IS the `=` over its inner numeric type — Eq-derivable iff the inner is. A `(Qty BigInt …)`
        // / `(Qty Rational …)` / `(Qty Int64 …)` leaf in a compound thus compares by its erased magnitude.
        Ty::Qty { inner, .. } => ty_derives_eq(db, inner, visited),
        // A function/closure, a free `Ty::Var`/`Any` — not `Eq`-derivable here (no native rep or not `Eq`).
        // Conservative: an unknown type declines the derive.
        _ => false,
    }
}

/// A collection ELEMENT/key/value type for the `Eq` check, admitting an unconstrained free var. Like
/// `ty_derives_eq` but a bare free `Ty::Var`/`Any` returns `true`: reaching a free-var leaf means the
/// enclosing collection was never constrained, i.e. it is EMPTY (a non-empty one would have solved the
/// element from its inserted values). An empty collection emits a concrete default rep (`BTreeSet<i64>`)
/// identically on both sides of a runtime `=`, so the native `==` type-checks and compares equal — which
/// is exactly what the value semantics want for a drained/empty collection. Nested collections recurse
/// (an empty `Set (List ?e)` is still fine). Any OTHER type defers to `ty_derives_eq` (a `List Float64`
/// still declines — its element is a solved non-Eq float, not a free var).
fn ty_leaf_eq_or_free(
    db: &mut Db,
    ty: &crate::ty::Ty,
    visited: &mut std::collections::HashSet<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Var(n) if *n < PARAM_SENTINEL_BASE => true,
        Ty::Any => true,
        // A `List` element / `Map` VALUE is a RAW `Vec<T>` slot (a float there is `Vec<f64>`, NOT `Eq`); a
        // `Set` ELEMENT / `Map` KEY is stored in the `Eq`-deriving `__CdzF{N}` ord-wrapper, so a float there
        // makes the `BTreeSet`/`BTreeMap` key `Eq` — use `ty_ord_key_eq_or_free` for those. (See its docs.)
        Ty::List(e) => ty_leaf_eq_or_free(db, e, visited),
        Ty::Set(e) => ty_ord_key_eq_or_free(db, e, visited),
        Ty::Map(k, v) => {
            ty_ord_key_eq_or_free(db, k, visited) && ty_leaf_eq_or_free(db, v, visited)
        }
        other => ty_derives_eq(db, other, visited),
    }
}

/// Whether a Set-ELEMENT / Map-KEY type maps to an `Eq`-deriving Rust key — like [`ty_leaf_eq_or_free`] but a
/// FLOAT leaf counts as `Eq`. A `BTreeSet`/`BTreeMap` key that is (or contains) a float is stored in the
/// `__CdzF{64,32}` total-order wrapper (`impl Eq`, NaN-canonicalized — see `CDZ_F64_DECL`), NOT a raw `f64`,
/// so the key type DOES derive `Eq` and a native `==` is the correct canonical set/map equality (the SAME
/// canonical form used for construction/lookup, matching the wasm heap walk). Mirrors `wrap_ord_key` /
/// `key_ty_has_wrappable_float`'s wrapping descent (a bare float, and a float REBUILT inside a Tuple/Record
/// key) so this agrees with what actually gets wrapped. A float nested in a `Sum`/`List` key is NOT wrapped
/// (it declines upstream as a non-ord-key via `ty_is_ord_key`), so any other shape defers to `ty_derives_eq`
/// (correctly: `Int`/`String`/`Symbol`/`Bool`/a float-free sum → `Eq`; a float-carrying sum → not). A bare
/// free var is an EMPTY collection (concrete default rep on both sides) → `Eq`, as in `ty_leaf_eq_or_free`.
fn ty_ord_key_eq_or_free(
    db: &mut Db,
    ty: &crate::ty::Ty,
    visited: &mut std::collections::HashSet<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty.strip_nominal_and_qty() {
        // A float KEY/element is lifted to the `Eq`-deriving `__CdzF{N}` wrapper.
        Ty::Float(_) => true,
        // A Tuple/Record key rebuilds wrapping each float element by position (`ord_key_type`) → `Eq` iff
        // every element is (a nested float is wrapped; a non-float element must itself be `Eq`).
        Ty::Tuple(elems) => elems.iter().all(|e| ty_ord_key_eq_or_free(db, e, visited)),
        Ty::Record(fields) => fields
            .values()
            .all(|t| ty_ord_key_eq_or_free(db, t, visited)),
        // An empty collection's element stays a free var (default rep on both sides) → `Eq`.
        Ty::Var(n) if *n < PARAM_SENTINEL_BASE => true,
        Ty::Any => true,
        // Any other key shape is not float-wrapped — the ordinary derive decides.
        other => ty_derives_eq(db, other, visited),
    }
}

/// Whether the type `ty` is REPRESENTABLE by this backend — every sum it mentions has an emittable Rust
/// enum (or maps to a built-in). A recursive sum (`IntList`, `Env`) has NO emitted enum (it needs `Box`),
/// so a function that takes/returns it must DECLINE too — else it would reference a type that was never
/// declared (`cannot find type IntList`). This is that guard: it re-checks each sum's declaration emits.
/// Recurses through compound structure (a tuple/record of a recursive sum is also unrepresentable).
pub(super) fn sum_representable(db: &mut Db, ty: &crate::ty::Ty) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Sum { decl, args, .. } => {
            // A built-in Option/Result is always representable (maps to std). Otherwise the declaration
            // must emit an enum — re-check via `emit_one_enum` (which declines a recursive/unrepresentable
            // sum). Then every type argument must be representable too.
            let decl_ok = match db.type_decl_by_occ(*decl) {
                Some(d) => {
                    let d = d.clone();
                    is_builtin_std_sum(db, &d) || decl_emits(db, &d)
                }
                None => false,
            };
            decl_ok && args.iter().all(|a| sum_representable(db, a))
        }
        Ty::Tuple(elems) => elems.iter().all(|e| sum_representable(db, e)),
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().all(|t| sum_representable(db, t))
        }
        // A NOMINAL newtype ERASES to its underlying type on the rust backend (`rust_type`'s `Ty::Nominal`
        // arm renders `inner`, no Rust definition emitted). Erasure works only when the unfold TERMINATES —
        // a RECURSIVE newtype `(type Lst (Mk (Option (Tuple Int64 Lst))))` unfolds forever: its `inner`
        // mentions its OWN decl (the μ back-edge), which `rust_type` would render as the bare name `Lst`
        // that is never defined → `cannot find type Lst` (an uncompilable crate). Rust needs a `Box`-
        // indirected NOMINAL emission for that (not yet done, like a recursive sum), so a recursive newtype
        // is NOT representable — decline, exactly as a recursive sum declines. A non-recursive newtype stays
        // representable iff its (finitely-unfolding) `inner` and any type args are.
        Ty::Nominal {
            decl, inner, args, ..
        } => {
            !mentions_decl(inner, *decl)
                && sum_representable(db, inner)
                && args.iter().all(|a| sum_representable(db, a))
        }
        // A LIST element must be representable; a `Vec<T>` needs no `Ord`, so no Ord check.
        Ty::List(e) => sum_representable(db, e),
        // A SET element / MAP key maps to `BTreeSet<T>`/`BTreeMap<K,_>`, which need `T`/`K: Ord`. `rust_type`
        // is pure and can't check a SUM key's Ord-derivability, so it maps the shape unconditionally — this
        // Db-aware gate is where a non-Ord key/element in a TYPE POSITION (a param/result `Set`/`Map` type,
        // with no construction op to catch it) DECLINES. Without it, a `(Set W)` param where `W` is a
        // float-carrying sum emits `BTreeSet<W>` though `enum W` derives no `Ord` → uncompilable (E0277,
        // Copilot PR#455 — the float-carrying-sum twin of the bare-float-key decline). `types::ty_is_ord` is
        // the Ord predicate (floats + float-carrying compounds/sums excluded; a sum orderable iff its enum
        // derives `Ord`; BigInt/Rational ARE Ord). The element/value must ALSO be representable — but a
        // non-Ord VALUE is fine (only the KEY needs `Ord`).
        // The KEY/element gate is `ty_is_ord_key` (not `ty_is_ord`): a BARE `Float` key/element is now
        // representable via the `CdzF64` total-order wrapper, while a float-CARRYING compound/sum key still
        // declines (the wrapper is not threaded through a compound). The element/value must also be
        // representable — but a non-Ord VALUE is fine (only the KEY needs `Ord`).
        Ty::Set(e) => sum_representable(db, e) && crate::backend::rust::types::ty_is_ord_key(db, e),
        Ty::Map(k, v) => {
            sum_representable(db, k)
                && sum_representable(db, v)
                && crate::backend::rust::types::ty_is_ord_key(db, k)
        }
        _ => true,
    }
}

/// A PRECISE phrase for WHY `ty` is not [`sum_representable`], for the decline diagnostic. The bare
/// "sum with no emitted Rust enum" is wrong for a recursive NEWTYPE (`(type Lst (Mk (Option (Tuple Int64
/// Lst))))`) — a newtype is not a sum, and it fails for a DIFFERENT reason (its ERASURE unfolds forever
/// through the μ back-edge, so it would need a `Box`-indirected NOMINAL emission, not an enum). Naming it
/// accurately points whoever picks up the gap (or a future me) at the right fix, instead of hunting for a
/// missing enum. Returns the first offending kind found in a left-to-right structural walk; falls back to
/// the sum phrasing when the offender is a genuine recursive/unrepresentable SUM. Call ONLY when
/// `sum_representable(db, ty)` is already known false (else it returns the sum fallback harmlessly).
pub(super) fn unrepresentable_reason(db: &mut Db, ty: &crate::ty::Ty) -> &'static str {
    use crate::ty::Ty;
    match ty {
        // A recursive newtype is the case whose erasure can't terminate — name it as such.
        Ty::Nominal { decl, inner, .. } if mentions_decl(inner, *decl) => {
            "a recursive newtype with no Box-indirected Rust representation"
        }
        // A non-recursive newtype erases to its inner; the offender is inside the inner.
        Ty::Nominal { inner, .. } => unrepresentable_reason(db, inner),
        Ty::Tuple(elems) => elems
            .iter()
            .find(|e| !sum_representable(db, e))
            .map(|e| unrepresentable_reason(db, &e.clone()))
            .unwrap_or("a sum with no emitted Rust enum (recursive/unrepresentable)"),
        Ty::List(e) | Ty::Set(e) => unrepresentable_reason(db, &e.clone()),
        Ty::Map(k, v) => {
            if !sum_representable(db, k) {
                unrepresentable_reason(db, &k.clone())
            } else {
                unrepresentable_reason(db, &v.clone())
            }
        }
        _ => "a sum with no emitted Rust enum (recursive/unrepresentable)",
    }
}

/// Whether declaration `decl` emits a Rust enum (its variant payloads all resolve to native types and it
/// is not recursive) — the representability of a NON-built-in sum. Mirrors `emit_one_enum`'s success.
fn decl_emits(db: &mut Db, decl: &crate::db::TypeDecl) -> bool {
    decl.variants.iter().all(|variant| {
        variant
            .payloads
            .iter()
            // Mode-INVARIANT: a closure payload is representable in both modes (only the SPELLING differs);
            // this is a yes/no representability check, not an emit, so `Sync` decides it.
            .all(|&pty| payload_rust_type(db, pty, decl, super::Mode::Sync).is_ok())
    })
}

/// Whether any of `variant`'s payload type-expressions is part of a recursive CYCLE back to the sum
/// declaration `decl_occ` — a RECURSIVE variant, whose payload field the enum boxes (`Box<…>`) to stay
/// finite-sized. A payload counts as recursive when its type can transitively REACH BACK to `decl_occ`
/// through the sum-reference graph (`reaches_decl`), which catches BOTH a direct self-reference (`(Cons
/// Int64 L)` — `L` reaches `L`) AND a MUTUAL cycle (`(type A (AN B))`+`(type B (BN A))` — A's payload `B`
/// reaches A through B's payload). A payload that reaches OTHER sums but never returns to `decl` (a plain
/// `(Outer Inner)` where `Inner` is acyclic) is NOT boxed. Used by `emit_one_enum` to box a variant's
/// field, and by the construct/match sites (via `variant_is_recursive`) to agree on the box/deref.
fn variant_payloads_mention(
    db: &mut Db,
    variant: &crate::db::Variant,
    decl: &crate::db::TypeDecl,
) -> bool {
    let occs = variant.payloads.clone();
    occs.iter().any(|&pty| {
        // Resolve the payload type PARAM-TOLERANTLY, the same way `payload_rust_type` does: a GENERIC sum's
        // payload mentions the decl's type PARAMETERS, so a plain `typeval_of` returns `None` (the params
        // are unbound) and the recursion check would MISS it — a generic recursive sum `(type Tree (Leaf a)
        // (Node (Tuple (Tree a) (Tree a))))` then emitted `Node((Tree<T0>, Tree<T0>))` UNBOXED → rustc E0072
        // "recursive type has infinite size". Resolve at the SENTINEL instantiation (each param → a sentinel
        // `Var`) so the self-reference `(Tree a)` appears as `Ty::Sum{Tree, [Var]}` and `reaches_decl` finds
        // it. A MONOMORPHIC sum has no params, so `typeval_of` resolves directly (no sentinel needed).
        let ty = if decl.params.is_empty() {
            crate::eval::typeval_of(db, pty)
        } else {
            sentinel_payload_ty(db, decl, pty)
        };
        ty.is_some_and(|ty| reaches_decl(db, &ty, decl.occ))
    })
}

/// Whether the type `ty` can transitively reach the sum declaration `decl_occ` through the sum-reference
/// graph — a CYCLE detector for boxing a recursive (self- OR mutually-recursive) variant field. A direct
/// mention of `decl_occ` is a hit; otherwise the transitive sum-graph reach from each sum `ty` mentions is
/// consulted through the MEMOIZED `sum_reachable_from` (an O(1) set membership, not a fresh DFS per query
/// — the O(N³)→O(N²) fix). A `List<Rose>` / `Vec`-like element does NOT continue the walk: those built-in
/// containers provide their own heap indirection (a Rust `Vec<Rose>` is finite-sized regardless), so a
/// self-reference UNDER a `List`/`Map`/`Set` needs no `Box` at the variant — only a DIRECT sum/tuple/record
/// payload position does. No `visited` set is needed: the sum expansion is cycle-guarded inside the memo,
/// and the tuple/record/args recursion here is over finite type TREES that cannot cycle.
fn reaches_decl(db: &mut Db, ty: &crate::ty::Ty, decl_occ: crate::ast::StructId) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Sum { decl: d, args, .. } | Ty::Nominal { decl: d, args, .. } => {
            if *d == decl_occ {
                return true;
            }
            // A reference to another sum: does IT (through its own payloads) reach back to `decl_occ`?
            // Also follow any type args (a generic instantiation) structurally.
            if args.iter().any(|a| reaches_decl(db, a, decl_occ)) {
                return true;
            }
            // The transitive sum-graph reach from `d` is a TARGET-INDEPENDENT set, memoized per decl — an
            // O(1) membership test rather than a fresh DFS per query.
            sum_reachable_from(db, *d).contains(&decl_occ)
        }
        // A DIRECT tuple/record payload position continues the walk — a `(Tuple Int64 L)` / `(Record (l
        // L))` reaches `decl` if any element/field does. (A `List`/`Map`/`Set` element does NOT — the
        // container's heap indirection already makes it finite; only a by-value position needs the Box.)
        Ty::Tuple(elems) => elems.iter().any(|e| reaches_decl(db, e, decl_occ)),
        Ty::Record(fields) => fields.values().any(|t| reaches_decl(db, t, decl_occ)),
        _ => false,
    }
}

/// The set of sum declarations TRANSITIVELY reachable from `start`'s variant payloads through the
/// sum-reference graph (the by-value positions a `Box` decision follows: a sum/nominal payload directly,
/// or one nested in a tuple/record payload — NOT under a `List`/`Map`/`Set`, whose heap indirection breaks
/// the size cycle). Target-INDEPENDENT and memoized per `start` in `db.sum_reachable`, so the recursive-
/// variant check is an O(1) membership test rather than a fresh DFS per variant (the O(N³)→O(N²) fix).
///
/// Computed by an iterative worklist (no recursion, so a cycle terminates by the `seen` guard) — and NOT
/// cached mid-walk: the WHOLE set for `start` is materialized, then inserted, so a partial (cycle-clipped)
/// result is never observed. `start` itself is NOT included unless the graph genuinely cycles back to it
/// (which is exactly the self-reference a recursive variant needs to detect).
fn sum_reachable_from(
    db: &mut Db,
    start: crate::ast::StructId,
) -> std::rc::Rc<crate::fxhash::FxHashSet<crate::ast::StructId>> {
    if let Some(hit) = db.sum_reachable.get(&start) {
        return hit.clone();
    }
    // Reachability closure over the CACHED per-decl out-edges (each computed once via `typeval_of`), so a
    // cycle of N sums costs O(N) edge-builds total, not O(N) per start (which was O(N²) `typeval_of`).
    let mut reached: crate::fxhash::FxHashSet<crate::ast::StructId> =
        crate::fxhash::FxHashSet::default();
    let mut expanded: std::collections::HashSet<crate::ast::StructId> =
        std::collections::HashSet::new();
    let mut work = vec![start];
    while let Some(d) = work.pop() {
        if !expanded.insert(d) {
            continue; // already expanded this decl's out-edges — cycle guard
        }
        for &e in sum_out_edges_of(db, d).iter() {
            if reached.insert(e) {
                work.push(e);
            }
        }
    }
    let rc = std::rc::Rc::new(reached);
    db.sum_reachable.insert(start, rc.clone());
    rc
}

/// The sum decls `d`'s variant payloads DIRECTLY mention (the sum-graph out-edges of `d`) — cached per
/// decl in `db.sum_out_edges` so the `typeval_of`-per-payload work is done ONCE per decl, not once per
/// (decl, reachability-query-start). Reading `d`'s payload type-expressions, evaluating each, and
/// collecting the sum/nominal decls each mentions by value (through tuple/record nesting + type args —
/// NOT under a `List`/`Map`/`Set`).
fn sum_out_edges_of(
    db: &mut Db,
    d: crate::ast::StructId,
) -> std::rc::Rc<Vec<crate::ast::StructId>> {
    if let Some(hit) = db.sum_out_edges.get(&d) {
        return hit.clone();
    }
    let payload_occs: Vec<crate::ast::StructId> = match db.type_decl_by_occ(d) {
        Some(td) => td
            .variants
            .iter()
            .flat_map(|v| v.payloads.clone())
            .collect(),
        None => Vec::new(),
    };
    let mut edges: crate::fxhash::FxHashSet<crate::ast::StructId> =
        crate::fxhash::FxHashSet::default();
    let mut sink: Vec<crate::ast::StructId> = Vec::new();
    for pty in payload_occs {
        if let Some(pt) = crate::eval::typeval_of(db, pty) {
            collect_sum_mentions(&pt, &mut edges, &mut sink);
        }
    }
    let rc = std::rc::Rc::new(edges.into_iter().collect::<Vec<_>>());
    db.sum_out_edges.insert(d, rc.clone());
    rc
}

/// Record every sum/nominal decl `ty` mentions in a by-value position (directly, or nested in a tuple/
/// record — the same positions `reaches_decl` follows), adding each to `reached` and queuing it in `work`
/// for expansion. A `List`/`Map`/`Set` element is NOT followed (heap indirection). Type ARGS are followed
/// (a generic instantiation's argument is a by-value payload). Pure structural walk over one type tree.
fn collect_sum_mentions(
    ty: &crate::ty::Ty,
    reached: &mut crate::fxhash::FxHashSet<crate::ast::StructId>,
    work: &mut Vec<crate::ast::StructId>,
) {
    use crate::ty::Ty;
    match ty {
        Ty::Sum { decl: d, args, .. } | Ty::Nominal { decl: d, args, .. } => {
            if reached.insert(*d) {
                work.push(*d);
            }
            for a in args.iter() {
                collect_sum_mentions(a, reached, work);
            }
        }
        Ty::Tuple(elems) => {
            for e in elems.iter() {
                collect_sum_mentions(e, reached, work);
            }
        }
        Ty::Record(fields) => {
            for t in fields.values() {
                collect_sum_mentions(t, reached, work);
            }
        }
        _ => {}
    }
}

/// Whether the `disc`-th variant of the sum TYPE `ty` is RECURSIVE (its payload mentions the sum's own
/// declaration) — the construct/match sites' shared predicate for whether to `Box::new`/deref the
/// payload, mirroring `emit_one_enum`'s per-variant box decision. `false` for a non-sum / out-of-range
/// disc / unresolvable declaration (no boxing).
pub(super) fn variant_is_recursive(db: &mut Db, ty: &crate::ty::Ty, disc: u32) -> bool {
    let decl_occ = match ty.strip_nominal() {
        crate::ty::Ty::Sum { decl, .. } => *decl,
        _ => return false,
    };
    let decl = match db.type_decl_by_occ(decl_occ) {
        Some(d) => d.clone(),
        None => return false,
    };
    let variant = match decl.variants.get(disc as usize) {
        Some(v) => v.clone(),
        None => return false,
    };
    variant_payloads_mention(db, &variant, &decl)
}

/// Whether the solved type `ty` mentions the sum declaration `decl` anywhere (directly or nested) — a
/// self-referential payload, which a Rust enum can't hold by value. Walks the compound structure.
pub(super) fn mentions_decl(ty: &crate::ty::Ty, decl: crate::ast::StructId) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Sum { decl: d, args, .. } => *d == decl || args.iter().any(|a| mentions_decl(a, decl)),
        // A NOMINAL newtype's back-edge is a `Ty::Nominal` carrying the SAME decl (the recursion point of
        // `(type Lst (Mk (Option (Tuple Int64 Lst))))` — the inner `Lst` is `Ty::Nominal{decl=Lst}`). Match
        // it directly, and descend into `inner`/`args` so a mention nested under a non-recursive wrapper is
        // still found. Without this arm a nominal self-reference slipped through (only `Ty::Sum` was
        // checked), leaving a dangling type name.
        Ty::Nominal {
            decl: d,
            inner,
            args,
            ..
        } => {
            *d == decl || mentions_decl(inner, decl) || args.iter().any(|a| mentions_decl(a, decl))
        }
        Ty::Tuple(elems) => elems.iter().any(|e| mentions_decl(e, decl)),
        Ty::Record(fields) => fields.values().any(|t| mentions_decl(t, decl)),
        // A `List`/`Set`/`Map`/`Qty` element/key/value position ALSO carries the μ back-edge: unlike the
        // by-value Box walk (`reaches_decl`, which SKIPS these because a `Vec`/`BTreeSet`/`BTreeMap` heap
        // pointer breaks the SIZE cycle), `types::rust_type` still ERASES a newtype THROUGH the container
        // element and emits the bare undeclared decl name at the self-reference — e.g. `(type Rose (Node
        // Int64 (List Rose)))` erases to `(i64, Vec<Rose>)` with a dangling `Rose`. So the newtype-erasure
        // termination check must follow these positions too, or a List/Map/Set-recursive newtype slips
        // through `sum_representable` and emits an uncompilable `cannot find type Rose` (breaker rose-tree
        // rt9). (This function feeds ONLY the newtype-representability decline + its diagnostic — NOT the
        // Box decision — so following containers here does not over-box a finite recursive sum field.)
        Ty::List(e) | Ty::Set(e) => mentions_decl(e, decl),
        Ty::Map(k, v) => mentions_decl(k, decl) || mentions_decl(v, decl),
        Ty::Qty { inner, .. } => mentions_decl(inner, decl),
        _ => false,
    }
}
