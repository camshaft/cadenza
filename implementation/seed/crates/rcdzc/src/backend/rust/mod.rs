//! The Rust-source backend — a STRUCTURED backend that emits ordinary Rust source.
//!
//! Where the wasm backend linearizes the core into a flat instruction stream, this backend consumes
//! the typed structured core DIRECTLY and prints it as Rust — the core's `if`/`match`/`let`/`call`
//! become Rust's own (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its Target Is
//! Linear: "a backend whose target has structured control flow consumes the typed structured core
//! directly … and never constructs the flat rung"). It reads the same columns every backend does
//! (`core_of`, `type_of`, the target-neutral [`Layout`]) — the concrete proof the pipeline above the
//! seam is target-neutral, not wasm-shaped.
//!
//! It emits a self-contained Rust module: one `pub fn` per export, named verbatim, with native scalar
//! parameter and result types. The point of the target is drop-in integration — a Cadenza-authored
//! module compiles to a `.rs` file that links into an existing Rust codebase as ordinary source, with
//! no component boundary, no runtime import, and no FFI.
//!
//! Value strategy (`backends-and-targets.md` §A Compound Value's Representation Is The Backend's
//! Choice): this backend uses the target's NATIVE aggregates rather than the shared value-heap runtime
//! (the "rust-ergonomic" strategy) — so a Cadenza integer is a Rust integer and no `cdz-runtime` is
//! linked. The scalar slice built here reaches only the scalar value language (integers, Bool, Unit);
//! a compound value or any construct the front already declined is DECLINED here too, attributed to
//! this target (`§A Backend Inherits The Front's Decline Boundaries`).
//!
//! CORRECTNESS: a Cadenza integer TRAPS on overflow, so a checked `+`/`-`/`*` emits `checked_*(…)`
//! with a trap on `None` — Rust's native `iN`/`uN` are exactly Cadenza's aliased widths with the same
//! wrapping-vs-checked distinction, so the numeric model maps across without a scratch-local guard
//! recipe (that recipe was a way to express checked arithmetic in the flat wasm rung; Rust expresses
//! it directly). The one executable semantics is the oracle either way (`§The meaning against which
//! every backend's output is judged MUST be the one executable semantics`).
//!
//! DEP-FREE, LIKE THE BYTE PATH: the Rust source is emitted as plain text (a `String`), exactly as the
//! wasm backend hand-emits bytes — no `syn`/`quote`/`prettyplease`. So this backend carries no new
//! dependency and ports to the Cadenza self-host on the same footing as the byte path (a source
//! string is as portable as a byte vector); `Target::Rust` is always available, not feature-gated.

mod enums;
mod expr;
mod types;

use crate::db::Db;
use crate::diag::Reject;
use crate::layout::Layout;

/// Which calling convention the Rust backend emits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Plain synchronous `fn`s — an ordinary Rust module, no runtime threading.
    Sync,
    /// ASYNC, GAS-METERED `async fn`s that thread a caller-supplied `env: &mut impl CdzEnv` and await
    /// `env.consume(1)` at each function entry, so the host meters fuel and can yield cooperatively.
    /// Every emitted call is `Box::pin(callee(env, …)).await` (the pin makes a recursive `async fn`
    /// well-sized — a recursive future is otherwise infinite; a non-recursive call inlines and never
    /// reaches here). The `CdzEnv` trait is emitted into the module preamble.
    Async,
}

impl Mode {
    fn is_async(self) -> bool {
        matches!(self, Mode::Async)
    }
}

/// The Rust type-PARAMETER name for the async gas/yield env (`async fn f<__CdzE: CdzEnv>(env: &mut
/// __CdzE, …)`). A `__`-prefixed reserved name — NOT a bare `E` — so it cannot collide with a user sum's
/// Rust type name (a `(type E …)` maps to `enum E`, which a bare `E` type param would shadow, breaking
/// `E::Variant`). Matches the `__pay`/`__p` reserved-local convention the expression emitter uses.
const ENV_TYPE_PARAM: &str = "__CdzE";

