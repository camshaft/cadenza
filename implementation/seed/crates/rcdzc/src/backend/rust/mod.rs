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

/// If `ty` is an integer type whose FIXED width is ILL-FORMED (outside the admitted `1..=64`), return the
/// CDZ0302 reject that names the fault — else `None`. An out-of-range width (the sentinel `Int0` a
/// `reduce_ctor` clamp leaves after a malformed/negative/over-ceiling width like `(Int -8)` / `(UInt 65)`)
/// is an ILL-FORMED TYPE: no value of it can exist, so a boundary of that type is a REJECT, not a target
/// limitation. This mirrors the wasm backend, which reaches CDZ0302 by fit-checking the ground literal
/// against the empty width-0 range (`select.rs`); the Rust backend's type-mapping decline would otherwise
/// fire FIRST and mask the diagnostic (a codeless "no native Rust representation" → the gate reads it as an
/// unimplemented-construct `todo` instead of the typed rejection `pass` the wasm target gives). MUST be
/// distinguished from a VALID-but-non-aliased width (`UInt7`, `UInt24` — in `1..=64`): that is a genuine
/// backend limitation with no native Rust primitive, which stays a codeless decline (todo, correct).
/// Whether `ty` is a function type (a closure value) — after stripping a nominal newtype wrapper. Used to
/// decline a closure crossing the EXPORT boundary (no closure-handle ABI on the Rust target).
fn is_fn_ty(ty: &crate::ty::Ty) -> bool {
    matches!(ty.strip_nominal(), crate::ty::Ty::Fn(_, _))
}

fn ill_formed_int_width_reject(ty: &crate::ty::Ty) -> Option<Reject> {
    use crate::ty::{Ty, Width};
    let Ty::Int(it) = ty else { return None };
    let Width::Fixed(w) = it.width else {
        return None;
    };
    if (1..=64).contains(&w) {
        return None;
    }
    Some(Reject::coded(
        crate::diag::Code::IntOutOfRange,
        format!(
            "`{}{w}` is not a valid integer type: a width must be in 1..=64 (a fixed-size integer wider \
             than 64 bits is reserved to the big-integer layer, and 0 is not a width)",
            if it.ground_signed() { "Int" } else { "UInt" }
        ),
    ))
}

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

