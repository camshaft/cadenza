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
    // In async/gas mode, the emitted module carries the `CdzEnv` trait the host implements — the fuel
    // meter + cooperative-yield point every emitted function awaits at entry.
    if mode.is_async() {
        out.push_str(CDZ_ENV_TRAIT);
    }
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

/// The gas/yield interface the async-mode module declares and the host implements. `consume` is `async`
/// so a host can `.await` a cooperative yield inside it (return control to the executor after metering);
/// it returns `impl Future` (RPITIT) rather than `async fn` in the trait so the emitted source is
/// lint-clean and needs no `async_trait` dependency. An implementation typically panics (or the future
/// never resolves) when fuel is exhausted — an emitted function awaits `consume(1)` at entry, so a
/// runaway computation is bounded at the granularity of a call.
const CDZ_ENV_TRAIT: &str = "\
/// The gas/yield interface: the host meters fuel in `consume` and MAY await a cooperative yield there.
pub trait CdzEnv {
    fn consume(&mut self, gas: u64) -> impl core::future::Future<Output = ()>;
}
";

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
    let loops = !params.is_empty() && expr::body_self_loops(db, body, def);
    let mut param_src = String::new();
    // In async/gas mode, the FIRST parameter is the caller-supplied gas/yield env, threaded into every
    // call. It precedes the source parameters; the source params keep their positions after it.
    if mode.is_async() {
        param_src.push_str("env: &mut E");
    }
    for (i, (binder, ty)) in params.iter().enumerate() {
        if i > 0 || mode.is_async() {
            param_src.push_str(", ");
        }
        let pname = param_name(db, *binder, i);
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
    let ret = types::rust_type(result).ok_or_else(|| {
        Reject::decline(format!(
            "`{name}`: result type {} has no native Rust representation",
            result.render_name()
        ))
    })?;
    // Render the body against the parameter environment. Selection reads the core + type columns on
    // demand, so a fault deep in the body surfaces here as a decline. In async mode the body's calls
    // become `Box::pin(callee(env, …)).await`; a self-tail-recursive body becomes a `loop` (so `def` is
    // passed to detect a self-call).
    let body_src = expr::emit_body(db, body, params, def, layout, mode)?;
    let vis = if public { "pub " } else { "" };
    // The function NAME is sanitized to a valid Rust identifier (`sum-to` → `sum_to`) — the SAME
    // mapping a `Core::Call` uses at the call site, so the declaration and every call agree. (A `-` is
    // the idiomatic Cadenza word separator but not a Rust ident char.)
    let ident = sanitize_ident(name);
    if mode.is_async() {
        // `async fn <name><E: CdzEnv>(env: &mut E, …) -> <ret> { env.consume(1).await; <body> }` — the
        // per-call fuel charge + cooperative-yield point at entry. `<E: CdzEnv>` is the host's env type.
        Ok(format!(
            "{vis}async fn {ident}<E: CdzEnv>({param_src}) -> {ret} {{\n    env.consume(1).await;\n{body_src}\n}}\n"
        ))
    } else {
        Ok(format!(
            "{vis}fn {ident}({param_src}) -> {ret} {{\n{body_src}\n}}\n"
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
    s
}

#[cfg(test)]
mod tests;