/// Emit a Rust-source artifact for the program in `db` under the boundary `layout`. Produces one
/// `pub fn` per export (verbatim name, native scalar signature), reading the shared columns on demand.
/// Declines — attributed to this target — for a construct the scalar slice does not yet render.
///
/// Emits EVERY reachable definition (`layout.order`), not just the exports: an export becomes a
/// `pub fn` (its verbatim name crosses the crate boundary), a reachable NON-export callee — a recursive
/// helper, a mutual-recursion partner — becomes a private `fn`. A `Core::Call` to such a callee then
/// renders as an ordinary Rust call of its emitted `fn`. Reachability is the SAME target-neutral set the
/// wasm backend emits (`layout::compute` closes it over `Core::Call` callees), so the two backends emit
/// the same functions; only the rendering differs. Recursion needs no special handling — a Rust `fn`
/// calls itself directly (native stack), so the wasm backend's tail-call-to-loop transform is simply
/// unnecessary here.
pub fn emit(db: &mut Db, layout: &Layout, mode: Mode) -> Result<Vec<u8>, Reject> {
    let mut out = String::new();
    out.push_str(PREAMBLE);
    // In async/gas mode the emitted functions thread the `CdzEnv` gas/yield capability. That trait lives
    // in the SHARED `cdz-rt` crate (not re-declared per module), so an application implements it ONCE and
    // every emitted module interoperates over the same type — bring it into scope with a `use`.
    if mode.is_async() {
        out.push_str(CDZ_RT_IMPORTS);
    }
    // Every sum type the program declares becomes a Rust `enum` (emitted before the functions that
    // construct/match/return it). A declaration with no native form (a recursive sum, an unrepresentable
    // payload) is skipped — a use of it declines at selection, so no orphan enum is emitted.
    out.push_str(&enums::emit_enum_decls(db));
    // A machine-readable descriptor per user sum (variant names + payload types in discriminant order) —
    // inert to rustc (`//` comments), read by the corpus gate to render a user-sum boundary value to its
    // canonical bare form. The enum decls above give rustc the types; these give the gate the structure.
    out.push_str(&enums::emit_sum_descriptors(db));
    // …and a descriptor per erased NEWTYPE (`// cdz-newtype[Pt]: <inner render_name>`), so the gate's value
    // renderer resolves a newtype-typed boundary value (`Pt`) to its erased inner type and renders it
    // structurally rather than `Display`-ing the erased Rust tuple. Inert to rustc (a `//` comment).
    out.push_str(&enums::emit_newtype_descriptors(db));
    for &def in &layout.order {
        let f = match layout.export_plan(def) {
            // An exported definition — a `pub fn` under its verbatim boundary name.
            Some(e) => emit_export(db, e, layout, mode)?,
            // A reachable non-export callee (reached via a runtime `Core::Call`) — a private `fn`.
            None => emit_fn(db, def, layout, mode)?,
        };
        out.push('\n');
        out.push_str(&f);
    }
    Ok(out.into_bytes())
}

/// The `use` an async-mode module emits to bring the shared runtime traits into scope. The `CdzEnv`
/// gas/yield capability now lives in the `cdz-rt` crate (a single shared definition), NOT re-declared in
/// each module — so an application implements it ONCE for `RcRuntime`/its own env type and every emitted
/// module uses that same trait (two modules interoperate). A downstream build depends on `cdz-rt`; the
/// corpus gate links it via `--extern cdz_rt=<rlib>`.
const CDZ_RT_IMPORTS: &str = "use cdz_rt::CdzEnv;\n";

/// The file preamble — a header comment marking the source as generated, and the lint allowances a
/// mechanically-emitted file needs (its names come verbatim from the source program, so they will not
/// follow Rust's `snake_case`/`UpperCamelCase` conventions; a nullary export takes no parameters and
/// may return a constant, which trips `clippy`'s "unused" and "trivial" lints — none of which is a
/// defect in generated code).
const PREAMBLE: &str = "\
// @generated by rcdzc (Cadenza → Rust backend). Do not edit by hand.
#![allow(non_snake_case, unused_parens, clippy::all)]
";

/// Emit one exported definition as a `pub fn` — its verbatim boundary name, solved parameter types,
/// and solved result type, from the target-neutral [`ExportPlan`] computed above the seam.
fn emit_export(
    db: &mut Db,
    e: &crate::layout::ExportPlan,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    emit_signature(
        db, &e.name, true, e.def, &e.params, &e.result, e.body, layout, mode,
    )
}

/// Emit a reachable NON-export definition as a private `fn` — a recursive helper or a mutual-recursion
/// partner a `Core::Call` names. Its name is the source name; its parameters come from
/// [`crate::layout::def_params`] (core types, no boundary-representability constraint — an internal
/// callee never crosses the crate edge); its result type is the body's solved type.
fn emit_fn(db: &mut Db, def: usize, layout: &Layout, mode: Mode) -> Result<String, Reject> {
    let name = db.defs[def].name.clone();
    let body = db.defs[def]
        .body
        .ok_or_else(|| Reject::decline(format!("definition `{name}` has no body")))?;
    let params = crate::layout::def_params(db, def);
    let result = crate::infer::type_of(db, body);
    emit_signature(db, &name, false, def, &params, &result, body, layout, mode)
}