/// The Rust VALUE-PARAMETER name for the async gas/yield env (`async fn f(<this>: &mut __CdzE, …)`),
/// threaded into every emitted call. A `__`-prefixed RESERVED name — NOT a bare `env` — so it cannot
/// collide with a SOURCE parameter literally named `env` (`(def (ev e env) …)`), which a bare `env` would
/// duplicate in the signature (rustc E0415 "bound more than once"). Matches the `__CdzE`/`__pay`/`__p`
/// reserved-name convention; user idents never begin with `__` (the sanitizer does not emit it).
pub(super) const ENV_PARAM: &str = "__cdz_env";

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
    // An export's BOUNDARY NAME must be a valid component-model kebab extern name — a LANGUAGE-level
    // ill-formedness (CDZ0201), not a wasm-only load concern: two source names colliding under kebab
    // normalization (`fA` + `f-a` → `f-a`), or a name with a digit-/hyphen-led or non-ASCII segment
    // (`step-by-2`), is rejected on EVERY backend. The wasm backend rejects these at export planning
    // (`kebab_export_collision`/`invalid_kebab_export_name`); the rust backend emits no component, so it
    // would otherwise silently emit a `pub fn` where wasm rejects — a differential outcome. Apply the SAME
    // two checks here so both backends agree (the corpus grades these `(error CDZ0201)`).
    if let Some(reject) = super::wasm::kebab_export_collision(layout) {
        return Err(reject);
    }
    if let Some(reject) = super::wasm::invalid_kebab_export_name(db, layout) {
        return Err(reject);
    }
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
    // Each REACHED lambda-lifted closure (`layout.lifted[k]`, reached by a `Core::Closure` in some body)
    // becomes a private `fn __lifted_{k}(<captures…>, <params…>) -> <ret>` — the closure VALUE a
    // `Core::Closure` builds calls into it. An UNREACHED slot is skipped (no `Core::Closure` names it, so
    // no closure value references it — emitting it would be dead code that might not even type). A lifted
    // body that declines (an unsupported construct) declines the whole module, exactly like any `fn`.
    for k in 0..layout.lifted.len() {
        if layout.lifted_reached.get(k).copied().unwrap_or(false) {
            let f = expr::emit_lifted_lambda(db, k, layout, mode)?;
            out.push('\n');
            out.push_str(&f);
        }
    }
    // A Float-keyed set/map emits a total-order float wrapper (a bare `f32`/`f64` is not `Ord`). Two
    // width-specific wrappers — `__CdzF64` over `u64` bits, `__CdzF32` over `u32` bits — since the key type
    // maps `Float64`→`__CdzF64` / `Float32`→`__CdzF32` (a `__CdzF64` around an `f32` would not type-check).
    // Each is emitted ONLY when the body references it, detected by scanning for its unambiguous CONSTRUCTOR
    // marker `<name>::new(` (NOT a raw type-name substring — the `__`-prefixed name is backend-reserved so a
    // user ident can never produce it, and `::new(` cannot appear except where the wrap emits it). Inserted
    // right after the preamble, before any use. Gating on the emitted text keeps the wrapper out of the
    // common float-free program (where an unused struct would be dead code). ORDER: F32 then F64, both after
    // the preamble — a program may key on either or both width.
    let insert_at = PREAMBLE.len();
    let mut prelude = String::new();
    // Inject each wrapper's decl when the emitted source USES it. A wrapper name appears in exactly two
    // genuine contexts, so gate on EITHER:
    //  - a COLLECTION TYPE parameter — `BTreeSet<__CdzF64>` / `BTreeMap<__CdzF64, V>` — always spelled
    //    `<__CdzF64` (the key/element is the first type arg). This covers the context-typed EMPTY collection
    //    (`Map.empty`/`Set.of (list)` at a float-keyed type) that annotates the type with NO constructor —
    //    the gap a `::new(`-only gate missed (rustc "cannot find type `__CdzF64`").
    //  - the CONSTRUCTOR — `__CdzF64::new(` — for a collection whose type is INFERRED (a bare
    //    `BTreeMap::new()` seed) so the type name never appears in an annotation, only the wrapped key does.
    // Both markers are collision-free: `sanitize_ident` escapes a leading `__` in every user ident, so a
    // user `(type __CdzF64 …)` emits `enum cdz_user___CdzF64` — which contains the BARE substring `__CdzF64`
    // (why a plain `out.contains("__CdzF64")` would SPURIOUSLY inject the struct) but NEVER `<__CdzF64` (a
    // set-element user sum is `<cdz_user___CdzF64`) nor `__CdzF64::new(` (its ctor is `cdz_user___CdzF64::A`).
    // The F32/F64 markers are distinct substrings, so each fires only for its own width.
    let uses = |w: &str| out.contains(&format!("<{w}")) || out.contains(&format!("{w}::new("));
    if uses("__CdzF32") {
        prelude.push_str(CDZ_F32_DECL);
    }
    if uses("__CdzF64") {
        prelude.push_str(CDZ_F64_DECL);
    }
    if !prelude.is_empty() {
        out.insert_str(insert_at, &prelude);
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
#![allow(non_snake_case, non_camel_case_types, unused_parens, clippy::all)]
";

/// A TOTAL-ORDER Float64 wrapper for use as a `BTreeSet` element / `BTreeMap` key — the ONE ordered
/// position a bare `f64` cannot occupy (it is `PartialOrd`, not `Ord`: NaN breaks totality). It stores the
/// float's CANONICAL BIT PATTERN and orders/compares by those bits, exactly mirroring the value-heap
/// runtime's `box-float` (`cdz-runtime` `op_box_float`): every NaN — of any incoming bit pattern —
/// canonicalizes to the ONE quiet NaN `f64::NAN.to_bits()` on construction, so two NaNs are the SAME key
/// (the corpus's "a set of two NaN floats dedups to one" / "a NaN map key is found by a differently-produced
/// NaN"); a non-NaN keeps its bits verbatim, so `-0.0` stays DISTINCT from `0.0`. Ordering is by the raw
/// `u64` bits — NOT numeric order — matching the runtime, which orders a float key by its canonical bytes
/// (`Set.to-list` / map enumeration order is by those bytes, not by magnitude). The name is `__`-prefixed
/// (backend-RESERVED — a user ident never begins with `__`, so it can never collide with a `(type CdzF64 …)`
/// the way the bare `CdzF64` did → rustc E0428). Emitted ONLY when a Float64-keyed set/map is present
/// (gated on the `__CdzF64::new(` marker); an unused struct would trip dead-code lints, so `#[allow(dead_code)]`.
const CDZ_F64_DECL: &str = "\
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct __CdzF64(u64);
#[allow(dead_code)]
impl __CdzF64 {
    fn new(v: f64) -> Self { __CdzF64(if v.is_nan() { f64::NAN.to_bits() } else { v.to_bits() }) }
    fn get(self) -> f64 { f64::from_bits(self.0) }
}
impl PartialEq for __CdzF64 { fn eq(&self, other: &Self) -> bool { self.0 == other.0 } }
impl Eq for __CdzF64 {}
impl PartialOrd for __CdzF64 { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) } }
impl Ord for __CdzF64 { fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.0.cmp(&other.0) } }
";

