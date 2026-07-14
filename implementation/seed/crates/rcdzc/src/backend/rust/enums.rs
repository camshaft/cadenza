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
pub fn emit_enum_decls(db: &mut Db) -> String {
    let mut out = String::new();
    let n = db.type_decls.len();
    for i in 0..n {
        if let Ok(decl_src) = emit_one_enum(db, i) {
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
/// Only MONOMORPHIC user sums get a descriptor: a built-in `Option`/`Result` renders via the driver's
/// head-type path (it maps to std's, not an emitted enum), and a GENERIC user sum's payload is a type
/// PARAMETER (`T0`) whose concrete type is not known per-declaration — an escape of one is a gap the
/// driver declines (no corpus case escapes a generic user sum yet), not a wrong render.
pub fn emit_sum_descriptors(db: &mut Db) -> String {
    let mut out = String::new();
    let n = db.type_decls.len();
    for i in 0..n {
        let decl = db.type_decls[i].clone();
        // Only a sum whose enum actually emits (non-built-in, non-recursive, native payloads) and that is
        // MONOMORPHIC (no type params — a param payload has no concrete render form here).
        if !decl.params.is_empty() || emit_one_enum(db, i).is_err() {
            continue;
        }
        let ident = types::sum_ident(&decl.name);
        let mut groups = Vec::with_capacity(decl.variants.len());
        for variant in &decl.variants {
            match variant_payload_render(db, variant) {
                // A payload variant: `(Name <payload-render>)`.
                Some(payload) => groups.push(format!("({} {})", variant.name, payload)),
                // A nullary variant: `(Name)`.
                None => groups.push(format!("({})", variant.name)),
            }
        }
        out.push_str(&format!("// cdz-sum[{ident}]: {}\n", groups.join(" ")));
    }
    out
}

/// The `render_name` of a variant's payload TYPE, or `None` for a nullary variant. A one-payload variant
/// is its payload type's render; a MULTI-payload variant's payload is one tuple, so it is the `(Tuple …)`
/// render of the payload types (matching the single-`Ty::Tuple` payload the core models and the enum
/// field `V((T0, T1))`). Used only for monomorphic sums, so `typeval_of` yields a concrete type.
fn variant_payload_render(db: &mut Db, variant: &crate::db::Variant) -> Option<String> {
    let tys: Vec<crate::ty::Ty> = variant
        .payloads
        .iter()
        .filter_map(|&occ| crate::eval::typeval_of(db, occ))
        .collect();
    match tys.len() {
        0 => None,
        1 => Some(tys[0].render_name()),
        _ => Some(crate::ty::Ty::Tuple(tys.into()).render_name()),
    }
}

/// Emit one sum declaration `db.type_decls[i]` as a Rust `enum`, or decline (a variant payload with no
/// native mapping, or a recursive sum). `Err` means "skip this declaration" — the caller drops it.
fn emit_one_enum(db: &mut Db, i: usize) -> Result<String, Reject> {
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
            payloads.push(payload_rust_type(db, pty_occ, &decl)?);
        }
        // A RECURSIVE variant — one whose payload mentions THIS sum (`(Cons Int64 L)`, `(Node L L)`) —
        // BOXES its whole payload field: a Rust enum containing itself by value is infinitely sized, so
        // the recursion goes behind one `Box<…>` indirection at the variant field. The box wraps the WHOLE
        // payload (`Cons(Box<(i64, L)>)`, `Node(Box<(L, L)>)`), a uniform ONE-box-per-recursive-variant
        // scheme the construct (`Box::new(payload)`) and match (`*__pay`) sites agree on — simpler than
        // boxing each recursive sub-position (which would need the box/deref threaded through every tuple
        // element). A non-recursive variant is unboxed as before.
        let recursive = variant_payloads_mention(db, variant, decl.occ);
        let field = |ty: String| {
            if recursive { format!("Box<{ty}>") } else { ty }
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

    // An ALL-NULLARY enum (every variant fieldless) is a bare discriminant — exactly the sums the seed
    // permits `=` on (a payload-carrying sum has no structural `=` in the seed). The `=` intrinsic lowers
    // (rust backend) to a native `x == y`, which needs `PartialEq`; the wasm backend compares the i32
    // discriminant directly, so the derive is what makes the two backends AGREE on realizability. Gate the
    // `PartialEq, Eq` derive on all-nullary: a payload-carrying variant would need its fields to be
    // `PartialEq` too (not guaranteed), so deriving unconditionally could fail to compile — all-nullary is
    // the safe, sufficient condition (mirrors the wasm backend's always-available discriminant compare).
    let all_nullary = decl.variants.iter().all(|v| v.payloads.is_empty());
    let derives = if all_nullary {
        "Clone, PartialEq, Eq"
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
    Ok(format!(
        "#[derive({derives})]\n#[allow(dead_code)]\npub enum {name}{generics} {{ {} }}\n",
        variants.join(", ")
    ))
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
    let ty = crate::eval::typeval_of(db, pty_occ)
        .ok_or_else(|| Reject::decline("a variant payload type does not resolve"))?;
    render_payload_ty(&ty, decl).ok_or_else(|| {
        Reject::decline(format!(
            "a variant payload type {} has no native Rust representation",
            ty.render_name()
        ))
    })
}

/// Render a payload type to its Rust form WITHIN declaration `decl` — like [`types::rust_type`], but a
/// SELF-REFERENCE to `decl` (a `Ty::Sum`/`Ty::Nominal` of the same `occ`) is rendered with the decl's OWN
/// type parameters (`Tree<T0, T1>` for a generic decl, bare `Tree` for a monomorphic one) rather than
/// whatever args the bare self-mention carries (none). This is what makes a generic recursive sum's
/// self-referential payload name the enum correctly (E0107 otherwise). Non-self sums/compounds delegate to
/// `types::rust_type` (their args are concrete). `None` for a type with no native Rust form.
fn render_payload_ty(ty: &crate::ty::Ty, decl: &crate::db::TypeDecl) -> Option<String> {
    use crate::ty::Ty;
    // A self-reference — the recursive mention of THIS declaration. Render the enum name + the decl's own
    // params (the recursion is closed by the enclosing `Box`).
    let is_self = match ty {
        Ty::Sum { decl: d, .. } | Ty::Nominal { decl: d, .. } => *d == decl.occ,
        _ => false,
    };
    if is_self {
        let name = types::sum_ident(&decl.name);
        if decl.params.is_empty() {
            return Some(name);
        }
        let ps: Vec<String> = (0..decl.params.len()).map(|k| format!("T{k}")).collect();
        return Some(format!("{name}<{}>", ps.join(", ")));
    }
    // A compound that may CONTAIN a self-reference — recurse so a nested `(Tuple Tree Tree)` /
    // `(List Tree)` renders the inner self-refs with the decl's params. Non-self leaves + concrete sums
    // delegate to `types::rust_type`.
    match ty {
        Ty::Tuple(elems) => {
            let parts: Option<Vec<String>> =
                elems.iter().map(|e| render_payload_ty(e, decl)).collect();
            let parts = parts?;
            match parts.len() {
                0 => Some("()".to_string()),
                1 => Some(format!("({},)", parts[0])),
                _ => Some(format!("({})", parts.join(", "))),
            }
        }
        _ => types::rust_type(ty),
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
        _ => true,
    }
}

/// Whether declaration `decl` emits a Rust enum (its variant payloads all resolve to native types and it
/// is not recursive) — the representability of a NON-built-in sum. Mirrors `emit_one_enum`'s success.
fn decl_emits(db: &mut Db, decl: &crate::db::TypeDecl) -> bool {
    decl.variants.iter().all(|variant| {
        variant
            .payloads
            .iter()
            .all(|&pty| payload_rust_type(db, pty, decl).is_ok())
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
    decl_occ: crate::ast::StructId,
) -> bool {
    let occs = variant.payloads.clone();
    occs.iter().any(|&pty| {
        crate::eval::typeval_of(db, pty).is_some_and(|ty| {
            reaches_decl(db, &ty, decl_occ, &mut std::collections::HashSet::new())
        })
    })
}

/// Whether the type `ty` can transitively reach the sum declaration `decl_occ` through the sum-reference
/// graph — a CYCLE detector for boxing a recursive (self- OR mutually-recursive) variant field. A direct
/// mention of `decl_occ` is a hit; otherwise, for each OTHER sum `ty` mentions, follow that sum's variant
/// payloads (its own reference edges) looking for a path back to `decl_occ`. `visited` (sum decl occs
/// already expanded) bounds the walk — a finite sum graph is fully explored once. A `List<Rose>` /
/// `Vec`-like element does NOT continue the walk here: those built-in containers provide their own heap
/// indirection (a Rust `Vec<Rose>` is finite-sized regardless), so a self-reference UNDER a `List`/`Map`/
/// `Set` needs no `Box` at the variant — only a DIRECT sum/tuple/record payload position does.
fn reaches_decl(
    db: &mut Db,
    ty: &crate::ty::Ty,
    decl_occ: crate::ast::StructId,
    visited: &mut std::collections::HashSet<crate::ast::StructId>,
) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Sum { decl: d, args, .. } | Ty::Nominal { decl: d, args, .. } => {
            if *d == decl_occ {
                return true;
            }
            // A reference to another sum: does IT (through its own payloads) reach back to `decl_occ`?
            // Expand it once (visited-guarded). Also follow any type args (a generic instantiation).
            if args.iter().any(|a| reaches_decl(db, a, decl_occ, visited)) {
                return true;
            }
            if visited.insert(*d)
                && let Some(td) = db.type_decl_by_occ(*d)
            {
                let payload_occs: Vec<crate::ast::StructId> = td
                    .variants
                    .iter()
                    .flat_map(|v| v.payloads.clone())
                    .collect();
                for pty in payload_occs {
                    if let Some(pt) = crate::eval::typeval_of(db, pty)
                        && reaches_decl(db, &pt, decl_occ, visited)
                    {
                        return true;
                    }
                }
            }
            false
        }
        // A DIRECT tuple/record payload position continues the walk — a `(Tuple Int64 L)` / `(Record (l
        // L))` reaches `decl` if any element/field does. (A `List`/`Map`/`Set` element does NOT — the
        // container's heap indirection already makes it finite; only a by-value position needs the Box.)
        Ty::Tuple(elems) => elems.iter().any(|e| reaches_decl(db, e, decl_occ, visited)),
        Ty::Record(fields) => fields
            .values()
            .any(|t| reaches_decl(db, t, decl_occ, visited)),
        _ => false,
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
    let variant = match db.type_decl_by_occ(decl_occ) {
        Some(d) => match d.variants.get(disc as usize) {
            Some(v) => v.clone(),
            None => return false,
        },
        None => return false,
    };
    variant_payloads_mention(db, &variant, decl_occ)
}

/// Whether the solved type `ty` mentions the sum declaration `decl` anywhere (directly or nested) — a
/// self-referential payload, which a Rust enum can't hold by value. Walks the compound structure.
fn mentions_decl(ty: &crate::ty::Ty, decl: crate::ast::StructId) -> bool {
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
        _ => false,
    }
}