/// Emit a function definition (shared by the export and non-export paths): `[pub] fn <name>(<params>)
/// -> <ret> { <body> }`. Each parameter renders as `<name>: <rust-type>`; a parameter type with no
/// native mapping (an unresolved/ambiguous or not-yet-supported type) declines. The result type maps
/// the same way (unit → `()`; a compound declines in the scalar slice). The body is the core of `body`
/// rendered as a Rust expression, with the parameters in scope by their emitted names.
#[allow(clippy::too_many_arguments)]
fn emit_signature(
    db: &mut Db,
    name: &str,
    public: bool,
    def: usize,
    params: &[(crate::ast::StructId, crate::ty::Ty)],
    result: &crate::ty::Ty,
    body: crate::ast::StructId,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    // Whether this function is compiled as a `loop` (it self-tail-calls). A looped function REASSIGNS
    // its parameter locals each iteration, so they are declared `mut`. Detected once here and again in
    // `emit_body` (both read the same predicate), so the signature's `mut` and the body's loop agree.
    let loops = !params.is_empty() && expr::body_loops(db, def);
    let mut param_src = String::new();
    // In async/gas mode, the FIRST parameter is the caller-supplied gas/yield env, threaded into every
    // call. It precedes the source parameters; the source params keep their positions after it.
    if mode.is_async() {
        param_src.push_str(&format!("env: &mut {ENV_TYPE_PARAM}"));
    }
    for (i, (binder, ty)) in params.iter().enumerate() {
        if i > 0 || mode.is_async() {
            param_src.push_str(", ");
        }
        let pname = param_name(db, *binder, i);
        // A sum type whose Rust `enum` did NOT emit (a recursive sum needs `Box`, deferred) has a name
        // but no declaration — a signature naming it would not compile (`cannot find type IntList`). So a
        // function that takes such a type declines HERE, consistently with the skipped enum decl.
        if !enums::sum_representable(db, ty) {
            return Err(Reject::decline(format!(
                "`{name}`: parameter type {} is a sum with no emitted Rust enum (recursive/unrepresentable)",
                ty.render_name()
            )));
        }
        let rty = types::rust_type(ty).ok_or_else(|| {
            Reject::decline(format!(
                "`{name}`: parameter type {} has no native Rust representation",
                ty.render_name()
            ))
        })?;
        // A looped function's params are reassigned per iteration → `mut`.
        let mut_kw = if loops { "mut " } else { "" };
        param_src.push_str(&format!("{mut_kw}{pname}: {rty}"));
    }
    // Same guard on the RESULT: a function returning a recursive sum (no emitted enum) declines.
    if !enums::sum_representable(db, result) {
        return Err(Reject::decline(format!(
            "`{name}`: result type {} is a sum with no emitted Rust enum (recursive/unrepresentable)",
            result.render_name()
        )));
    }
    // A DIVERGING body — its core is provably `Core::Trap` (a bare `(trap …)`, a zero-arm match on a
    // `Never` scrutinee, a call reduced to one) — has a `Never` result type (a fresh `Ty::Var`/`Any`) with
    // no native Rust rep, but NO value ever returns: the body `panic!`s. Emit Rust's NEVER type `!` as the
    // return type (`fn main() -> ! { panic!(…) }` is valid), mirroring the wasm backend which crosses such
    // an export as a no-result function (`Core::Trap` guard there too). Checked BEFORE the `rust_type`
    // decline so a diverging `Any`/`Var` result is not misdiagnosed as an unrepresentable type. Gated on
    // `Core::Trap` specifically — a genuinely-unconstrained (non-diverging) result var still declines, as it
    // has no defined value to return.
    let diverges = types::rust_type(result).is_none()
        && matches!(crate::lower::core_of(db, body), crate::core::Core::Trap);
    let ret = if diverges {
        "!".to_string()
    } else {
        types::rust_type(result).ok_or_else(|| {
            Reject::decline(format!(
                "`{name}`: result type {} has no native Rust representation",
                result.render_name()
            ))
        })?
    };
    // Render the body against the parameter environment. Selection reads the core + type columns on
    // demand, so a fault deep in the body surfaces here as a decline. In async mode the body's calls
    // become `Box::pin(callee(env, …)).await`; a self-tail-recursive body becomes a `loop` (so `def` is
    // passed to detect a self-call).
    let body_src = expr::emit_body(db, body, params, def, layout, mode)?;
    let vis = if public { "pub " } else { "" };
    // The function NAME via `fn_ident` — sanitized (`sum-to` → `sum_to`) and UNIQUED per definition when a
    // β-copied do-local worker would otherwise emit two `fn`s of the same name (E0428). The SAME mapping a
    // `Core::Call` uses at the call site (it also calls `fn_ident`), so the declaration and every call
    // agree — including a recursive self-call, which resolves to this def and so re-derives this ident.
    let ident = fn_ident(db, layout, def);
    // A machine-readable note of the fn's CADENZA result type — its `render_name` (e.g. `Int64`,
    // `(Tuple Int64 Bool)`, `(Record (a Int64) (b Int64))`). The Rust return type erases the structural
    // detail a boundary render needs (field NAMES, `Tuple`-vs-`Record` distinction), so a consumer that
    // must reproduce the value's canonical text form — the corpus gate — reads it from here. Inert to
    // rustc (a `//` comment); present on every emitted fn, keyed by ident so a caller finds the right one.
    // For a DIVERGING body the emitted return type is `!` (not a value type); note it as `!` so the gate
    // driver recognizes the export never returns and CALLS it without a `println!` (binding/printing a `!`
    // is an "unreachable statement" + `()`-not-`Display` build error). A `Never` result's `render_name` is
    // `_` — indistinguishable from other holes and NOT one of the driver's diverging markers — so keying the
    // note on the actual emitted `!` type is what makes the driver's divergence handling fire.
    let ret_note = if diverges {
        format!("// cdz-return[{ident}]: !\n")
    } else {
        format!("// cdz-return[{ident}]: {}\n", result.render_name())
    };
    if mode.is_async() {
        // `async fn <name><__CdzE: CdzEnv>(env: &mut __CdzE, …) -> <ret> { env.consume(1).await; <body> }`
        // — the per-call fuel charge + cooperative-yield point at entry. The env TYPE PARAMETER is named
        // `__CdzE` (not a bare `E`) so it can NEVER collide with a user sum's Rust type name (a sum named
        // `E` would otherwise shadow the type param, making `E::Variant` unresolvable) — the `__` prefix
        // marks it backend-reserved, matching the emitted `__pay`/`__p` locals.
        Ok(format!(
            "{ret_note}{vis}async fn {ident}<{ENV_TYPE_PARAM}: CdzEnv>({param_src}) -> {ret} {{\n    env.consume(1).await;\n{body_src}\n}}\n"
        ))
    } else {
        Ok(format!(
            "{ret_note}{vis}fn {ident}({param_src}) -> {ret} {{\n{body_src}\n}}\n"
        ))
    }
}