/// The Float32 twin of [`CDZ_F64_DECL`] — a total-order wrapper over the `u32` bit pattern, canonicalizing
/// every NaN to the one quiet `f32::NAN.to_bits()` (the f32 twin of the runtime's `box-float32`). Needed
/// because a `Float32`-keyed set/map maps to `__CdzF32` (a `__CdzF64` around an `f32` value would not
/// type-check, and a lossy `as f64` widen would collapse distinct f32 keys). Same `__`-reserved name +
/// `::new(`-marker gating as the F64 wrapper.
const CDZ_F32_DECL: &str = "\
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct __CdzF32(u32);
#[allow(dead_code)]
impl __CdzF32 {
    fn new(v: f32) -> Self { __CdzF32(if v.is_nan() { f32::NAN.to_bits() } else { v.to_bits() }) }
    fn get(self) -> f32 { f32::from_bits(self.0) }
}
impl PartialEq for __CdzF32 { fn eq(&self, other: &Self) -> bool { self.0 == other.0 } }
impl Eq for __CdzF32 {}
impl PartialOrd for __CdzF32 { fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> { Some(self.cmp(other)) } }
impl Ord for __CdzF32 { fn cmp(&self, other: &Self) -> std::cmp::Ordering { self.0.cmp(&other.0) } }
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
    // A closure (`Ty::Fn`) crossing the EXPORT boundary declines: an exported fn is called by the gate
    // driver (and any real consumer) with VALUE arguments written as literals, and there is no way to
    // synthesize an `Rc<dyn Fn>` argument at that boundary — nor to render a returned closure as a value.
    // (The corpus's closure round-trip cases pass a scalar where the export's closure PARAM sits, expecting
    // the wasm handle-ABI to route it; the Rust target has no such boundary.) An INTERNAL closure — passed
    // to a recursive helper, the case runtime closures exist for — is unaffected: it never crosses an
    // export edge, so this guard (gated on `public`) does not touch it. Decline cleanly (todo), not fail.
    if public && (params.iter().any(|(_, t)| is_fn_ty(t)) || is_fn_ty(result)) {
        return Err(Reject::decline(format!(
            "`{name}`: a function-typed value cannot cross the Rust export boundary (no closure handle ABI)"
        )));
    }
    // Whether this function is compiled as a `loop` (it self-tail-calls). A looped function REASSIGNS
    // its parameter locals each iteration, so they are declared `mut`. Detected once here and again in
    // `emit_body` (both read the same predicate), so the signature's `mut` and the body's loop agree.
    let loops = !params.is_empty() && expr::body_loops(db, def);
    let mut param_src = String::new();
    // In async/gas mode, the FIRST parameter is the caller-supplied gas/yield env, threaded into every
    // call. It precedes the source parameters; the source params keep their positions after it.
    if mode.is_async() {
        param_src.push_str(&format!("{ENV_PARAM}: &mut {ENV_TYPE_PARAM}"));
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
        // An ILL-FORMED integer width in a parameter type is a REJECT (CDZ0302), not a target decline —
        // catch it before the codeless "no native rep" decline so the diagnostic matches the wasm target.
        if let Some(reject) = ill_formed_int_width_reject(ty) {
            return Err(reject);
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
    // An ILL-FORMED integer width in the RESULT type is a REJECT (CDZ0302), not a decline — the twin of
    // the parameter check above, matching the wasm target (`(: 5 (Int -8))` → CDZ0302, not a codeless
    // decline). NOT for a DIVERGING body: it produces no value, so a `!` return is legitimate regardless of
    // the nominal result width (the `diverges` guard below wins). Checked before the type-mapping decline.
    if !diverges && let Some(reject) = ill_formed_int_width_reject(result) {
        return Err(reject);
    }
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
    // For a QUANTITY result, ALSO emit the unit's canonical VALUE-form spelling (`// cdz-unit[ident]:
    // <value-form>`) beside the type note. `render_name` carries the unit as `Unit::render` — the TYPE
    // surface (bare `(Unit.base …)`, `Unit.*`/`Unit.^ -1` for a derived unit) — but cdz-run prints a
    // quantity VALUE with the DOTTED value-form unit (`((. Unit base) …)`, a `Unit./` quotient for a
    // derived unit). The gate's boundary render needs THAT spelling, and reconstructing it from the type
    // string is fragile; `render_value_form` produces it directly (mirroring `lower::unit_value_ast`), so
    // the driver splices it verbatim. Inert to rustc; keyed by ident like the return note.
    let unit_note = match result {
        crate::ty::Ty::Qty { unit, .. } => {
            // A quantity DISPLAYS at its dimension's REFERENCE unit (scale dropped) — `5 kilometer` prints
            // `5000 meter`, NOT `5 kilometer`. So the value-form unit is `unit.at_reference()` (the same
            // exponent map at scale 1/1). For a scale-1 unit this is `unit` itself (byte-neutral). Plus, a
            // NON-scale-1 unit needs the magnitude SCALED to that reference: emit its `num/den` in a
            // `// cdz-scale[ident]:` note so the harness multiplies the boundary value (a scale-1 unit emits
            // NO scale note — the magnitude is displayed as stored). Both notes are inert `//` comments.
            let (num, den) = unit.scale();
            let scale_note = if (num, den) == (1, 1) {
                String::new()
            } else {
                format!("// cdz-scale[{ident}]: {num}/{den}\n")
            };
            format!(
                "// cdz-unit[{ident}]: {}\n{scale_note}",
                unit.at_reference().render_value_form()
            )
        }
        _ => String::new(),
    };
    let ret_note = format!("{ret_note}{unit_note}");
    if mode.is_async() {
        // `async fn <name><__CdzE: CdzEnv>(env: &mut __CdzE, …) -> <ret> { env.consume(1).await; <body> }`
        // — the per-call fuel charge + cooperative-yield point at entry. The env TYPE PARAMETER is named
        // `__CdzE` (not a bare `E`) so it can NEVER collide with a user sum's Rust type name (a sum named
        // `E` would otherwise shadow the type param, making `E::Variant` unresolvable) — the `__` prefix
        // marks it backend-reserved, matching the emitted `__pay`/`__p` locals.
        Ok(format!(
            "{ret_note}{vis}async fn {ident}<{ENV_TYPE_PARAM}: CdzEnv>({param_src}) -> {ret} {{\n    {ENV_PARAM}.consume(1).await;\n{body_src}\n}}\n"
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
    // A LEADING `__` is the backend-RESERVED namespace: the emitter injects `__`-prefixed idents for its
    // own machinery — `__CdzF32`/`__CdzF64` (the float-key wrappers), `__CdzE`/`__cdz_env` (the async gas
    // env), `__pay`/`__p` (match-payload locals), `__lifted_N`, the render `__r`/`__e{n}`/… locals. A
    // Cadenza name CAN legally begin with `_` (the lexer's `is_ident_start` accepts it) and would otherwise
    // pass through here UNCHANGED — so a user `(type __CdzF64 …)` / `(def __pay …)` would emit the SAME
    // Rust ident as the injected one → rustc E0428 duplicate definition / a captured local. Escape a
    // leading `__` with a `cdz_user_` prefix so a user ident can NEVER land in the `__`-reserved space
    // (a generated `__…` never starts with `cdz_user_`, and this map is applied at BOTH the declaration and
    // every reference — all through `sanitize_ident` — so they still agree). A single leading `_` is left
    // alone (only the DOUBLE underscore is reserved), keeping the common `_unused`-style name readable.
    if s.starts_with("__") {
        return format!("cdz_user_{s}");
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
