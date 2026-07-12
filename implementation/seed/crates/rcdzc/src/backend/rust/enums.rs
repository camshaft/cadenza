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
        // `T{k}`. A payload with no native mapping declines the whole enum. A recursive payload (one
        // mentioning THIS sum) also declines for now (needs `Box`).
        let mut payloads = Vec::with_capacity(variant.payloads.len());
        for &pty_occ in &variant.payloads {
            payloads.push(payload_rust_type(db, pty_occ, &decl)?);
        }
        match payloads.len() {
            // A nullary variant is a unit variant (`None`).
            0 => variants.push(vname),
            // A one-payload variant carries its payload directly (`Some(T)`).
            1 => variants.push(format!("{vname}({})", payloads[0])),
            // A MULTI-payload variant's payload is ONE TUPLE, not several positional fields — the core
            // models a multi-payload variant's payload as a single `Ty::Tuple` (`core.rs` SumNew: "a tuple
            // handle built from the payloads"; the front-end types it `Ty::Tuple`), and the match side reads
            // it as one bound value indexed `.0`/`.1`. So the enum field is that tuple: `Cons((T0, T1))` —
            // the SAME shape construction (`SumNew`) and matching (`SumPayload`) agree on. (A native
            // `Cons(T0, T1)` would disagree with the single-tuple payload the match binds → non-compiling.)
            _ => variants.push(format!("{vname}(({}))", payloads.join(", "))),
        }
    }

    // `#[derive(Clone)]` so a matched-and-rebuilt value (a payload read then re-wrapped) and a value used
    // more than once compose without move-out errors — the emitted code treats a sum value like any other
    // Cadenza value (freely copyable, pure). `#[allow(dead_code)]` because a declared-but-unused variant
    // (or the whole enum) is normal in generated code.
    // `pub` so the enum crosses the module boundary with the `pub fn`s that construct/consume it — a
    // consumer of the emitted `.rs` (or the gate's driver) can name the type and its variants. `Clone`
    // so a matched-and-rebuilt value composes; `#[allow(dead_code)]` since a declared-but-unused variant
    // is normal in generated code. Variants are `pub` implicitly (an enum's variants share its visibility).
    Ok(format!(
        "#[derive(Clone)]\n#[allow(dead_code)]\npub enum {name}{generics} {{ {} }}\n",
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
    // THIS sum (a recursive sum) declines — a Rust enum cannot contain itself by value.
    let ty = crate::eval::typeval_of(db, pty_occ)
        .ok_or_else(|| Reject::decline("a variant payload type does not resolve"))?;
    if mentions_decl(&ty, decl.occ) {
        return Err(Reject::decline(
            "a recursive sum needs Box indirection (not yet emitted)",
        ));
    }
    types::rust_type(&ty).ok_or_else(|| {
        Reject::decline(format!(
            "a variant payload type {} has no native Rust representation",
            ty.render_name()
        ))
    })
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

/// Whether the solved type `ty` mentions the sum declaration `decl` anywhere (directly or nested) — a
/// self-referential payload, which a Rust enum can't hold by value. Walks the compound structure.
fn mentions_decl(ty: &crate::ty::Ty, decl: crate::ast::StructId) -> bool {
    use crate::ty::Ty;
    match ty {
        Ty::Sum { decl: d, args, .. } => *d == decl || args.iter().any(|a| mentions_decl(a, decl)),
        Ty::Tuple(elems) => elems.iter().any(|e| mentions_decl(e, decl)),
        Ty::Record(fields) => fields.values().any(|t| mentions_decl(t, decl)),
        _ => false,
    }
}