/// The Rust identifier for parameter `index`, from its source name occurrence. Falls back to a
/// positional `p{index}` when the occurrence carries no readable name (a defensive default — an
/// exported parameter always has a name in practice).
fn param_name(db: &Db, binder: crate::ast::StructId, index: usize) -> String {
    db.ast
        .as_name(binder)
        .map(sanitize_ident)
        .unwrap_or_else(|| format!("p{index}"))
}

/// Make a source name a valid, non-colliding Rust identifier. Cadenza names allow characters Rust
/// identifiers do not (notably `-`, the idiomatic word separator — `sum-to`), so each such character
/// becomes `_`; a name that would start with a digit is prefixed. The mapping is deterministic, so a
/// reference to the same name maps the same way everywhere it appears.
pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut s = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c == '_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit()) {
            s.push(c);
        } else if c.is_ascii_digit() {
            // A leading digit: prefix so the identifier is valid.
            s.push('_');
            s.push(c);
        } else {
            // Any other character (notably `-`) becomes an underscore.
            s.push('_');
        }
    }
    if s.is_empty() {
        s.push('_');
    }
    // A Cadenza identifier may be a RUST KEYWORD (`loop`, `type`, `while`, `for`, `mut`, `impl`, …) — a
    // valid Cadenza name but reserved in Rust, so emitting it verbatim as a `fn`/binder name is invalid Rust
    // (`fn loop(…)` → rustc "expected `{`, found `(`"). rustc round-trips a keyword-named symbol as a RAW
    // identifier `r#loop`, accepted for EVERY keyword except a handful (`crate`/`self`/`Self`/`super` can't
    // be raw — and `_` is the wildcard, not a name) — mangle those with a reserved prefix instead. This is
    // the identifier-emission twin of the `-`→`_` sanitization above; the SAME mapping applies at the `fn`
    // declaration and every call/reference (all go through `sanitize_ident`), so they agree. wasm is
    // unaffected (function names there are indices, not identifiers).
    if is_rust_raw_ident_exception(&s) {
        return format!("cdz_kw_{s}");
    }
    if is_rust_keyword(&s) {
        return format!("r#{s}");
    }
    s
}

/// The Rust keywords that CANNOT be written as a raw identifier (`r#…`) — so a Cadenza def/binder named one
/// is mangled with a reserved prefix instead. `_` is the wildcard (never a raw ident); the rest are the
/// path-sensitive keywords rustc rejects after `r#`.
fn is_rust_raw_ident_exception(s: &str) -> bool {
    matches!(s, "crate" | "self" | "Self" | "super" | "_")
}

/// Whether `s` is a Rust reserved word (a strict OR reserved keyword) — one that must be emitted as a raw
/// identifier `r#s` when it is a Cadenza-source name. Excludes the raw-ident exceptions (handled by
/// [`is_rust_raw_ident_exception`]). The set is the Rust 2021 keyword list; `match`/`fn`/`if`/`else`/`let`
/// are Cadenza reserved words too (they never reach here as a def name) but are listed for completeness so
/// any surviving occurrence is escaped rather than emitted raw.
fn is_rust_keyword(s: &str) -> bool {
    matches!(
        s,
        // strict keywords
        "as" | "break" | "const" | "continue" | "dyn" | "else" | "enum" | "extern" | "false"
            | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match" | "mod" | "move"
            | "mut" | "pub" | "ref" | "return" | "static" | "struct" | "trait" | "true" | "type"
            | "unsafe" | "use" | "where" | "while"
            // 2018+ strict
            | "async" | "await"
            // reserved (future) keywords
            | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override" | "priv"
            | "typeof" | "unsized" | "virtual" | "yield" | "try" | "gen"
    )
}

/// The Rust `fn` identifier for a reachable definition — the ONE name both the declaration
/// ([`emit_fn`]/[`emit_export`]) and every `Core::Call` to it must agree on.
///
/// An EXPORT keeps its verbatim boundary name (it crosses the crate edge; export names are unique). A
/// non-export uses [`sanitize_ident`], UNIQUED per definition when its sanitized name COLLIDES with another
/// emitted definition's. The collision arises from β-copying: a helper with a do-local recursive worker
/// (`def helper(x) = (def (fac n) …); fac(x)`) called from N sites is inlined N times, each copy carrying
/// its OWN `fac` DEFINITION (a distinct `db.defs` index) but the SAME source name — so N `fn fac` at module
/// scope, which rustc rejects (E0428 "the name `fac` is defined multiple times"). The wasm backend never
/// collides because a function's identity there is its INDEX, not its name; the Rust backend must likewise
/// give each colliding copy a distinct name. Suffixing the def INDEX (`fac_7`) is deterministic and unique,
/// and — read identically at the declaration and the call site — keeps the recursive self-call pointing at
/// its own copy. A def whose name is unique among the emitted set is left un-suffixed (the common case, so
/// ordinary programs are byte-identical).
pub(crate) fn fn_ident(db: &Db, layout: &crate::layout::Layout, def: usize) -> String {
    // The Rust identifier for ANY def is its SANITIZED name (`sum-to` → `sum_to`) — the `-` etc. that
    // Cadenza allows are not Rust ident chars, so a boundary name is still sanitized for the emitted `fn`.
    let base = match layout.export_plan(def) {
        Some(e) => sanitize_ident(&e.name),
        None => sanitize_ident(&db.defs[def].name),
    };
    // An EXPORT is never suffixed: export names are unique, its `pub fn` name is the crate's public entry,
    // and a call to it (from another def) must name it stably. Only a NON-export can collide (a β-copied
    // do-local worker inlined at N sites yields N same-named definitions), so only it disambiguates.
    if layout.export_plan(def).is_some() {
        return base;
    }
    // Does ANY other emitted definition resolve to the same sanitized ident? If so, this non-export must
    // disambiguate against it (whether the other is an export or another β-copy).
    let collides = layout.order.iter().any(|&other| {
        other != def
            && base
                == match layout.export_plan(other) {
                    Some(e) => sanitize_ident(&e.name),
                    None => sanitize_ident(&db.defs[other].name),
                }
    });
    if collides {
        // The def index is a stable per-definition unique key (the wasm backend's function-index identity,
        // surfaced here as a name suffix). Underscore-joined so it stays a valid identifier.
        format!("{base}_{def}")
    } else {
        base
    }
}

#[cfg(test)]
mod tests;
