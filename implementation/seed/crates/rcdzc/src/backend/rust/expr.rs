//! Render a `Core` node as a Rust expression — the Rust backend's "selection".
//!
//! This is the structured backend's analogue of the wasm backend's instruction selection, but instead
//! of flattening the core into a stack-machine sequence it prints the core's structure as Rust's own
//! (`backends-and-targets.md` §A Backend Linearizes The Core Only If Its Target Is Linear). Each core
//! form maps to a Rust expression: `If` → an `if/else`, `Match` → a `match`, `Let` → a block with
//! `let` bindings, `Call` → a function call, `Arith` → a checked operation, `Compare` → a comparison,
//! `Convert` (`.wrap`) → an `as` cast. The machine representation is a read-off of the solved type
//! (`reference-compiler.md` §A Value's Machine Representation Follows Its Solved Type At Selection) —
//! read via `type_of`, exactly as the wasm backend reads it.
//!
//! NUMERIC MODEL — the correctness heart. A Cadenza integer TRAPS on overflow (`numeric-model.md`
//! §Overflow Is Defined). Rust's native `iN`/`uN` are Cadenza's aliased widths with the SAME
//! wrapping-vs-checked distinction, so:
//!   - `+`/`-`/`*` emit `<lhs>.checked_add(<rhs>)` (etc.) unwrapped with a trap on `None` — the direct
//!     expression of the wasm backend's carry/borrow/round-trip guard-and-range-check recipe (that
//!     recipe existed to express checked arithmetic in the flat rung; `checked_*` IS it, at any width);
//!   - `/`/`%` emit `checked_div`/`checked_rem`, which return `None` on ÷0 AND on `MIN / -1` — exactly
//!     the two cases the numeric model traps (wasm traps these natively);
//!   - `&`/`|`/`^` are total → the plain Rust operator;
//!   - `.wrap` truncates via an `as` cast, which in Rust keeps the low bits and reinterprets at the
//!     target — bit-identical to `IntValue::wrap_to` (`numeric-model.md` §wrap never traps).
//!
//! The trap is a Rust `panic!` (an aborting trap), the native analogue of a wasm `unreachable`.
//!
//! A construct this scalar slice does not render — a runtime shift (whose out-of-range-count trap is
//! not yet expressed), a compound value, a poison — DECLINES, attributed to this target
//! (`backends-and-targets.md` §A Backend Inherits The Front's Decline Boundaries).

use super::Mode;
use super::types;
use crate::ast::{IntValue, StructId};
use crate::core::Core;
use crate::db::Db;
use crate::diag::{Code, Reject};
use crate::infer::type_of;
use crate::layout::Layout;
use crate::lower::core_of;
use crate::resolved::Prim;
use crate::ty::{IntTy, Sign, Ty, Width};
use std::collections::HashMap;

/// The environment a body is rendered in: a map from a binder occurrence (a parameter, or a kept
/// `let` binding) to the Rust identifier it reads as. A `Core::Param`/`Core::LocalRef` looks its
/// binder up here. Populated with the parameters at the export, and extended with each `let` binding.
type Env = HashMap<StructId, String>;

/// The rendering context threaded through every `emit` — the emission [`Mode`] (sync vs async/gas) and,
/// when the function being emitted is compiled as a tail `loop`, the [`LoopGroup`] describing how a tail
/// call to a group member iterates in place. A struct (not a bare `Mode`) so these ride along without
/// widening every helper's argument list; the callee-name/param lookups a `Core::Call` needs read `db`
/// directly, so the boundary `Layout` is not needed here (the caller passes it only to `emit_body`).
#[derive(Clone)]
pub struct Ctx<'a> {
    pub mode: Mode,
    /// The boundary layout — read by a `Core::Call` to derive the callee's UNIQUED Rust `fn` ident
    /// ([`super::fn_ident`]), so a call to a β-copied do-local worker names ITS copy, matching the copy's
    /// declaration (the two agree because both go through `fn_ident`, which suffixes a colliding def index).
    pub layout: &'a Layout,
    /// `Some` iff the function being emitted is compiled as a `loop` (it tail-calls into its own tail-
    /// recursion group). A tail call to a group member reassigns the shared parameter locals (+ the
    /// `which` state, for a mutual group) and `continue`s instead of recursing; every other tail
    /// position `break`s its value out of the loop. `None` = an ordinary body (no loop).
    pub loop_group: Option<&'a LoopGroup>,
    /// The sum-match payload bindings in scope — one per `(scrutinee, path)` a matched arm bound the
    /// payload of. A `Core::SumPayload { scrutinee, path }` in an arm body resolves to the identifier
    /// here (the arm's Rust `match` pattern bound the payload to it), instead of re-extracting it. Empty
    /// outside a sum match; extended (a fresh `Ctx`) per arm by `emit_sum_match`.
    pub sum_binds: Vec<SumBind>,
    /// The SOLVED TYPE of a sub-value at a switch/bind path, recorded as each match arm DESCENDS — the
    /// Rust-backend twin of `lower`'s `path_types`. A `Payload` step's target type depends on WHICH variant
    /// the enclosing arm entered (`(type W (A Int64) (V (Option Int64)))`: `V`'s payload is `Option Int64`,
    /// `A`'s is `Int64`), which the FLATTENED path cannot encode. An arm at disc `d` records its payload's
    /// type (`variant_payload_ty(subject, d)`) here, keyed by the bind path; a NESTED switch then resolves
    /// its subject type by lookup (longest-prefix match + walk the remaining `Elem`s) instead of
    /// re-deriving it variant-0-first. Empty at the root (the scrutinee's own type resolves directly).
    pub sum_path_types: Vec<(Vec<crate::core::PathStep>, Ty)>,
    /// Set true ONLY while emitting the base-map operand of an enclosing `Map.insert`/`Map.remove`. An
    /// empty `Map.empty` in that position has its `BTreeMap<K,V>` element types INFERRED by the enclosing
    /// `.insert(k, v)`/`.remove(&k)` (rustc reads K/V from the inserted key/value), so a bare
    /// `BTreeMap::new()` compiles — and any spelled annotation there would OVER-CONSTRAIN it (grounding an
    /// open var to the default `Int64` would clash with a Rational/String/Bytes key the insert actually
    /// uses → E0308). So `MapNew` SUPPRESSES its ground-annotation when this is set. When it is NOT set
    /// (an empty map used get-only / passed through — e.g. an empty-Map HANDLER STATE), the annotation is
    /// needed (nothing else fixes K/V) and `MapNew` grounds it. `false` in every other context.
    pub map_typed_by_enclosing_insert: bool,
    /// The `Set` twin of `map_typed_by_enclosing_insert`: set true ONLY while emitting the base-set operand
    /// of an enclosing `Set.insert`/`Set.remove`. An empty `Set.of (list)` in that position has its
    /// `BTreeSet<T>` element type INFERRED by the enclosing `.insert(e)`/`.remove(&e)`, so a bare
    /// `BTreeSet::new()` compiles and any grounded annotation would OVER-CONSTRAIN it (grounding to `Int64`
    /// clashes with a String/Bytes/BigInt element the insert uses → E0308). When NOT set (an empty set used
    /// len-only / passed through — the `(Set.len (Set.of (list)))` E0282 breaker found), `SetOf` grounds the
    /// open element var so the annotation is spellable. A SEPARATE flag from the map one so a map-insert
    /// value that CONTAINS an empty set still grounds the set. `false` in every other context.
    pub set_typed_by_enclosing_insert: bool,
    /// A MATCH SCRUTINEE pre-bound to a Rust `let` local — `(scrutinee StructId, local name)`. A sum match
    /// over a NON-TRIVIAL scrutinee (a `Core::Call`/compound, not a pure param/local read) binds it ONCE
    /// (`{ let __ms = <scrutinee>; <body> }`) and records the mapping here; every `emit_sum_payload` read of
    /// that scrutinee then references the LOCAL instead of RE-EMITTING the scrutinee expression. Without this
    /// a match binder used K times re-emits the scrutinee K times — and a RECURSIVE-CALL scrutinee re-emitted
    /// per binder is `2^depth` calls (an exponential blow-up: `(match (f (+ n 1)) ((Mk a _) (Mk a a)))` calls
    /// `f` twice per level). The Rust-backend twin of the wasm backend's materialize-scrutinee-once fix
    /// (keep the `MatchSum` wrapper for a non-reusable scrutinee, read one slot). Empty outside such a match.
    pub scrut_locals: Vec<(StructId, String)>,
    /// The type this expression is EXPECTED to produce, from its consuming context — set ONLY when the
    /// context fixes a type the node's own `type_of` may leave unsolved. The case: an empty `Set.of
    /// (list)` / `Map.empty` passed as a CALL ARGUMENT whose callee PARAM type is a concrete collection
    /// (`Set Float64`) — the empty node's element is an unsolved VAR at the construction site, so grounding
    /// it to the default (`i64`) spells `BTreeSet<i64>` while the param wants `BTreeSet<__CdzF64>` → E0308
    /// (breaker: empty-Set-at-call-arg). When set, `SetOf`/`MapNew` annotate from this expected type
    /// instead of the default-grounded node type. `None` in every other context (the node's own solved
    /// type governs). Reset to `None` when descending into a sub-expression that does NOT inherit it.
    pub expected_ty: Option<Ty>,
}

/// A payload bound by a sum-match arm's Rust pattern: the scrutinee occurrence + access path the
/// `Core::SumPayload` reads, and the Rust identifier the pattern bound it to. A `SumPayload` matching
/// `(scrutinee, path)` renders as this `name`. `boxed` when the bound payload field is a `Box<…>` (a
/// RECURSIVE variant's field) — a read of it derefs (`*name` for the whole payload, `(*name).i` for a
/// tuple element), the deref twin of the construct site's `Box::new`.
#[derive(Clone)]
pub struct SumBind {
    pub scrutinee: StructId,
    pub path: Vec<crate::core::PathStep>,
    pub name: String,
    pub boxed: bool,
}

/// Describes a tail-recursion group compiled as a shared `loop`. A group of ONE member is plain self-
/// tail-recursion; a group of MANY same-signature members that tail-call each other (a mutual-recursion
/// SCC) share ONE loop dispatched by a `which` state variable. Each member's body renders with its own
/// parameter binders mapped to the SHARED positional locals `__p0, __p1, …` (members may name their
/// params differently but share the signature), and a tail call to member `k` sets `which = k` +
/// reassigns the shared locals + `continue`s (a PARALLEL move: all args into temps before any store, so
/// an arg reading an old param value is correct).
pub struct LoopGroup {
    /// The group's members (`db.defs` indices). `members[0]` is the function being emitted — it enters
    /// the loop at `which = 0` (its own body runs first). A tail `Core::Call` to `members[k]` iterates
    /// with `which = k`. A single-member group is a self-loop (no `which` needed, but harmless).
    pub members: Vec<usize>,
    /// The shared parameter identifiers `__p0…__pN` (one per signature position). Emitted `let mut` and
    /// reassigned each iteration; a member body's own param names map to these.
    pub shared_params: Vec<String>,
    /// The group's result integer type, if integer — a bare-literal tail leaf (`break 0`) is grounded to
    /// it so every `break` yields the SAME Rust type (a `loop` requires all `break` values agree). `None`
    /// for a non-integer result (Bool/unit). All members share the signature, so one result type serves.
    pub result_it: Option<IntTy>,
    /// The group's result FLOAT width (32/64), if float — the float twin of `result_it`. A bare-literal
    /// tail leaf (`break 0.5`) is a `Core::ConstFloat` that DEFAULTS to Float64, so under a `Float32`
    /// result it would `break f64::from_bits(…)` out of an `-> f32` loop → rustc E0308 (the tail-position
    /// sibling of the non-tail match-arm float grounding `emit_match_impl`'s `result_fw` already does).
    /// Ground each break leaf to this width via `emit_grounded_float`. `None` for a non-float result.
    pub result_ft: Option<u32>,
}

impl LoopGroup {
    fn is_mutual(&self) -> bool {
        self.members.len() > 1
    }
}

/// Per-FUNCTION emitted-source-size backstop (bytes of ONE function body's Rust text). The Rust emit
/// walk RE-DESCENDS each shared `Core` `StructId` and re-emits its subtree, so a handler that partial-
/// evaluates into a threaded state referenced 2+ times per step — either fanned out over K>=3 resume/
/// next-state branches, OR a BRANCH-FREE chain where each step's compound feeds the next as a value
/// used 2x (the `nsq1` Newton-sqrt witness: `(x + t/x)/2` chained over 5 `improve`s → ~2^5 × the nested
/// structure) — serializes as a TREE whose ONE function body blows up super-linearly. Unlike the wasm
/// backend (whose invalid module trips the engine function-size limit and whose emit already declines via
/// `EMIT_INSTRUCTION_BUDGET`), the Rust backend has no such bound today, so it hands `rustc` a multi-MB
/// single expression that never finishes parse/typecheck — the corpus gate reports the opaque "artifact
/// did not build" (a `Ran::BadArtifact` FAIL) instead of a clean decline.
///
/// This bound DECLINES cleanly (reject-not-BadArtifact) once ONE function body's emitted text crosses it,
/// so a run-away body grades `todo` (the compiler can't-yet-handle it) rather than emitting an unbuildable
/// artifact. It is the Rust twin of the wasm `EMIT_INSTRUCTION_BUDGET`; the durable LINEAR fix is the same
/// sharing-aware emit (bind a 2+-reached node once into a `Core::Let` slot), a separate increment blocked
/// on the Perceus dup/drop seam.
///
/// CALIBRATION (corpus-wide rust-emit sweep 2026-08-15, 6912 cases, RE-TUNED after a false-decline
/// regression). The largest single function body that `rustc` actually BUILDS + PASSES is the `cbk1`
/// circuit-breaker case in `spec/semantics/14c-effects-and-handlers` (= the sweep's "case 564") at
/// **~5.49MB** — it compiles + runs green on the corpus rust/rust-async gate. (An earlier 4_000_000 cut
/// was mis-calibrated: a local `rustc` probe with a 149s cap looked like a timeout on cbk1 and I wrongly
/// judged 5.49MB "unbuildable", so the 4MB cut FALSE-DECLINED cbk1 — a real pass→todo regression on
/// rust+rust-async, caught by corpus-bugfix's clean rust `--check`. The gate's `rustc` DOES build cbk1;
/// it just takes longer than my impatient local cap.) The smallest genuinely-UNBUILDABLE handler-fold
/// PROBE (not a corpus case) is `rps1` ~6.89MB / `nsq1` ~7.07MB (these exceed the gate's `rustc` build
/// timeout → the opaque "artifact did not build"). So the buildable-vs-unbuildable boundary is the narrow
/// band (5.49MB, 6.89MB]. 6_000_000 sits above the largest BUILDABLE corpus body (cbk1 5.49MB, ~9%
/// headroom) and below every unbuildable probe (rps1 6.89MB, ~13% margin) — no currently-passing corpus
/// case is false-declined (cbk1 is the corpus emit-size MAX; every other case is ≤ ~2.16MB), while `nsq1`,
/// `rps1`, and the dst*/pwm/trn explosions still decline cleanly. NOTE: buildable/unbuildable overlap by
/// SIZE is inherently narrow here (rustc build time ≈ size but not exactly); the durable robust fix is
/// sharing-aware emit (collapses these to linear), a separate increment blocked on the Perceus seam — this
/// SIZE budget is the interim soundness backstop.
pub(super) const RUST_FN_EMIT_BUDGET: usize = 6_000_000;

/// Decline a function whose emitted body exceeds [`RUST_FN_EMIT_BUDGET`] — the per-function backstop that
/// turns a super-linear emit into a clean `todo` decline instead of an unbuildable multi-MB artifact.
/// Called on each emitted function body (ordinary + lifted). See [`RUST_FN_EMIT_BUDGET`] for the mechanism
/// and calibration.
pub(super) fn enforce_fn_emit_budget(body: &str) -> Result<(), Reject> {
    if body.len() > RUST_FN_EMIT_BUDGET {
        return Err(Reject::decline(
            "emit-walk function-size budget exceeded: a handler-derived Core DAG serializes as a tree \
             whose one function body exceeds what rustc can compile (a resume/next-state fan-out or a \
             chained compound threaded state re-descended per reference); pending sharing-aware emit that \
             binds a shared subtree once",
        ));
    }
    Ok(())
}

/// Render a function body as the Rust expression that is its return value. Builds the initial
/// environment from the function's parameters (each binder → its emitted name), then renders the body
/// core. `self_def` is the function's own `db.defs` index — used to detect a SELF-tail-call and, when
/// the body has one, compile the whole body as a `loop` (bounded stack in sync mode; no `Box::pin` poll-
/// chain in async mode). Shared by the export and non-export paths (both pass their `(binder, type)`
/// parameter list). The result is a single expression (the function's tail expression), indented once.
pub fn emit_body(
    db: &mut Db,
    body: StructId,
    params: &[(StructId, Ty)],
    self_def: usize,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    // The tail-recursion group this function belongs to, compiled as a shared `loop` (§keeps deep tail
    // recursion bounded — vital in async, where a `Box::pin` poll-chain would otherwise be as deep as
    // the recursion). Empty = no loop (an ordinary body). One member = self-recursion; many = a mutual
    // SCC dispatched by a `which` state.
    let members = if params.is_empty() {
        Vec::new()
    } else {
        loop_group(db, self_def)
    };
    if !members.is_empty() {
        return emit_loop_body(db, params, self_def, &members, layout, mode);
    }
    // No loop: render the body directly.
    let mut env: Env = HashMap::new();
    for (i, (binder, _)) in params.iter().enumerate() {
        env.insert(*binder, super::param_name(db, *binder, i));
    }
    let ctx = Ctx {
        mode,
        layout,
        loop_group: None,
        sum_binds: Vec::new(),
        sum_path_types: Vec::new(),
        map_typed_by_enclosing_insert: false,
        set_typed_by_enclosing_insert: false,
        scrut_locals: Vec::new(),
        expected_ty: None,
    };
    let expr = emit(db, body, &env, &ctx)?;
    // E0308 FIX — the rust twin of the wasm tail-call wrap (fz 38551 / `7529f6901`). A body that is a
    // direct `Core::Call` emits the CALLEE's natural result rust type; the call-site narrowing ascription
    // `(: (f x) T)` is absorbed as type-only (there is NO Core cast/narrow node — see
    // [[fz-tailcall-narrowint-ascription-return-call-elides-i64-to-i32-wrap]]), so rustc sees the callee's
    // (wider/narrower) int where this fn's signature declares `T` -> `error[E0308] mismatched types`.
    // When the body is such a Call whose callee-result rust INT type differs from this fn's declared result
    // INT type, coerce with an `as <prim>` cast — truncate/extend, matching the wasm `i32.wrap_i64` the tail
    // fix emits. Scoped to INT<->INT scalars: `type_of(body)` is the ASCRIBED (fn-result) type, the callee
    // body's `type_of` is the un-narrowed natural type; any non-Int shape or matching prim is left verbatim
    // (no spurious cast, which would be a silent wrong value rather than a caught build error).
    let expr = if let Core::Call { callee, .. } = core_of(db, body) {
        match db.defs[callee].body {
            Some(callee_body) => {
                let result_ty = type_of(db, body);
                let callee_ty = type_of(db, callee_body);
                let coerce = match (result_ty.strip_nominal(), callee_ty.strip_nominal()) {
                    (Ty::Int(_), Ty::Int(_)) => {
                        let ncx = db.name_ctx();
                        let rt = types::rust_type(&ncx, &result_ty);
                        let ct = types::rust_type(&ncx, &callee_ty);
                        if rt != ct { rt } else { None }
                    }
                    _ => None,
                };
                match coerce {
                    Some(prim) => format!("({expr}) as {prim}"),
                    None => expr,
                }
            }
            None => expr,
        }
    } else {
        expr
    };
    Ok(format!("    {expr}"))
}

/// Wrap an emitted KEY/element expression for a Set element / Map key SLOT when its type is a bare
/// `Float`: a `BTreeSet`/`BTreeMap` key is the WIDTH-SPECIFIC total-order wrapper (`__CdzF64` for a
/// `Float64`, `__CdzF32` for a `Float32`), so a bare float key value must be lifted with the matching
/// `__CdzF{64,32}::new(<e>)` (NaN-canonicalizing on construction, mirroring the runtime's `box-float`).
/// A non-float key stays verbatim. This is the emit twin of [`types::ord_key_type`]'s type substitution —
/// the two MUST agree on BOTH which slots become a wrapper AND the WIDTH, or the emitted value's type would
/// not match the collection's key type (a `__CdzF64::new` around an `f32` is a type error — the very bug
/// this width-split fixes). A TUPLE key REBUILDS the tuple wrapping each float element by position
/// (`(__CdzF64::new(k.0), k.1)`), matching `ord_key_type`'s per-element threading — so a `(Tuple Float Int)`
/// key crosses (v-runtime differential). A float nested in a SUM payload still declines upstream via `ty_is_ord_key`
/// (a later increment); this rebuilds Tuples + Records (sorted-field order) + wraps bare floats.
/// Whether a KEY/element type contains a FLOAT leaf that [`wrap_ord_key`] would lift to a `__CdzF{N}`
/// wrapper — a bare float, OR a float nested inside a tuple/record (which `wrap_ord_key` rebuilds
/// element-by-element). This is the recursion-aware guard the tuple/record rebuild arms gate on: it MUST
/// track `wrap_ord_key`'s own descent (Float / Tuple / Record) so the two agree on whether a rebuild is
/// needed — and, transitively, agree with [`types::ord_key_type`]'s wrapped key TYPE. A shallow
/// direct-`Float` check would return false for a field of type `(Tuple Float Int)`, skipping the rebuild
/// while `ord_key_type` still wraps that field's type → a bare `f64` value in a `__CdzF64` slot (rustc
/// E0308). A nominal is peeled (the boundary erases the tag); any other shape has no wrappable float.
fn key_ty_has_wrappable_float(ty: &Ty) -> bool {
    match ty.strip_nominal_and_qty() {
        Ty::Float(_) => true,
        Ty::Tuple(elems) => elems.iter().any(key_ty_has_wrappable_float),
        Ty::Record(fields) => fields.values().any(key_ty_has_wrappable_float),
        _ => false,
    }
}

/// Whether a KEY/element type needs ANY ord-wrapper at a `BTreeSet`/`BTreeMap` position — a wrappable FLOAT
/// (`__CdzF{N}`) OR a flip-order `Option` (`__CdzOpt`, #42 witness 2). Drives the `wrap_ord_key` Option arm's
/// inner-payload decision + the read-side unwrap gates. Mirrors `ord_key_type`'s wrapping so the value and
/// type agree. (Bare Option key, or Option nested — this walks the same Float/Tuple/Record/Option shapes the
/// wrap descends; a plain Int/String/etc. needs no wrap.)
fn key_ty_needs_ord_wrap(ncx: &crate::ty::NameCtx, ty: &Ty) -> bool {
    match ty.strip_nominal_and_qty() {
        Ty::Float(_) => true,
        s if types::is_flip_order_option_key_shallow(ncx, s) => true,
        Ty::Tuple(elems) => elems.iter().any(|e| key_ty_needs_ord_wrap(ncx, e)),
        Ty::Record(fields) => fields.values().any(|t| key_ty_needs_ord_wrap(ncx, t)),
        _ => false,
    }
}

/// Whether `ty` is a COMPOUND (tuple/record/list/sum) that CONTAINS a float leaf at any depth — the shape
/// whose `Set.to-list`/`Map.to-list` must DECLINE. Per `03-equality-and-observation.sexp:626 §319` a
/// compound containing a float leaf has NO blessed total order, so its ordered enumeration is not defined
/// (matching wasm — v-wasm-opt's `float_ok` is a BARE-ROOT-only privilege that does not propagate into a
/// compound's components). A BARE `Ty::Float` is NOT this — it enumerates by canonical byte (to_bits) order
/// (19-sets.sexp:1494), so this returns FALSE for a bare float. CONSTRUCTION + lookup of a float-carrying
/// compound key/element STILL work (the `__CdzF` wrapper gives rust's `BTree*` a total order for
/// insert/contains/remove — breaker pin 211 keeps the champ hash/eq surface); ONLY the ordered TO-LIST
/// enumeration declines, so this guard lives at `SetToList`/`MapToList`, NOT at the key-admissibility gate
/// (`ty_is_ord_key`).
fn is_float_carrying_compound(ty: &Ty) -> bool {
    match ty.strip_nominal() {
        // A BARE float enumerates by canonical bytes — NOT a float-carrying compound.
        Ty::Float(_) => false,
        Ty::Tuple(_) | Ty::Record(_) | Ty::List(_) | Ty::Sum { .. } => {
            key_ty_has_wrappable_float_deep(ty)
        }
        _ => false,
    }
}

/// Whether a float leaf appears anywhere in `ty`'s component tree (tuple/record/list/sum, any depth) —
/// the DEEP companion of [`key_ty_has_wrappable_float`] that also descends `List`/`Sum` (the #34 faces
/// include a float-leaf list + nested tuple, not just direct tuple/record fields).
fn key_ty_has_wrappable_float_deep(ty: &Ty) -> bool {
    match ty.strip_nominal_and_qty() {
        Ty::Float(_) => true,
        Ty::Tuple(elems) => elems.iter().any(key_ty_has_wrappable_float_deep),
        Ty::Record(fields) => fields.values().any(key_ty_has_wrappable_float_deep),
        Ty::List(elem) => key_ty_has_wrappable_float_deep(elem),
        Ty::Map(k, v) => key_ty_has_wrappable_float_deep(k) || key_ty_has_wrappable_float_deep(v),
        // A SUM's payloads: a float in any variant's payload makes the sum float-carrying. (Recursion is
        // bounded by the finite type tree here; a recursive sum's payload type is the nominal, whose
        // strip_nominal is the sum — but its ARGS are what carry a float, walked via the payload types the
        // caller passes. For the #34 faces (non-recursive float-leaf compounds) this direct walk suffices;
        // a recursive-sum float leaf is a later face if it arises.)
        _ => false,
    }
}

fn wrap_ord_key(ncx: &crate::ty::NameCtx, expr: String, key_ty: &Ty) -> String {
    // A `Qty` erases to its inner numeric, so a Qty-over-Float KEY VALUE wraps in `__CdzF{N}` exactly like a
    // bare float (the value `expr` is already the erased `f64`/`f32`) — else a raw `f64` key sits in a
    // `__CdzF64` slot / `f64: Ord` E0277 (qkm1/qkm3). Peel `Qty` (possibly under a nominal) here so the value
    // wrap agrees with `ord_key_type`'s Qty peel. A Qty inner is always numeric (never nominal/compound).
    if let Ty::Qty { inner, .. } = key_ty.strip_nominal() {
        return wrap_ord_key(ncx, expr, inner);
    }
    match key_ty {
        Ty::Float(ft) if ft.ground_width() == 32 => format!("__CdzF32::new({expr})"),
        Ty::Float(_) => format!("__CdzF64::new({expr})"),
        // A tuple with a float leaf ANYWHERE (a direct element OR nested in a tuple/record element): bind
        // the key once (it may be a non-trivial expr), then rebuild it wrapping each element by position —
        // the per-element `wrap_ord_key` recurses into a nested tuple/record and wraps its float leaves too.
        // The guard is the RECURSIVE `key_ty_has_wrappable_float` (not a shallow direct-Float check): it MUST
        // agree with `ord_key_type`, which threads the wrapper through nested tuples/records, so a field of
        // type `(Tuple Float Int)` gets a wrapped key TYPE and therefore needs a wrapped key VALUE too — a
        // shallow guard would skip the rebuild and emit a bare `f64` into a `__CdzF64` slot (rustc E0308).
        // A float-free tuple has no wrappable leaf → skip the rebuild (emit verbatim) so it stays unchanged.
        Ty::Tuple(elems) if elems.iter().any(key_ty_has_wrappable_float) => {
            let parts: Vec<String> = elems
                .iter()
                .enumerate()
                .map(|(i, e)| wrap_ord_key(ncx, format!("__k.{i}"), e))
                .collect();
            let rebuilt = if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            };
            format!("{{ let __k = {expr}; {rebuilt} }}")
        }
        // A RECORD erases to a Rust tuple in SORTED-field order (a `BTreeMap` iterates sorted), so a record
        // with a float leaf ANYWHERE (a direct field OR nested in a field's tuple/record) rebuilds exactly
        // like a tuple — wrap each field at its sorted position `.i`, recursing into a nested field. Same
        // RECURSIVE guard as the tuple arm (a field of type `(Tuple Float)` gets a wrapped key TYPE from
        // `ord_key_type` and so must be rebuilt too — a shallow direct-Float check would miss it, E0308).
        // Same identity-skip for a float-free record (no wrappable leaf → verbatim).
        Ty::Record(fields) if fields.values().any(key_ty_has_wrappable_float) => {
            let parts: Vec<String> = fields
                .values()
                .enumerate()
                .map(|(i, t)| wrap_ord_key(ncx, format!("__k.{i}"), t))
                .collect();
            let rebuilt = if parts.len() == 1 {
                format!("({},)", parts[0])
            } else {
                format!("({})", parts.join(", "))
            };
            format!("{{ let __k = {expr}; {rebuilt} }}")
        }
        // An `Option`-KEY at a key/element position wraps into `__CdzOpt::new(<opt>)` — the declared-order
        // wrapper (Some<None) that overrides std `Option`'s `None<Some` (#42 witness 2). The inner payload is
        // itself mapped through `wrap_ord_key` when it needs wrapping (e.g. `Option Float64` → the `Some`
        // payload wraps to `__CdzF64`): `.map(|__ov| <wrapped __ov>)`. A payload needing NO wrap (the common
        // `Option Int64`/`Option String`) passes the `Option` through unmapped (`__CdzOpt::new(<opt>)`). The
        // wrapped TYPE `types::ord_key_type` spells (`__CdzOpt<inner_ord_key>`) agrees with this value.
        Ty::Sum { args, .. } if types::is_flip_order_option_key_shallow(ncx, key_ty) => {
            let inner = &args[0];
            if key_ty_needs_ord_wrap(ncx, inner) {
                // The payload itself wraps — map the inner Option value: `Some(p)` → `Some(<wrap p>)`.
                let wrapped_inner = wrap_ord_key(ncx, "__ov".to_string(), inner);
                format!("__CdzOpt::new(({expr}).map(|__ov| {wrapped_inner}))")
            } else {
                format!("__CdzOpt::new({expr})")
            }
        }
        _ => expr,
    }
}

/// The Rust identifier for lambda-lifted closure slot `k` — the `fn` a `Core::Closure { code: k }` value
/// calls into. A backend-reserved name (`__`) that cannot collide with a source def.
pub(super) fn lifted_ident(k: usize) -> String {
    format!("__lifted_{k}")
}

/// The Rust identifier for the per-closure `EnvClosure` synth STRUCT of lifted slot `k` (Option A, async
/// mode) — `__Clos_{k}`, a backend-reserved (`__`) name that cannot collide with a source type. The struct
/// carries the closure's captures as fields; its `EnvClosure::call` impl forwards them + the env into the
/// async lifted fn `__lifted_{k}`. Emitted at module level (see `emit_closure_struct`) beside the lifted fn.
pub(super) fn closure_struct_ident(k: usize) -> String {
    format!("__Clos_{k}")
}

/// Wrap a closure VALUE expression for emission: `{ <cap_lets><expr> }` when there are capture `let`s to
/// scope, else the bare `<expr>`. A no-capture closure has no `let`s, so a `{ expr }` block around it is
/// needless and trips `unused_braces` under the gate's `-D warnings` (the value appears in arbitrary
/// positions — a `.call(..)` argument, a function/return value — each of which the lint flags). Shared by
/// the sync (`Rc<dyn Fn>`) and async (`Rc<dyn EnvClosure>`) `Core::Closure` emits so both stay warning-clean.
/// Emit a rust expression that builds the R2 VALUE-FORM node (a `cadenza_ast::ast::StructId`) for the value
/// `val_expr` of type `ty`, into a `cadenza_ast::ast::Builder` in scope as `__b` — the native-rust twin of
/// `cdz-runtime`'s `encode_value_recursive`. Mirrors the runtime's post-order build: a SCALAR is one
/// `atom_leaf`; a TUPLE is `(tuple …)` — a `list` headed by the `tuple` name-atom with each element built
/// recursively. Each `__b.<method>(…)` borrows `__b` mutably, so a compound sequences its children into
/// block-local `let`s (block scoping means the fixed temp names never collide across nesting) before the
/// final `list`. INCREMENTAL: fixed-width Int (as `i64` → a `BigInt` leaf), Bool, and Tuple-of-those are
/// wired (the round-trip corpus shape); any other shape DECLINES (the encode stays a `todo` on rust, never a
/// miscompile). The result BYTES match the runtime's `value-encode` by construction (v-runtime: all three
/// codecs share `ast-encoding.md`, and `codec::encode` canonicalizes) — though the round-trip cases only need
/// rust-internal `decode∘encode = id`, which reusing `cadenza_ast::codec` for both directions guarantees.
/// Whether `ty` is a scalar the runtime can ORDER + encode as a Set element / Map key (mirrors
/// `const_key_order` / `set_elements_canonical`, which handle only scalars). A non-scalar element/key makes
/// the wasm `value-encode` DECLINE, so the native rust codec must decline it too — else it would encode a
/// value wasm rejects (a cross-backend divergence). `Float` is excluded: it is not `Ord` (no total canonical
/// order), so a `BTreeSet<f64>`/float-keyed `BTreeMap` would not even compile.
fn is_orderable_scalar_key(ty: &Ty) -> bool {
    matches!(
        ty.strip_nominal(),
        Ty::Int(_) | Ty::Bool | Ty::Char | Ty::String | Ty::Symbol
    )
}

/// The shared `(list …)`-CHILDREN loop of the encode List + Set arms of [`emit_value_form`]: bind the `list`
/// head name-atom, then push each element's value-form node, cloning the element so a non-Copy element is not
/// moved out of the `.iter()` borrow. Returns the statements that populate `__lc` (headed by `list`) over
/// `src_var`; the caller finishes with `__b.list(__lc)` — the List arm as its result, the Set arm bound to
/// `__lst` inside the 2-child `((. Set of) (list …))`. `child` is the per-element node expr emitted with the
/// element bound to `__ev`. BYTE-IDENTICAL to the previous two inline copies (same string assembly).
fn emit_list_children(child: &str, src_var: &str) -> String {
    format!(
        "let __lh = __b.name(\"list\"); let mut __lc = vec![__lh]; \
         for __el in {src_var}.iter() {{ let __ev = __el.clone(); let __c = {child}; __lc.push(__c); }}"
    )
}

/// Whether `ty` is (or transitively contains) a sum that references its OWN declaration through ANY payload
/// position — INCLUDING through a collection (`List`/`Map`/`Set`) element/key/value. This is DISTINCT from
/// [`super::enums::variant_is_recursive`], which excludes collections (a `Vec<Ast>` enum FIELD is finite-sized
/// and needs no `Box`, so the Rust enum type is fine). But the native-rust value-form encode/decode WALKS the
/// structure, and that walk recurses through the collection too — so a codec over a recursive type (`type Ast
/// … (List (List Ast))`) emits UNBOUNDED rust that HANGS rustc. Detect it up front and decline: the native
/// value codec of a recursive type is a later increment (the wasm-only-corpus doc), and an honest decline is
/// far better than a compile hang. `seen` cycle-guards the sum-decl graph so the walk terminates.
fn value_codec_type_is_recursive(
    db: &mut Db,
    ty: &Ty,
    seen: &mut std::collections::BTreeSet<crate::ast::StructId>,
) -> bool {
    match ty.strip_nominal() {
        Ty::Sum { decl, args, .. } => {
            let decl_occ = *decl;
            if !seen.insert(decl_occ) {
                return true; // re-entering a sum decl already on this path = a cycle = recursive
            }
            let args = args.clone();
            let payload_occs: Vec<crate::ast::StructId> = match db.type_decl_by_occ(decl_occ) {
                Some(d) => d.variants.iter().flat_map(|v| v.payloads.clone()).collect(),
                None => Vec::new(),
            };
            let rec = payload_occs.iter().any(|&occ| {
                crate::eval::typeval_of(db, occ)
                    .is_some_and(|pty| value_codec_type_is_recursive(db, &pty, seen))
            }) || args
                .iter()
                .any(|a| value_codec_type_is_recursive(db, a, seen));
            seen.remove(&decl_occ);
            rec
        }
        Ty::List(e) | Ty::Set(e) => value_codec_type_is_recursive(db, e, seen),
        Ty::Map(k, v) => {
            value_codec_type_is_recursive(db, k, seen) || value_codec_type_is_recursive(db, v, seen)
        }
        Ty::Tuple(elems) => elems
            .iter()
            .any(|e| value_codec_type_is_recursive(db, e, seen)),
        Ty::Record(fields) => fields
            .values()
            .any(|t| value_codec_type_is_recursive(db, t, seen)),
        _ => false,
    }
}

fn emit_value_form(db: &mut Db, ty: &Ty, val_expr: &str) -> Result<String, Reject> {
    match ty.strip_nominal() {
        Ty::Bool => Ok(format!(
            "__b.atom_leaf(cadenza_ast::ast::Leaf::Bool({val_expr}))"
        )),
        Ty::Int(_) => Ok(format!(
            "__b.atom_leaf(cadenza_ast::ast::Leaf::Int {{ value: cadenza_ast::ast::IntValue::from_i64(({val_expr}) as i64), radix: cadenza_ast::ast::Radix::Dec }})"
        )),
        // A CHAR is a native `char`; String/Symbol are `String`; Bytes is `Vec<u8>` — each a single leaf
        // (std-only, no external type beyond `Arc`). `Arc::from(&str)`/`Arc::from(&[u8])` build the leaf's
        // `Arc<str>`/`Arc<[u8]>` from a borrow of the owned value.
        Ty::Char => Ok(format!(
            "__b.atom_leaf(cadenza_ast::ast::Leaf::Char({val_expr}))"
        )),
        Ty::String | Ty::Symbol => Ok(format!(
            "__b.atom_leaf(cadenza_ast::ast::Leaf::Str(std::sync::Arc::from(({val_expr}).as_str())))"
        )),
        Ty::Bytes => Ok(format!(
            "__b.atom_leaf(cadenza_ast::ast::Leaf::Bytes(std::sync::Arc::from(({val_expr}).as_slice())))"
        )),
        // A FLOAT is an `f64`/`f32` → a `Leaf::Float(Decimal)` built by the EXACT-shortest-decimal
        // `Decimal::from_f64`/`from_f32` (cadenza-ast, mirroring the runtime `float_leaf`) — NOT lossy bits.
        // Float32 uses `from_f32` (a promoted f32's shortest decimal differs from the f64's). A non-finite
        // value (NaN/inf) has no canonical form → `from_*` returns None → PANIC, matching the runtime
        // value-encode trap (and the `Ast.Float` NaN guard). A bare float declines at lower; this arm is the
        // recursion base for a float NESTED in a compound (`(tuple 1.5 2.5)`).
        Ty::Float(ft) => {
            let ctor = if ft.ground_width() == 32 {
                format!("cadenza_ast::ast::Decimal::from_f32({val_expr})")
            } else {
                format!("cadenza_ast::ast::Decimal::from_f64({val_expr})")
            };
            Ok(format!(
                "__b.atom_leaf(cadenza_ast::ast::Leaf::Float(match {ctor} {{ Some(__d) => __d, None => panic!(\"a non-canonical float (NaN/inf) has no canonical value form\") }}))"
            ))
        }
        // A BIGINT is a `cdz_num::Big` → a KIND_INT `Leaf::Int` (byte-identical to a fixed Int's int-body:
        // the runtime BigInt leaf IS a KIND_INT leaf), framed `(: <int> BigInt)`. `cadenza_ast::Leaf::Int`
        // now holds an `IntValue` (the dependency-light rep, #3926); build it from the runtime bignum's exact
        // decimal string via `num_bigint` (a linked extern) → `IntValue::from_bigint` — `parse_bytes(.., 10)`
        // never fails on `to_decimal_string`'s output.
        Ty::BigInt => Ok(format!(
            "__b.atom_leaf(cadenza_ast::ast::Leaf::Int {{ value: cadenza_ast::ast::IntValue::from_bigint(&num_bigint::BigInt::parse_bytes(({val_expr}).to_decimal_string().as_bytes(), 10).unwrap()), radix: cadenza_ast::ast::Radix::Dec }})"
        )),
        // A RATIONAL is a `cdz_num::Rational` → a single NAME leaf whose text is `num/den` (normalized:
        // lowest terms, sign on num, den>0), framed `(: <num>/<den> Rational)` — NOT a record and NOT the
        // 2-handle heap node (v-runtime's runtime value-encode renders the folded `num/den` text directly).
        // `Rational::to_display_string` produces exactly that `num/den` string.
        Ty::Rational => Ok(format!("__b.name(&({val_expr}).to_display_string())")),
        Ty::Tuple(elems) => {
            let mut body = String::from("{ let __h = __b.name(\"tuple\");");
            let mut vars = vec!["__h".to_string()];
            for (i, et) in elems.iter().enumerate() {
                let child = emit_value_form(db, et, &format!("({val_expr}).{i}"))?;
                body.push_str(&format!(" let __e{i} = {child};"));
                vars.push(format!("__e{i}"));
            }
            body.push_str(&format!(" __b.list(vec![{}]) }}", vars.join(", ")));
            Ok(body)
        }
        // A LIST is a `Vec<T>` — a RUNTIME-length `(list e0 e1 …)`: loop the vec building each element node
        // (cloning the element so a non-Copy element is not moved out of the borrow), push onto the children.
        Ty::List(elem) => {
            let child = emit_value_form(db, elem, "__ev")?;
            Ok(format!(
                "{{ let __lv = {val_expr}; {} __b.list(__lc) }}",
                emit_list_children(&child, "__lv")
            ))
        }
        // A MAP is a `BTreeMap<K, V>` rendered `(map (k1 v1) … (kn vn))` — head `map`, each entry a `(key
        // value)` 2-list, in canonical KEY order (the `BTreeMap` iterates sorted = canonical, no sort). Only a
        // SCALAR KEY is orderable (decline otherwise, matching the runtime); the VALUE is any encodable shape.
        Ty::Map(k, v) => {
            if !is_orderable_scalar_key(k) {
                return Err(Reject::decline(
                    "Value.encode native rust: Map key is not an orderable scalar (Int/Bool/Char/String/Symbol)",
                ));
            }
            let kform = emit_value_form(db, k, "__mk")?;
            let vform = emit_value_form(db, v, "__mvv")?;
            Ok(format!(
                "{{ let __mv = {val_expr}; let __mh = __b.name(\"map\"); let mut __mc = vec![__mh]; \
                 for (__k, __v) in __mv.iter() {{ let __mk = __k.clone(); let __mvv = __v.clone(); \
                 let __ke = {kform}; let __ve = {vform}; let __pair = __b.list(vec![__ke, __ve]); __mc.push(__pair); }} \
                 __b.list(__mc) }}"
            ))
        }
        // A SET is a `BTreeSet<T>` rendered `((. Set of) (list e1 … en))` — a 2-child list: the member-access
        // head `(. Set of)` then a `(list …)` of the elements, in canonical order (the `BTreeSet` iterates
        // sorted = canonical, no explicit sort). Only a SCALAR element is orderable/encodable (the runtime
        // declines a non-scalar element), so decline it too.
        Ty::Set(elem) => {
            if !is_orderable_scalar_key(elem) {
                return Err(Reject::decline(
                    "Value.encode native rust: Set element is not an orderable scalar (Int/Bool/Char/String/Symbol)",
                ));
            }
            let child = emit_value_form(db, elem, "__ev")?;
            Ok(format!(
                "{{ let __sv = {val_expr}; \
                 let __sof = {{ let __d = __b.name(\".\"); let __sm = __b.name(\"Set\"); let __of = __b.name(\"of\"); __b.list(vec![__d, __sm, __of]) }}; \
                 {} \
                 let __lst = __b.list(__lc); __b.list(vec![__sof, __lst]) }}",
                emit_list_children(&child, "__sv")
            ))
        }
        // A RECORD is a Rust TUPLE (`tuple_type(fields.values())`, so tuple position i == the i-th field in
        // the `BTreeMap<Symbol,_>`'s sorted-key order) rendered as `(record (= k0 v0) (= k1 v1) …)`. Each
        // field is a nested `(= <name> <value>)` list; the field NAME is the Symbol's `name` (db-free). Bind
        // the record once, build each field node in a block (so `=`/key/value temps don't collide), positional
        // `.i` reads the field value. Field order matches the runtime (both iterate the sorted-key type map).
        Ty::Record(fields) => {
            let mut s = format!("{{ let __rv = {val_expr}; let __rh = __b.name(\"record\");");
            let mut vars = vec!["__rh".to_string()];
            for (i, (sym, fty)) in fields.iter().enumerate() {
                let fname = &*sym.name;
                let fval = emit_value_form(db, fty, &format!("(__rv).{i}"))?;
                s.push_str(&format!(
                    " let __f{i} = {{ let __eq = __b.name(\"=\"); let __k = __b.name(\"{fname}\"); let __fv = {fval}; __b.list(vec![__eq, __k, __fv]) }};"
                ));
                vars.push(format!("__f{i}"));
            }
            s.push_str(&format!(" __b.list(vec![{}]) }}", vars.join(", ")));
            Ok(s)
        }
        // A SUM value renders `(Head payload…)` per variant (the runtime `variant_form_template` /
        // `Shape::Sum`): nullary → `(Head unit)`, single-payload → `(Head <payload-form>)`. `match` the
        // native enum on each variant (path from `sum_variant_path_of_ty`), building the head + payload node.
        // INCREMENTAL: arity 0/1 non-recursive with a BARE head (Option/Sign-shape). DECLINE the not-yet-wired
        // arms — multi-payload (a `Spread`, flattened not tuple-wrapped), a recursive/boxed variant, and a
        // prelude-shadowed variant whose head is qualified `(. Type Variant)` — so the frame stays exact.
        sum_ty @ Ty::Sum { decl, .. } => {
            let sum_ty = sum_ty.clone();
            let decl_occ = *decl;
            let variants: Vec<(u32, String, usize)> = {
                let td = db.type_decl_by_occ(decl_occ).ok_or_else(|| {
                    Reject::decline("Value.encode native rust: sum decl not found")
                })?;
                td.variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i as u32, v.name.clone(), v.payloads.len()))
                    .collect()
            };
            // A qualified head (a `.` member) means a prelude-shadowed variant — the runtime writes
            // `(. Type Variant)`, which this slice does not yet build. Decline rather than emit a wrong head.
            if variants.iter().any(|(_, h, _)| h.contains('.')) {
                return Err(Reject::unsupported(
                    "Value.encode native rust does not support a prelude-shadowed sum variant (qualified head)",
                ));
            }
            let mut arms = String::new();
            for (disc, head, arity) in variants {
                if super::enums::variant_is_recursive(db, &sum_ty, disc) {
                    return Err(Reject::unsupported(
                        "Value.encode native rust does not support a recursive (boxed) sum variant",
                    ));
                }
                let path = sum_variant_path_of_ty(db, &sum_ty, disc)?;
                match arity {
                    0 => arms.push_str(&format!(
                        "{path} => {{ let __sh = __b.name(\"{head}\"); let __su = __b.name(\"unit\"); __b.list(vec![__sh, __su]) }}, "
                    )),
                    1 => {
                        let pty = variant_payload_ty(db, &sum_ty, disc).ok_or_else(|| {
                            Reject::decline("Value.encode native rust: sum variant payload type unresolved")
                        })?;
                        let pform = emit_value_form(db, &pty, "__sp")?;
                        arms.push_str(&format!(
                            "{path}(__sp) => {{ let __sh = __b.name(\"{head}\"); let __spv = {pform}; __b.list(vec![__sh, __spv]) }}, "
                        ));
                    }
                    // MULTI-payload: the core models the payload as ONE `Ty::Tuple`, boxed as a single field
                    // `Enum::V((T0, T1, …))`, but the canonical value form is FLAT — `(Head p0 p1 …)`, the
                    // elements SPLICED directly under the head (a `Shape::Spread`, NOT a `(tuple …)` wrapper).
                    // Bind the tuple `__sp` and push each `__sp.i` element node as a child of the head.
                    _ => {
                        let pty = variant_payload_ty(db, &sum_ty, disc).ok_or_else(|| {
                            Reject::decline("Value.encode native rust: sum variant payload type unresolved")
                        })?;
                        let Ty::Tuple(elems) = pty.strip_nominal() else {
                            return Err(Reject::decline(
                                "Value.encode native rust: multi-payload sum variant payload is not a tuple",
                            ));
                        };
                        let mut arm =
                            format!("{path}(__sp) => {{ let __sh = __b.name(\"{head}\"); let mut __sc = vec![__sh];");
                        for (i, et) in elems.iter().enumerate() {
                            let eform = emit_value_form(db, et, &format!("(__sp).{i}"))?;
                            arm.push_str(&format!(" let __se{i} = {eform}; __sc.push(__se{i});"));
                        }
                        arm.push_str(" __b.list(__sc) }, ");
                        arms.push_str(&arm);
                    }
                }
            }
            Ok(format!("(match {val_expr} {{ {arms} }})"))
        }
        other => Err(Reject::unsupported(format!(
            "Value.encode native rust does not support value shape {other:?} (supported: Int/Bool/Char/String/Bytes/Tuple/List/Record/Sum)"
        ))),
    }
}

/// The shared outer frame of every LEAF-SCALAR decode arm in [`emit_value_reconstruct`]: match `node` as an
/// `Atom` leaf, apply `leaf_arm` (a `Leaf::X(..) => Some(..)` mapping), and fall through to `None` on any
/// shape/leaf mismatch (a TOTAL decode). Each scalar inverse (Bool/Int/Char/String/Bytes/Float/BigInt/
/// Rational) passes ONLY its differing leaf pattern + success map, so the eight arms no longer repeat this
/// frame verbatim. BYTE-IDENTICAL to the previous inline copies (same string assembly) — the round-trip
/// corpus + golden pins hold.
fn decode_leaf(arenas: &str, node: &str, leaf_arm: &str) -> String {
    format!(
        "(match {arenas}.get({node}) {{ cadenza_ast::ast::Struct::Atom(__l) => \
         match {arenas}.leaf(*__l) {{ {leaf_arm}, _ => None }}, \
         _ => None }})"
    )
}

/// Emit a rust expression of type `Option<T>` that RECONSTRUCTS a native value of type `ty` from the
/// value-form node `node` (a `cadenza_ast::ast::StructId`) in the decoded `Arenas` `arenas` — the inverse of
/// [`emit_value_form`], the native-rust twin of `cdz-runtime`'s value-decode walk. A SCALAR reads its leaf
/// (`None` on shape/leaf mismatch → the decode is TOTAL); a TUPLE checks the `(tuple …)` list shape then
/// reconstructs each element, threaded through an IIFE closure with `?` so any element mismatch yields
/// `None`. INCREMENTAL: Int/Bool/Tuple wired (the round-trip corpus shape); other shapes decline.
fn emit_value_reconstruct(
    db: &mut Db,
    ty: &Ty,
    arenas: &str,
    node: &str,
) -> Result<String, Reject> {
    match ty.strip_nominal() {
        Ty::Bool => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Bool(__v) => Some(*__v)",
        )),
        Ty::Int(_) => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Int { value, .. } => value.to_i64()",
        )),
        // The leaf-scalar inverses of the `emit_value_form` Char/String/Bytes arms.
        Ty::Char => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Char(__v) => Some(*__v)",
        )),
        Ty::String | Ty::Symbol => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Str(__s) => Some(__s.to_string())",
        )),
        Ty::Bytes => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Bytes(__b) => Some(__b.to_vec())",
        )),
        // A FLOAT leaf inverse: reconstruct the f64/f32 EXACTLY from the `Decimal` by rebuilding its
        // `<sig>e<exp>` scientific text (sign + significand + base-10 exponent) and re-parsing. `Decimal`'s
        // significand is now a big-endian byte magnitude (`Vec<u8>`, #3926 — no `Display`), so render it as a
        // decimal via `num_bigint` (a linked extern): `BigInt::from_bytes_be(Plus, &significand)` (non-negative;
        // the sign is `__d.negative`). `parse(from_f64(f)) == f` bit-exact (the shortest decimal round-trips).
        // `.ok()` → None on the (unreachable-for-a-valid-leaf) parse failure. Float32 parses as `f32`.
        Ty::Float(ft) => {
            let parse_ty = if ft.ground_width() == 32 {
                "f32"
            } else {
                "f64"
            };
            Ok(decode_leaf(
                arenas,
                node,
                &format!(
                    "cadenza_ast::ast::Leaf::Float(__d) => format!(\"{{}}{{}}e{{}}\", if __d.negative {{ \"-\" }} else {{ \"\" }}, num_bigint::BigInt::from_bytes_be(num_bigint::Sign::Plus, &__d.significand), __d.exponent).parse::<{parse_ty}>().ok()"
                ),
            ))
        }
        // A BIGINT leaf inverse: read the `Leaf::Int` value (now an `IntValue`, #3926) and rebuild
        // `cdz_num::Big` from its little-endian base-2^32 limbs. Bridge through `num_bigint` (a linked extern)
        // via `IntValue::to_bigint`, then `to_u32_digits` → the exact `mag` layout (trailing-zero-stripped;
        // sign maps to `Big.neg`). Exact, no i64 clamp.
        Ty::BigInt => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Int { value, .. } => { let (__sg, __mag) = value.to_bigint().to_u32_digits(); Some(cdz_num::Big { neg: __sg == num_bigint::Sign::Minus, mag: __mag }) }",
        )),
        // A RATIONAL leaf inverse: read the `num/den` NAME text, split on `/`, parse each half into a
        // `cdz_num::Big` (num_bigint parse → LE u32 limbs, exact) and rebuild via `Rational::new` (which
        // re-normalizes — idempotent, the encoded text is already normalized). None on a bad shape/parse.
        Ty::Rational => Ok(decode_leaf(
            arenas,
            node,
            "cadenza_ast::ast::Leaf::Name(__s) => { let __parts: std::vec::Vec<&str> = __s.splitn(2, '/').collect(); if __parts.len() != 2 { None } else { match (num_bigint::BigInt::parse_bytes(__parts[0].as_bytes(), 10), num_bigint::BigInt::parse_bytes(__parts[1].as_bytes(), 10)) { (Some(__nn), Some(__dd)) => { let (__ns, __nm) = __nn.to_u32_digits(); let (__ds, __dm) = __dd.to_u32_digits(); Some(cdz_num::Rational::new(cdz_num::Big { neg: __ns == num_bigint::Sign::Minus, mag: __nm }, cdz_num::Big { neg: __ds == num_bigint::Sign::Minus, mag: __dm })) }, _ => None } } }",
        )),
        Ty::Tuple(elems) => {
            let n = elems.len();
            let mut body = format!(
                "(|| {{ let __items = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get({node}) {{ __i }} else {{ return None }}; \
                 if __items.len() != {} || {arenas}.head_name({node}) != Some(\"tuple\") {{ return None }}; ",
                n + 1
            );
            let mut results = Vec::with_capacity(n);
            for (i, et) in elems.iter().enumerate() {
                let child = emit_value_reconstruct(db, et, arenas, &format!("__items[{}]", i + 1))?;
                body.push_str(&format!("let __e{i} = {child}?; "));
                results.push(format!("__e{i}"));
            }
            // A 1-tuple needs the trailing comma; join covers the rest.
            let tail = if n == 1 { "," } else { "" };
            body.push_str(&format!("Some(({}{tail})) }})()", results.join(", ")));
            Ok(body)
        }
        // A LIST: check the `(list …)` shape, reconstruct each element into a `Vec` (`?` on any element
        // mismatch → the whole decode is None). The inverse of the List encode loop.
        Ty::List(elem) => {
            let child = emit_value_reconstruct(db, elem, arenas, "__items[__ix]")?;
            Ok(format!(
                "(|| {{ let __items = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get({node}) {{ __i }} else {{ return None }}; \
                 if __items.is_empty() || {arenas}.head_name({node}) != Some(\"list\") {{ return None }}; \
                 let mut __out = Vec::new(); \
                 for __ix in 1..__items.len() {{ __out.push({child}?); }} \
                 Some(__out) }})()"
            ))
        }
        // A MAP: check the `(map …)` shape, reconstruct each `(key value)` entry into a `BTreeMap`. Inverse
        // of the Map encode; `?` on any key/value/shape mismatch → None.
        Ty::Map(k, v) => {
            if !is_orderable_scalar_key(k) {
                return Err(Reject::decline(
                    "Value.decode native rust: Map key is not an orderable scalar (Int/Bool/Char/String/Symbol)",
                ));
            }
            let kc = emit_value_reconstruct(db, k, arenas, "__pair[0]")?;
            let vc = emit_value_reconstruct(db, v, arenas, "__pair[1]")?;
            Ok(format!(
                "(|| {{ let __mitems = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get({node}) {{ __i }} else {{ return None }}; \
                 if {arenas}.head_name({node}) != Some(\"map\") {{ return None }} \
                 let mut __out = std::collections::BTreeMap::new(); \
                 for __ix in 1..__mitems.len() {{ \
                   let __pair = if let cadenza_ast::ast::Struct::List(__p) = {arenas}.get(__mitems[__ix]) {{ __p }} else {{ return None }}; \
                   if __pair.len() != 2 {{ return None }} \
                   let __kk = {kc}?; let __vv = {vc}?; __out.insert(__kk, __vv); }} \
                 Some(__out) }})()"
            ))
        }
        // A SET: check the `((. Set of) (list …))` 2-child shape, reconstruct each element of the inner
        // `(list …)` and insert into a `BTreeSet` (`?` on any mismatch → None). Inverse of the Set encode.
        Ty::Set(elem) => {
            if !is_orderable_scalar_key(elem) {
                return Err(Reject::decline(
                    "Value.decode native rust: Set element is not an orderable scalar (Int/Bool/Char/String/Symbol)",
                ));
            }
            let child = emit_value_reconstruct(db, elem, arenas, "__litems[__ix]")?;
            Ok(format!(
                "(|| {{ let __souter = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get({node}) {{ __i }} else {{ return None }}; \
                 if __souter.len() != 2 {{ return None }} \
                 let __listnode = __souter[1]; \
                 let __litems = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get(__listnode) {{ __i }} else {{ return None }}; \
                 if {arenas}.head_name(__listnode) != Some(\"list\") {{ return None }} \
                 let mut __out = std::collections::BTreeSet::new(); \
                 for __ix in 1..__litems.len() {{ __out.insert({child}?); }} \
                 Some(__out) }})()"
            ))
        }
        // A RECORD: check the `(record (= k v) …)` shape, then for each field read the `(= k v)` triple's
        // VALUE (its 3rd child) and reconstruct positionally into the tuple (matching the encode's sorted-key
        // field order). Positional (not by-key) is sound because encode + decode iterate the SAME sorted-key
        // type map. `?` on any field mismatch → None.
        Ty::Record(fields) => {
            let n = fields.len();
            let mut body = format!(
                "(|| {{ let __items = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get({node}) {{ __i }} else {{ return None }}; \
                 if __items.len() != {} || {arenas}.head_name({node}) != Some(\"record\") {{ return None }}; ",
                n + 1
            );
            let mut results = Vec::with_capacity(n);
            for (i, (_sym, fty)) in fields.iter().enumerate() {
                let fval = emit_value_reconstruct(db, fty, arenas, &format!("__g{i}[2]"))?;
                body.push_str(&format!(
                    "let __g{i} = if let cadenza_ast::ast::Struct::List(__gg) = {arenas}.get(__items[{}]) {{ __gg }} else {{ return None }}; \
                     if __g{i}.len() != 3 {{ return None }}; let __r{i} = {fval}?; ",
                    i + 1
                ));
                results.push(format!("__r{i}"));
            }
            let tail = if n == 1 { "," } else { "" };
            body.push_str(&format!("Some(({}{tail})) }})()", results.join(", ")));
            Ok(body)
        }
        // A SUM: the node is `(Head payload…)`. Dispatch on `head_name`: a matching nullary variant checks
        // the `(Head unit)` 2-child shape and constructs the bare/turbofish variant path; a single-payload
        // variant reconstructs `__sitems[1]` and constructs `Enum::V(payload)`. Unknown head / wrong arity →
        // None. The inverse of the encode Sum arm; same INCREMENTAL scope (arity 0/1, non-recursive, bare head).
        sum_ty @ Ty::Sum { decl, .. } => {
            let sum_ty = sum_ty.clone();
            let decl_occ = *decl;
            let variants: Vec<(u32, String, usize)> = {
                let td = db.type_decl_by_occ(decl_occ).ok_or_else(|| {
                    Reject::decline("Value.decode native rust: sum decl not found")
                })?;
                td.variants
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (i as u32, v.name.clone(), v.payloads.len()))
                    .collect()
            };
            if variants.iter().any(|(_, h, _)| h.contains('.')) {
                return Err(Reject::unsupported(
                    "Value.decode native rust does not support a prelude-shadowed sum variant (qualified head)",
                ));
            }
            let mut body = format!(
                "(|| {{ let __sitems = if let cadenza_ast::ast::Struct::List(__i) = {arenas}.get({node}) {{ __i }} else {{ return None }}; \
                 if __sitems.is_empty() {{ return None }}; let __shn = {arenas}.head_name({node}); "
            );
            for (disc, head, arity) in variants {
                if super::enums::variant_is_recursive(db, &sum_ty, disc) {
                    return Err(Reject::unsupported(
                        "Value.decode native rust does not support a recursive (boxed) sum variant",
                    ));
                }
                let path = sum_variant_path_of_ty(db, &sum_ty, disc)?;
                match arity {
                    0 => {
                        let ctor = nullary_variant_path(&db.name_ctx(), &sum_ty, disc, &path);
                        body.push_str(&format!(
                            "if __shn == Some(\"{head}\") {{ if __sitems.len() != 2 {{ return None }} return Some({ctor}); }} "
                        ));
                    }
                    1 => {
                        let pty = variant_payload_ty(db, &sum_ty, disc).ok_or_else(|| {
                            Reject::decline(
                                "Value.decode native rust: sum variant payload type unresolved",
                            )
                        })?;
                        let precon = emit_value_reconstruct(db, &pty, arenas, "__sitems[1]")?;
                        body.push_str(&format!(
                            "if __shn == Some(\"{head}\") {{ if __sitems.len() != 2 {{ return None }} let __sp = ({precon})?; return Some({path}(__sp)); }} "
                        ));
                    }
                    // MULTI-payload: `(Head p0 p1 …)` — arity+1 children (head + one per element).
                    // Reconstruct each `__sitems[1+i]` into the tuple, then construct `Enum::V((p0, p1, …))`
                    // (the core's single-tuple-payload model). The inverse of the encode Spread arm.
                    _ => {
                        let pty = variant_payload_ty(db, &sum_ty, disc).ok_or_else(|| {
                            Reject::decline(
                                "Value.decode native rust: sum variant payload type unresolved",
                            )
                        })?;
                        let Ty::Tuple(elems) = pty.strip_nominal() else {
                            return Err(Reject::decline(
                                "Value.decode native rust: multi-payload sum variant payload is not a tuple",
                            ));
                        };
                        let n = elems.len();
                        let mut arm = format!(
                            "if __shn == Some(\"{head}\") {{ if __sitems.len() != {} {{ return None }} ",
                            n + 1
                        );
                        let mut results = Vec::with_capacity(n);
                        for (i, et) in elems.iter().enumerate() {
                            let precon = emit_value_reconstruct(
                                db,
                                et,
                                arenas,
                                &format!("__sitems[{}]", i + 1),
                            )?;
                            arm.push_str(&format!("let __se{i} = ({precon})?; "));
                            results.push(format!("__se{i}"));
                        }
                        arm.push_str(&format!(
                            "return Some({path}(({}))); }} ",
                            results.join(", ")
                        ));
                        body.push_str(&arm);
                    }
                }
            }
            body.push_str("None })()");
            Ok(body)
        }
        other => Err(Reject::unsupported(format!(
            "Value.decode native rust does not support value shape {other:?} (supported: Int/Bool/Char/String/Bytes/Tuple/List/Record/Sum)"
        ))),
    }
}

/// Emit a rust expression building the TYPE-NODE `cadenza_ast::ast::StructId` in `__b` for `ty`, mirroring
/// the compiler's `type_ast`/`Ty::render_name`. This is the `<type-node>` half of the `(: <value>
/// <type-node>)` value-form FRAME that the runtime `value-encode` op wraps EVERY escaping compound in
/// (`sum_shape_descriptor` → `Framed(type_node, inner)` for Tuple/Record/List/Set/Map/generic-sum, or
/// `Named(name, inner)` for a monomorphic sum — both render `(: value X)`, differing only in whether `X`
/// is a structured node or a bare name). Without this frame the native rust `Value.encode` produced a
/// BARE `(tuple …)` document that DIVERGED from the wasm face (measured: 35 vs 70 bytes for a
/// `(tuple 5 105)`), so `Value.encode` was not a stable cross-backend content-address — a real bug the
/// self-consistent round-trip corpus masked. A SCALAR's node is its bare render-name atom
/// (`Int64`/`String`/`Bool`/`Char`/`Bytes`/…); a `Tuple`/`List` is the structured `(Tuple …)`/`(List …)`
/// application; a `Record` is the LOWERCASE `(record (name T) …)` (mirroring the descriptor `type_node_of`,
/// not `type_ast` — see the Record arm). Covers exactly the shapes [`emit_value_form`] wires; others
/// decline in lockstep.
fn emit_type_node(ty: &Ty, ncx: &crate::ty::NameCtx) -> Result<String, Reject> {
    match ty.strip_nominal() {
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Bytes
        | Ty::Float(_) => {
            let name = ty.render_name(ncx);
            Ok(format!("__b.name(\"{name}\")"))
        }
        Ty::Tuple(elems) => {
            let mut s =
                String::from("{ let __th = __b.name(\"Tuple\"); let mut __tc = vec![__th];");
            for (i, et) in elems.iter().enumerate() {
                let child = emit_type_node(et, ncx)?;
                s.push_str(&format!(" let __tt{i} = {child}; __tc.push(__tt{i});"));
            }
            s.push_str(" __b.list(__tc) }");
            Ok(s)
        }
        Ty::List(elem) => {
            let child = emit_type_node(elem, ncx)?;
            Ok(format!(
                "{{ let __th = __b.name(\"List\"); let __te = {child}; __b.list(vec![__th, __te]) }}"
            ))
        }
        // Set/Map type nodes: `(Set e)` / `(Map k v)` — mirror `type_node_of` (matches `render_name`).
        Ty::Set(elem) => {
            let child = emit_type_node(elem, ncx)?;
            Ok(format!(
                "{{ let __th = __b.name(\"Set\"); let __te = {child}; __b.list(vec![__th, __te]) }}"
            ))
        }
        Ty::Map(k, v) => {
            let kc = emit_type_node(k, ncx)?;
            let vc = emit_type_node(v, ncx)?;
            Ok(format!(
                "{{ let __th = __b.name(\"Map\"); let __tk = {kc}; let __tv = {vc}; __b.list(vec![__th, __tk, __tv]) }}"
            ))
        }
        // A RECORD's type node mirrors the DESCRIPTOR path `type_node_of` (what `Value.encode` uses), NOT
        // `type_ast`: LOWERCASE `record` head (the SAME atom as the value form's `(record …)` head, so the
        // codec interns it once), and each field is a bare `(name <type>)` node — head = field name, one
        // child = the field's type node — NOT a `(: name T)` colon ascription. (The two differed: the
        // runtime bakes `type_node_of` for the escaping-value descriptor, `type_ast` is the fixed-shape
        // static-template renderer; using the latter emitted a capital `Record` + colon fields that DIVERGED
        // from the wasm face — 9 vs 8 leaves — a real cross-backend bug v-runtime's per-side pin caught.)
        Ty::Record(fields) => {
            let mut s =
                String::from("{ let __th = __b.name(\"record\"); let mut __tc = vec![__th];");
            for (i, (sym, fty)) in fields.iter().enumerate() {
                let fname = &*sym.name;
                let child = emit_type_node(fty, ncx)?;
                s.push_str(&format!(
                    " let __tf{i} = {{ let __tk = __b.name(\"{fname}\"); let __tv = {child}; __b.list(vec![__tk, __tv]) }}; __tc.push(__tf{i});"
                ));
            }
            s.push_str(" __b.list(__tc) }");
            Ok(s)
        }
        // A SUM's type node: the bare NAME for a monomorphic sum (`Sign`), or the parametric `(Name arg…)`
        // for a generic instantiation (`(Option Int64)`) — matches `type_ast`/`Ty::render_name`. `name_of`
        // resolves the declared name from the render-context.
        Ty::Sum { decl, args, .. } => {
            let name = ncx
                .name_of(*decl)
                .ok_or_else(|| {
                    Reject::decline("Value.encode native rust: sum type name unresolved")
                })?
                .to_string();
            if args.is_empty() {
                Ok(format!("__b.name(\"{name}\")"))
            } else {
                let mut s =
                    format!("{{ let __th = __b.name(\"{name}\"); let mut __tc = vec![__th];");
                for (i, a) in args.iter().enumerate() {
                    let child = emit_type_node(a, ncx)?;
                    s.push_str(&format!(" let __ta{i} = {child}; __tc.push(__ta{i});"));
                }
                s.push_str(" __b.list(__tc) }");
                Ok(s)
            }
        }
        other => Err(Reject::unsupported(format!(
            "Value.encode native rust does not support a type-node for value shape {other:?} (supported: Int/Bool/Char/String/Bytes/Tuple/List/Record/Sum)"
        ))),
    }
}

fn wrap_closure_value(cap_lets: &str, closure_expr: &str) -> String {
    if cap_lets.is_empty() {
        closure_expr.to_string()
    } else {
        format!("{{ {cap_lets}{closure_expr} }}")
    }
}

/// Emit the per-closure `EnvClosure` STRUCT + impl for lifted slot `k`, async mode (Option A). The struct
/// holds the captures as fields `__c{j}: <capture-ty>` (async spelling); its `call` clones each capture,
/// destructures the single `A` arg back into the lifted lambda's flat params, then forwards the env plus the
/// captures plus the params into the async `__lifted_{k}`, boxing the returned future. This is the
/// object-safe closure VALUE a `Core::Closure` builds (`Rc::new(__Clos_{k} { … }) as Rc<dyn
/// EnvClosure<A,R>>`). Declines if a capture, param, or result type has no async representation (the lifted
/// fn itself would already have declined).
pub(super) fn emit_closure_struct(
    db: &mut Db,
    k: usize,
    layout: &Layout,
) -> Result<String, Reject> {
    let lam = layout.lifted[k].clone();
    let struct_ident = closure_struct_ident(k);
    let lifted = lifted_ident(k);
    // Capture FIELDS: `__c{j}: <async ty of the captured binding>` (same type the lifted fn's leading
    // capture params take, so the forward type-checks). A capture with no async rep declines.
    let mut fields = Vec::with_capacity(lam.captures.len());
    let mut field_clones = Vec::with_capacity(lam.captures.len());
    for (j, &cap_binder) in lam.captures.iter().enumerate() {
        let cty = type_of(db, cap_binder);
        let rty = types::async_closure_type(&db.name_ctx(), &cty).ok_or_else(|| {
            Reject::decline(format!(
                "async closure capture {j} type {} has no native Rust representation",
                cty.render_name(&db.name_ctx())
            ))
        })?;
        fields.push(format!("    __c{j}: {rty},"));
        // Clone each capture into the forwarded call — `call` takes `&self`, so it may not MOVE a field out
        // (it is callable repeatedly). A Copy field's `.clone()` is a plain copy.
        field_clones.push(format!("self.__c{j}.clone()"));
    }
    // The single `A` arg destructured back into the flat lifted params. Arity 0 → `()` (ignore); arity 1 →
    // the bare `__a0`; arity ≥2 → a tuple pattern `(__a0, __a1, …)`. `EnvClosure::call`'s `arg: A` binds it.
    let arity = lam.params.len();
    let arg_pat = match arity {
        0 => "_".to_string(),
        1 => "__a0".to_string(),
        _ => format!(
            "({})",
            (0..arity)
                .map(|i| format!("__a{i}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    // The forwarded arguments into `__lifted_k(env, caps…, params…)`: env first, then cloned captures, then
    // the destructured params in order.
    let mut fwd = vec![super::ENV_PARAM.to_string()];
    fwd.extend(field_clones);
    fwd.extend((0..arity).map(|i| format!("__a{i}")));
    // `A`/`R` for the `impl EnvClosure<A,R>` header — the SAME spelling the value cast + type positions use.
    let (a_ty, r_ty) = types::env_closure_args(&db.name_ctx(), &lam.params, &lam.ret_ty)
        .ok_or_else(|| {
            Reject::decline("an async closure's arg/result type has no native Rust representation")
        })?;
    // `#[derive(Clone)]`: a closure value is `Rc<dyn EnvClosure>` (shared via Rc clone), but a captured
    // closure-in-closure or a clone-on-read of the struct itself needs Clone; deriving it is harmless (all
    // fields are Clone — they are captured values, each `needs_clone_on_read`-safe).
    let fields_src = if fields.is_empty() {
        String::new()
    } else {
        format!("\n{}\n", fields.join("\n"))
    };
    // `Box` is fully qualified as `::std::boxed::Box` throughout: a USER sum named `Box` (`(type Box (Bin
    // …))`) emits `enum Box`, which would shadow the std `Box` in `Box::pin`/`Pin<Box<…>>` → E0107 "enum
    // Box takes 0 generic arguments". The fully-qualified path can never be shadowed. (Mirrors the async
    // `Core::Call` arm's `::std::boxed::Box::pin`.)
    Ok(format!(
        "// cdz-closure-struct[{k}]\n#[derive(Clone)]\nstruct {struct_ident} {{{fields_src}}}\nimpl cdz_rt::EnvClosure<{a_ty}, {r_ty}> for {struct_ident} {{\n    fn call<'a>(&self, {env}: &'a mut dyn cdz_rt::DynCdzEnv, arg: {a_ty}) -> std::pin::Pin<::std::boxed::Box<dyn core::future::Future<Output = {r_ty}> + 'a>> {{\n        let {arg_pat} = arg;\n        ::std::boxed::Box::pin({lifted}({}))\n    }}\n}}\n",
        fwd.join(", "),
        env = super::ENV_PARAM,
    ))
}

/// Emit lambda-lifted closure slot `k` as a private `fn __lifted_{k}(<captures…>, <params…>) -> <ret>`.
///
/// The lifted lambda (`layout.lifted[k]` — a [`crate::lower::LiftedLambda`]) is the body of a `(fn …)`
/// that could not be β-reduced (it is passed to a recursive function). On the wasm backend it becomes a
/// standalone function taking the closure CELL as slot 0 (the env) and reading captures back out of it;
/// on the RUST backend, captures are passed as ORDINARY LEADING PARAMETERS (the closure value forwards
/// the captured values it holds), so `Core::Captured { index }` reads the `index`-th capture PARAM
/// directly — no env-cell indirection. The captures come FIRST (in `captures` order = the wasm cell index
/// `1 + position`), then the lambda's own `params`. The body is rendered against an env mapping each
/// capture binder and param binder to its emitted identifier. A capture/param/result type with no native
/// Rust mapping declines the whole lifted fn (hence the module) — the same boundary every `fn` draws.
pub(super) fn emit_lifted_lambda(
    db: &mut Db,
    k: usize,
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    // Clone the lifted lambda's shape out of the layout so `db` can be borrowed mutably while emitting.
    let lam = layout.lifted[k].clone();
    // OPTION A — UNIFORM async closure ABI. In async mode EVERY lifted closure is emitted as an `async fn`
    // taking the gas/yield env as `&mut dyn DynCdzEnv` (the object-safe facet — NOT the generic `&mut __CdzE`
    // a TOP-LEVEL async fn takes, because the closure VALUE that wraps this fn is `Rc<dyn EnvClosure<A,R>>`
    // and a trait object cannot be generic over `__CdzE`). The body emits in Async mode so its `Core::Call`s
    // thread env/await; entry gas is charged via the object-safe `consume_boxed`. This is UNIFORM (a
    // call-free async body becomes an async fn too — env unused but present) so the closure VALUE form is one
    // ABI everywhere, letting a `Ty::Fn` TYPE position spell it (`async_closure_type`) without needing to
    // observe `body_has_call` (a per-value property a type can't see). In SYNC mode a lifted lambda stays an
    // ordinary sync `fn` — its inner emit must not thread env.
    let closure_async = mode.is_async();
    let mode = if closure_async {
        Mode::Async
    } else {
        Mode::Sync
    };
    let mut params_src = String::new();
    let mut env: Env = HashMap::new();
    let mut first = true;
    // A UNIFORM async lifted fn threads the env as its FIRST parameter, as the object-safe `&mut dyn
    // DynCdzEnv` (see above). The `EnvClosure::call` wrapper forwards its own `env` into this fn.
    if closure_async {
        params_src.push_str(&format!("{}: &mut dyn DynCdzEnv", super::ENV_PARAM));
        first = false;
    }
    // Captures FIRST — each an ordinary leading parameter `__cap{j}: <ty>`. `Core::Captured{index:j}`
    // reads it. The capture's TYPE is the solved type of the captured binding (read off its occurrence).
    for (j, &cap_binder) in lam.captures.iter().enumerate() {
        let cty = type_of(db, cap_binder);
        // Async: a captured CLOSURE spells the `EnvClosure` ABI (a captured closure value is `Rc<dyn
        // EnvClosure>`); `async_closure_type` == `rust_type` for a closure-free capture.
        let rty = super::async_or_rust_type(&db.name_ctx(), &cty, mode).ok_or_else(|| {
            Reject::decline(format!(
                "lifted lambda capture {j} type {} has no native Rust representation",
                cty.render_name(&db.name_ctx())
            ))
        })?;
        let cname = format!("__cap{j}");
        if !first {
            params_src.push_str(", ");
        }
        params_src.push_str(&format!("{cname}: {rty}"));
        env.insert(cap_binder, cname);
        first = false;
    }
    // Then the lambda's own PARAMETERS, in order.
    for (i, (binder, ty)) in lam.params.iter().enumerate() {
        // Async: a closure-typed param (a higher-order lifted lambda) spells the `EnvClosure` ABI.
        let rty = super::async_or_rust_type(&db.name_ctx(), ty, mode).ok_or_else(|| {
            Reject::decline(format!(
                "lifted lambda parameter {i} type {} has no native Rust representation",
                ty.render_name(&db.name_ctx())
            ))
        })?;
        let pname = super::param_name(db, *binder, i);
        if !first {
            params_src.push_str(", ");
        }
        params_src.push_str(&format!("{pname}: {rty}"));
        env.insert(*binder, pname);
        first = false;
    }
    // Async: a closure-typed RESULT (a lifted lambda returning a closure) spells the `EnvClosure` ABI.
    let ret = super::async_or_rust_type(&db.name_ctx(), &lam.ret_ty, mode).ok_or_else(|| {
        Reject::decline(format!(
            "lifted lambda result type {} has no native Rust representation",
            lam.ret_ty.render_name(&db.name_ctx())
        ))
    })?;
    let ctx = Ctx {
        mode,
        layout,
        loop_group: None,
        sum_binds: Vec::new(),
        sum_path_types: Vec::new(),
        map_typed_by_enclosing_insert: false,
        set_typed_by_enclosing_insert: false,
        scrut_locals: Vec::new(),
        expected_ty: None,
    };
    let body = emit(db, lam.body, &env, &ctx)?;
    let ident = lifted_ident(k);
    if closure_async {
        // A UNIFORM async lifted closure body → `async fn` charging entry gas via the OBJECT-SAFE
        // `consume_boxed` (the `&mut dyn DynCdzEnv` env can't call the RPITIT `consume`). Mirrors a
        // top-level async fn's `env.consume(1).await`. The per-closure `EnvClosure` struct (emitted in
        // `mod.rs`) wraps this fn into the `Rc<dyn EnvClosure<A,R>>` value.
        return Ok(format!(
            "// cdz-lifted[{k}]\nasync fn {ident}({params_src}) -> {ret} {{\n    {}.consume_boxed(1).await;\n    {body}\n}}\n",
            super::ENV_PARAM
        ));
    }
    Ok(format!(
        "// cdz-lifted[{k}]\nfn {ident}({params_src}) -> {ret} {{\n    {body}\n}}\n"
    ))
}

/// Emit a tail-recursion group's shared `loop` for the member being defined (`self_def`, which is
/// `members[0]`). The shared positional locals `__p0…` are initialized from this member's params; for a
/// MUTUAL group a `which` state selects the member body each iteration (this member enters at `which =
/// 0`). Each member's body renders in TAIL position with ITS param binders mapped to the shared locals.
fn emit_loop_body(
    db: &mut Db,
    params: &[(StructId, Ty)],
    self_def: usize,
    members: &[usize],
    layout: &Layout,
    mode: Mode,
) -> Result<String, Reject> {
    let shared_params: Vec<String> = (0..params.len()).map(|i| format!("__p{i}")).collect();
    let body_ty = type_of(db, db.defs[self_def].body.unwrap());
    let result_it = match &body_ty {
        Ty::Int(it) => Some(*it),
        _ => None,
    };
    // The float twin: `float_width_of_ty` strips a nominal/Qty wrapper so a `(Qty Float32 …)` result
    // grounds break leaves to f32. `None` when the result is not a float (it is then int, or a non-scalar).
    let result_ft = float_width_of_ty(&body_ty);
    let group = LoopGroup {
        members: members.to_vec(),
        shared_params: shared_params.clone(),
        result_it,
        result_ft,
    };
    let ctx = Ctx {
        mode,
        layout,
        loop_group: Some(&group),
        sum_binds: Vec::new(),
        sum_path_types: Vec::new(),
        map_typed_by_enclosing_insert: false,
        set_typed_by_enclosing_insert: false,
        scrut_locals: Vec::new(),
        expected_ty: None,
    };
    // Initialize the shared locals from THIS member's params (its param name → `__pi`), then the body.
    let mut init = String::new();
    for (i, (binder, _)) in params.iter().enumerate() {
        let pname = super::param_name(db, *binder, i);
        init.push_str(&format!("let mut __p{i} = {pname}; "));
    }
    if group.is_mutual() {
        // A mutual group dispatches on `which` (this member enters at 0 = its own body). The dispatch is
        // an if-chain over the members; each member body renders in tail position with its params mapped.
        let mut dispatch = String::new();
        for (k, &m) in members.iter().enumerate() {
            let body = db.defs[m]
                .body
                .ok_or_else(|| Reject::decline("a loop member has no body"))?;
            let env = member_env(db, m, &shared_params);
            let b = emit_tail(db, body, &env, &ctx)?;
            if k == 0 {
                dispatch.push_str(&format!("if which == 0 {{ {b} }}"));
            } else if k + 1 < members.len() {
                dispatch.push_str(&format!(" else if which == {k} {{ {b} }}"));
            } else {
                // Last member: the unconditional `else` (reached by elimination).
                dispatch.push_str(&format!(" else {{ {b} }}"));
            }
        }
        Ok(format!(
            "    let mut which: u32 = 0; {init}\n    loop {{\n        {dispatch}\n    }}"
        ))
    } else {
        // A single-member self-loop: no `which`, just this member's body in tail position.
        let env = member_env(db, self_def, &shared_params);
        let body = db.defs[self_def].body.unwrap();
        let b = emit_tail(db, body, &env, &ctx)?;
        Ok(format!("    {init}\n    loop {{\n        {b}\n    }}"))
    }
}

/// The rendering environment for loop member `m`: its own parameter binders mapped to the SHARED
/// positional locals `__p0…` (members may name their params differently but share the signature by
/// position). So a reference to member `m`'s parameter `i` reads `__pi`.
fn member_env(db: &mut Db, m: usize, shared_params: &[String]) -> Env {
    let mut env: Env = HashMap::new();
    let mparams = crate::layout::def_params(db, m);
    for (i, (binder, _)) in mparams.iter().enumerate() {
        if let Some(name) = shared_params.get(i) {
            env.insert(*binder, name.clone());
        }
    }
    env
}

/// Whether the function `self_def` is compiled as a tail `loop` (it belongs to a non-empty tail-
/// recursion group). `pub(super)` so `emit_signature` reads the SAME predicate to decide whether to
/// declare params `mut` (a looped function reassigns its params). Agrees with `emit_body` by calling the
/// same `loop_group`.
pub(super) fn body_loops(db: &mut Db, self_def: usize) -> bool {
    !loop_group(db, self_def).is_empty()
}

/// Whether a call to `callee` (reaching the async `Core::Call` emit arm) needs a `Box::pin` indirection —
/// i.e. whether `callee`'s emitted `async fn` future is SELF-REFERENTIAL (infinitely sized), which Rust
/// requires be broken by pinning at the recursive call (E0733).
///
/// The future is infinite iff `callee` can transitively reach ITSELF through AWAITED calls. The emitted
/// await-call graph is ALL `Core::Call` edges EXCEPT a def's TAIL calls to members of its OWN loop group —
/// those the backend emits as a `continue` (a loop iteration), not an awaited call, so they do NOT grow
/// the future. This is the crucial distinction the operator's directive turns on:
///   - a NON-recursive callee → no cycle → NO pin (the over-boxing to fix: every leaf/helper was pinned);
///   - a FULLY tail-recursive callee (loop-transformed, e.g. an accumulator `sum-to`) → its only self-edge
///     is a tail-`continue`, pruned → no awaited cycle → NO pin (the future is a finite `loop`);
///   - a callee with a NON-TAIL recursive call (e.g. `remove`, whose self-call is an operand of `push`,
///     inside a loop that couldn't eliminate it) → an awaited self-edge remains → a cycle → PIN (E0733
///     otherwise). This is exactly the operator's "truly recursive async where we couldn't loop it".
///
/// Worklist over the pruned graph from `callee`; a `seen` set bounds it, so a cycle terminates. Reads the
/// SAME edge relations the emitter uses (`layout::callees_of` for all edges, `tail_callees` + `loop_group`
/// for the pruned tail edges), so the pin decision matches what is actually emitted.
fn call_needs_pin(db: &mut Db, callee: usize) -> bool {
    // The AWAITED callees of `def` — every call the emitter renders as an awaited `callee(…).await`, as
    // opposed to a loop `continue`. The emitter turns a call into a `continue` ONLY when it is a TAIL call
    // to a member of `def`'s own loop group; EVERY other call (a non-tail call anywhere, or a tail call to
    // a non-member) is awaited. Crucially a callee can appear in BOTH positions — `rem`'s tail self-call
    // is a `continue` but its NON-tail self-call (an operand of `push`) is awaited — so we must classify by
    // POSITION, not by callee-set membership (a set-based prune would wrongly drop `rem` entirely). Walk
    // the body collecting all calls, marking tail-position ones, and keep a callee as awaited if it has ANY
    // non-tail occurrence OR is a tail call to a non-loop-member.
    fn awaited_callees(db: &mut Db, def: usize) -> Vec<usize> {
        let body = match db.defs[def].body {
            Some(b) => b,
            None => return Vec::new(),
        };
        let group = loop_group(db, def);
        let all = crate::layout::callees_of(db, body);
        if group.is_empty() {
            return all; // not a loop → no call is a `continue` → every call is awaited.
        }
        let mut tail = Vec::new();
        tail_callees(db, body, &mut tail);
        // A callee is awaited unless ALL its occurrences are tail calls to a loop-group member. It has a
        // non-tail occurrence iff it is called at all AND is not SOLELY a tail-member call — approximated
        // as: keep it awaited when it is NOT a group member, OR it appears in a non-tail position. We
        // detect a non-tail occurrence by the callee being in `all` but its tail-only status being false.
        let mut nontail = Vec::new();
        nontail_callees(db, body, &mut nontail);
        all.into_iter()
            .filter(|c| nontail.contains(c) || !(group.contains(c) && tail.contains(c)))
            .collect()
    }
    // The callees appearing in a NON-tail position (an operand, a call argument, a scrutinee) — the calls
    // the emitter always awaits (never a `continue`). The complement of `tail_callees`'s tail positions:
    // descend tail-transparent forms (if/let/match) NOT collecting the tail slot, and collect EVERY call
    // reached through a non-tail child (operands of arith, args of a call, etc.).
    fn nontail_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
        match core_of(db, id) {
            // A call node: its own callee is in tail position HERE (handled by the caller's descent), but
            // its ARGS are non-tail — collect calls inside them.
            Core::Call { args, .. } => {
                for &a in args.iter() {
                    collect_all_calls(db, a, out);
                }
            }
            // Tail-transparent forms: the cond/binding-values are non-tail; the branch/body slots are tail
            // (recurse tail-transparently). A `let` binding value is non-tail.
            Core::If {
                cond, then_, else_, ..
            } => {
                collect_all_calls(db, cond, out);
                nontail_callees(db, then_, out);
                nontail_callees(db, else_, out);
            }
            Core::Let { bindings, body } => {
                for (_, v) in bindings.iter().copied() {
                    collect_all_calls(db, v, out);
                }
                nontail_callees(db, body, out);
            }
            Core::Match { arms, .. } => {
                for a in arms {
                    nontail_callees(db, a.body, out);
                }
            }
            Core::MatchList { arms, .. } => {
                for a in arms {
                    nontail_callees(db, a.body, out);
                }
            }
            // Any other form is a non-tail expression: every call inside it is awaited.
            other => {
                let _ = other;
                collect_all_calls(db, id, out);
            }
        }
    }
    // Every callee reached anywhere under `id` (all positions) — a call in a non-tail expression is
    // awaited regardless of nesting, so this gathers the full set to mark as non-tail.
    fn collect_all_calls(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
        for c in crate::layout::callees_of(db, id) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut work = awaited_callees(db, callee);
    while let Some(c) = work.pop() {
        if c == callee {
            return true; // an awaited path returns to `callee` — the future is self-referential.
        }
        if !seen.insert(c) {
            continue;
        }
        work.extend(awaited_callees(db, c));
    }
    false
}

/// The tail-recursion group `self_def` belongs to — the members compiled into ONE shared `loop`, with
/// `self_def` FIRST (it enters the loop at its own `which = 0`). Empty = no loop. Mirrors the wasm
/// backend's `mutual_loop_group`:
///  - forward tail-reachability from `self_def`, staying within SAME-SIGNATURE defs (a differently-typed
///    tail callee can't share the loop's positional locals);
///  - keep only members that tail-reach BACK to `self_def` (the genuine SCC) — a one-way tail callee is
///    not part of the cycle and stays an ordinary (boxed) call;
///  - a single member is a loop ONLY if it actually self-tail-calls; else no loop (empty).
///
/// Members are `self_def` then the rest ascending, so the `which` discriminants are stable. Discriminants
/// are LOCAL to each member's own emitted loop (control never crosses between two members' loops — each
/// is a complete copy of the group), so `self`-first differing per member is fine.
fn loop_group(db: &mut Db, self_def: usize) -> Vec<usize> {
    let Some(self_sig) = sig_types(db, self_def) else {
        return Vec::new();
    };
    // Forward tail-reachability within the same signature.
    let mut reach: Vec<usize> = vec![self_def];
    let mut i = 0;
    while i < reach.len() {
        let d = reach[i];
        i += 1;
        if let Some(body) = db.defs[d].body {
            let mut callees = Vec::new();
            tail_callees(db, body, &mut callees);
            for c in callees {
                if !reach.contains(&c) && sig_types(db, c).as_ref() == Some(&self_sig) {
                    reach.push(c);
                }
            }
        }
    }
    // Keep the SCC: members that tail-reach back to `self_def` (plus `self_def` itself).
    let mut members: Vec<usize> = reach
        .iter()
        .copied()
        .filter(|&d| d == self_def || tail_reaches(db, d, self_def, &reach))
        .collect();
    members.sort_unstable();
    members.retain(|&d| d != self_def);
    members.insert(0, self_def);
    if members.len() == 1 {
        // A lone member loops only if it self-tail-calls.
        let body = match db.defs[self_def].body {
            Some(b) => b,
            None => return Vec::new(),
        };
        if body_has_tail_call_to(db, body, &members) {
            return members;
        }
        return Vec::new();
    }
    members
}

/// The signature of `def` as its parameter + result RUST types (the string forms), or `None` if any
/// type has no native mapping. Two defs share a loop only if these agree — they reassign the SAME shared
/// positional locals, so the widths must match position-for-position (and the result type must match so
/// every member's `break` yields one type).
fn sig_types(db: &mut Db, def: usize) -> Option<Vec<String>> {
    let params = crate::layout::def_params(db, def);
    let body = db.defs[def].body?;
    let mut sig = Vec::new();
    for (_, ty) in &params {
        sig.push(types::rust_type(&db.name_ctx(), ty)?);
    }
    // A sentinel separates params from result so `(u8)->u16` ≠ `(u8,u16)->()` etc.
    sig.push("->".to_string());
    let bty = type_of(db, body);
    sig.push(types::rust_type(&db.name_ctx(), &bty)?);
    Some(sig)
}

/// The defs called in TAIL position from the body at `id` (an `if` branch, a `let` body, a `match` arm)
/// — the tail-recursion edges. A call in a NON-tail position (an operand) is NOT an edge. Mirrors
/// [`emit_tail`]'s propagation so the group and the emission agree.
fn tail_callees(db: &mut Db, id: StructId, out: &mut Vec<usize>) {
    match core_of(db, id) {
        Core::Call { callee, .. } if !out.contains(&callee) => out.push(callee),
        Core::Call { .. } => {}
        Core::If { then_, else_, .. } => {
            tail_callees(db, then_, out);
            tail_callees(db, else_, out);
        }
        Core::Let { body, .. } => tail_callees(db, body, out),
        Core::Match { arms, .. } => {
            for a in arms {
                tail_callees(db, a.body, out);
            }
        }
        Core::MatchList { arms, .. } => {
            for a in arms {
                tail_callees(db, a.body, out);
            }
        }
        _ => {}
    }
}

/// Whether `from` tail-reaches `target` staying within `within` (a transitive closure over the tail
/// edges) — used to keep only the genuine SCC members (those that tail-cycle back to `self_def`).
fn tail_reaches(db: &mut Db, from: usize, target: usize, within: &[usize]) -> bool {
    let mut seen: Vec<usize> = vec![from];
    let mut i = 0;
    while i < seen.len() {
        let d = seen[i];
        i += 1;
        if let Some(body) = db.defs[d].body {
            let mut callees = Vec::new();
            tail_callees(db, body, &mut callees);
            for c in callees {
                if c == target {
                    return true;
                }
                if within.contains(&c) && !seen.contains(&c) {
                    seen.push(c);
                }
            }
        }
    }
    false
}

/// Whether the body at `id` makes a tail call to ANY member of `members` (the loop's iteration edge). A
/// call in a NON-tail position is not an edge. Mirrors [`emit_tail`]'s propagation.
fn body_has_tail_call_to(db: &mut Db, id: StructId, members: &[usize]) -> bool {
    match core_of(db, id) {
        Core::Call { callee, .. } => members.contains(&callee),
        Core::If { then_, else_, .. } => {
            body_has_tail_call_to(db, then_, members) || body_has_tail_call_to(db, else_, members)
        }
        Core::Let { body, .. } => body_has_tail_call_to(db, body, members),
        Core::Match { arms, .. } => arms
            .iter()
            .any(|a| body_has_tail_call_to(db, a.body, members)),
        Core::MatchList { arms, .. } => arms
            .iter()
            .any(|a| body_has_tail_call_to(db, a.body, members)),
        _ => false,
    }
}

/// Render the node at `id` GROUNDED to the context integer type `it` — the width/signedness of the
/// construct the node sits in (an arithmetic/comparison op, an `if`/`match` result). A bare integer
/// LITERAL is width-polymorphic: its own `type_of` defaults to `Int64` (unification fixes the parent's
/// type from the definite operand but does NOT thread that width back onto the literal node), so a
/// literal operand of a narrow op would otherwise render `1u64 as i64` and produce a Rust type mismatch
/// against the narrow context (`u8::checked_add(i64)` → E0308). Grounding renders the literal at the
/// context width (`1u8`), exactly as the wasm backend's `emit_operand`/`emit_branch` normalize a bare
/// literal to the op/branch machine width. A NON-literal node already carries its own definite type, so
/// it emits unchanged.
/// Emit element/payload node `id`, GROUNDING an empty-list value to a target `List(elem)` type. An empty
/// `(list)` node's own `type_of` often leaves its element unsolved (`List ?`), so `Core::ListNew` emits a
/// bare `vec![]` rustc cannot infer in a construction slot (E0282 "type annotations needed for `Vec<_>`").
/// When the element sits in a slot whose SOLVED type is a representable `List(elem)` — a tuple element, a
/// record field, a sum payload — annotate `Vec::<elem>::new()`. `target` is that slot's declared type
/// (`None` when unknown → emit unchanged). Only rewrites the exact bare `vec![]`; a non-empty or
/// already-typed emit passes through byte-identical.
fn emit_elem_grounding_empty_list(
    db: &mut Db,
    id: StructId,
    target: Option<&Ty>,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // GROUND a bare deferred-width literal ELEMENT/FIELD to the compound's DECLARED slot type. A tuple/
    // record element that is a `Core::ConstInt`/`Core::ConstFloat` has its OWN solved type default to
    // Int64/Float64 when the checker didn't pin it from context — so a `(record (x 100))` at field type
    // Int8 would emit `100u64 as i64` into an `(i8,)` slot (rustc E0308), the compound twin of the match-
    // arm/if-branch literal grounding (`emit_grounded`/`emit_branch`). When the slot type is a concrete
    // narrow Int/Float, ground the literal to it here (the checker guarantees a fitting literal — a
    // non-fitting one is a CDZ fault that aborts before emit). A non-literal / non-narrow element falls
    // through to the empty-list grounding and the plain emit below.
    if let Some(t) = target {
        match t.strip_nominal() {
            Ty::Int(it) => return emit_grounded(db, id, *it, env, ctx),
            Ty::Float(ft) => return emit_grounded_float(db, id, ft.ground_width(), env, ctx),
            _ => {}
        }
    }
    let a = emit(db, id, env, ctx)?;
    if a == "vec![]"
        && let Some(t) = target
        && let Ty::List(elem) = t.strip_nominal()
    {
        // GROUND the element's still-open vars to the `Int64` default before spelling `Vec::<T>::new()`.
        // A tuple/record/sum-payload empty-list field whose two match arms did NOT unify their element
        // type (one arm supplies `List Int64`, the empty-list arm keeps `List Any`) leaves the slot type
        // `List(Any)` — `rust_type(Any)` is `None`, so without grounding this bailed to a bare `vec![]`
        // that rustc cannot infer in a tuple-return position (E0282, breaker #18 n18c). The empty list has
        // NO element, so grounding its phantom element type is behavior-neutral, and rustc unifies the
        // `Vec::<i64>::new()` with the sibling arm's `Vec<i64>`. A genuinely non-Int64 sibling would error
        // LOUDLY at rustc (E0308), never a silent miscompile — the same contract `ground_open_vars` carries
        // for empty `Map`/`Set`. (wasm's list handle needs no spelled element type, so it ran regardless —
        // NOT proof the type was solved.)
        if let Some(rust_elem) = types::rust_type(&db.name_ctx(), &types::ground_open_vars(elem)) {
            return Ok(format!("Vec::<{rust_elem}>::new()"));
        }
    }
    // A generic-sum CONSTRUCTION whose SLOT type carries an UNCONSTRAINED type arg — a bare `(Ok x)` / `(None)`
    // whose Err / element type the checker left FREE because the value is DISCARDED (List.len'd, stored in a
    // record/list only its length is read, …) — emits e.g. `Ok(22016)` : `Result<i64, _>`, which rustc cannot
    // infer in a slot with no downstream use (E0282 "type annotations needed"; wasm needs no such
    // materialization). The `target` IS the checker's SOLVED field/element/payload slot type; a free var there
    // is GENUINELY free — a record/tuple/sum-payload slot is a DIRECT construction, not a join (a join threads
    // the SOLVED join type as the target — `Core::If`'s result annotation — so grounding is a no-op there and
    // never conflicts with a sibling). GROUND the slot's free vars and PIN the construction with a type
    // ascription so the emitted enum is fully typed. Only a DIRECT `SumNew` value with a free-var target — a
    // non-sum value, or a fully-solved target, is byte-identical to before (this only annotates the E0282 tail).
    if let Some(t) = target
        && matches!(core_of(db, id), Core::SumNew { .. })
        && t.has_free_var()
        && let Some(rt) = types::rust_type(&db.name_ctx(), &types::ground_open_vars(t))
    {
        return Ok(format!("{{ let __v: {rt} = {a}; __v }}"));
    }
    Ok(a)
}

/// Emit a FLOAT operand grounded to `width` (32/64) — the float twin of `emit_grounded`. A `Core::ConstFloat`
/// literal's OWN solved type DEFAULTS to Float64 when the checker didn't pin it (a bare `1.5` in `(= x 1.5)`
/// where `x: Float32`), so emitting it as-is (`f64::from_bits(…)`) then feeding it to the caller's f32
/// compare/arith is WRONG: the equality path's `.to_bits() as u32` takes the LOW 32 BITS of the f64 pattern
/// (0x0 for 1.5) instead of the f32 bits (0x3fc00000) → the compare is ALWAYS FALSE (a silent wrong value);
/// the arith path emits `x * <f64>` → rustc E0277 (`f32 * f64`). Ground the LITERAL to the op's width so
/// both operands share the type. A NON-literal operand carries its own concrete float type and emits as-is
/// (a genuine width disagreement is a type fault that aborts before emit, like the integer path).
fn emit_grounded_float(
    db: &mut Db,
    id: StructId,
    width: u32,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    if let Core::ConstFloat(d) = core_of(db, id) {
        return Ok(if width == 32 {
            let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
            format!("f32::from_bits({bits}u32)")
        } else {
            format!("f64::from_bits({}u64)", d.to_f64_bits())
        });
    }
    let rendered = emit(db, id, env, ctx)?;
    // WIDTH NORMALIZATION for a CONTROL-FLOW / non-literal FLOAT operand — the float twin of `emit_grounded`'s
    // integer cast (above). An `if`/`match` whose branches are bare deferred-width `ConstFloat`s is solved at
    // its OWN type, which DEFAULTS to Float64, while the enclosing op wants `width`. Emitting unchanged renders
    // an `f64` sub-expression where `f32` is required (`x_f32 + (if c 1.0 2.0)` → rustc E0308 / E0277). Cast the
    // operand to the op's width at the consuming site with `as f32`/`as f64` (the narrowing is exact for a
    // deferred literal defaulted to f64; a genuine fixed-width disagreement is a type FAULT that aborts before
    // emit). Like the integer twin, read the CALLEE's result type for a `Core::Call` (the call-site ascription
    // is type-only and would mask a real mismatch). Only cast a FLOAT operand whose solved width DIFFERS from
    // `width`; a matching-width or non-float operand emits unchanged (no redundant `as`).
    let emitted_ty = if let Core::Call { callee, .. } = core_of(db, id) {
        match db.defs[callee].body {
            Some(cb) => type_of(db, cb),
            None => type_of(db, id),
        }
    } else {
        type_of(db, id)
    };
    if let Ty::Float(op_ft) = emitted_ty.strip_nominal()
        && op_ft.ground_width() != width
    {
        let target = if width == 32 { "f32" } else { "f64" };
        return Ok(format!("(({rendered}) as {target})"));
    }
    Ok(rendered)
}

/// The Int/Float grounding width of a homogeneous-container SLOT type (a List/Set element or a Map key/value):
/// a narrow `Int` width to feed [`emit_grounded`], or a `Float` width to feed [`emit_grounded_float`], so a
/// bare deferred-width literal element/key/value is emitted at the container's SETTLED width rather than its
/// own `Int64`/`Float64` default — the fix for the mixed-width `vec![<f64>, <f32>]` / `BTreeMap` E0308 the
/// wasm side already avoids via `box_op_for`. `(None, None)` for a non-numeric or unresolved slot.
fn container_slot_grounding(t: &Ty) -> (Option<IntTy>, Option<u32>) {
    match t.strip_nominal() {
        Ty::Int(it) => (Some(*it), None),
        Ty::Float(ft) => (None, Some(ft.ground_width())),
        _ => (None, None),
    }
}

fn emit_grounded(
    db: &mut Db,
    id: StructId,
    it: IntTy,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    if let Core::ConstInt(v) = core_of(db, id) {
        return emit_const_int_at(&db.name_ctx(), it, &v);
    }
    // NESTED-ARITH CONSUMING WIDTH (rust twin of the wasm `emit_operand_into` fix, select.rs). A nested
    // `+`/`-`/`*` whose OWN width is DEFERRED (its operands are bare literals, or `if`/`match` branches of
    // bare literals) grounds to the i64 DEFAULT, so it computes AND range-checks at i64; the generic emit-
    // then-cast path below would then truncate that i64 result `as iN`, SILENTLY WRAPPING an inner overflow
    // (`(+ (+ (if c 100 10) (if d 100 10)) 5) : Int8`, inner 100+100=200 → `200 as i8` = -56) rather than
    // TRAPPING — an `as` cast is not a range check (v-wasmtime-migration mig-2). Emit the inner op AT the
    // consuming width `it` so it computes AND range-checks (`checked_*`) there, trapping the inner overflow
    // exactly as wasm does. Only for a DEFERRED-width op: a fixed inner width is honored inside `emit_arith`.
    if let Core::Arith { op, lhs, rhs } = core_of(db, id)
        && matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
        && !int_ty_of(db, id).width_is_fixed()
    {
        return emit_arith(db, id, op, lhs, rhs, env, ctx, Some(it));
    }
    let rendered = emit(db, id, env, ctx)?;
    // WIDTH NORMALIZATION for a CONTROL-FLOW / non-literal operand. A bare literal is grounded above; but
    // an operand that is an `if`/`match` (or any node) whose BRANCHES are bare deferred-width literals is
    // solved at its OWN type — which defaults to Int64 — while the enclosing op wants the NARROW width
    // `it`. Emitting it unchanged renders an `i64` sub-expression where an `iN` is required (`(if … {
    // 100i64 } …).checked_add(100i8)` → rustc E0308). Reconcile HERE, at the consuming site, by wrapping
    // the operand down to the op's width with an `as <target>` cast — the native mirror of the wasm
    // backend's `i32.wrap_i64` narrow-value normalization. SOUND: a genuine fixed-width disagreement is a
    // type FAULT (CDZ0203) that aborts before emit, so a wider-than-`it` operand reaching here is a
    // deferred literal defaulted to Int64 whose low bits ARE its value; the cast truncates to `it` exactly
    // as the wasm wrap does, and the enclosing op's own overflow check then traps a true overflow (`(: (+
    // (if … 100 0) 100) Int8)` n=3 → 100+100=200 overflows Int8 → panics, matching wasm's trap). Only cast
    // when the operand's OWN solved integer type DIFFERS from `it` (same width → emit unchanged, no
    // redundant `as`); a non-integer operand emits unchanged.
    // The operand's ACTUAL EMITTED integer type. For a `Core::Call` it is the CALLEE's own result type,
    // NOT `type_of(id)`: the call-site narrowing/widening ascription `(: (f x) T)` is absorbed as type-only
    // (no Core cast node), so `type_of(id)` reports the ascribed op width and MASKS a real mismatch — the
    // OPERAND-position twin of the `emit_body` E0308 fix (fz-38551 tail-call class / fz-38592: `(+ (: (rec)
    // UInt64) 3)` emitted `(rec()).checked_add(3u64)` = i64 `.checked_add` u64 → E0308). A non-Call node
    // emits at its own solved `type_of` (the `if`/`match` branch-width case this guard already handled).
    let emitted_ty = if let Core::Call { callee, .. } = core_of(db, id) {
        match db.defs[callee].body {
            Some(cb) => type_of(db, cb),
            None => type_of(db, id),
        }
    } else {
        type_of(db, id)
    };
    if let Ty::Int(op_it) = emitted_ty
        && (op_it.ground_signed(), op_it.ground_width()) != (it.ground_signed(), it.ground_width())
        && let Some(target) = types::rust_type(&db.name_ctx(), &Ty::Int(it))
    {
        // Parenthesize the rendered operand before the `as` so the cast binds to the WHOLE expression
        // regardless of its shape (an `if`/`match`/block would otherwise let `as` bind only to the last
        // sub-expression). `unused_parens` is allowed in the emitted header, so redundant parens are fine.
        return Ok(format!("(({rendered}) as {target})"));
    }
    Ok(rendered)
}

/// Render the node at `id` in TAIL position inside a self-loop (`ctx.self_loop` is `Some`) — the result
/// each path produces is the function's result. Tail-ness PROPAGATES through the result-producing
/// sub-positions (an `if`'s branches, a `match`'s arm bodies, a `let`'s body); the condition/scrutinee/
/// binding values are NOT tail (they are ordinary values, emitted via `emit`). At a tail LEAF:
///  - a SELF tail-call `f(a…)` becomes the parallel move `{ let (t…) = (a…); p0 = t0; …; continue }` —
///    all args computed into temps before any param is overwritten, then jump to the loop top;
///  - any other value `v` becomes `break v` (yielding the loop's — the function's — result).
///
/// Emit `body` under the FLOW-REFINEMENT frame that `cond` establishes for the given branch polarity, so a
/// guard-elision check inside the branch (`provably_no_overflow` → `value_range` → `db.refined_range`) sees
/// the range facts the guard implies (`(if (< a 100) (+ a 1) …)` → `a ∈ [_, 99]` in the then branch, so the
/// `+ a 1` overflow guard is elided). This mirrors the wasm backend's `refined_frame_for_branch` push/pop
/// (select.rs) — the SAME backend-agnostic refinement computation — so both backends make the identical
/// elision decision. The frame is INTERSECTED onto the current one (`refined_frame_for_branch` takes the
/// base) and popped after `body`, so refinements nest correctly and never leak past the branch. A condition
/// that yields no single-variable interval (a non-comparison, an `or`'s then) contributes an unchanged
/// frame — a safe no-op.
fn with_branch_refinement<F>(
    db: &mut Db,
    cond: StructId,
    then_branch: bool,
    body: F,
) -> Result<String, Reject>
where
    F: FnOnce(&mut Db) -> Result<String, Reject>,
{
    let base = db.current_refinements();
    let frame =
        crate::backend::common::diverge::refined_frame_for_branch(db, cond, then_branch, base);
    db.push_range_refinements(frame);
    let out = body(db);
    db.pop_range_refinements();
    out
}

/// Returns a Rust STATEMENT/expression usable as the loop body. Only called when `ctx.self_loop` is set.
fn emit_tail(db: &mut Db, id: StructId, env: &Env, ctx: &Ctx) -> Result<String, Reject> {
    let group = ctx
        .loop_group
        .expect("emit_tail is only called inside a loop group");
    match core_of(db, id) {
        // A tail call to a GROUP MEMBER iterates the loop: reassign the shared positional locals (+ the
        // `which` state, for a mutual group) and `continue`. A tail call to a NON-member is not a loop
        // edge — it falls to the `break <value>` leaf below (an ordinary boxed/awaited call in async).
        Core::Call { callee, args } if group.members.contains(&callee) => {
            // Ground each arg to the callee's param width, exactly as the ordinary call arm.
            let param_tys = crate::layout::def_params(db, callee);
            let mut rendered = Vec::new();
            for (i, &a) in args.iter().enumerate() {
                match param_tys.get(i).map(|(_, t)| t) {
                    Some(Ty::Int(it)) => rendered.push(emit_grounded(db, a, *it, env, ctx)?),
                    // A COLLECTION param (Set/Map/List) with a fully-concrete element: thread it as the
                    // arg's EXPECTED type, so an empty `Set.of (list)`/`Map.empty` arg annotates from the
                    // param's element type rather than the default-grounded `i64` (breaker: an empty-Set at
                    // a call-arg with a declared Float64 elem emitted `BTreeSet<i64>` ≠ the param's
                    // `BTreeSet<__CdzF64>` → E0308). Only when the param type has no free var — else there
                    // is nothing better than the node's own type.
                    Some(pt @ (Ty::Set(_) | Ty::Map(_, _) | Ty::List(_))) if !pt.has_free_var() => {
                        let mut arg_ctx = ctx.clone();
                        arg_ctx.expected_ty = Some(pt.clone());
                        rendered.push(emit(db, a, env, &arg_ctx)?);
                    }
                    _ => rendered.push(emit(db, a, env, ctx)?),
                }
            }
            if rendered.len() != group.shared_params.len() {
                return Err(Reject::decline("tail-call arity mismatch"));
            }
            // Parallel move: bind all new values into temps, THEN assign each shared local — so an arg
            // that reads an old param value (`f(n-1, acc+n)`) sees the pre-iteration locals. For a mutual
            // group, also set `which` to the callee's member index (which member body runs next).
            let temps: Vec<String> = (0..rendered.len()).map(|i| format!("__t{i}")).collect();
            let binds = if rendered.is_empty() {
                String::new()
            } else {
                format!("let ({},) = ({},); ", temps.join(", "), rendered.join(", "))
            };
            let moves = group
                .shared_params
                .iter()
                .zip(&temps)
                .map(|(p, t)| format!("{p} = {t};"))
                .collect::<Vec<_>>()
                .join(" ");
            let set_which = if group.is_mutual() {
                let k = group.members.iter().position(|&m| m == callee).unwrap();
                format!("which = {k}; ")
            } else {
                String::new()
            };
            Ok(format!("{{ {binds}{set_which}{moves} continue; }}"))
        }
        // An `if` in tail position: both branches are tail; the condition is an ordinary value.
        Core::If { cond, then_, else_ } => {
            let c = emit(db, cond, env, ctx)?;
            // FLOW-REFINEMENT parity (see the non-tail `Core::If` arm): emit each branch under the range
            // facts its guard establishes so an in-branch guard-elision check sees the narrowed range.
            let t = with_branch_refinement(db, cond, true, |db| emit_tail(db, then_, env, ctx))?;
            let e = with_branch_refinement(db, cond, false, |db| emit_tail(db, else_, env, ctx))?;
            Ok(format!("if {c} {{ {t} }} else {{ {e} }}"))
        }
        // A `let` in tail position: its bindings are ordinary values, its body is tail.
        Core::Let { bindings, body } => {
            let mut extended = env.clone();
            let mut lines = String::new();
            for (binder, value) in bindings.iter() {
                let name = local_name(db, *binder, &extended);
                let v = emit(db, *value, &extended, ctx)?;
                lines.push_str(&format!("let {name} = {v}; "));
                extended.insert(*binder, name);
            }
            let b = emit_tail(db, body, &extended, ctx)?;
            Ok(format!("{{ {lines}{b} }}"))
        }
        // A `match` in tail position: each arm body is tail. (Delegates to the shared match emitter with
        // a tail flag so arm bodies go through `emit_tail`.)
        Core::Match { scrutinee, arms } => {
            emit_match_impl(db, id, scrutinee, &arms, env, ctx, true)
        }
        // A LIST match in tail position: each arm body is tail (so a self-recursive list walker iterates
        // the enclosing loop rather than growing the stack). Delegates with the tail flag.
        Core::MatchList { scrutinee, arms } => {
            emit_list_match_impl(db, scrutinee, &arms, env, ctx, true)
        }
        // Any other tail leaf: its value is the loop's result — `break` it out. A bare-literal leaf is
        // grounded to the function's result width so every `break` in the loop yields the same type.
        _ => {
            let v = match (group.result_it, group.result_ft) {
                (Some(it), _) => emit_grounded(db, id, it, env, ctx)?,
                (None, Some(w)) => emit_grounded_float(db, id, w, env, ctx)?,
                (None, None) => emit(db, id, env, ctx)?,
            };
            Ok(format!("break {v};"))
        }
    }
}

/// Render the node at `id` as a Rust expression string. Exhaustive over `Core`; a form without a
/// scalar rendering declines. Reads the core + type columns on demand. The rendered expression is
/// parenthesized where needed so it composes as a sub-expression without precedence surprises.
fn emit(db: &mut Db, id: StructId, env: &Env, ctx: &Ctx) -> Result<String, Reject> {
    match core_of(db, id) {
        // An integer constant, written as its two's-complement BIT PATTERN in the unsigned type of its
        // width, then cast to the target type — the same bit-pattern emit the wasm backend does
        // (`to_i64_bits`/`to_i32_bits`). This one spelling covers a signed negative (`-128: Int8` =
        // `128u8 as i8`) and an unsigned value at/above the signed max (`UInt64.max` = `…u64`) alike.
        // The constant must FIT its width (checked here, CDZ0302 — a value that does not fit never
        // reaches a well-typed program, but selection re-checks rather than truncate silently).
        Core::ConstInt(v) => {
            // A CONSTANT BigInt folds to `Core::ConstInt` retyped `Ty::BigInt` upstream. On this backend
            // it must materialize a `cdz_num::Big` value (NOT a fixed-width int literal), so a BigInt op /
            // a BigInt-typed export sees a `Big`. In-i64 range → `Big::from_i64`; a beyond-i64 constant →
            // `Big::from_sign_magnitude_bytes(&[sign, LE-magnitude…])` (the runtime's canonical leaf form,
            // the same route the wasm backend's `bigint-of-bytes` takes). `IntValue.magnitude` is
            // BIG-endian, so reverse it for the LE form the parser expects. `is_bigint_valued` also covers
            // a `Qty{inner:BigInt}`-typed constant — a BigInt-magnitude quantity's constant erases to a
            // `Big`, NOT an i64 (else a BigInt op over the erased Qty would `.mul()` on a mismatched i64).
            if is_bigint_valued(&type_of(db, id)) {
                Ok(const_big_expr(&v))
            } else {
                emit_const_int(db, id, &v)
            }
        }
        Core::ConstBool(b) => Ok(if b {
            "true".to_string()
        } else {
            "false".to_string()
        }),
        // Unit is Rust's `()`.
        Core::Unit => Ok("()".to_string()),
        // A STRING constant → a Rust `String` (`"…".to_string()`). The literal's bytes are escaped for a
        // Rust string literal — `\`, `"`, newline/CR/tab, and any non-printable byte via `\u{..}` — so the
        // emitted source is valid regardless of the string's content. A Cadenza `String` is owned text, so
        // `.to_string()` gives the owned `String` the type map (`Ty::String`→`String`) expects.
        Core::ConstStr(s) => Ok(format!("{}.to_string()", rust_string_literal(&s))),
        // A CHAR constant → a Rust `char` literal `'…'`. Escapes `'`/`\`/the whitespace controls and any
        // other control/non-printable scalar via `\u{..}` so the literal is always valid; a printable
        // scalar (incl a UTF-8 letter) is emitted verbatim. `Ty::Char` maps to `char`, so this crosses as
        // a `char` value (a sum payload / tuple element).
        Core::ConstChar(c) => Ok(rust_char_literal(c)),
        // A parameter or kept-let reference — read the identifier its binder maps to. A binder with no
        // environment entry is a compiler bug (a ref whose binding was not brought into scope), so
        // decline rather than emit a dangling name.
        Core::Param { binder } | Core::LocalRef { binder } => {
            let name = env
                .get(&binder)
                .cloned()
                .ok_or_else(|| Reject::decline("reference has no bound Rust identifier"))?;
            // A NON-COPY binding (a `Vec` list — the native strategy's first move-only type) may be read in
            // more than one position; Rust would MOVE it on the first by-value use and reject the rest
            // (E0382). Cadenza values are persistent/shareable, so `.clone()` every non-Copy binding read —
            // the value-level analogue of the wasm backend's Perceus dup (a clone is always sound; the
            // rust backend is a correctness oracle, not a perf target, so over-cloning is acceptable). A
            // COPY binding (a scalar, an all-scalar tuple/record, a non-payload-heavy enum) is read as-is —
            // Rust copies it implicitly, and a spurious `.clone()` there is a needless-clone lint under
            // `-D warnings`. `needs_clone_on_read` is conservative: it clones only a type that is provably
            // non-Copy in the emitted Rust (a `Vec`, i.e. a `List`), leaving every existing Copy case
            // byte-identical.
            if needs_clone_on_read(db, id) {
                Ok(format!("{name}.clone()"))
            } else {
                Ok(name)
            }
        }
        // An `if` → Rust's `if cond { then } else { else }`. Rust's `if` is an expression, so it yields
        // the branch value directly — the structured target expresses the core's `If` as itself. Both
        // branches must produce the `if`'s RESULT type; a bare-literal branch is GROUNDED to that width
        // (via `emit_branch`) so a default-Int64 literal opposite a narrow branch does not mismatch the
        // block's type — the same reconciliation the wasm backend's `emit_branch` does.
        Core::If { cond, then_, else_ } => {
            let c = emit(db, cond, env, ctx)?;
            // FLOW-REFINEMENT (both-backend parity with wasm's `refined_frame_for_branch` push/pop): each
            // branch is emitted under the range facts its guard establishes, so a guard-elision check
            // (`provably_no_overflow` → `value_range`) inside a branch sees the narrowed range and drops a
            // dead overflow guard — e.g. `(if (< a 100) (+ a 1) 0)` elides the `+ a 1` guard in the then
            // branch. Without this the rust backend saw no refinement (wasm did), keeping a guard wasm
            // elides — a correct-but-divergent decision this closes. Symmetric push/pop around EACH branch.
            let t =
                with_branch_refinement(db, cond, true, |db| emit_branch(db, then_, id, env, ctx))?;
            let e =
                with_branch_refinement(db, cond, false, |db| emit_branch(db, else_, id, env, ctx))?;
            let bare = format!("if {c} {{ {t} }} else {{ {e} }}");
            // ANNOTATE the `if` result when it is a GENERIC SUM (`Option<…>`/`Result<…>`/a user generic
            // enum). A branch that is a bare nullary generic variant (`Option::None`) carries no type
            // parameter, and rustc types the branches LEFT-TO-RIGHT — so a `None`-first `if` fails to infer
            // (E0282) even though the sibling `Some` fixes it. Wrapping in `{ let __if: <ty> = …; __if }`
            // with the if's OWN solved type (well-typed even when a leaf isn't) gives rustc the type up
            // front. Only for a generic sum with a spellable type (the ambiguity case); every other result
            // type keeps the bare `if` (a monomorphic sum / scalar / collection branch is never ambiguous).
            if let ty @ Ty::Sum { args, .. } = type_of(db, id).strip_nominal()
                && !args.is_empty()
                && let Some(rty) = types::rust_type(&db.name_ctx(), ty)
            {
                Ok(format!("{{ let __if: {rty} = {bare}; __if }}"))
            } else {
                Ok(bare)
            }
        }
        // A short-circuiting boolean connective → Rust's own `&&`/`||`, which short-circuit with
        // exactly the core's semantics: `rhs` is evaluated ONLY on the non-short-circuiting branch, so
        // a trapping/effectful `rhs` is shielded just as the core's `if lhs then rhs else false`
        // (`and`) / `if lhs then true else rhs` (`or`) prescribes (core-semantics.md §Boolean
        // Connectives Short-Circuit). The structured target expresses the connective as itself.
        Core::And { lhs, rhs, is_and } => {
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let op = if is_and { "&&" } else { "||" };
            Ok(format!("({l} {op} {r})"))
        }
        // An A-normal `let` sequence → a Rust block: each binding a `let name = value;`, then the body
        // as the block's tail expression. A kept binding names a runtime value used more than once, so
        // Rust computes it once and each `LocalRef` reads the binding — the same "name it once" the
        // core's `Let` encodes. No `drop` bookkeeping: Rust owns the value's lifetime (the native
        // strategy — `§A Backend … native aggregates`), so the Perceus dance the wasm backend does is
        // simply not needed here.
        Core::Let { bindings, body } => {
            let mut extended = env.clone();
            let mut lines = String::new();
            for (binder, value) in bindings.iter() {
                let name = local_name(db, *binder, &extended);
                let v = emit(db, *value, &extended, ctx)?;
                lines.push_str(&format!("let {name} = {v}; "));
                extended.insert(*binder, name.clone());
                // ALSO map the VALUE node → this binding's name. A closure that captures a let-bound value
                // records the capture as the VALUE node itself (lowering inlines the value into
                // `Core::Closure.captures`, NOT a `LocalRef` to the binder), so the closure build-site would
                // RE-EMIT the value — a second host call / recomputation (the double-emit bug) — unless it
                // can see the value is already bound. Keying the value node lets the capture emit reference
                // `name` instead of re-emitting (see the `Core::Closure` arm's `env.get(&c)` check).
                extended.insert(*value, name);
            }
            let b = emit(db, body, &extended, ctx)?;
            Ok(format!("{{ {lines}{b} }}"))
        }
        // A scalar `match` → Rust's `match`. Each arm renders `pattern => body`; a literal probe is the
        // literal pattern (written in the scrutinee's type), a wildcard/binder is `_`. `lower`
        // guaranteed exhaustiveness (a wildcard tail, or full Bool coverage), so the Rust match is
        // exhaustive too. The scrutinee is rendered once (Rust binds it), not re-tested per arm.
        Core::Match { scrutinee, arms } => emit_match(db, id, scrutinee, &arms, env, ctx),
        // A runtime comparison → the Rust comparison operator. Signedness/width are already baked into
        // the operands' Rust types (a `u32` compares unsigned, an `i8` signed), so the operator alone
        // is correct — no `_s`/`_u` variant selection like wasm needs. Both operands must share one
        // type; a bare-literal operand is GROUNDED to the comparison's integer type (the non-literal
        // side's width) so `(< a 5)` over a narrow `a` does not compare `u8 < i64` (Rust E0308).
        Core::Compare { op, lhs, rhs } => {
            let sym =
                compare_sym(op).ok_or_else(|| Reject::decline("not a comparison operator"))?;
            // A DIVERGING OPERAND makes the comparison dead — the twin of `emit_arith`'s guard, for the
            // `Core::Compare` emit path. `(< (+ (trap) 1) 2)` would otherwise emit `panic!("unreachable") <
            // 2` — comparing Rust's `!`/`()` with `i64` (E0277). Cadenza evaluates lhs THEN rhs, so if lhs
            // diverges emit it alone; if rhs diverges, lhs runs for effect then rhs aborts. Uses the same
            // transitive `arith_operand_diverges` so a diverging operand NESTED in arith (the common shape,
            // since a bare `(< (trap) 1)` needs a heap-walk both backends decline) is caught at any depth.
            if arith_operand_diverges(db, lhs) {
                return emit(db, lhs, env, ctx);
            }
            if arith_operand_diverges(db, rhs) {
                let l = emit(db, lhs, env, ctx)?;
                let r = emit(db, rhs, env, ctx)?;
                return Ok(format!("{{ let _ = {l}; {r} }}"));
            }
            match operand_int_ty(db, lhs, rhs) {
                Some(it) => {
                    let l = emit_grounded(db, lhs, it, env, ctx)?;
                    let r = emit_grounded(db, rhs, it, env, ctx)?;
                    Ok(format!("({l} {sym} {r})"))
                }
                // A non-integer comparison (Bool operands) — no width to reconcile, emit as-is.
                None => {
                    let l = emit(db, lhs, env, ctx)?;
                    let r = emit(db, rhs, env, ctx)?;
                    Ok(format!("({l} {sym} {r})"))
                }
            }
        }
        // RUNTIME STRING/SYMBOL ORDERING — a `<`/`<=`/`>`/`>=` on two String/Symbol values. Rust's `String`
        // (and `str`) ordering IS content-lexicographic (byte order over the UTF-8), matching the blessed
        // total order (core-semantics.md §Compound Ordering / 17-symbols §order) — so emit the operands
        // compared with the native operator directly. A String value in the Rust backend is a `String`, so
        // `(l < r)` compiles and gives lexicographic order; the wasm backend does the equivalent byte-lex
        // walk. `=`/`≠` do NOT reach here (equality routes to the structural `ValueEq`); only the four
        // ordering ops, so `compare_sym` always yields a relational operator.
        Core::StrCmp { op, lhs, rhs } => {
            let sym = compare_sym(op)
                .ok_or_else(|| Reject::decline("StrCmp carries a non-compare prim"))?;
            if arith_operand_diverges(db, lhs) {
                return emit(db, lhs, env, ctx);
            }
            if arith_operand_diverges(db, rhs) {
                let l = emit(db, lhs, env, ctx)?;
                let r = emit(db, rhs, env, ctx)?;
                return Ok(format!("{{ let _ = {l}; {r} }}"));
            }
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            Ok(format!("({l} {sym} {r})"))
        }
        // RUNTIME FLOAT EQUALITY under the CANONICAL BYTE FORM — `nan == nan` TRUE, `-0.0 != +0.0`, all
        // NaN equal (core-semantics.md §Floating-Point Equality Follows The Canonical Byte Form). NOT
        // Rust's `==` on floats (IEEE: `nan != nan`, `-0.0 == 0.0` — a miscompile). Canonicalize each
        // operand to its integer bit pattern with NaN folded to one canonical form
        // (`if x.is_nan() { CANON_NAN_BITS } else { x.to_bits() }`), then compare the bit patterns with
        // integer `==`. Must be byte-identical to the wasm backend's `select`-based bit compare (the
        // differential sweep checks this). Equality only — float ordering is a separate ruling.
        Core::FloatCompare {
            op,
            lhs,
            rhs,
            width,
        } => {
            // Both operands share the op's float type, but a bare-literal operand's OWN solved type defaults
            // to Float64 when unpinned (`(= x 1.5)` with `x: Float32`), so it must be GROUNDED to the op's
            // `width` — else the f64 literal's low 32 bits feed the f32 equality compare (always false) or a
            // `f32 </*/…* f64` arith E0277/E0308. `emit_grounded_float` emits a `ConstFloat` at `width`.
            let l = emit_grounded_float(db, lhs, width, env, ctx)?;
            let r = emit_grounded_float(db, rhs, width, env, ctx)?;
            if op == Prim::FEq {
                // EQUALITY under the CANONICAL BYTE FORM (nan==nan, -0.0 != +0.0) — NaN-canonicalizing bit
                // compare, NOT Rust's `==` (IEEE). Must be byte-identical to the wasm select-based compare.
                let (canon_nan, bits_ty) = if width == 32 {
                    ("0x7FC0_0000u32", "u32")
                } else {
                    ("0x7FF8_0000_0000_0000u64", "u64")
                };
                let canon = |v: &str| {
                    format!(
                        "({{ let __f = {v}; if __f.is_nan() {{ {canon_nan} }} else {{ __f.to_bits() as {bits_ty} }} }})"
                    )
                };
                Ok(format!("({} == {})", canon(&l), canon(&r)))
            } else {
                // ORDERING (`< <= > >=`) — RAW IEEE partial order. Rust's `PartialOrd` for f64/f32 gives
                // EXACTLY this: a NaN operand → false (unordered), `-0.0`/`+0.0` compare equal. So emit the
                // native Rust operator directly — matching the wasm raw `f64.lt`/etc. (the ordering relation
                // DISAGREES with the equality above on NaN + signed zero, by design).
                let sym = match op {
                    Prim::FLt => "<",
                    Prim::FLe => "<=",
                    Prim::FGt => ">",
                    Prim::FGe => ">=",
                    _ => return Err(Reject::decline("FloatCompare carries a non-compare prim")),
                };
                Ok(format!("({l} {sym} {r})"))
            }
        }
        // A runtime arithmetic op.
        Core::Arith { op, lhs, rhs } => emit_arith(db, id, op, lhs, rhs, env, ctx, None),
        // A float CONSTANT → a Rust float literal at the node's width. Emitted via `f64::from_bits`/
        // `f32::from_bits` of the canonical bit pattern so the EXACT value (incl. `-0.0`, a subnormal)
        // round-trips — a decimal spelling could lose a bit. The width is the node's solved type.
        Core::ConstFloat(d) => {
            // `float_width_of` = strip_nominal → peel `Ty::Qty` → strip_nominal (the float twin of
            // `int_ty_of`). A `(Qty Float32 u)` magnitude — AND a NOMINAL newtype wrapping such a Qty as a
            // heap value (`(type Len (Q (Qty Float32 …)))`, `Ty::Nominal { inner: Qty }`) — grounds to f32.
            // Without the leading strip a nominal wrapper falls to the f64 DEFAULT → `f64::from_bits(…)`
            // into an `f32` slot (`BTreeMap<_, f32>`) → Rust E0308 / invalid wasm. Mirrors the wasm
            // backend's ConstFloat width reader (also strip→peel→strip now).
            let width = float_width_of(db, id);
            if width == 32 {
                let bits = (f64::from_bits(d.to_f64_bits()) as f32).to_bits();
                Ok(format!("f32::from_bits({bits}u32)"))
            } else {
                Ok(format!("f64::from_bits({}u64)", d.to_f64_bits()))
            }
        }
        // A constant NaN float (`Float64.nan`/`(. Float64 nan)`) → the EXPLICIT CANONICAL NaN via
        // `from_bits`, NOT `f64::NAN`. Rust's `f64::NAN` happens to be `0x7FF8…` on every current target,
        // but its exact payload is platform-defined; the fleet's float-eq work canonicalizes NaN to a
        // FIXED byte form (`CANON_NAN_BITS` = `0x7FF8_0000_0000_0000` / `0x7FC0_0000`, the same constants the
        // `FloatCompare` canonicalizer + the wasm backend use), so emitting those exact bits makes the
        // ConstFloatNan value byte-identical to the canonical NaN across backends regardless of the
        // platform payload — no reliance on `f64::NAN`'s payload. (Width from the node's solved type.)
        Core::ConstFloatNan => {
            // strip→peel→strip via `float_width_of` — a `(Qty Float32)` or nominal-over-Qty NaN emits at
            // width 32 (see the `ConstFloat` arm — strip→peel→strip catches a nominal-over-Qty wrapper).
            let width = float_width_of(db, id);
            Ok(if width == 32 {
                "f32::from_bits(0x7FC0_0000u32)".to_string()
            } else {
                "f64::from_bits(0x7FF8_0000_0000_0000u64)".to_string()
            })
        }
        // A constant positive-INFINITY float (`Float64.Infinity`) → `f64::INFINITY` / `f32::INFINITY`.
        // Unlike NaN, `INFINITY`'s bit pattern is fully defined and identical across targets/backends
        // (`0x7FF0…` / `0x7F80_0000`), so the language-level constant maps directly with no canonicalization
        // needed — byte-identical to the wasm backend's `+inf` const. (Width from the node's solved type.)
        Core::ConstFloatInf => {
            let width = float_width_of(db, id);
            Ok(if width == 32 {
                "f32::INFINITY".to_string()
            } else {
                "f64::INFINITY".to_string()
            })
        }
        // A runtime `.wrap` conversion → an `as` cast to the target Rust type. Rust's `as` between
        // integers keeps the low bits and reinterprets at the target sign — bit-identical to
        // `IntValue::wrap_to`, and total (never panics), as `.wrap` requires.
        Core::Convert { op, operand } => match op {
            Prim::Wrap => {
                let dst = int_ty_of(db, id);
                let rty = types::rust_type(&db.name_ctx(), &Ty::Int(dst)).ok_or_else(|| {
                    Reject::decline("wrap target width has no native Rust representation")
                })?;
                let operand_s = emit(db, operand, env, ctx)?;
                let width = dst.ground_width();
                // An `as` cast keeps the STORAGE width's low bits (`as u8` = low 8), but `.wrap` to an
                // UNUSUAL width N (not a machine boundary — `UInt4`/`UInt48`/`Int4`, stored in the next-larger
                // primitive) must keep the low N bits AND reinterpret them at the target sign. An aliased
                // width (8/16/32/64) needs neither: the `as` cast IS the exact truncation + reinterpretation.
                if matches!(width, 8 | 16 | 32 | 64) {
                    Ok(format!("({operand_s} as {rty})"))
                } else if dst.ground_signed() {
                    // A SIGNED unusual width keeps the low N bits then SIGN-EXTENDS from bit N-1 (so a set
                    // bit N-1 makes the value negative — `(Int 4).wrap 8` = -8, matching `IntValue::wrap_to`:
                    // low 4 bits = 8, bit 3 set → 8 - 2^4 = -8, NOT the byte-cast's +8). Achieve it with a
                    // left-then-arithmetic-right shift in the SIGNED storage type: `(v << (bits-N)) >> (bits-N)`
                    // pushes bit N-1 to the storage sign bit, and Rust's `>>` on a signed integer is an
                    // ARITHMETIC shift that replicates it back down — the standard narrow-signed sign-extend.
                    // `bits` is the storage primitive's width (8/16/32/64, the smallest ≥ N); `bits - N ≥ 1`
                    // for an unusual width (N is never a machine boundary), so the shift is always in range.
                    // (A plain `& mask` would keep the low bits but NOT reinterpret the sign — the bug this
                    // fixes: it silently returned +8 for `(Int 4).wrap 8` rather than -8.)
                    let storage_bits: u32 = match width {
                        w if w <= 8 => 8,
                        w if w <= 16 => 16,
                        w if w <= 32 => 32,
                        _ => 64,
                    };
                    let shift = storage_bits - width;
                    Ok(format!("((({operand_s} as {rty}) << {shift}) >> {shift})"))
                } else {
                    // An UNSIGNED unusual width keeps the low N bits (`(UInt 4).wrap 17` = `17 & 0xF` = 1). The
                    // mask literal is written in the storage type `rty`; `2^N - 1` fits it (N ≤ storage width).
                    let mask: u64 = (1u64 << width) - 1;
                    Ok(format!("(({operand_s} as {rty}) & {mask}{rty})"))
                }
            }
            // A runtime int→float conversion `Float N.of-int` → an `as f64`/`as f32` cast (total,
            // round-to-nearest, matches the wasm `convert_i64_s`). The target width is the node's type.
            Prim::FloatOfInt => {
                let idty = type_of(db, id);
                let rty = types::rust_type(&db.name_ctx(), &idty).ok_or_else(|| {
                    Reject::decline("of-int target has no native Rust representation")
                })?;
                let operand_s = emit(db, operand, env, ctx)?;
                Ok(format!("({operand_s} as {rty})"))
            }
            // A runtime float-WIDTH conversion `Float N.of` → an `as f64`/`as f32` cast: Rust's `as`
            // between floats demotes with rounding (f64→f32) / promotes exactly (f32→f64) / is the
            // identity (same width) — matching the wasm demote/promote. Target width is the node's type.
            Prim::FloatOf => {
                let idty = type_of(db, id);
                let rty = types::rust_type(&db.name_ctx(), &idty).ok_or_else(|| {
                    Reject::decline("of target has no native Rust representation")
                })?;
                let operand_s = emit(db, operand, env, ctx)?;
                Ok(format!("({operand_s} as {rty})"))
            }
            _ => Err(Reject::decline("not a runtime conversion")),
        },
        // A boolean negation `!operand`.
        Core::Not { operand } => {
            let o = emit(db, operand, env, ctx)?;
            Ok(format!("(!{o})"))
        }
        // A runtime call → `callee(args…)`. The callee is a reachable definition every backend emits
        // (`layout::compute` closed the reachable set over `Core::Call`), rendered as its own `fn` (a
        // `pub fn` for an export, a private `fn` otherwise) — so a call names it by its source name,
        // whether or not it is exported. Each argument is GROUNDED to the callee's corresponding
        // parameter width: a bare literal arg (`(f 1)`) defaults to Int64 on its own, so a narrow
        // parameter would otherwise get an `i64` literal (the same width mismatch the operand fix
        // addressed) — read the callee's param types and ground each literal arg to its position's type.
        Core::Call { callee, args } => {
            let name = db.defs[callee].name.clone();
            if name.is_empty() {
                return Err(Reject::decline("call to an unnamed definition"));
            }
            let param_tys = crate::layout::def_params(db, callee);
            let mut rendered = Vec::new();
            for (i, &a) in args.iter().enumerate() {
                // Ground a literal arg to the callee's param type at this position; a non-literal arg,
                // or a position past the known params (arity is checked upstream), emits as-is.
                match param_tys.get(i).map(|(_, t)| t) {
                    Some(Ty::Int(it)) => rendered.push(emit_grounded(db, a, *it, env, ctx)?),
                    // A COLLECTION param (Set/Map/List) with a fully-concrete element: thread it as the
                    // arg's EXPECTED type, so an empty `Set.of (list)`/`Map.empty` arg annotates from the
                    // param's element type rather than the default-grounded `i64` (breaker: an empty-Set at
                    // a call-arg with a declared Float64 elem emitted `BTreeSet<i64>` ≠ the param's
                    // `BTreeSet<__CdzF64>` → E0308). Only when the param type has no free var — else there
                    // is nothing better than the node's own type.
                    Some(pt @ (Ty::Set(_) | Ty::Map(_, _) | Ty::List(_))) if !pt.has_free_var() => {
                        let mut arg_ctx = ctx.clone();
                        arg_ctx.expected_ty = Some(pt.clone());
                        rendered.push(emit(db, a, env, &arg_ctx)?);
                    }
                    _ => rendered.push(emit(db, a, env, ctx)?),
                }
            }
            // The callee ident via `fn_ident` — the SAME uniqued name its declaration emits, so a call to a
            // β-copied do-local worker names ITS copy (`fac_7`), not a sibling copy's identically-named fn.
            let ident = super::fn_ident(db, ctx.layout, callee);
            if ctx.mode.is_async() {
                // Async/gas mode: thread `env` as the callee's first argument. The call is wrapped in
                // `Box::pin(…).await` ONLY when the callee's async future is SELF-REFERENTIAL — a
                // RECURSIVE callee (it transitively calls back to itself) that the backend did NOT
                // loop-transform. Rust sizes an `async fn`'s future at compile time; a recursive future
                // is infinite and MUST be pinned at the recursive call, but a NON-recursive callee's
                // future is finite and needs no box, and a LOOP-TRANSFORMED recursive callee is finite
                // too (its self-call became a `continue`, not an awaited future). Boxing EVERY call — the
                // prior behaviour — over-allocated on every leaf/helper/loop call (operator: "boxing
                // functions a lot that it doesn't need to be … only box on truly recursive async
                // functions where we couldn't transform it into a loop"). `env` is the shared gas/yield
                // cell each call reborrows.
                let needs_pin = call_needs_pin(db, callee);
                //
                // A NESTED async call — one whose result is an ARGUMENT to this call (`cnt(env, mk(env,
                // k).await)`) — would borrow `env` mutably TWICE at once: Rust reborrows `env` for the
                // OUTER call's first arg and holds it while evaluating the second arg, which reborrows
                // `env` again for the inner call (E0499 "borrow `*env` as mutable more than once"). A
                // sibling pair (two calls as separate operands of one op) is fine — those borrows are
                // sequential — but an argument-nested call is not. So HOIST any argument that itself
                // contains an `.await` into a `let` evaluated BEFORE this call: each hoisted call's `env`
                // reborrow completes (its `.await` releases it) before the next statement, so no two are
                // ever live together. Args with no `.await` (scalars, field reads) stay inline.
                let needs_hoist = rendered.iter().any(|a| a.contains(".await"));
                // Render `<callee>(<env>, <args>).await`, wrapped in a fully-qualified
                // `::std::boxed::Box::pin(…)` ONLY when `needs_pin` (a recursive, non-loop callee). The
                // `Box` path is fully qualified so a user sum named `Box` cannot shadow it.
                let call_of = |args: String| -> String {
                    if needs_pin {
                        format!("::std::boxed::Box::pin({ident}({args})).await")
                    } else {
                        format!("{ident}({args}).await")
                    }
                };
                if needs_hoist {
                    let mut binds = String::new();
                    let mut call_args = Vec::with_capacity(rendered.len());
                    for (i, a) in rendered.iter().enumerate() {
                        if a.contains(".await") {
                            let tmp = format!("__aarg{i}");
                            binds.push_str(&format!("let {tmp} = {a}; "));
                            call_args.push(tmp);
                        } else {
                            call_args.push(a.clone());
                        }
                    }
                    let env_param = super::ENV_PARAM;
                    let args = if call_args.is_empty() {
                        env_param.to_string()
                    } else {
                        format!("{env_param}, {}", call_args.join(", "))
                    };
                    Ok(format!("{{ {binds}{} }}", call_of(args)))
                } else {
                    let env_param = super::ENV_PARAM;
                    let args = if rendered.is_empty() {
                        env_param.to_string()
                    } else {
                        format!("{env_param}, {}", rendered.join(", "))
                    };
                    Ok(call_of(args))
                }
            } else {
                Ok(format!("{ident}({})", rendered.join(", ")))
            }
        }
        // A runtime TUPLE → Rust's native tuple literal `(e0, e1, …)`. Each element is rendered
        // recursively (a scalar or a nested tuple), so a tuple of scalars and a nested tuple both
        // compose directly. The native-aggregate value strategy: a Cadenza tuple IS a Rust tuple, no
        // heap handle (unlike the wasm backend's `arr-alloc`). A 1-tuple needs the trailing comma
        // `(e,)` to be a tuple rather than a parenthesized expression.
        Core::Tuple { elems } => {
            // The tuple's SOLVED element types — used to GROUND an empty-list element whose own `type_of`
            // left its element unsolved (`List ?` → a bare `vec![]` rustc can't infer, E0282). A tuple's own
            // type is resolved from context (e.g. an erased-newtype payload `(Seq (List Term) Term)` emits as
            // a plain tuple whose type is `(Vec<Term>, Term)`), so the element type IS spellable here.
            let elem_tys: Vec<Option<Ty>> = match type_of(db, id).strip_nominal() {
                Ty::Tuple(ts) if ts.len() == elems.len() => ts.iter().cloned().map(Some).collect(),
                _ => vec![None; elems.len()],
            };
            let mut parts = Vec::with_capacity(elems.len());
            for (k, &e) in elems.iter().enumerate() {
                parts.push(emit_elem_grounding_empty_list(
                    db,
                    e,
                    elem_tys.get(k).and_then(|o| o.as_ref()),
                    env,
                    ctx,
                )?);
            }
            let trailing = if parts.len() == 1 { "," } else { "" };
            Ok(format!("({}{trailing})", parts.join(", ")))
        }
        // A runtime RECORD → a Rust tuple literal in SORTED FIELD-NAME order — the SAME representation
        // as a tuple (a record is structural/anonymous; at run time it IS a positional array in sorted
        // key order). The `fields` `BTreeMap` iterates sorted, so its VALUES in order are the tuple
        // elements; the field names are compile-time-only (they became positions), re-appearing only in
        // the boundary render. A field read is a `Core::Proj` at the sorted index (handled below), so
        // this only builds. (Nominal records → a named Rust struct is a future refinement.)
        Core::Record { fields } => {
            // The record's SOLVED field types in sorted-key order (its Rust tuple positions) — to ground an
            // empty-list field to `Vec::<T>::new()` (see `emit_elem_grounding_empty_list`).
            let field_tys: Vec<Option<Ty>> = match type_of(db, id).strip_nominal() {
                Ty::Record(ts) if ts.len() == fields.len() => {
                    ts.values().cloned().map(Some).collect()
                }
                _ => vec![None; fields.len()],
            };
            let mut parts = Vec::with_capacity(fields.len());
            // `fields` (a `BTreeMap`) iterates in sorted key order — its values in order are the tuple
            // elements, matching the sorted-field positions `Ty::Record`/`Core::Proj` use.
            for (k, &v) in fields.values().enumerate() {
                parts.push(emit_elem_grounding_empty_list(
                    db,
                    v,
                    field_tys.get(k).and_then(|o| o.as_ref()),
                    env,
                    ctx,
                )?);
            }
            let trailing = if parts.len() == 1 { "," } else { "" };
            Ok(format!("({}{trailing})", parts.join(", ")))
        }
        // A runtime tuple/record PROJECTION `(. t i)` → Rust's tuple field access `(<operand>).index`.
        // The index is within the operand's static arity (checked before selection — for a record it is
        // the field's SORTED index, matching the `Core::Record` element order above), so it is always a
        // valid Rust field. Parenthesize the operand so a compound operand expression binds correctly.
        Core::Proj { operand, index } => {
            let t = emit(db, operand, env, ctx)?;
            // A projection reads a FIELD by reference-into-place; if that field's type is NON-COPY (a `Vec`,
            // or a compound holding one) and the projection result is used in more than one position, Rust
            // would MOVE the field out on the first by-value use and reject the rest (E0382) — the same
            // reason a non-Copy binding read clones. So clone a non-Copy projection result too (a Copy
            // field — the common scalar case — is read in place, byte-identical to before).
            if needs_clone_on_read(db, id) {
                Ok(format!("({t}).{index}.clone()"))
            } else {
                Ok(format!("({t}).{index}"))
            }
        }
        // A runtime LIST construction `(list e0 e1 …)` → the Rust `vec![e0, e1, …]` macro (an owned
        // `Vec<T>`, the native map for `List T`). Elements are lowered on demand; a homogeneous element
        // type, so no per-element boxing (unlike the wasm backend's typed `vec-push`). The empty list
        // `(list)` → `vec![]` (its element type comes from the surrounding annotation, which the emitted
        // `Vec<T>` signature fixes). A NEW `Vec` per construction — matching Cadenza's persistent
        // list value semantics.
        Core::ListNew { elems } => {
            // An EMPTY list `vec![]` has no element to infer its type from, so in a position that does not
            // fix the type (a sum/tuple payload whose structure is erased at the Rust level — e.g. a `(list)`
            // as the `Thm.Seq` hypothesis field) rustc reports E0282 "type annotations needed for `Vec<_>`".
            // Annotate the element type from the node's SOLVED `Ty::List(elem)` when it is representable —
            // `Vec::<T>::new()`. A non-empty list infers its element from the first element, so it stays the
            // bare `vec![…]` (byte-identical to before). An empty list whose element type is NOT representable
            // (a free var, a fn) emits the bare `vec![]` and relies on downstream inference as before (no
            // regression — the annotation is a pure ADD when we can spell the type).
            // ASYNC: a list of CLOSURES spells `Vec<Rc<dyn EnvClosure<A,R>>>`, so the element annotation uses
            // `async_closure_type` (via `async_or_rust_type`) — else a `Vec::<Rc<dyn Fn>>::new()` mismatches
            // the `Rc<dyn EnvClosure>` closures pushed in (E0308). Byte-identical for a closure-free element.
            if elems.is_empty()
                && let Ty::List(elem) = type_of(db, id).strip_nominal()
                && let Some(rust_elem) = super::async_or_rust_type(&db.name_ctx(), elem, ctx.mode)
            {
                return Ok(format!("Vec::<{rust_elem}>::new()"));
            }
            // FALLBACK for an empty `(list)` whose OWN `type_of` left its element UNSOLVED (`List ?`) — e.g.
            // an empty-list CALL ARGUMENT: the node's own type is `List ?`, but the caller threaded the
            // callee's param type into `ctx.expected_ty` (`Core::Call`'s List-param arm). Annotate from that
            // when it is a representable `List(elem)`. Without this a `(rev b (list))` emits `rev(…, vec![])`
            // whose `vec![]` rustc can't infer (E0282) — surfaced once breaker #18's `unused_braces` no-build
            // was fixed and the E0282 underneath became the failure. Mirrors `emit_elem_grounding_empty_list`
            // but keyed on the expected (param/slot) type rather than the node's own solved type.
            if elems.is_empty()
                && let Some(exp) = ctx.expected_ty.as_ref()
                && let Ty::List(elem) = exp.strip_nominal()
                && let Some(rust_elem) = super::async_or_rust_type(&db.name_ctx(), elem, ctx.mode)
            {
                return Ok(format!("Vec::<{rust_elem}>::new()"));
            }
            // An element's expected type is NOT this list's expected type — CLEAR `expected_ty` when
            // recursing into elements so a NESTED inner `(list)` (e.g. the empty `(list)` in `(list (list 1
            // 2) (list) …)` at expected `List(List Int64)`) does not inherit the OUTER list type and
            // wrongly ground to `Vec::<Vec<i64>>::new()` (the fallback above would misfire → E0308). A
            // non-empty element infers from its own contents; an inner empty list falls to its own solved
            // type (or a bare `vec![]` as before — no regression, and NOT mis-grounded to the outer type).
            let elem_ctx = if ctx.expected_ty.is_some() {
                let mut c = ctx.clone();
                c.expected_ty = None;
                Some(c)
            } else {
                None
            };
            let use_ctx = elem_ctx.as_ref().unwrap_or(ctx);
            // The list's SETTLED element WIDTH, if integer — a bare in-range literal element defaults its
            // OWN `type_of` to Int64 (`Nu64 as i64`), but the list is `Vec<ew>`, so a MIXED-width literal
            // list `(list (: 127 Int32) 32767)` emits `vec![(127u32 as i32), (32767u64 as i64)]` — a
            // HETEROGENEOUS `Vec` rustc rejects (E0308). The front-end + wasm UNIFY the element type (wasm
            // coerces both to Int32); the rust `vec!` must too. Ground each element to the list's element
            // width (`emit_grounded` renders a bare literal at that width, a no-op when already width-
            // carrying) — the List twin of the Map entry-key/value + Set-element sibling-width render
            // (corpus-bugfix, fuzzer cdz-smith differential). Only INTEGER elements ground; a non-int
            // element type leaves the bare emit (a wrong render would still error LOUD at rustc).
            // FLOAT twin of the integer element grounding (fuzzer cdz-smith wasm-vs-rust E0308): a bare
            // deferred-width float element defaults its OWN `type_of` to Float64 (`f64::from_bits(…)`), but a
            // homogeneous `(list 1.0 (: 2.0 Float32))` unifies to `List Float32` (inference settles the
            // sibling width), so a bare `1.0` next to an f32 sibling would emit `vec![<f64>, <f32>]` into a
            // `Vec<f32>` → rustc E0308. Ground each element to the list's SETTLED element width — Int via
            // `emit_grounded`, Float via `emit_grounded_float` (both no-ops when the element already carries
            // that width). The wasm side follows the settled width via `box_op_for`; the rust `vec!` must too.
            let (elem_it, elem_fw) = match type_of(db, id).strip_nominal() {
                Ty::List(elem) => container_slot_grounding(elem),
                _ => (None, None),
            };
            let mut parts = Vec::with_capacity(elems.len());
            for &e in elems.iter() {
                let part = if let Some(it) = elem_it {
                    emit_grounded(db, e, it, env, use_ctx)?
                } else if let Some(fw) = elem_fw {
                    emit_grounded_float(db, e, fw, env, use_ctx)?
                } else {
                    emit(db, e, env, use_ctx)?
                };
                parts.push(part);
            }
            Ok(format!("vec![{}]", parts.join(", ")))
        }
        // `List.len` → `<list>.len() as i64` (the result is an Int64). `.len()` is a `usize`; cast to the
        // machine `i64` a Cadenza length crosses as. Parenthesize the operand so a compound expression binds.
        Core::ListLen { operand } => {
            // A DIVERGING operand makes the `.len()` dead — the twin of `emit_arith`/`Compare`'s guard, for
            // the `.len()` receiver. A `List.len` of a provably-diverging list (`(List.len (g 7))` where `g`
            // always traps — e.g. a violated `@ensures` folds `g`'s body to `Core::Trap`) would otherwise
            // emit `(panic!("unreachable")).len()` — a method call on Rust's `!`, which E0599s ("no method
            // `len` for type `!`"). Cadenza evaluates the operand before `.len()`, so if it diverges the whole
            // expression is just that divergence — emit the operand alone (the `.len()` never runs).
            if arith_operand_diverges(db, operand) {
                return emit(db, operand, env, ctx);
            }
            let v = emit(db, operand, env, ctx)?;
            Ok(format!("(({v}).len() as i64)"))
        }
        // `List.push` → append `elem`, returning the NEW list (Cadenza lists are persistent; a `Vec` is
        // owned, so consume the operand into a `mut` local, push, and yield it — value semantics agree).
        Core::ListPush { list, elem } => {
            let l = emit(db, list, env, ctx)?;
            let e = emit(db, elem, env, ctx)?;
            Ok(format!("{{ let mut __v = {l}; __v.push({e}); __v }}"))
        }
        // `List.prepend` → insert `elem` at the FRONT, returning the NEW list (persistent semantics; the
        // front-growth twin of `List.push`). Consume the operand into a `mut` local and `insert(0, …)`.
        Core::ListPrepend { list, elem } => {
            let l = emit(db, list, env, ctx)?;
            let e = emit(db, elem, env, ctx)?;
            Ok(format!("{{ let mut __v = {l}; __v.insert(0, {e}); __v }}"))
        }
        // `List.concat` → the two lists joined in order (`lhs` then `rhs`). Consume `lhs` into a `mut`
        // local and `extend` it with `rhs`, returning it — one new `Vec`, order-preserving.
        Core::ListConcat { lhs, rhs } => {
            let a = emit(db, lhs, env, ctx)?;
            let b = emit(db, rhs, env, ctx)?;
            Ok(format!("{{ let mut __v = {a}; __v.extend({b}); __v }}"))
        }
        // `Map.merge(a, b)` → `BTreeMap::extend`, which overwrites with the RIGHT operand's values on an
        // overlapping key = last-writer / b-wins, matching the CHAMP `map-merge`. Both operands are moved
        // (consumed), mirroring `List.concat`. A non-Ord (float-carrying) key has no `BTreeMap` rep and
        // declines, exactly as `Map.insert` does. (v-rust-backend refines the float-key wrapper / verify.)
        Core::MapMerge { lhs, rhs } => {
            let kt = match crate::infer::type_of(db, id).strip_nominal() {
                Ty::Map(mk, _) => (**mk).clone(),
                _ => crate::ty::Ty::Any,
            };
            if !types::ty_is_ord_key(db, &kt) {
                return Err(Reject::decline(
                    "a Map.merge over a non-Ord key (a float-carrying key) has no BTreeMap rep on the Rust backend",
                ));
            }
            let a = emit(db, lhs, env, ctx)?;
            let b = emit(db, rhs, env, ctx)?;
            Ok(format!("{{ let mut __m = {a}; __m.extend({b}); __m }}"))
        }
        // `List.update` → replace the element at `index`, returning the NEW list; an out-of-bounds index
        // TRAPS (Cadenza `List.update` traps OOB, `value-heap-runtime.md`). The index is an Int64 occurrence
        // cast to `usize` (a negative index or `>= len` → the trap). KEYSTONE: TRAP KIND: the wasm runtime's
        // `List.update` OOB is a GENERIC `unreachable` abort (the runtime op traps message-lessly under
        // `panic = abort`), which the corpus grades `(trap "unreachable")` — NOT a bounds-specific string.
        // So panic `"unreachable"` (whose `trap_kind` is `unreachable`) to AGREE with wasm; `"index out of
        // bounds"` would classify `out-of-bounds`, a KIND MISMATCH the gate reads as a still-todo differential
        // (the trap fired, wrong kind). One op's trap = one canonical kind across both backends (tick-57).
        // On a 64-bit host `usize` does NOT wrap a 2^32 index (the wasm i32-wrap hazard the corpus warns of
        // has no rust analogue — `(2^32) as usize` stays `2^32 >= len` → traps correctly).
        Core::ListUpdate { list, index, elem } => {
            let l = emit(db, list, env, ctx)?;
            let i = emit(db, index, env, ctx)?;
            let e = emit(db, elem, env, ctx)?;
            Ok(format!(
                "{{ let mut __v = {l}; let __i = ({i}) as usize; \
                 if __i >= __v.len() {{ panic!(\"unreachable\") }} __v[__i] = {e}; __v }}"
            ))
        }
        // `List.at` → the FALLIBLE indexed read, yielding a built-in `Option` (which maps to Rust's OWN
        // `Option<T>` — the harness renders it). In range → `Some(<element>.clone())` (the runtime `vec-get`
        // BORROWS, so the `Some` payload owns an independent clone), else `None`. The index is a scalar cast
        // to `usize` (a negative index wraps huge → `>= len` → `None`, never a panic — `List.at` is total).
        // `disc_some`/`disc_none` are the wasm discriminants, irrelevant on the native-`Option` rust path.
        Core::ListAt { list, index, .. } => {
            // BOUNDED-INDEX (below-len) FACET — both-backend parity with wasm's `List.at` bounds elision.
            // When the index is flow-known `< len(this list)` (an enclosing `(< i (List.len xs))` guard) AND
            // provably NON-NEGATIVE, the `__i < __v.len()` check is redundant → emit the unconditional read.
            // BOTH conditions are required here: `__i` is `({i}) as usize`, so a NEGATIVE index wraps to a
            // huge `usize` that the length check would (correctly) reject — eliding the check on a possibly-
            // negative index would index-panic. The wasm side elides each half independently; the rust cast
            // couples them, so parity means the fully-in-range case (the common `(< i (len)) & (i >= 0)`).
            // Keyed on COLLECTION IDENTITY (`index_provably_below_len` matches the accessed list's binder).
            let below_len = crate::lower::index_provably_below_len(db, index, list)
                && crate::lower::value_provably_nonneg(db, index);
            let l = emit(db, list, env, ctx)?;
            let i = emit(db, index, env, ctx)?;
            if below_len {
                Ok(format!(
                    "{{ let __v = {l}; let __i = ({i}) as usize; Some(__v[__i].clone()) }}"
                ))
            } else {
                Ok(format!(
                    "{{ let __v = {l}; let __i = ({i}) as usize; \
                     if __i < __v.len() {{ Some(__v[__i].clone()) }} else {{ None }} }}"
                ))
            }
        }
        // MAP construction `(map (k v) …)` → a `BTreeMap` built by inserting each entry in SOURCE ORDER (a
        // later duplicate key overwrites — `BTreeMap::insert` does exactly that, matching the runtime).
        Core::MapNew { entries, .. } => {
            // `BTreeMap<K,V>` needs `K: Ord` — a FLOAT key declines (only `PartialOrd`; see `SetOf`).
            // Check the first entry's KEY node type (concrete here); an EMPTY map has no key to inspect
            // and only fails once an entry is inserted — caught by the `MapInsert` guard.
            // A bare `Float` key is OK — it keys via `CdzF64` (`ty_is_ord_key`); a float-carrying compound
            // key still declines.
            if let Some(&(k0, _)) = entries.first()
                && let kt = type_of(db, k0)
                && !types::ty_is_ord_key(db, &kt)
            {
                return Err(Reject::decline(
                    "a Map with a non-Ord key (a float-carrying key, or a Bytes key whose order the spec does not bless) has no BTreeMap rep on the Rust backend",
                ));
            }
            // The map's SETTLED key/value widths — a bare in-range literal entry key/value defaults its own
            // `type_of` to Int64 (`Nu64 as i64`), but the map is `BTreeMap<kw, vw>`, so an unannotated entry
            // renders at the wrong width (a chained `Map.insert` folds to this `MapNew`, so `(30)` into a
            // `BTreeMap<i64, u8>` value slot rendered `(30u64 as i64)` → E0308). Ground each entry key/value
            // to the map's key/value type (`emit_grounded` renders a bare literal at that width, no-op when
            // already width-carrying) — the Map twin of the Set-element / list-element sibling-width render
            // (adv-68, v-inference: the emit half of #1780's CDZ0302 range check).
            // Both Int AND Float widths (the float twin closes the same wasm-vs-rust E0308 the list-element
            // fix does: a bare deferred-width Float key/value next to an f32-settled sibling would emit an
            // f64 into an f32-typed `BTreeMap` slot). `emit_grounded`/`emit_grounded_float` are no-ops when
            // the entry already carries the settled width.
            let (key_it, key_fw) = match type_of(db, id).strip_nominal() {
                Ty::Map(mk, _) => container_slot_grounding(mk),
                _ => (None, None),
            };
            let (val_it, val_fw) = match type_of(db, id).strip_nominal() {
                Ty::Map(_, mv) => container_slot_grounding(mv),
                _ => (None, None),
            };
            // The map's SETTLED key type — `wrap_ord_key` must wrap by THIS (a `__CdzF32`/`__CdzF64` Ord
            // shell for a float key), NOT the individual entry key's own `type_of`: a bare deferred-width
            // float key (`2.0` beside a `(: 1.0 Float32)` sibling) defaults its `type_of` to Float64, so
            // wrapping by it emitted `__CdzF64::new(<f32>)` into a `BTreeMap<__CdzF32, _>` (operator ruling:
            // SUPPORT float map keys like float Sets; the E0605/E0308 map-KEY bug). Fall back to the entry's
            // own type only when the map key type is unresolved.
            let map_key_ty: Option<Ty> = match type_of(db, id).strip_nominal() {
                Ty::Map(mk, _) => Some((**mk).clone()),
                _ => None,
            };
            let mut lines = String::new();
            for (k, v) in entries.iter() {
                let ke = if let Some(it) = key_it {
                    emit_grounded(db, *k, it, env, ctx)?
                } else if let Some(fw) = key_fw {
                    emit_grounded_float(db, *k, fw, env, ctx)?
                } else {
                    emit(db, *k, env, ctx)?
                };
                let kt = match map_key_ty.as_ref() {
                    Some(t) => t.clone(),
                    None => type_of(db, *k),
                };
                let ke = wrap_ord_key(&db.name_ctx(), ke, &kt);
                let ve = if let Some(it) = val_it {
                    emit_grounded(db, *v, it, env, ctx)?
                } else if let Some(fw) = val_fw {
                    emit_grounded_float(db, *v, fw, env, ctx)?
                } else {
                    emit(db, *v, env, ctx)?
                };
                lines.push_str(&format!("__m.insert({ke}, {ve}); "));
            }
            // ANNOTATE `__m` with the node's `BTreeMap<K,V>` type. When the node maps concretely, spell it
            // directly. When it does NOT — an `Map.empty` whose K/V are still unsolved VARS at this node
            // (its type is fixed only by DOWNSTREAM use, e.g. an empty-Map HANDLER STATE whose K/V are
            // pinned through the get/put effect ops, NOT at the construction site) — GROUND the open vars
            // to the default (`Int64`) and annotate with that. A bare `BTreeMap::new()` there is
            // uninferrable when the only use is a `.get()` (which can't fix K/V from the map alone) → rustc
            // E0282; the grounded annotation gives the common integer-typed shape a concrete type. If the
            // map is genuinely used at a non-default element type reachable only through unsolved vars, the
            // grounded annotation is WRONG and rustc errors LOUDLY at `new()` (a build failure graded todo),
            // never a silent miscompile — strictly safer than the bare `new()` that E0282s for every open
            // case. (Earlier this was a bare `new()`, which E0282'd an empty-Map handler state — a
            // Todo→Fail regression once the effects inline/hoist fix made that shape reach the rust emit.)
            let map_ty = type_of(db, id);
            // In ASYNC mode a Map VALUE that is a closure spells `Rc<dyn EnvClosure<A,R>>`, so the `__m`
            // annotation must use `async_closure_type` (via `async_or_rust_type`) — else a
            // `BTreeMap<K, Rc<dyn Fn>>` annotation mismatches the `Rc<dyn EnvClosure>` values inserted (a
            // closure-as-map-value case: E0308). A closure-free map annotation is byte-identical to `rust_type`.
            let ncx = db.name_ctx();
            let map_type_str =
                |t: &Ty| -> Option<String> { super::async_or_rust_type(&ncx, t, ctx.mode) };
            let ann = if ctx.map_typed_by_enclosing_insert {
                // The enclosing `.insert`/`.remove` will fix K/V — a bare `new()` infers, and an annotation
                // would OVER-CONSTRAIN (grounding an open var here clashes with a Rational/String/Bytes key
                // the insert actually uses → E0308). Leave it unannotated; rustc reads the types from the use.
                match map_type_str(&map_ty) {
                    Some(t) => format!(": {t}"),
                    None => String::new(),
                }
            } else if map_ty.has_free_var()
                && let Some(exp @ Ty::Map(ek, _)) = &ctx.expected_ty
                && !ek.has_free_var()
            {
                // An empty `Map.empty` whose OWN type is open but whose KEY is fixed by the consuming
                // context (`ek` solved), threaded here as `expected_ty`. Two shapes, one render:
                //   (a) a CALL-ARG whose callee param is a concrete `Map` (value fully solved) — annotate it
                //       directly (the Set-twin of the empty-Set-at-call-arg E0308 fix);
                //   (b) a GET-ONLY lookup whose VALUE is fixed only by the downstream match-join, which may
                //       leave a free INTERIOR (a nested `List Any` — breaker ms9-family ms13/ns1/ej*).
                // Prefer the MODE-AWARE `map_type_str` (spells an async closure map VALUE as
                // `Rc<dyn EnvClosure>` — a call-arg map of closures) — it succeeds when the value is fully
                // solved (both shapes (a) and every concrete case). It DECLINES (None) on a free `Ty::Any`/
                // `Var` interior (shape (b), a nested match-join `List Any`); THERE fall to `rust_type_holes`,
                // which renders the solved OUTER shape (`Vec`/`BTreeSet`) with interior free vars as inference
                // HOLES `_` — the outer shape satisfies rustc method resolution while `_` lets rustc solve the
                // interior from the use (ms9 scalar → i64; ms13 → `Vec<i64>`; ns1 nested → `Vec<Vec<i64>>`).
                // The old code grounded a free interior to the DEFAULT `i64`, under-approximating a nested
                // value (`List Any` → wrongly `Vec<i64>` → E0308 at `.push(vec![..])`) — the miscompile-CLASS
                // this fixes. Sound: a get-only empty-map lookup always MISSES (the `Some` arm is dead), and a
                // concrete call-arg value is exact — a wrong OUTER shape errors LOUD at rustc, never silent.
                match map_type_str(exp).or_else(|| types::rust_type_holes(&ncx, exp)) {
                    Some(t) => format!(": {t}"),
                    None => String::new(),
                }
            } else {
                // No enclosing insert to infer from (get-only / pass-through — e.g. an empty-Map handler
                // state): GROUND the open vars so the annotation is spellable (else E0282). See
                // `ground_open_vars` for why grounding to the default is safe (a wrong ground → loud rustc
                // error, never a silent miscompile).
                match map_type_str(&types::ground_open_vars(&map_ty)) {
                    Some(t) => format!(": {t}"),
                    None => String::new(),
                }
            };
            Ok(format!(
                "{{ let mut __m{ann} = std::collections::BTreeMap::new(); {lines}__m }}"
            ))
        }
        // `Map.insert` → add-or-replace, returning the NEW map (persistent → consume into a `mut` local).
        Core::MapInsert { map, key, val, .. } => {
            // `BTreeMap<K,V>` needs `K: Ord` — a float key declines (the key node type is concrete even
            // when the base map is empty, the Map twin of the empty-Set float-insert case).
            let kt = type_of(db, key);
            if !types::ty_is_ord_key(db, &kt) {
                return Err(Reject::decline(
                    "a Map with a non-Ord key (a float-carrying key, or a Bytes key whose order the spec does not bless) has no BTreeMap rep on the Rust backend",
                ));
            }
            // The base map's element types are fixed by THIS `.insert(k, v)` — flag it so an empty
            // `Map.empty` base emits a bare `BTreeMap::new()` (inferred) rather than a grounded annotation
            // that could over-constrain the key type (a Rational/String/Bytes key → E0308).
            let mut map_ctx = ctx.clone();
            map_ctx.map_typed_by_enclosing_insert = true;
            let m = emit(db, map, env, &map_ctx)?;
            // The map's SETTLED key/value widths — a bare in-range literal key/value defaults its own
            // `type_of` to Int64 (`Nu64 as i64`), but the map is `BTreeMap<kw, vw>`, so an unannotated
            // key/value renders at the wrong width (`__m.insert((2u64 as i64), (30u64 as i64))` into
            // `BTreeMap<i64, u8>` → the `u8` value slot gets an `i64` → E0308). Ground each to the map's
            // key/value type (`emit_grounded` renders a bare literal at that width, no-op when already
            // width-carrying) — the Map twin of the Set-element / list-element sibling-width render (adv-68).
            let (key_it, val_it): (Option<IntTy>, Option<IntTy>) =
                match type_of(db, id).strip_nominal() {
                    Ty::Map(mk, mv) => {
                        let ki = match mk.strip_nominal() {
                            Ty::Int(it) => Some(*it),
                            _ => None,
                        };
                        let vi = match mv.strip_nominal() {
                            Ty::Int(it) => Some(*it),
                            _ => None,
                        };
                        (ki, vi)
                    }
                    _ => (None, None),
                };
            let k = match key_it {
                Some(it) => emit_grounded(db, key, it, env, ctx)?,
                None => emit(db, key, env, ctx)?,
            };
            let k = wrap_ord_key(&db.name_ctx(), k, &kt);
            let v = match val_it {
                Some(it) => emit_grounded(db, val, it, env, ctx)?,
                None => emit(db, val, env, ctx)?,
            };
            Ok(format!(
                "{{ let mut __m = {m}; __m.insert({k}, {v}); __m }}"
            ))
        }
        // `Map.lookup` → the fallible keyed read → Rust's own `Option`: `BTreeMap::get` borrows, returns
        // `Option<&V>`; `.cloned()` gives an owned `Option<V>` (the harness renders a native Option).
        Core::MapLookup { map, key, .. } => {
            // An inlined empty `Map.empty` map operand (a `let`-bound `Map.empty` β-substituted to here) has
            // its OWN `type_of` = `Map(Var, Var)` — fully OPEN, because inference fixes K/V only through this
            // lookup's key + the DOWNSTREAM match arms, not at the construction node. Emitting it grounds to
            // the DEFAULT `BTreeMap<i64,i64>`, but the use is `.get(&"k".to_string())` (String key) → a Rust
            // TYPE ERROR (E0308) at the lookup: the wrong default-ground annotation mismatches the String key,
            // so the artifact FAILS TO BUILD. A backend DIFFERENTIAL, not a runtime miscompile — wasm folds
            // the String key + runs correct (breaker ms9). RECONSTRUCT the map's type at this
            // lookup from the SOLVED evidence — the KEY's type (`key_ty`, here String) + this lookup's RESULT
            // `Option<V>` payload (the map's value type) — and thread it as `expected_ty`. The KEY is solved
            // here (it IS the lookup key); the VALUE may still be a free var (fixed only by the downstream
            // match-join — a List/Set/Map value, breaker ms9-family ms13/ms6/ns1/ej*). We do NOT ground a
            // free value: `Core::MapNew` annotates the SOLVED key and leaves a free value an INFERENCE HOLE
            // `_` (below), so rustc solves it from the use (ms9: value=i64; ms13: value=Vec<i64> via `.push`).
            // Grounding a free value to the DEFAULT (i64) was wrong for a collection value (E0308 on the
            // `Some`-arm vs the `None => vec![]` arm) — the miscompile-CLASS this reconstruction now fixes.
            // Only when the map operand's own type is OPEN (a concrete map needs no hint) and key is concrete.
            let key_ty = type_of(db, key);
            let map_ty = type_of(db, map);
            // Reconstruct ONLY when the lookup result is EXACTLY `Option<V>` (the map's value type = V). If
            // it deviates (a prior lowering/inference bug produced a non-`Option` result here), do NOT
            // fabricate a var / a misleading `expected_ty` — SKIP reconstruction and fall through to the
            // prior path, so the unexpected shape surfaces (fails LOUD at rustc) rather than being papered
            // over by a wrong annotation. This is a MISCOMPILE fix, so a fail-loud floor is the safe default
            // (github-liaison/Copilot #2456: the `_ => Ty::Var(0)` fallback could mask a real bug).
            let val_ty = match type_of(db, id).strip_nominal() {
                Ty::Sum { decl, args, .. }
                    if args.len() == 1
                        && db
                            .type_decl_by_occ(*decl)
                            .is_some_and(|d| d.name == "Option") =>
                {
                    Some(args[0].clone())
                }
                _ => None,
            };
            let reconstructed = if map_ty.has_free_var() && !key_ty.has_free_var() {
                match val_ty {
                    // Result deviated from `Option<V>` — fail-loud, do NOT reconstruct (see above).
                    None => None,
                    Some(v) => {
                        // The map's VALUE type. Prefer the SOLVED lookup-result payload `V`. When `V` is
                        // still FREE (inference fixes it only through the DOWNSTREAM match-join, invisible at
                        // this node — breaker ms9-family), use the consuming match's RESULT type, threaded
                        // here as `expected_ty` by `Core::MatchSum`: the `Some` arm returns the payload into
                        // that join, so the join type IS the value type. For ms9 the join is `Int64` (→ the
                        // existing `BTreeMap<String, i64>`); for ms13/ms6/ns1/ej* it is a COLLECTION (`List`/
                        // `Set`) → `BTreeMap<String, Vec<_>>`, which the old `ground_open_vars(free V)`→
                        // default `i64` got WRONG (the `Some`-arm `i64` vs the `None => vec![]` `Vec`
                        // mismatched: E0308 — the miscompile-CLASS this now fixes). The join's OUTER shape is
                        // solved; an INTERIOR element may still be free (the join under-approximates a NESTED
                        // value — `(list (list n))` gives the join only `List Any`, truly `List (List i64)`),
                        // so we render interior free vars as inference HOLES `_` in `MapNew` (the outer shape
                        // satisfies rustc method resolution; the `_` interior is solved from the actual use).
                        // Sound: a `Map.empty` lookup always MISSES, so the `Some` arm is dead — a wrong outer
                        // shape errors LOUD at rustc, never a runtime miscompile. A bare `Var`/`Any` hint
                        // carries no shape → fall back to the free payload (grounded → the get-only default).
                        let v_eff = if v.has_free_var() {
                            ctx.expected_ty
                                .as_ref()
                                .filter(|t| !matches!(t, Ty::Var(_) | Ty::Any))
                                .cloned()
                                .unwrap_or(v)
                        } else {
                            v
                        };
                        // Do NOT ground here — a free interior stays free so `MapNew` renders it as a `_`
                        // hole. (A fully-solved `v_eff` — the ms9 scalar / a solved collection — is untouched.)
                        Some(Ty::Map(Box::new(key_ty.clone()), Box::new(v_eff)))
                    }
                }
            } else {
                None
            };
            let m = if let Some(exp) = reconstructed {
                let mut c = ctx.clone();
                c.expected_ty = Some(exp);
                emit(db, map, env, &c)?
            } else {
                emit(db, map, env, ctx)?
            };
            let k = emit(db, key, env, ctx)?;
            // Wrap a bare-float lookup key in `CdzF64::new` to match the map's `CdzF64` key type (and
            // NaN-canonicalize — a differently-produced NaN finds the stored entry, the corpus case).
            let kt = type_of(db, key);
            let k = wrap_ord_key(&db.name_ctx(), k, &kt);
            Ok(format!("({m}).get(&({k})).cloned()"))
        }
        // `Map.remove` → drop the key, returning the new map (removing an absent key is total, `remove`
        // just returns the prior value which we discard). Persistent → consume into a `mut` local.
        Core::MapRemove { map, key, .. } => {
            // The base map's element types are fixed by THIS `.remove(&k)` (the key type) — flag it so an
            // empty base emits a bare inferred `new()` rather than an over-constraining grounded annotation.
            let mut map_ctx = ctx.clone();
            map_ctx.map_typed_by_enclosing_insert = true;
            let m = emit(db, map, env, &map_ctx)?;
            let k = emit(db, key, env, ctx)?;
            let kt = type_of(db, key);
            let k = wrap_ord_key(&db.name_ctx(), k, &kt);
            Ok(format!("{{ let mut __m = {m}; __m.remove(&({k})); __m }}"))
        }
        // `Map.len` (the node is `MapSize`) → the distinct-key count as `Int64`.
        Core::MapSize { map } => {
            // A diverging operand makes the `.len()` dead (see `Core::ListLen`) — emit it alone, else
            // `(panic!(…)).len()` E0599s on `!`.
            if arith_operand_diverges(db, map) {
                return emit(db, map, env, ctx);
            }
            let m = emit(db, map, env, ctx)?;
            Ok(format!("(({m}).len() as i64)"))
        }
        // `Map.to-list` → a `List (Tuple k v)` in CANONICAL KEY order — a `BTreeMap` iterates sorted, so a
        // plain `.iter()` gives that order; clone each key/value into an owned `(K, V)` tuple → `Vec<(K,V)>`.
        Core::MapToList { map, .. } => {
            // A float-CARRYING COMPOUND KEY has no blessed total order (03:626 §319) → the ordered
            // key-enumeration DECLINES, matching wasm. A BARE float key still enumerates (canonical bytes).
            // Map construction/lookup over such a key still work (the guard is HERE, at to-list, not insert).
            if let Ty::Map(ref k, _) = type_of(db, map)
                && is_float_carrying_compound(k)
            {
                return Err(Reject::decline(
                    "Map.to-list over a float-carrying compound key declines: a compound containing a float leaf has no blessed total order (matching wasm; the map itself + lookup/remove still work, only the ordered enumeration is undefined)",
                ));
            }
            let m = emit(db, map, env, ctx)?;
            // A float-KEYED map iterates `CdzF64` keys; the `List (Tuple Float64 V)` key element is a bare
            // `f64`, so UNWRAP the key via `.get()`. The value is unaffected (a float VALUE stays `f64`).
            let map_ty = type_of(db, map);
            let key_is_float = matches!(
                map_ty,
                Ty::Map(ref k, _) if matches!(**k, Ty::Float(_))
            );
            let key_is_opt = matches!(
                map_ty,
                Ty::Map(ref k, _) if types::is_flip_order_option_key_shallow(&db.name_ctx(), k)
            );
            if key_is_float {
                Ok(format!(
                    "({m}).iter().map(|(__k, __v)| (__k.get(), __v.clone())).collect::<Vec<_>>()"
                ))
            } else if key_is_opt {
                // A `__CdzOpt` key (Clone, not Copy) unwraps via `.clone().get()` → the inner Option (#42).
                Ok(format!(
                    "({m}).iter().map(|(__k, __v)| (__k.clone().get(), __v.clone())).collect::<Vec<_>>()"
                ))
            } else {
                Ok(format!(
                    "({m}).iter().map(|(__k, __v)| (__k.clone(), __v.clone())).collect::<Vec<_>>()"
                ))
            }
        }
        // SET construction `(Set.of (list …))` → a `BTreeSet` built by inserting each element (duplicates
        // collapse at insert, matching the runtime dedup).
        Core::SetOf { elems, .. } => {
            // A `BTreeSet<T>` needs `T: Ord`. A FLOAT element is only `PartialOrd`, so a float (or
            // float-containing) element makes the set uninstantiable — DECLINE rather than emit an
            // uncompilable `BTreeSet<f64>` (the runtime orders a float set by canonical bytes; the Rust
            // backend has no total float order). Check the first ELEMENT node type (concrete here); an
            // EMPTY `Set.of (list)` has no element to inspect, and only fails once something is inserted —
            // caught by the `SetInsert` guard below.
            // A bare `Float` element is OK — it keys via the `CdzF64` wrapper (`ty_is_ord_key`). A
            // float-CARRYING compound element still declines (the wrapper isn't threaded through it).
            // ELEMENT-TYPE grounding is ORDER-INDEPENDENT: a Set is homogeneous, so all elements share ONE
            // element type, but `type_of` on a given node can be UNDER-GROUND (a `(Nil unit)` variant of
            // `(Box a)` reads as `Box <openvar>` while a sibling `(Full k)` reads as the solved `Box Int64`).
            // Check the BEST-grounded element (the first whose type `ty_is_ord_key` accepts), NOT `elems[0]`:
            // else `Set.of (list (Nil unit) (Full k))` (Nil first) declined "non-Ord" while `(Full k)` first
            // compiled — an order-dependent decline (breaker/v-inference, post-#1674, ground_open_vars class).
            // Decline ONLY when NO element grounds to an Ord-key type (a genuinely-open or float-carrying set).
            let elem_tys: Vec<Ty> = elems.iter().map(|&e| type_of(db, e)).collect();
            if !elem_tys.is_empty() && !elem_tys.iter().any(|et| types::ty_is_ord_key(db, et)) {
                return Err(Reject::decline(
                    "a Set with a non-Ord element (a float-carrying element, or a Bytes element whose order the spec does not bless) has no BTreeSet rep on the Rust backend",
                ));
            }
            // The set's SETTLED element type — a bare in-range literal element defaults its OWN `type_of`
            // to Int64 (`Nu64 as i64`), but the set is `BTreeSet<elem_width>`, so an unannotated element
            // renders at the wrong width (`__s.insert((41u64 as i64))` into `BTreeSet<u64>` → E0308). Ground
            // each element to the set's element type (`emit_grounded` renders a bare literal at that width and
            // is a no-op when the element already carries the width) — the Set twin of the list-element
            // sibling-width render (#1766) and the compound-slot literal grounding above (adv-68, v-inference).
            // Both Int AND Float element widths (the float twin closes the same wasm-vs-rust E0308 as the
            // list/map fix — a bare deferred-width float element beside an f32-settled sibling).
            let (set_elem_it, set_elem_fw) = match type_of(db, id).strip_nominal() {
                Ty::Set(elem) => container_slot_grounding(elem),
                _ => (None, None),
            };
            let mut lines = String::new();
            for e in elems.iter() {
                let ee = if let Some(it) = set_elem_it {
                    emit_grounded(db, *e, it, env, ctx)?
                } else if let Some(fw) = set_elem_fw {
                    emit_grounded_float(db, *e, fw, env, ctx)?
                } else {
                    emit(db, *e, env, ctx)?
                };
                // Wrap a bare-float element in `CdzF64::new` (the set's element type is `CdzF64`).
                let et = type_of(db, *e);
                let ee = wrap_ord_key(&db.name_ctx(), ee, &et);
                lines.push_str(&format!("__s.insert({ee}); "));
            }
            // ANNOTATE `__s` with the node's solved `BTreeSet<T>` type. When it maps concretely, spell it
            // directly. When the element type is still an unsolved VAR at this node (an empty `Set.of (list)`
            // whose element is fixed only by DOWNSTREAM use): if an enclosing `Set.insert`/`Set.remove` will
            // fix it, leave a bare `BTreeSet::new()` (rustc infers, and a grounded annotation would
            // OVER-CONSTRAIN — clashing with a String/Bytes/BigInt element the insert uses → E0308); else
            // (len-only / pass-through, the `(Set.len (Set.of (list)))` E0282 breaker found) GROUND the open
            // var to the default so the annotation is spellable — a bare `new()` there is uninferrable → rustc
            // E0282. A wrong ground → a LOUD rustc error at `new()` (a build failure graded todo), never a
            // silent miscompile — strictly safer than the bare `new()`. The exact twin of the empty-Map fix.
            let set_ty = type_of(db, id);
            let ncx = db.name_ctx();
            let ann = if ctx.set_typed_by_enclosing_insert {
                match types::rust_type(&ncx, &set_ty) {
                    Some(t) => format!(": {t}"),
                    None => String::new(),
                }
            } else if set_ty.has_free_var()
                && let Some(exp) = &ctx.expected_ty
                && matches!(exp, Ty::Set(_))
                && !exp.has_free_var()
            {
                // The node's element is unsolved here (an empty `Set.of (list)` at a CALL-ARG position),
                // but the consuming context (the callee's param type) FIXES it — annotate from the expected
                // `Set` type, not the default `i64` ground (which would clash with the param → E0308).
                match types::rust_type(&ncx, exp) {
                    Some(t) => format!(": {t}"),
                    None => String::new(),
                }
            } else {
                match types::rust_type(&ncx, &types::ground_open_vars(&set_ty)) {
                    Some(t) => format!(": {t}"),
                    None => String::new(),
                }
            };
            Ok(format!(
                "{{ let mut __s{ann} = std::collections::BTreeSet::new(); {lines}__s }}"
            ))
        }
        // `Set.contains` → the total membership predicate → a `bool` directly (unlike `Map.lookup`'s Option).
        Core::SetContains { set, elem, .. } => {
            let s = emit(db, set, env, ctx)?;
            let e = emit(db, elem, env, ctx)?;
            // Wrap a bare-float probe in `CdzF64::new` so it matches the set's `CdzF64` element type (and
            // NaN-canonicalizes — a NaN probe finds a stored NaN, the corpus's nan-membership case).
            let et = type_of(db, elem);
            let e = wrap_ord_key(&db.name_ctx(), e, &et);
            Ok(format!("({s}).contains(&({e}))"))
        }
        // `Set.insert`/`Set.remove` → the new set (persistent → consume into a `mut` local; insert of a
        // present element / remove of an absent one is a total no-op value).
        Core::SetInsert { set, elem, .. } => {
            // `BTreeSet<T>` needs `T: Ord` — a float element declines (see `SetOf`). The inserted
            // element's type is concrete here even when the base set is empty (the `Set.of (list)` /
            // float-insert miscompile: an empty base's element type is an unsolved var, but the insert
            // fixes it to the float). Check the element node type.
            let et = type_of(db, elem);
            if !types::ty_is_ord_key(db, &et) {
                return Err(Reject::decline(
                    "a Set with a non-Ord element (a float-carrying element, or a Bytes element whose order the spec does not bless) has no BTreeSet rep on the Rust backend",
                ));
            }
            // The base set's element type is fixed by THIS `.insert(e)` — flag it so an empty `Set.of (list)`
            // base emits a bare inferred `new()` rather than a grounded annotation that could over-constrain
            // the element (a String/Bytes/BigInt element → E0308). The `Map.insert` twin.
            let mut set_ctx = ctx.clone();
            set_ctx.set_typed_by_enclosing_insert = true;
            let s = emit(db, set, env, &set_ctx)?;
            let e = emit(db, elem, env, ctx)?;
            let e = wrap_ord_key(&db.name_ctx(), e, &et);
            Ok(format!("{{ let mut __s = {s}; __s.insert({e}); __s }}"))
        }
        Core::SetRemove { set, elem, .. } => {
            let et = type_of(db, elem);
            // The base set's element type is fixed by THIS `.remove(&e)` — flag it so an empty base emits a
            // bare inferred `new()` rather than an over-constraining grounded annotation (see `SetInsert`).
            let mut set_ctx = ctx.clone();
            set_ctx.set_typed_by_enclosing_insert = true;
            let s = emit(db, set, env, &set_ctx)?;
            let e = emit(db, elem, env, ctx)?;
            let e = wrap_ord_key(&db.name_ctx(), e, &et);
            Ok(format!("{{ let mut __s = {s}; __s.remove(&({e})); __s }}"))
        }
        // `Set.len` → the cardinality (deduped) as `Int64`.
        Core::SetLen { set } => {
            // A diverging operand makes the `.len()` dead (see `Core::ListLen`).
            if arith_operand_diverges(db, set) {
                return emit(db, set, env, ctx);
            }
            let s = emit(db, set, env, ctx)?;
            Ok(format!("(({s}).len() as i64)"))
        }
        // `Set.to-list` → a `List` in CANONICAL (sorted) order — `BTreeSet::iter` is sorted; clone each.
        // A float set iterates `CdzF64` (the wrapper), but the `List Float64` element is a bare `f64`, so
        // UNWRAP each via `.get()` (the wrapper→f64 read). The iteration order is the wrapper's `Ord` (by
        // canonical bits), matching the runtime's canonical-byte order for a float `Set.to-list`.
        Core::SetToList { set, .. } => {
            // A float-CARRYING COMPOUND element has no blessed total order (03:626 §319) → the ordered
            // enumeration DECLINES, matching wasm. A BARE float element still enumerates (canonical bytes).
            // Construction/lookup of such a set still work (the guard is HERE, at to-list, not at SetOf).
            if let Ty::Set(ref e) = type_of(db, set)
                && is_float_carrying_compound(e)
            {
                return Err(Reject::decline(
                    "Set.to-list over a float-carrying compound element declines: a compound containing a float leaf has no blessed total order (matching wasm; the set itself + contains/remove still work, only the ordered enumeration is undefined)",
                ));
            }
            let s = emit(db, set, env, ctx)?;
            let elem_ty = match type_of(db, set) {
                Ty::Set(e) => Some((*e).clone()),
                _ => None,
            };
            let elem_is_float = matches!(elem_ty, Some(Ty::Float(_)));
            // A `__CdzOpt`-wrapped Option element unwraps via `.get()` (the wrapper→Option read), exactly
            // like a float `__CdzF` element (#42 witness 2). The iteration order is the wrapper's declared
            // `Ord` (Some<None), matching the runtime's Set.to-list Option order. Only a BARE Option element
            // wraps (a nested Option-in-tuple element is a later face; the wrap+unwrap for that would need to
            // rebuild the tuple element-wise on read, as the float tuple does — deferred to keep this bounded).
            let elem_is_opt = elem_ty
                .as_ref()
                .map(|t| types::is_flip_order_option_key_shallow(&db.name_ctx(), t))
                .unwrap_or(false);
            if elem_is_float {
                // `__CdzF` is Copy → `__f.get()` reads the f64 (byte-identical to before).
                Ok(format!(
                    "({s}).iter().map(|__f| __f.get()).collect::<Vec<_>>()"
                ))
            } else if elem_is_opt {
                // `__CdzOpt` is Clone (not Copy) → clone the ref then `.get()` reads the inner Option.
                Ok(format!(
                    "({s}).iter().map(|__f| __f.clone().get()).collect::<Vec<_>>()"
                ))
            } else {
                Ok(format!("({s}).iter().cloned().collect::<Vec<_>>()"))
            }
        }
        // `Set.union`/`intersection`/`difference` → the binary set-algebra ops. Rust's `BTreeSet` methods
        // take a `&other` and yield an iterator of `&T`; clone + collect into a new `BTreeSet`. Both
        // operands are consumed (a NEW set is returned), matching the runtime's persistent semantics.
        Core::SetAlgebra { op, lhs, rhs } => {
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let method = match op {
                crate::core::SetAlgebraOp::Union => "union",
                crate::core::SetAlgebraOp::Intersection => "intersection",
                crate::core::SetAlgebraOp::Difference => "difference",
            };
            Ok(format!(
                "({l}).{method}(&({r})).cloned().collect::<std::collections::BTreeSet<_>>()"
            ))
        }
        // BYTES construction `(Bytes.of (list …))` → a `Vec<u8>`, each element an Int64 in 0..=255 with a
        // RUNTIME RANGE CHECK: an element `< 0` or `> 255` TRAPS (matching the wasm `bytes-set` range-check
        // + the constant fold's CDZ0304). The check runs before the `as u8` truncation so an out-of-range
        // value halts rather than silently wrapping.
        Core::BytesOf { elems } => {
            let mut lines = String::new();
            for e in elems.iter() {
                let ee = emit(db, *e, env, ctx)?;
                lines.push_str(&format!(
                    "{{ let __e = {ee}; if __e < 0 || __e > 255 {{ panic!(\"byte value out of range\") }} __b.push(__e as u8); }} "
                ));
            }
            Ok(format!(
                "{{ let mut __b: Vec<u8> = Vec::new(); {lines}__b }}"
            ))
        }
        // A baked byte-constant materializes directly as its `Vec<u8>` — the leaf twin of `BytesOf` (no
        // per-element sub-emit, the bytes are already known). Same runtime Bytes value a `BytesOf` of the
        // identical constant elements would build.
        Core::ConstBytes(bytes) => {
            let elems = bytes
                .iter()
                .map(|b| format!("{b}u8"))
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!("vec![{elems}]"))
        }
        // `Bytes.len` → the byte count as `Int64`.
        Core::BytesLen { operand } => {
            // A diverging operand makes the `.len()` dead (see `Core::ListLen`).
            if arith_operand_diverges(db, operand) {
                return emit(db, operand, env, ctx);
            }
            let v = emit(db, operand, env, ctx)?;
            Ok(format!("(({v}).len() as i64)"))
        }
        // `String.scalar-len` → the Unicode SCALAR (codepoint) count. A String value is a Rust `String`, so
        // `.chars().count()` is the scalar count (`.len()` would be the BYTE count — that is `BytesLen`).
        // The native twin of the wasm lead-byte-counting walk.
        Core::StrScalarLen { operand } => {
            // A diverging operand makes the `.chars().count()` dead (see `Core::ListLen`).
            if arith_operand_diverges(db, operand) {
                return emit(db, operand, env, ctx);
            }
            let v = emit(db, operand, env, ctx)?;
            Ok(format!("(({v}).chars().count() as i64)"))
        }
        // `Bytes.at` → the fallible byte read → native `Option`: a byte is a raw `u8` value zero-extended
        // to the `Int64` `Some` payload (unlike `List.at`, no clone — a `u8` is Copy).
        Core::BytesAt { bytes, index, .. } => {
            let v = emit(db, bytes, env, ctx)?;
            let i = emit(db, index, env, ctx)?;
            Ok(format!(
                "{{ let __v = {v}; let __i = ({i}) as usize; \
                 if __i < __v.len() {{ Some(__v[__i] as i64) }} else {{ None }} }}"
            ))
        }
        // `Bytes.concat` / `String.concat` → the two sequences joined (persistent → consume `lhs` into a
        // `mut` local, append `rhs`). A String is a UTF-8 `Bytes` leaf, so `String.concat` LOWERS to this
        // same node — but the emitted Rust differs by result type: a `String` appends with `push_str`
        // (`String::extend(String)` needs `IntoIterator`, which `String` is not — E0277), a `Vec<u8>`
        // appends with `extend`. Dispatch on the node's solved type.
        Core::BytesConcat { lhs, rhs } => {
            let a = emit(db, lhs, env, ctx)?;
            let b = emit(db, rhs, env, ctx)?;
            if matches!(type_of(db, id).strip_nominal(), Ty::String) {
                Ok(format!(
                    "{{ let mut __b = {a}; __b.push_str(&({b})); __b }}"
                ))
            } else {
                Ok(format!("{{ let mut __b = {a}; __b.extend({b}); __b }}"))
            }
        }
        // `Bytes.slice` → the fallible sub-range read → native `Option`. Guard `start >= 0 && len >= 0`
        // on the RAW i64 values BEFORE the `usize` cast (a negative would wrap to a huge `usize`), then
        // `start + len <= bytes-len` via a CHECKED add — `(start as usize) + (len as usize)` can OVERFLOW
        // usize (wrap to a small sum in release) for two near-`i64::MAX` operands, which would pass the
        // guard and then PANIC on the out-of-range index; `Bytes.slice` must be TOTAL (return `None`), so
        // `checked_add` maps the overflow to `None`. The computed `__end` is reused for the slice so it is
        // evaluated once and the range is exactly the guarded one.
        Core::BytesSlice {
            bytes, start, len, ..
        } => {
            let v = emit(db, bytes, env, ctx)?;
            let s = emit(db, start, env, ctx)?;
            let l = emit(db, len, env, ctx)?;
            Ok(format!(
                "{{ let __v = {v}; let __start = {s}; let __len = {l}; \
                 if __start >= 0 && __len >= 0 {{ \
                     match (__start as usize).checked_add(__len as usize) {{ \
                         Some(__end) if __end <= __v.len() => Some(__v[(__start as usize)..__end].to_vec()), \
                         _ => None, \
                     }} \
                 }} else {{ None }} }}"
            ))
        }
        // `Bytes.compact` → a content-equal sequence with independent storage. A `Vec<u8>` is already flat
        // and owned, so on the native rep this is a NO-OP: return the operand (the rope-flatten the wasm
        // runtime does has no analogue for a `Vec`).
        Core::BytesCompact { operand } => emit(db, operand, env, ctx),
        // Runtime `Blake3.of` on the RUST backend is not yet emitted: emitted programs do not link the
        // `blake3` crate (unlike the wasm path's `hash-blake3` heap op). DECLINE cleanly (a rust-baseline
        // TODO, never a miscompile) until the rust-emit blake3 dep lands (coordinated with v-rust-backend).
        // (Internal: the wasm backend covers these via heap ops 91/92/93; the Rust backend has no linked
        // analogue. The user message stays a clean "not supported on the Rust backend" statement.)
        Core::Blake3Of { .. } => Err(crate::diag::Reject::unsupported(
            "Blake3.of on a runtime Bytes is not supported on the Rust backend (available on the wasm backend)",
        )),
        Core::AstPrint { .. } => Err(crate::diag::Reject::unsupported(
            "Ast.print on a runtime Ast is not supported on the Rust backend (available on the wasm backend)",
        )),
        Core::AstEncode { .. } => Err(crate::diag::Reject::unsupported(
            "Ast.encode on a runtime Ast is not supported on the Rust backend (available on the wasm backend)",
        )),
        Core::AstDecode { .. } => Err(crate::diag::Reject::unsupported(
            "Ast.decode of a runtime byte sequence is not supported on the Rust backend (available on the wasm backend)",
        )),
        // `String.at`/`String.scalar-at` on a RUNTIME string → the i-th UNICODE SCALAR, fallibly, as a
        // one-scalar `(Option String)`. `.chars()` iterates by scalar value (matching the spec's
        // scalar-value addressing — NOT bytes), `.nth(i)` picks it, `.map(to_string)` wraps the scalar as
        // a one-scalar String → native `Option<String>`. A negative index → a huge `usize` → `nth` returns
        // None (total, matching the runtime's out-of-range → None). The char-payload variant is the same
        // slice at the source; here the result is always `(Option String)`.
        Core::StrAt { string, index, .. } => {
            let s = emit(db, string, env, ctx)?;
            let i = emit(db, index, env, ctx)?;
            Ok(format!(
                "{{ let __s = {s}; let __i = ({i}) as usize; __s.chars().nth(__i).map(|__c| __c.to_string()) }}"
            ))
        }
        // `String.scalar-at` on a RUNTIME string → the `index`-th Unicode SCALAR, fallibly, as a native
        // `Option<char>`. The Char-payload twin of `StrAt` (which yields `(Option String)`): `.chars()`
        // iterates by Unicode scalar (spec scalar-value addressing, NOT byte), `.nth(i)` reads the i-th and
        // is `None` past the end — and a Cadenza `Char` maps to a Rust `char` (see `Core::ConstChar`), so
        // `chars().nth(i)` IS the `Option<Char>` directly (no `.to_string()` box the String twin needs).
        // `disc_some`/`disc_none` are the wasm sum tags, irrelevant on the native-`Option` rust path.
        Core::StrScalarAt { operand, index, .. } => {
            let s = emit(db, operand, env, ctx)?;
            let i = emit(db, index, env, ctx)?;
            Ok(format!(
                "{{ let __s = {s}; let __i = ({i}) as usize; __s.chars().nth(__i) }}"
            ))
        }
        // `String.slice` on a RUNTIME string → the half-open SCALAR sub-range `[start, end)`, fallibly, as a
        // native `Option<String>`. `.chars()` iterates by Unicode scalar (matching the spec's scalar-value
        // addressing, NOT byte), collected once into a `Vec<char>` so the two bounds index the same scalar
        // sequence. Valid iff `0 <= start <= end <= scalar-len` (signed guard on the RAW i64s BEFORE the
        // `usize` cast — a negative bound would wrap to a huge `usize`); then the selected scalars re-collect
        // into a `String` (`start == end` → the empty string, `Some`, not `None`). Any out-of-range bound →
        // `None` (total, matching the runtime walk). The multi-scalar twin of `StrAt`.
        Core::StrSlice {
            string, start, end, ..
        } => {
            let s = emit(db, string, env, ctx)?;
            let a = emit(db, start, env, ctx)?;
            let b = emit(db, end, env, ctx)?;
            Ok(format!(
                "{{ let __cs: ::std::vec::Vec<char> = ({s}).chars().collect(); \
                 let __a = {a}; let __b = {b}; let __len = __cs.len() as i64; \
                 if __a >= 0 && __a <= __b && __b <= __len {{ \
                     Some(__cs[(__a as usize)..(__b as usize)].iter().collect::<String>()) \
                 }} else {{ None }} }}"
            ))
        }
        // `String.from-bytes` on a RUNTIME `Bytes` → the TOTAL UTF-8 decode `Bytes → (Option String)`.
        // Rust's `String::from_utf8` performs EXACTLY the strict validation the runtime `str-from-bytes`
        // does — rejecting invalid bytes, overlong encodings, AND surrogate code points (the three spec
        // failure modes) — and `.ok()` maps the `Result` to `Option<String>` (None on failure, never a
        // trap). Consumes the `Vec<u8>` (matching the runtime's consume).
        Core::StrFromBytes { bytes, .. } => {
            let b = emit(db, bytes, env, ctx)?;
            Ok(format!("String::from_utf8({b}).ok()"))
        }
        // `Value.encode` (R2) renders a runtime value to its canonical binary-AST document. The wasm backend
        // calls the `value-encode` runtime op (walks a tagged heap handle); the NATIVE rust encoder is
        // TYPE-DIRECTED — build the SAME value-form (`cadenza_ast::ast::Arenas`) the runtime builds, then
        // `cadenza_ast::codec::encode` (the linked `cadenza-ast` rlib). `emit_value_form` recurses over the
        // type, mirroring `cdz-runtime`'s `encode_value_recursive` post-order build (scalar → `atom_leaf`,
        // Tuple → `(tuple …)`, etc.). INCREMENTAL: Int/Bool/Tuple wired (the round-trip case shape); other
        // shapes DECLINE (todo, never a fail). NOTE: `Value.decode` (the inverse) is the follow-up slice; the
        // round-trip corpus cases need BOTH, so they stay todo until decode lands. See `native-rust-r2-value-codec`.
        Core::ValueEncode { value, .. } => {
            let vty = type_of(db, value);
            // A RECURSIVE type's value-form encode walks unbounded depth; the native-rust static emit would
            // generate rust that HANGS rustc (the recursion runs through a collection payload, e.g. `Ast =
            // … (List (List Ast))`). Decline up front — the recursive-type value codec is a later increment.
            if value_codec_type_is_recursive(db, &vty, &mut std::collections::BTreeSet::new()) {
                return Err(Reject::unsupported(
                    "Value.encode native rust does not support a recursive-type value codec (its value-form walk would generate non-terminating rust)",
                ));
            }
            // The `<type-node>` half of the `(: <value> <type-node>)` frame — computed under a scoped
            // `name_ctx` borrow (released before the `&mut db` `emit` call below).
            let tnode = {
                let ncx = db.name_ctx();
                emit_type_node(&vty, &ncx)?
            };
            let val = emit(db, value, env, ctx)?;
            let form = emit_value_form(db, &vty, "__vv")?;
            // Wrap the bare value form in the `(: <value> <type-node>)` frame the runtime `value-encode`
            // op applies ONCE at the root (nested compounds stay bare) — `sum_shape_descriptor`'s
            // `Framed`/`Named` wrapper. This makes the native rust document byte-identical to the wasm face
            // (both `(: (tuple …) (Tuple Int64 Int64))`), so `Value.encode` is a stable cross-backend
            // content-address; `Value.decode` peels the same frame.
            Ok(format!(
                "{{ let __vv = {val}; let mut __b = cadenza_ast::ast::Builder::new(); \
                 let __inner = {form}; let __tn = {tnode}; let __colon = __b.name(\":\"); \
                 let __root = __b.list(vec![__colon, __inner, __tn]); \
                 cadenza_ast::codec::encode(&__b.finish(__root)) }}"
            ))
        }
        // `Value.decode b` (R2) — the inverse `∀a. Bytes → (Option a)`, TOTAL. `cadenza_ast::codec::decode`
        // parses the bytes to an `Arenas` (`None` on a malformed document); `emit_value_reconstruct` then
        // walks it to the native value of the CALL-SITE target type `a` (peeled from the node's `(Option a)`
        // type — grounded by typing), yielding `Option<a>` = the built-in `(Option a)` (Rust's own `Option`,
        // so `disc_some`/`disc_none` are unused on this path). `None` on any shape mismatch — never a trap.
        // Reusing `cadenza_ast::codec` for BOTH directions makes `decode ∘ encode = id` on rust by
        // construction (the round-trip corpus property). INCREMENTAL: Int/Bool/Tuple wired; other shapes decline.
        Core::ValueDecode { bytes, .. } => {
            let node_ty = type_of(db, id);
            let target = match node_ty.strip_nominal() {
                Ty::Sum { args, .. } if args.len() == 1 => args[0].clone(),
                _ => {
                    return Err(Reject::decline(
                        "Value.decode result type is not a resolved (Option a) — the target type is unsolved",
                    ));
                }
            };
            // A RECURSIVE target type's value-form reconstruct walks unbounded depth → non-terminating rust
            // that HANGS rustc (recursion through a collection payload). Decline up front (later increment).
            if value_codec_type_is_recursive(db, &target, &mut std::collections::BTreeSet::new()) {
                return Err(Reject::unsupported(
                    "Value.decode native rust does not support a recursive-type value codec (its value-form reconstruct would generate non-terminating rust)",
                ));
            }
            let b = emit(db, bytes, env, ctx)?;
            // PEEL the `(: <value> <type-node>)` frame the encoder wraps the document in: the decoded root
            // is the 3-element `(: value type)` list, so extract child[1] (the bare value node) and
            // reconstruct the target type from THAT. A malformed/unframed root (wrong head or arity) → None.
            let recon = emit_value_reconstruct(db, &target, "__a", "__valnode")?;
            Ok(format!(
                "(match cadenza_ast::codec::decode(&({b})) {{ Some(__a) => (|| {{ \
                 let __items = if let cadenza_ast::ast::Struct::List(__i) = __a.get(__a.root) {{ __i }} else {{ return None }}; \
                 if __items.len() != 3 || __a.head_name(__a.root) != Some(\":\") {{ return None }}; \
                 let __valnode = __items[1]; {recon} }})(), None => None }})"
            ))
        }
        // `Core::StrToBytes` is the "canonicalize a runtime text leaf" op. On the wasm side it backs THREE
        // surface ops (all a `bytes-compact` byte-rope flatten): `String.to-bytes` (String → Bytes),
        // `Symbol.of` (String → Symbol, intern), and `Symbol.to-string` (Symbol → String). On the RUST
        // native rep:
        //  - `String.to-bytes` (result `Ty::Bytes`) → `String::into_bytes` → the `Vec<u8>` the result maps to.
        //  - a Symbol↔String retag (`Symbol.of`: result `Ty::Symbol`; `Symbol.to-string`: Symbol operand,
        //    result `Ty::String`) — BOTH sides map to Rust's `String`, and a `String` is ALREADY a flat
        //    canonical leaf (the wasm `bytes-compact` rope-flatten has no analogue on the native rep), so the
        //    retag is the IDENTITY on the operand's `String` value — emit it unchanged. (Interning canonicalizes
        //    by CONTENT; two equal-content Strings are already `==`, so no runtime intern table is needed to
        //    match the value semantics — the same reason `Bytes.compact` is a no-op here.)
        Core::StrToBytes { string } => {
            let s = emit(db, string, env, ctx)?;
            // A Bytes RESULT is the byte view (`into_bytes`); a Symbol/String result (a Symbol retag) keeps
            // the `String` value as-is.
            if matches!(type_of(db, id).strip_nominal(), Ty::Bytes) {
                Ok(format!("({s}).into_bytes()"))
            } else {
                Ok(s)
            }
        }
        // `str-nfc-normalize` (FINDING #23) — canonicalize a String value to NFC, matching the wasm backend's
        // `str-nfc-normalize` runtime op. The native rust rep of a String is `String`, so the equivalent is
        // `.nfc().collect::<String>()` via `unicode-normalization` (the same crate cdz-nfc uses for the wasm NFC
        // component). That crate is ALREADY in the emitted-program link closure — it is a `std`-feature dep of
        // `cadenza_ast` (beside `num_bigint`), staged in the `<cadenza_ast>/deps` dir the rust-exec harness passes
        // and `--extern unicode_normalization=`-linked there (cdz-rust-run/src/run.rs, mirroring `num_bigint`), so
        // no NEW rust-target dependency is added. Emit a UFCS call (not a `use`) — matching the house style of
        // fully-qualified paths (`::std::boxed::Box::new`) — so no trait import is required in the driver: the
        // operand is an owned `String`, `&(…)[..]` yields the `&str` the `UnicodeNormalization` impl is on. This
        // fixes the wasm-vs-rust divergence breaker reported (`String.concat`/`from-bytes` skipped NFC on rust →
        // `=`/set-membership/byte-len all disagreed with wasm); the 3-way NFC pins now agree across both backends.
        Core::NfcNormalize { string } => {
            let s = emit(db, string, env, ctx)?;
            Ok(format!(
                "unicode_normalization::UnicodeNormalization::nfc(&({s})[..]).collect::<String>()"
            ))
        }
        // A SUM VALUE CONSTRUCTION → the Rust enum variant `<Enum>::<Variant>(payloads…)`. The enum +
        // variant names come from the node's solved `Ty::Sum` declaration at the disc's index (the
        // discriminant IS the variant's declaration-order position). A nullary variant is the bare
        // `<Enum>::<Variant>` (no parens); a payload variant carries its args positionally — matching the
        // emitted `enum <Enum> { <Variant>(T…), … }`.
        Core::SumNew { disc, payloads } => {
            let path = sum_variant_path(db, id, disc)?;
            // The variant's DECLARED payload types (single → the type; multi → the tuple's elements), used
            // to GROUND an empty-list payload whose own `type_of` left its element unsolved (`List ?`). In a
            // sum payload slot the element type IS known from the declaration, so annotate `Vec::<T>::new()`
            // rather than the bare `vec![]` rustc can't infer (E0282 "type annotations needed for Vec<_>").
            let sum_ty = type_of(db, id);
            let payload_decl_tys: Vec<Option<Ty>> = match variant_payload_ty(db, &sum_ty, disc) {
                Some(Ty::Tuple(elems)) if elems.len() == payloads.len() => {
                    elems.iter().cloned().map(Some).collect()
                }
                Some(t) if payloads.len() == 1 => vec![Some(t)],
                _ => vec![None; payloads.len()],
            };
            let mut args = Vec::with_capacity(payloads.len());
            for (k, &p) in payloads.iter().enumerate() {
                args.push(emit_elem_grounding_empty_list(
                    db,
                    p,
                    payload_decl_tys.get(k).and_then(|o| o.as_ref()),
                    env,
                    ctx,
                )?);
            }
            // A RECURSIVE variant's payload field is a `Box<…>` (the enum boxes it to stay finite), so its
            // payload value is wrapped `Box::new(…)` — the deref twin at the match site reads `*__pay`.
            // A non-recursive variant's field is unboxed. `wrap` applies the box exactly when the enum decl
            // did (`variant_is_recursive` is the shared predicate).
            let ty = type_of(db, id);
            // The reify `Ast` sum's `Float` variant must carry a CANONICAL float: a non-canonical value
            // (NaN / ±inf) has no canonical value form, so wasm's value-encode boundary TRAPS on it
            // ("encode bytes are not a valid canonical value form"). A compile-time-constant NaN payload is
            // already declined at `lower_ctor` (uniform compile-decline, ruling A); a RUNTIME-produced
            // non-canonical float (`(Ast.Float (- x nan))`, x a param) can't be compile-declined and reaches
            // here — guard it with a runtime `is_finite()` check that PANICS (an aborting trap), matching
            // wasm's runtime trap so the two backends AGREE (adv-ast-float-nan differential, v-runtime route).
            // Only the `Ast.Float` variant — an ordinary float value crosses fine.
            if args.len() == 1 && crate::lower::is_ast_float_variant(db, &ty, disc) {
                let v = args[0].clone();
                args[0] = format!(
                    "{{ let __f = {v}; if !__f.is_finite() {{ panic!(\"an Ast.Float node cannot carry a non-canonical float (NaN/inf has no canonical value form)\") }} __f }}"
                );
            }
            let boxed = super::enums::variant_is_recursive(db, &ty, disc);
            let wrap = |payload: String| {
                // Fully-qualify `::std::boxed::Box::new` (not the prelude `Box`) — the deref twin of the
                // enum field's `::std::boxed::Box<…>` — so a user sum NAMED `Box` cannot shadow it.
                if boxed {
                    format!("::std::boxed::Box::new({payload})")
                } else {
                    payload
                }
            };
            match args.len() {
                // A nullary variant is the bare path (`None`, `Shape::Circle` with no payload) — EXCEPT
                // for a GENERIC sum, where a bare `Option::None` gives rustc nothing to infer the type
                // parameter from when the constructor sits in a position without an expected type (e.g. an
                // `if`/`match` branch whose OTHER arm is the `Some`, but which rustc types left-to-right —
                // the `None` branch comes first and can't see the `Some`'s type). Emit a TURBOFISH with the
                // node's solved type args (`Option::<(Vec<Term>, Term)>::None`) so the type is explicit.
                // A MONOMORPHIC sum (no args) keeps the bare path. This is the nullary-generic-variant twin
                // of the empty-collection annotation — a construct with no operand to carry its element type.
                0 => Ok(nullary_variant_path(&db.name_ctx(), &ty, disc, &path)),
                // A one-payload variant carries its payload directly (`Some(x)`), boxed if recursive.
                1 => Ok(format!("{path}({})", wrap(args[0].clone()))),
                // A MULTI-payload variant carries ONE TUPLE (matching the enum decl's `V((T0, T1))` and the
                // core's single-`Ty::Tuple` payload model, which the match side reads as one indexed value).
                _ => Ok(format!(
                    "{path}({})",
                    wrap(format!("({})", args.join(", ")))
                )),
            }
        }
        // A poison reaching selection is a fault the collector surfaces before emission; reaching here
        // is a decline rather than emitted code (same as the wasm backend).
        Core::Poison(reject) => Err(reject),
        // A SUM MATCH → a Rust `match` on the scrutinee, dispatching on the variant. Each arm's
        // continuation is a leaf body or a nested switch (the decision tree). A payload BINDER in a body
        // is not bound in the arm pattern here — it resolves to a `Core::SumPayload` that re-extracts the
        // payload — so the arm pattern ignores the payload (`Enum::V { .. }` / `Enum::V(_)`).
        Core::MatchSum { scrutinee, root } => {
            // The match's RESULT integer type, if any — a Leaf arm body is grounded to it so a
            // default-Int64 literal arm beside a NARROW-width arm (a widened sum payload) does not yield
            // mismatched `if`/`match` branches + a wrong fn return width (Rust E0308). The scalar-`match`
            // path (`emit_match_impl`) already does this via `result_it`; the sum-decision-tree path did
            // not, so a `(match b ((A 0) 100) ((A x) x) …)` over a `UInt8` payload emitted `if … { 100i64 }
            // else { x_u8 }` — mismatched. Compute it here and thread it through the whole tree.
            let result_it = match type_of(db, id) {
                Ty::Int(it) => Some(it),
                _ => None,
            };
            // MATERIALIZE a NON-TRIVIAL scrutinee ONCE. Every payload binder reads the scrutinee via
            // `emit_sum_payload`, which RE-EMITS the scrutinee expression per read — so a binder used K times
            // re-emits it K times. For a RECURSIVE-CALL scrutinee that is `2^depth` calls (an exponential:
            // `(match (f (+ n 1)) ((Mk a _) (Mk a a)))` re-emits `f` twice per level → hang). If the scrutinee
            // is not a trivial re-emittable read (a param/local — cheap + side-effect-free to repeat), bind it
            // to a fresh `let __ms{n}` ONCE and record `(scrutinee, local)` so each `emit_sum_payload` reads
            // the LOCAL. This is the Rust twin of the wasm backend's materialize-scrutinee-once fix. The scope
            // of the `let` is the whole match, wrapped in a block. (A constant `SumNew` scrutinee folds in
            // `emit_sum_payload` against its payload nodes — no re-emit blow-up — so it needs no binding; only
            // a runtime non-trivial scrutinee does.)
            // Thread the match's RESULT type down to the scrutinee as `expected_ty` — so a `Core::MapLookup`
            // scrutinee `(Map.lookup m k)` over an empty `Map.empty` whose VALUE type is fixed only by THIS
            // match-join (not at the lookup: its payload is a free `Var`) can reconstruct the map's value
            // type from the join (breaker ms9-family ms13/ms6/ns1/ej*). Only when the result is solved (a
            // free-var join gives no hint). Harmless to other scrutinee kinds (they ignore `expected_ty`).
            // Thread down only when the result has a CONCRETE OUTER SHAPE — a bare top-level `Var`/`Any`
            // gives no shape to reconstruct a map value from (and would just re-ground to the default), so
            // skip it. An interior free var (`Set(Var)` / `List(Any)`: the element is fixed downstream) is
            // FINE — the reconstruction grounds the interior (`ground_open_vars`) and the OUTER shape is
            // exactly what avoids the E0282 that a bare `_` value hole would raise.
            let match_result = type_of(db, id);
            let scrut_ctx = if matches!(match_result, Ty::Var(_) | Ty::Any) {
                None
            } else {
                let mut c = ctx.clone();
                c.expected_ty = Some(match_result);
                // CLEAR the enclosing-insert flag for the scrutinee: an enclosing `Map.insert`/`.remove`
                // types the map it operates on (here the MATCH RESULT `inner`, which becomes the insert's
                // base), NOT the scrutinee's OWN lookup map. When the scrutinee is `(Map.lookup m k)` whose
                // value is itself a collection (ej3, the Map-of-Maps face), the flag would wrongly send `m`'s
                // `MapNew` down the bare-`new()` branch — but `.get(&k)` fixes only `m`'s KEY, not its VALUE
                // (the inner collection, unused at the lookup), so a bare `new()` E0282s. Clearing the flag
                // lets `m` take the reconstruction branch and annotate `BTreeMap<key, <join-value holed>>`
                // (the value's outer shape from the join, interior holed for the downstream insert to fix).
                c.map_typed_by_enclosing_insert = false;
                c.set_typed_by_enclosing_insert = false;
                Some(c)
            };
            let scrut_emit_ctx = scrut_ctx.as_ref().unwrap_or(ctx);
            if scrutinee_needs_materialize(db, scrutinee) {
                let sv = emit(db, scrutinee, env, scrut_emit_ctx)?;
                let local = format!("__ms{}", scrutinee.0);
                let mut c = ctx.clone();
                c.scrut_locals.push((scrutinee, local.clone()));
                let body = emit_sum_match(db, scrutinee, &root, result_it, env, &c)?;
                return Ok(format!("{{ let {local} = {sv}; {body} }}"));
            }
            emit_sum_match(db, scrutinee, &root, result_it, env, ctx)
        }
        // A LIST match `(match xs ((list) …) ((list a .. rest) …) …)` → a length-tested `if`/`else if`
        // chain over `xs.len()`. Each arm's condition is `== n` (fixed arity), `>= lead` (rest pattern),
        // or always (bare binder/`_`); the first satisfied arm's body runs. The scrutinee is bound ONCE
        // to a local so `.len()` and each element/rest binder (`SumPayload{Elem(i)}` → `xs[i]`,
        // `SumPayload{RestFrom(k)}` → `xs[k..].to_vec()`) read the same value. Exhaustiveness (every length
        // covered) is checked in `lower`, so the chain always ends in a catch-all arm — a defensive final
        // `else` panics `unreachable` to satisfy Rust's need for a total `if`/`else` expression.
        Core::MatchList { scrutinee, arms } => emit_list_match(db, scrutinee, &arms, env, ctx),
        // The SUB-VALUE of a sum scrutinee at a path, read by a variant pattern's binder. Rust binds in
        // the pattern, not by a separate accessor, so this re-matches the scrutinee to extract the
        // payload at `path`: `match <scrut> { <Enum>::<V>(p) => <walk path into p>, _ => unreachable!() }`.
        // Control is already in the matched arm (the disc was checked), so the `_` arm is unreachable.
        // Scrutinees here are pure (a param/local), so re-matching is cheap and observably identical.
        Core::SumPayload { scrutinee, path } => {
            emit_sum_payload(db, id, scrutinee, &path, env, ctx)
        }
        // `Option.expect`/`Result.expect` → `match <scrut> { <Enum>::<Present>(p) => p, _ => panic!() }`.
        // The present variant is `disc_present` (Some/Ok = 0); its single payload binds to a fresh name
        // and IS the expression's value; any other variant panics (the absent-variant trap — a Rust panic
        // is a Cadenza trap, matching the wasm `unreachable`). Scrutinee is pure (a param/local/call), so
        // matching it inline is sound.
        Core::SumExpect {
            scrutinee,
            disc_present,
        } => emit_sum_expect(db, scrutinee, disc_present, env, ctx),
        // A RUNTIME CLOSURE VALUE `(fn …)` that survived to run time (passed to a recursive fn) → a Rust
        // `Rc<dyn Fn(…) -> …>` that forwards its captured values + call args to the lifted `fn __lifted_k`.
        // The captures are emitted at the BUILD site (values in the enclosing scope) and MOVED into the
        // closure; each call then invokes `__lifted_k(<cap0>, …, <a0>, …)`. The closure's arity comes from
        // the lifted lambda's param count; a fresh `__a{i}` binds each. `Rc::new` makes it Clone (so a
        // multiply-used closure clones on read). (C1: works for any capture set — C1's gate cases are
        // no-capture combinators, but the emit handles captures uniformly.)
        Core::Closure { code, captures } => {
            let lam = ctx.layout.lifted[code].clone();
            let arity = lam.params.len();
            let ident = lifted_ident(code);
            // Emit each capture value + bind it to a `move`d local so the closure owns it.
            let mut cap_lets = String::new();
            let mut cap_names = Vec::with_capacity(captures.len());
            for (j, &c) in captures.iter().enumerate() {
                // If this capture node is ALREADY BOUND in scope (an enclosing `let` mapped its value node
                // → a Rust name; see the `Core::Let` arm), REFERENCE that binding rather than re-emitting the
                // value. Lowering inlines a let-bound value into `captures` as the VALUE node itself, so a
                // naive `emit(c)` re-runs it — a SECOND host call / recomputation (the double-emit bug: a
                // build-time host result captured by a returned closure fired the host op twice). Referencing
                // the binding evaluates it ONCE (at the `let`). Clone for a non-Copy captured value (the
                // closure moves it in; the binding may be read elsewhere too).
                let cv = if let Some(bound) = env.get(&c).cloned() {
                    if needs_clone_on_read(db, c) {
                        format!("{bound}.clone()")
                    } else {
                        bound
                    }
                } else {
                    emit(db, c, env, ctx)?
                };
                let cn = format!("__c{j}");
                cap_lets.push_str(&format!("let {cn} = {cv}; "));
                cap_names.push(cn);
            }
            let params: Vec<String> = (0..arity).map(|i| format!("__a{i}")).collect();
            // The forwarded call: captures first (in cell order), then the closure's args. A capture is
            // CLONED into each call — the closure is an `Fn` (callable repeatedly), so it may NOT MOVE a
            // captured variable out on a call (rustc E0507); cloning gives each invocation its own value
            // and leaves the capture intact for the next call. A Copy capture's `.clone()` is a plain copy.
            let mut call_args: Vec<String> =
                cap_names.iter().map(|c| format!("{c}.clone()")).collect();
            call_args.extend(params.iter().cloned());
            // Coerce EXPLICITLY to the `Rc<dyn Fn(…) -> …>` trait-object type when the node's solved
            // function type maps concretely (an `as` cast triggers the unsizing coercion). Without it,
            // `Rc::new(closure)` has a UNIQUE per-closure concrete type, so two closures of the "same"
            // Cadenza function type do NOT unify — `vec![mk(1), mk(2)]` or a match yielding two closures
            // would be an E0308 "expected closure, found a different closure". The cast makes every closure
            // of a given `Ty::Fn` the SAME `Rc<dyn Fn>` type, so they compose in a list/if/match.
            //
            // The `dyn` type comes from the LIFTED LAMBDA'S OWN param + result types (`lam.params[i].1`,
            // `lam.ret_ty`) — the AUTHORITATIVE concrete machine types the lifted `fn` signature is built
            // from (they must map, or `emit_lifted_lambda` itself declines). This is more reliable than
            // `type_of(id)`: a closure literal stored in a heap COMPOUND element (`(tuple (fn (x) …) …)`,
            // `(list (fn (x) …) …)`) does NOT get its arrow type grounded at the closure NODE from the
            // compound-element context (the solver leaves a var at the node), so `type_of(id)` returned a
            // non-concrete `Ty::Fn` → a spurious decline even though the lambda's body fully determines the
            // arity + types (breaker: closure-in-heap-compound-element). Building from `lam` makes every
            // closure whose lifted body is representable spell its `Rc<dyn Fn(…)->…>` — and it is the SAME
            // string for two closures of the same lifted signature, so the `as` cast still unifies them in a
            // list/if/match. Fall back to `type_of(id)` only if a `lam` param/result somehow does not map
            // (it would already have declined at the lifted-fn emit, so this is belt-and-suspenders).
            // ASYNC (Option A uniform ABI): the closure VALUE is `Rc<dyn EnvClosure<A,R>>` — a per-closure
            // synth `struct Clos_{code}` (its captures as fields, emitted at module level in `mod.rs`) whose
            // `EnvClosure::call` forwards `env` + its cloned captures + the (destructured) arg into the async
            // lifted fn and boxes the future. Here we build the VALUE: `Rc::new(Clos_{code} { __c0, … }) as
            // Rc<dyn EnvClosure<A,R>>`. The captures are the already-`move`d `__c{j}` locals (bound by
            // `cap_lets`); the struct owns them. `A`/`R` come from the lifted lambda's own params/ret via
            // `env_closure_args` (the SAME spelling `async_closure_type(Ty::Fn)` produces at a type position,
            // so this value fits a param/result/field slot spelled by `async_closure_type`).
            if ctx.mode.is_async() {
                let (a_ty, r_ty) = types::env_closure_args(&db.name_ctx(), &lam.params, &lam.ret_ty).ok_or_else(|| {
                    Reject::decline(
                        "an async closure whose function type is not fully representable has no native Rust representation",
                    )
                })?;
                let struct_ident = closure_struct_ident(code);
                // The struct literal fields: `__c{j}: __c{j}` (field name == the moved local name).
                let field_inits: Vec<String> = (0..cap_names.len())
                    .map(|j| format!("__c{j}: __c{j}"))
                    .collect();
                let closure_expr = format!(
                    "std::rc::Rc::new({struct_ident} {{ {} }}) as std::rc::Rc<dyn cdz_rt::EnvClosure<{a_ty}, {r_ty}>>",
                    field_inits.join(", ")
                );
                // Wrap in a block ONLY when there are capture `let`s to scope — a NO-capture closure is a bare
                // expression, and `{ expr }` around it trips `unused_braces` under the gate's `-D warnings`
                // (the block appears in an arbitrary position: a `.call(...)` arg, a return value, …).
                return Ok(wrap_closure_value(&cap_lets, &closure_expr));
            }
            let dyn_ty = {
                let ncx = db.name_ctx();
                let mut param_tys = Vec::with_capacity(lam.params.len());
                let mut ok = true;
                for (_, ty) in &lam.params {
                    match types::rust_type(&ncx, ty) {
                        Some(rt) => param_tys.push(rt),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                }
                let ret_ty = types::rust_type(&ncx, &lam.ret_ty);
                match (ok, ret_ty) {
                    (true, Some(ret)) => {
                        format!("std::rc::Rc<dyn Fn({}) -> {ret}>", param_tys.join(", "))
                    }
                    // A `lam` type that does not map (should not happen — the lifted fn would decline) — fall
                    // back to the node's solved type; decline if that also fails.
                    _ => {
                        let idty = type_of(db, id);
                        types::rust_type(&db.name_ctx(), &idty).ok_or_else(|| {
                            Reject::decline(
                                "a closure whose function type is not fully solved here has no native Rust representation",
                            )
                        })?
                    }
                }
            };
            let closure_expr = format!(
                "std::rc::Rc::new(move |{}| {ident}({})) as {dyn_ty}",
                params.join(", "),
                call_args.join(", ")
            );
            Ok(wrap_closure_value(&cap_lets, &closure_expr))
        }
        // Apply a runtime closure at full arity → a direct call. SYNC: `(<closure>)(<a0>,…)`. ASYNC (Option A):
        // `<closure>.call(env, <arg-or-tuple>).await` — the `EnvClosure::call` takes the env at the call +
        // one `A` arg (a multi-arg closure tuples its args into `A`, matching the lifted convention), and its
        // returned future is awaited. `.await` inside an async fn body is the same shape a `Core::Call` uses.
        Core::CallClosure { closure, args } => {
            let c = emit(db, closure, env, ctx)?;
            let mut rendered = Vec::with_capacity(args.len());
            for &a in args.iter() {
                rendered.push(emit(db, a, env, ctx)?);
            }
            if ctx.mode.is_async() {
                // `.call(env, arg).await` reborrows `env` for the whole call — so if the CLOSURE expr `c` OR
                // any ARG contains its own `.await` (a nested async call `foldn(env,…).await`, e.g. the shape
                // `(f (foldn f z m))`), that inner reborrow of `env` is still LIVE while `.call` reborrows it
                // → E0499 "borrow `*__cdz_env` as mutable more than once". HOIST every `.await`-containing
                // operand into a `let` BEFORE the `.call`: each hoisted reborrow completes (its `.await`
                // releases `env`) before the next statement, so no two are ever live together. This mirrors
                // the async `Core::Call` arm's `needs_hoist`. Operands with no `.await` (scalars, field reads,
                // a bare closure local) stay inline.
                let mut binds = String::new();
                let hoist = |expr: String, tmp: &str, binds: &mut String| -> String {
                    if expr.contains(".await") {
                        binds.push_str(&format!("let {tmp} = {expr}; "));
                        tmp.to_string()
                    } else {
                        expr
                    }
                };
                let c = hoist(c, "__cclos", &mut binds);
                let hoisted_args: Vec<String> = rendered
                    .into_iter()
                    .enumerate()
                    .map(|(i, a)| hoist(a, &format!("__carg{i}"), &mut binds))
                    .collect();
                // Tuple ≥2 args into the single `A` (arity 1 = the bare arg; arity 0 = `()`), matching
                // `env_closure_args`/`async_closure_type`'s `A` convention + the struct's `call` destructure.
                let arg = match hoisted_args.len() {
                    0 => "()".to_string(),
                    1 => hoisted_args.into_iter().next().unwrap(),
                    _ => format!("({})", hoisted_args.join(", ")),
                };
                let call = format!("({c}).call({}, {arg}).await", super::ENV_PARAM);
                // Wrap in a block only when we hoisted (else a bare braced expr trips `unused_braces`).
                return Ok(if binds.is_empty() {
                    call
                } else {
                    format!("{{ {binds}{call} }}")
                });
            }
            Ok(format!("({c})({})", rendered.join(", ")))
        }
        // A read of the k-th CAPTURED free variable inside a lifted body → the capture PARAMETER the lifted
        // fn bound it to (the env maps its binder to `__cap{index}`). On the rust backend a capture is an
        // ordinary leading param, so this is a plain identifier read — no env-cell `arr-get`. `Captured`
        // resolves via the env like a `Param`; if absent (a compiler-bug shape), decline.
        Core::Captured { index, .. } => {
            // The lifted-lambda emit inserted `__cap{index}` into the env keyed by the capture's binder,
            // but a `Captured` node carries only the INDEX, not the binder. The env is keyed by binder, so
            // resolve by the reserved name directly: the lifted emit names capture j `__cap{j}`.
            Ok(format!("__cap{index}"))
        }
        // `trap` → a Rust `panic!` (a Cadenza trap, matching the wasm `unreachable`). Rust's `panic!`
        // returns the never type `!`, which coerces to ANY expected type — the runtime counterpart of
        // `trap`'s `Never` unifying with any position. The panic message is the literal `"unreachable"`
        // (NOT `"trap"`): the differential gate classifies a trap outcome by its reason (`trap_kind`), and
        // an explicit `(trap …)` / uninhabited-match lowers to the `unreachable` KIND on BOTH backends
        // (the wasm side traps `wasm 'unreachable' instruction executed`), so the rust panic must carry a
        // reason that classifies the same way — else a `(trap "unreachable")` case grades todo on rust
        // though it correctly halts. (An ARITHMETIC trap — div-by-zero/overflow — is a separate `checked_*`
        // panic carrying its own op-named reason, not `Core::Trap`, so this literal is only the non-
        // arithmetic explicit trap, whose canonical kind IS `unreachable`.)
        Core::Trap => Ok("panic!(\"unreachable\")".to_string()),
        // A KIND-PRESERVING divide-by-zero trap (demoted from a const `(/ 1 0)` in a conditional branch).
        // `panic!` returns `!` (coerces to any position, like `Core::Trap`), but with a reason that
        // `trap_kind` classifies as `div-by-zero` — agreeing with the wasm side's native `i64.div_s` ÷0 trap
        // — so a `(trap "divide by zero")` case grades PASS identically on both backends (operator ruling).
        Core::TrapDivZero => Ok("panic!(\"divide by zero\")".to_string()),
        // A KIND-PRESERVING integer-overflow trap (demoted from a const arithmetic overflow in a conditional
        // branch). `panic!` returns `!` (any position, like `Core::Trap`) with a reason `trap_kind`
        // classifies as `overflow` — agreeing with the wasm side's native `i32.div_s` MIN/-1 overflow trap.
        Core::TrapOverflow => Ok("panic!(\"integer overflow\")".to_string()),
        // EFFECT NON-LOCAL EXIT (an abortive handler arm's non-tail perform). The effect lowering + its
        // EFFECT NON-LOCAL EXIT (CASE 1) — the Rust twin of the wasm `emit(value); Lir::Return`. The fold
        // produces `HandleAbort` ONLY for a WHOLE-DEF-BODY handle (see the `Core::HandleAbort` doc), which the
        // Rust backend lowers as a plain `fn`, so a native `return <value>` performs the non-local exit: the
        // abort value IS the enclosing function's result (== the handle result, E4-guaranteed), abandoning the
        // pending continuation. `return e` is a `!`-typed expression (coerces to any result position, like
        // `Core::Trap`'s `panic!`), so it validates wherever the abort sits — e.g. an `if` branch.
        Core::HandleAbort { value, .. } => {
            let v = emit(db, value, env, ctx)?;
            Ok(format!("return {v}"))
        }
        // Runtime BigInt ops → `cdz_num::Big` value ops (the SAME bignum the wasm runtime uses, shared by
        // source via the `cdz-num` crate). `Big` methods BORROW their operands and return an owned `Big`.
        // `BigInt.of x` on a runtime fixed-width int — widen the i64-slot value into a `Big`. (A CONSTANT
        // source folds to `Core::ConstInt` retyped BigInt upstream and emits via the int path; this is the
        // runtime widen.)
        Core::BigIntOfI64 { value } => {
            let v = emit(db, value, env, ctx)?;
            // `BigInt.of` is `∀a. (Int a) -> BigInt`, so the operand may be UNSIGNED. Widen BY VALUE through
            // `i128`: every fixed-width int (signed or unsigned, ≤64 bits) fits an `i128` LOSSLESSLY and keeps
            // its true sign (`u64::MAX as i128` = +18446744073709551615, not -1). `Big` has no public
            // `from_i128`/`from_u64`, but `i128_to_sign_magnitude_bytes_into` writes `[sign][LE magnitude]`
            // (≤17 bytes) and `from_sign_magnitude_bytes` rebuilds the `Big`. Correct for BOTH a signed
            // negative and a large unsigned — one uniform path.
            // KEYSTONE: the operand MUST be widened `as <its own rust int type> as i128` — NOT a bare `(v) as i128`.
            // A genuine `UInt64` operand whose emit carries an i64 REP (e.g. a `(bin (u64 n))` binder, whose
            // BinIntRead assembles bits into an i64) would SIGN-EXTEND under `(i64-expr) as i128` — the top
            // half of u64 flips negative (`2^63+9` → -9223372036854775799, `% 1000` = -799 not 817). Casting
            // to the operand's solved rust type FIRST (`(v) as u64 as i128` for a UInt64 operand) widens
            // UNSIGNED. For an Int64 operand it is `as i64` (a no-op, elided). This is the `BigInt.of` twin of
            // the BinIntRead cast; both target the same u64-carried-as-i64 sign-extension. (corpus-bugfix
            // finding #4, wasm oracle 817; rust/rust-async sign-extended to -799.)
            let vit = int_ty_of(db, value);
            let opt = types::rust_type(&db.name_ctx(), &Ty::Int(vit))
                .unwrap_or_else(|| "i64".to_string());
            let widened = if opt == "i64" {
                format!("({v}) as i128")
            } else {
                format!("({v}) as {opt} as i128")
            };
            Ok(format!(
                "{{ let mut __buf = [0u8; 17]; \
                 let __n = cdz_num::Big::i128_to_sign_magnitude_bytes_into({widened}, &mut __buf) \
                 .expect(\"i128 fits 17 bytes\"); \
                 cdz_num::Big::from_sign_magnitude_bytes(&__buf[..__n]) }}"
            ))
        }
        // `Int64.of b` on a runtime `Big` — the checked narrowing back to i64, which TRAPS out of range at
        // run time (matching the wasm `bigint-to-i64-checked`, which lowers to a wasm `unreachable`).
        // `to_i64_checked` returns `Option<i64>`. Panic with a message that CLASSIFIES as `unreachable`
        // under the gate's `trap_kind` (the same reason the shift-count guard and the runtime Rational
        // zero-denominator guard panic "unreachable"): the numeric-model out-of-range narrowing is a
        // non-arithmetic trap, so both backends must grade the SAME kind. A bare "BigInt value out of Int64
        // range" message does NOT classify (it lacks any `trap_kind` substring), so the case graded `todo`
        // (an unconfirmed trap) even though the behavior already matched wasm — the exact gap the corpus
        // case "truncating a rational whose integer part exceeds Int64 traps" documents.
        Core::BigIntToI64 { operand } => {
            let b = emit(db, operand, env, ctx)?;
            Ok(format!(
                "({b}).to_i64_checked().unwrap_or_else(|| panic!(\"unreachable: BigInt value out of Int64 range\"))"
            ))
        }
        // `Char.to-int c` on a runtime char (Char-rep 1/N) — the native rust rep of a `Char` is `char`, so
        // the total scalar-value read `Char -> Int64` is `(c as u32) as i64` (a code point is non-negative
        // and fits i64). A CONSTANT char folded to a ConstInt in `lower`; this is the runtime operand.
        Core::CharToInt { operand } => {
            let c = emit(db, operand, env, ctx)?;
            Ok(format!("(({c}) as u32 as i64)"))
        }
        // `Char.from-int n` on a runtime Int64 (Char-rep 4/N follow-on) — the FALLIBLE, TOTAL conversion
        // `Int64 -> (Option Char)`. The native rust rep of a `Char` is `char` and the built-in `Option` maps
        // to Rust's own `Option`, so `u32::try_from(n).ok().and_then(char::from_u32)` IS the value: `try_from`
        // rejects a negative / `> u32` value (→ None) and `char::from_u32` performs the EXACT scalar-validity
        // test (excludes surrogates + `> U+10FFFF`) — the same test `lower`'s constant fold applies. Never
        // panics/traps. `disc_some`/`disc_none` are the wasm sum tags, irrelevant on the native-`Option` path.
        Core::IntToCharChecked { operand, .. } => {
            let n = emit(db, operand, env, ctx)?;
            Ok(format!("u32::try_from({n}).ok().and_then(char::from_u32)"))
        }
        // A runtime BigInt binary op — `+`/`-`/`*`/`/`/`%`. `add`/`sub`/`mul` are total; `div`/`rem` go
        // through `divmod` (returns `None` on a zero divisor → TRAP, matching the wasm `bigint-div`).
        Core::BigIntBinOp { op, lhs, rhs } => {
            // The `Big` methods (`add`/`mul`/…) require BOTH operands to emit as a `cdz_num::Big`. That
            // holds when each operand's type IS a BigInt — either bare `Ty::BigInt` OR a `Ty::Qty { inner:
            // BigInt }` (a QUANTITY over a BigInt magnitude: `lower` erases the `Ty::Qty` wrapper to its
            // inner, so `(Qty.of (BigInt.of x) u)` emits the same `Big` as `(BigInt.of x)` — the unit is
            // compile-time-only). So a `Qty{inner:BigInt}` operand is FINE. What must still DECLINE is a
            // CONSTANT magnitude that reaches here typed as a plain `Ty::Int` (it emits an `i64` literal,
            // not a `Big` — `.mul(&Big)` on an `i64` is E0308/E0599). `is_bigint_valued` accepts the two
            // BigInt shapes and rejects the bare-Int one. (Mirrors the wasm backend's
            // `Ty::Qty { inner, .. } if matches!(*inner, Ty::BigInt)` treatment.)
            if !is_bigint_valued(&type_of(db, lhs)) || !is_bigint_valued(&type_of(db, rhs)) {
                return Err(Reject::unsupported(
                    "a BigInt op whose operand is neither BigInt nor a BigInt-magnitude quantity (a \
                     bare-Int-typed operand would emit an i64, not a Big) is not supported",
                ));
            }
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let expr = match op {
                crate::core::BigIntOp::Add => format!("({l}).add(&({r}))"),
                crate::core::BigIntOp::Sub => format!("({l}).sub(&({r}))"),
                crate::core::BigIntOp::Mul => format!("({l}).mul(&({r}))"),
                // Truncating quotient / remainder; `divmod` traps (via `expect`) on a zero divisor, the
                // same runtime trap the wasm `bigint-div`/`-rem` raise.
                crate::core::BigIntOp::Div => {
                    format!("({l}).divmod(&({r})).expect(\"BigInt divide by zero\").0")
                }
                crate::core::BigIntOp::Rem => {
                    format!("({l}).divmod(&({r})).expect(\"BigInt remainder by zero\").1")
                }
            };
            Ok(expr)
        }
        // A runtime BigInt COMPARISON — three-way `cmp` (`core::cmp::Ordering`) reduced to the operator's
        // fixed compare, mirroring the wasm lowering (`bigint-cmp` then a fixed compare-with-zero). Result
        // is a `bool`. `=`/`≠` compare the `Ordering` to `Equal`; the relational ops compare the sign.
        Core::BigIntCmp { op, lhs, rhs } => {
            // Both operands must be BigInt-VALUED to emit as `Big` (bare `BigInt` or a `Qty{inner:BigInt}`
            // whose wrapper erases; a bare-`Int` constant would emit an i64 → declines). See `BigIntBinOp`.
            if !is_bigint_valued(&type_of(db, lhs)) || !is_bigint_valued(&type_of(db, rhs)) {
                return Err(Reject::unsupported(
                    "a BigInt comparison whose operand is neither BigInt nor a BigInt-magnitude quantity \
                     is not supported by the Rust backend",
                ));
            }
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let cmp = format!("({l}).cmp(&({r}))");
            // `BigIntCmp` carries one of the relational prims `Lt`/`Gt`/`Le`/`Ge`/`Eq` (there is no `Ne`
            // Prim — `≠` lowers to `not =` upstream, so it never reaches here). Reduce the three-way
            // `Ordering` to the operator's bool, mirroring the wasm `bigint-cmp`-then-fixed-compare.
            let expr = match op {
                Prim::Eq => format!("({cmp} == core::cmp::Ordering::Equal)"),
                Prim::Lt => format!("({cmp} == core::cmp::Ordering::Less)"),
                Prim::Gt => format!("({cmp} == core::cmp::Ordering::Greater)"),
                Prim::Le => format!("({cmp} != core::cmp::Ordering::Greater)"),
                Prim::Ge => format!("({cmp} != core::cmp::Ordering::Less)"),
                _ => {
                    return Err(Reject::decline(
                        "unexpected non-relational Prim in a BigInt comparison",
                    ));
                }
            };
            Ok(expr)
        }
        // Runtime Rational ops → `cdz_num::Rational` value ops (num/den `Big` pair, canonical normalized;
        // mirrors the wasm runtime's `rational-*` byte-for-byte). Each fixed-width int operand widens to a
        // `Big` BY VALUE through `i128` (a `uN as i128` keeps its true sign — the same unsigned-safe path
        // as `BigIntOfI64`, since `Rational.of` operands are `∀a.(Int a)`).
        //
        // `Rational.of n d` — build `n/d`, normalized. A ZERO denominator has no rational value and TRAPS
        // at run time (numeric-model.md #A Rational With A Zero Denominator Is Not A Value), the rational
        // analogue of a runtime integer divide-by-zero; the const-foldable case is rejected CDZ0304 at
        // `lower`, this is the runtime companion. `cdz_num::Rational::new` DOES panic on a zero denominator,
        // but its message ("Rational with zero denominator") does NOT classify under the gate's `trap_kind`,
        // so the trap graded `todo` (an unconfirmed trap) rather than matching the corpus's `unreachable`.
        // Emit an EXPLICIT guard panicking "unreachable" — the SAME non-arithmetic trap kind the wasm
        // backend lowers this to (`wasm 'unreachable' instruction executed`) — so both backends grade PASS
        // (the rust shift-count guard uses the same "unreachable"-classifying message for the identical
        // reason). Bind the denominator `Big` once (it may be a side-effecting expression, and the guard
        // reads it before the move into `new`).
        Core::RationalOfInts { num, den } => {
            let n = emit_int_as_big(db, num, env, ctx)?;
            let d = emit_int_as_big(db, den, env, ctx)?;
            // Bind the NUMERATOR before the denominator so the two operands EVALUATE in source order
            // (num-then-den). This matters when an operand has an observable side effect — a host call
            // (`(Rational.of (Env.rate-num) (Env.rate-den))`): the host-call sequence must be num-then-den
            // (the wasm oracle's + source order). Binding `__d` first would fire the denominator's host call
            // before the numerator's, reversing the observed sequence.
            Ok(format!(
                "{{ let __n = {n}; let __d = {d}; if __d.is_zero() {{ panic!(\"unreachable\") }} cdz_num::Rational::new(__n, __d) }}"
            ))
        }
        Core::RationalOfIntWiden { value } => {
            let n = emit_int_as_big(db, value, env, ctx)?;
            Ok(format!("cdz_num::Rational::from_big({n})"))
        }
        // `Rational.numerator`/`denominator` → read the `Big` numerator/denominator out of the normalized
        // pair (`cdz_num::Rational` has public `num`/`den` fields — the same canonical form the wasm
        // `rational-num`/`rational-den` ops read). Clone the `Big` (the Rational operand is borrowed, its
        // components are owned by it). Result is a `Big` (`Ty::BigInt`), matching the `Rational → BigInt`
        // surface — a numerator/denominator can exceed i64.
        Core::RationalNum { operand } => {
            let r = emit(db, operand, env, ctx)?;
            Ok(format!("({r}).num.clone()"))
        }
        Core::RationalDen { operand } => {
            let r = emit(db, operand, env, ctx)?;
            Ok(format!("({r}).den.clone()"))
        }
        // A runtime Rational binary op → the `Rational` method (borrow both, return owned). `div` traps on
        // a zero divisor (mirrors `rational-div`).
        Core::RationalBinOp { op, lhs, rhs } => {
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let m = match op {
                crate::core::RationalOp::Add => "add",
                crate::core::RationalOp::Sub => "sub",
                crate::core::RationalOp::Mul => "mul",
                crate::core::RationalOp::Div => "div",
            };
            Ok(format!("({l}).{m}(&({r}))"))
        }
        // A runtime Rational comparison → three-way `cmp` reduced to the Prim's bool (mirrors the BigInt
        // comparison; no `Ne` Prim — `≠` is `not =` upstream).
        Core::RationalCmp { op, lhs, rhs } => {
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let cmp = format!("({l}).cmp(&({r}))");
            let expr = match op {
                Prim::Eq => format!("({cmp} == core::cmp::Ordering::Equal)"),
                Prim::Lt => format!("({cmp} == core::cmp::Ordering::Less)"),
                Prim::Gt => format!("({cmp} == core::cmp::Ordering::Greater)"),
                Prim::Le => format!("({cmp} != core::cmp::Ordering::Greater)"),
                Prim::Ge => format!("({cmp} != core::cmp::Ordering::Less)"),
                _ => {
                    return Err(Reject::decline(
                        "unexpected non-relational Prim in a Rational comparison",
                    ));
                }
            };
            Ok(expr)
        }
        // A constant Rational — a normalized `IntValue` num/den pair (folded in `lower`). Build each `Big`
        // via the same const-BigInt materialization (in-i64 → `from_i64`, beyond → `from_sign_magnitude_bytes`)
        // then `Rational::new` (which re-normalizes — a no-op on an already-normalized pair, so byte-identical).
        Core::ConstRational(n, d) => {
            let nb = const_big_expr(&n);
            let db_ = const_big_expr(&d);
            Ok(format!("cdz_num::Rational::new({nb}, {db_})"))
        }
        // A runtime `(bin …)` CONSTRUCTION of fixed-width INT segments → a `Vec<u8>` built segment by
        // segment, mirroring the wasm `BinBuild` (select.rs). Each segment: materialize its value as an
        // i64, RANGE-CHECK it against the segment's (signed, bits) width (a defensive backstop — the width
        // type already bounds it, so this is normally dead; a genuine out-of-range TRAPS "binary value does
        // not fit segment", the runtime companion of the constant CDZ0304), then write `w` bytes MSB-first
        // (`le` reverses the byte position within the segment). Byte-identical to the wasm heap build.
        Core::BinBuild { segs } => {
            // Build each segment into a fixed `[0u8; w]` array (byte positions written explicitly, `le`
            // counting the position down within the segment), then extend the buffer in declaration order.
            let mut body = String::from("{ let mut __b: Vec<u8> = Vec::new(); ");
            for s in &segs {
                let w = s.width as u32;
                let bits = w * 8;
                let v = emit(db, s.value, env, ctx)?;
                body.push_str(&format!("{{ let __v: i64 = ({v}) as i64; "));
                if bits < 64 {
                    if s.signed {
                        let hi = (1i64 << (bits - 1)) - 1;
                        let lo = -(1i64 << (bits - 1));
                        body.push_str(&format!(
                            "if __v > {hi}i64 || __v < {lo}i64 {{ panic!(\"binary value does not fit segment\") }} "
                        ));
                    } else {
                        let ceil = 1i64 << bits;
                        body.push_str(&format!(
                            "if __v < 0 || __v >= {ceil}i64 {{ panic!(\"binary value does not fit segment\") }} "
                        ));
                    }
                }
                body.push_str(&format!("let mut __seg = [0u8; {w}]; "));
                for p in 0..w {
                    let shift = (w - 1 - p) * 8;
                    let pos_in_seg = if s.little_endian { w - 1 - p } else { p };
                    let byte = if shift > 0 {
                        format!("(((__v as u64) >> {shift}) as u8)")
                    } else {
                        "(__v as u8)".to_string()
                    };
                    body.push_str(&format!("__seg[{pos_in_seg}usize] = {byte}; "));
                }
                body.push_str("__b.extend_from_slice(&__seg); } ");
            }
            body.push_str("__b }");
            Ok(body)
        }
        // A runtime `(bits v k)` RUN packed MSB-first into a `Vec<u8>` — mirror the wasm `BinBitsBuild`. The
        // run is byte-aligned (CDZ0220), so the total byte count + every flush position + the bit-cursor are
        // STATIC; only the field values are runtime. An i64 accumulator collects open bits MSB-first,
        // flushing whole bytes from its top as they close. Each field range-checks `0 <= v < 2^k` (a
        // defensive backstop — the `(UInt k)` field type already bounds it; a real miss traps).
        Core::BinBitsBuild { fields } => {
            let total_bits: u32 = fields.iter().map(|f| f.k).sum();
            let total_bytes = total_bits / 8;
            let mut body = format!(
                "{{ let mut __b: Vec<u8> = Vec::with_capacity({total_bytes}); let mut __acc: i64 = 0; "
            );
            let mut nbits: u32 = 0;
            for f in &fields {
                let k = f.k;
                let v = emit(db, f.value, env, ctx)?;
                // `ceil = 2^k` and `mask = 2^k - 1` computed in u128 so a WIDE field cannot overflow the
                // shift (Copilot PR#516): `1i64 << k` is `i64::MIN` at k==63 and a shift-overflow at k==64,
                // which would emit a NEGATIVE `ceil` (breaking the `__v >= ceil` range-check) and a wrong
                // `mask`. `lower` caps a RUNTIME bit-field at k ≤ 56 (a wider one declines "…wider than 56
                // bits is not yet built"), so a k in 57..=64 is not reachable here TODAY — but compute in
                // u128 regardless so the emit stays correct if that cap ever moves, matching the wasm side's
                // width-agnostic pack. The range check compares `__v` (a non-negative-checked i64) against
                // `ceil` in u128 (a k-bit UNSIGNED field: `0 <= v < 2^k`); the mask is `2^k - 1`.
                let ceil: u128 = 1u128 << k;
                let mask: u128 = ceil - 1;
                body.push_str(&format!(
                    "{{ let __v: i64 = ({v}) as i64; \
                     if __v < 0 || (__v as u128) >= {ceil}u128 {{ panic!(\"binary value does not fit segment\") }} \
                     __acc = (__acc << {k}) | ((__v as u128 & {mask}u128) as i64); }} "
                ));
                nbits += k;
                while nbits >= 8 {
                    let shift = nbits - 8;
                    let byte = if shift > 0 {
                        format!("(((__acc as u64) >> {shift}) as u8)")
                    } else {
                        "(__acc as u8)".to_string()
                    };
                    body.push_str(&format!("__b.push({byte}); "));
                    nbits -= 8;
                    if nbits == 0 {
                        body.push_str("__acc = 0; ");
                    } else {
                        let m = (1i64 << nbits) - 1;
                        body.push_str(&format!("__acc &= {m}i64; "));
                    }
                }
            }
            body.push_str("__b }");
            Ok(body)
        }
        // Read a fixed-width INT segment out of a runtime `Bytes` (`Vec<u8>`) scrutinee at a STATIC offset —
        // the value a `bin`-pattern binder decodes. Assemble `w` bytes MSB-first (`le` reversed) into an i64,
        // then sign-extend a signed segment narrower than 64 bits (an unsigned one is already zero-extended).
        // The caller's length probe guaranteed the read is in bounds. Mirror the wasm `BinIntRead`.
        Core::BinIntRead {
            bytes,
            byte_offset,
            off_plus,
            width,
            signed,
            little_endian,
        } => {
            let v = emit(db, bytes, env, ctx)?;
            let w = width as u32;
            let mut body = format!("{{ let __bytes = {v}; ");
            // §4a DYNAMIC OFFSET: `pos = byte_offset + p + off_plus`. Bind `off_plus` (an i64 count) once as a
            // usize `__off`; `None` = a static offset (`__off = 0`, no extra emit). The caller's length probe
            // guaranteed each `pos` in bounds (matching the wasm read; a `Vec<u8>` index panics on overrun).
            let off_expr = match off_plus {
                None => "0usize".to_string(),
                Some(op) => format!("(({}) as usize)", emit(db, op, env, ctx)?),
            };
            body.push_str(&format!("let __off = {off_expr}; let mut __acc: i64 = 0; "));
            for p in 0..w {
                let shift = (w - 1 - p) * 8;
                let stat = if little_endian {
                    byte_offset + (w - 1 - p)
                } else {
                    byte_offset + p
                };
                let term = if shift > 0 {
                    format!("((__bytes[{stat}usize + __off] as i64) << {shift})")
                } else {
                    format!("(__bytes[{stat}usize + __off] as i64)")
                };
                body.push_str(&format!("__acc |= {term}; "));
            }
            if signed && w < 8 {
                let sh = (8 - w) * 8;
                body.push_str(&format!("__acc = (__acc << {sh}) >> {sh}; "));
            }
            // `__acc` holds the segment's BIT PATTERN in an i64 (the byte-assembly + signed narrow-width
            // sign-extension above). The BINDER's solved type, however, is the segment's own int type — and
            // a `(u64 n)` segment binds a genuine `UInt64` (`Ty::Int` unsigned-64; the v-core-opt typing fix
            // `7ff56255f`). Its Rust type is `u64`, not `i64`, so returning the raw `i64 __acc` mismatches
            // every downstream use (`% n`, `Int64.of n`) — rust E0308 (wasm has no static type to clash).
            // CAST the assembled bits to the binder's Rust int type: `0x8000_0000_0000_0001 as u64` =
            // 2^63+1 (the true unsigned value), so `% 1000` runs unsigned → 809, and `Int64.of` narrows a
            // genuine `u64`. A narrower/signed segment already solves to `Int64`, so its cast is `as i64` —
            // a no-op elided here (keeps the common case byte-identical + dodges the `unnecessary_cast` lint).
            let idit = int_ty_of(db, id);
            let rt = types::rust_type(&db.name_ctx(), &Ty::Int(idit))
                .unwrap_or_else(|| "i64".to_string());
            if rt == "i64" {
                body.push_str("__acc }");
            } else {
                body.push_str(&format!("(__acc as {rt}) }}"));
            }
            Ok(body)
        }
        // Read the FINAL `(bytes rest)` segment — the tail of the `Vec<u8>` scrutinee from `byte_offset +
        // off_plus` to the end, as an owned `Vec<u8>`. The caller's length probe guaranteed `len >= start`.
        // Mirror the wasm `BinRestRead` (`bytes-slice(bytes, start, len - start)`).
        Core::BinRestRead {
            bytes,
            byte_offset,
            off_plus,
        } => {
            let v = emit(db, bytes, env, ctx)?;
            let off_expr = match off_plus {
                None => "0usize".to_string(),
                Some(op) => format!("(({}) as usize)", emit(db, op, env, ctx)?),
            };
            Ok(format!(
                "{{ let __bytes = {v}; __bytes[{byte_offset}usize + {off_expr}..].to_vec() }}"
            ))
        }
        // Read a DEPENDENT-SIZE `(bytes payload n)` segment — exactly `n` bytes from `byte_offset + off_plus`,
        // as an owned `Vec<u8>`, where `n` is the runtime value of an earlier segment (`len` emits an i64).
        // The caller's length probe guaranteed `start + n <= len`. Mirror the wasm `BinSizedRead`.
        Core::BinSizedRead {
            bytes,
            byte_offset,
            off_plus,
            len,
        } => {
            let v = emit(db, bytes, env, ctx)?;
            let n = emit(db, len, env, ctx)?;
            let off_expr = match off_plus {
                None => "0usize".to_string(),
                Some(op) => format!("(({}) as usize)", emit(db, op, env, ctx)?),
            };
            Ok(format!(
                "{{ let __bytes = {v}; let __start = {byte_offset}usize + {off_expr}; let __n = ({n}) as usize; __bytes[__start..__start + __n].to_vec() }}"
            ))
        }
        // A host call crosses the host boundary. The wasm backend routes it to a component import; the Rust
        // backend (a standalone binary, no component model) emits a CALL to a crate-root shim fn the RUNNER
        // supplies (the gate driver generates it from the case's recorded host-responses; a real embedder
        // implements the same fns). The shim name derives from the CANONICAL host-op key
        // (`canonical_host_op_key` — kebab-normalized effect + verbatim op) → a Rust ident, and the driver
        // derives the SAME ident from the recorded response key (also kebab-normalizing the effect), so the
        // emitted `crate::<ident>()` names EXACTLY the generated fn regardless of the corpus key's casing.
        // H1 slice: a NO-ARG, fixed-width-INTEGER-result op (`ask.ask -> Int64`); ARGS or a non-integer
        // result is a later increment and DECLINES cleanly (reject-don't-miscompile).
        Core::HostCall {
            effect,
            op,
            args,
            result,
        } => {
            let shim = host_shim_ident(&crate::effects::canonical_host_op_key(&effect, &op));
            // ARGUMENTS (H3): the op's args cross the boundary EVALUATED, in source LEFT-TO-RIGHT order (the
            // host-call sequence the wasm oracle records; the arg VALUES themselves are not compared — the
            // corpus host_calls key is the op name only). Bind each arg to a `let __ha<i>` in order so a
            // multi-arg call (or an arg that is itself a host call) evaluates strictly left-to-right, then
            // pass them. Each arg must be a fixed-width INTEGER guest value (marshalled `as i64` to the shim's
            // i64 param); a non-integer arg (float/string/bytes/compound) is a later increment → decline.
            let mut bindings = String::new();
            let mut call_args = Vec::with_capacity(args.len());
            for (i, &a) in args.iter().enumerate() {
                let a_ty = type_of(db, a);
                let Some(a_rt) = types::rust_type(&db.name_ctx(), &a_ty) else {
                    return Err(Reject::unsupported(
                        "the Rust backend does not support a host call with an argument of no native Rust type",
                    ));
                };
                let av = emit(db, a, env, ctx)?;
                // Bind each arg to `let __ha<i>` in source order (pins eval order). An INTEGER arg casts to
                // `i64` (uniform marshal, matching H3); a STRING/BYTES arg passes as-is (`String`/`Vec<u8>`).
                // The generated shim param is GENERIC (`fn shim<A>(_a: A)`), so it accepts any arg type and
                // ignores the VALUE — the recorded host response is keyed per-op, arg-independent, and the
                // corpus host-call sequence compares the op NAME only (args are documentation). A
                // float/compound arg has no boundary form yet → decline.
                let bound = if int_rust_ty(&a_rt) {
                    format!("({av}) as i64")
                } else if a_rt == "bool" {
                    // A BOOL argument marshals to `i64` (0/1) — the SAME uniform integer marshal an int arg
                    // uses, matching wasm (which reps Bool as i32 and crosses it fine). `as i64` doesn't apply
                    // to `bool` in Rust, so cast through `i64::from(<bool>)`. The generated shim param is
                    // generic and ignores the value; the corpus host-call sequence compares the op name only.
                    format!("i64::from({av})")
                } else if a_rt == "String" || a_rt == "Vec<u8>" {
                    av
                } else if a_rt == "()" {
                    // A UNIT-typed argument (H9): a nullary-ish host op is written `(io.fetch unit)` — the
                    // `unit` operand carries no data but may be an EFFECTFUL expression, so evaluate it for
                    // its side effect and yield `()` (mirrors H8's unit-result `{ …; () }` shape). The
                    // generic shim param accepts `()`, and eval-order is pinned by the `let` binding below.
                    format!("{{ {av}; () }}")
                } else {
                    return Err(Reject::unsupported(
                        "the Rust backend does not support a host call with a non-integer/bool/string/bytes/unit argument",
                    ));
                };
                bindings.push_str(&format!("let __ha{i} = {bound}; "));
                call_args.push(format!("__ha{i}"));
            }
            let call = format!("crate::{shim}({})", call_args.join(", "));
            // Marshal the shim's return to the op's declared result. The shim's OWN return type is chosen by
            // the runner to match the result kind (the gate driver keys it on the response value text): an
            // INTEGER/BOOL result → the shim returns `i64` (int casts to width; bool reads `!= 0`); a FLOAT
            // result → the shim returns `f64` directly, cast to the declared float width (`as f32`/`as f64`).
            // Any other result (string/bytes/compound) needs its own boundary form → a later increment.
            let marshalled = match types::rust_type(&db.name_ctx(), &result) {
                Some(t) if int_rust_ty(&t) => format!("({call} as {t})"),
                Some(t) if t == "bool" => format!("({call} != 0)"),
                Some(t) if t == "f32" || t == "f64" => format!("({call} as {t})"),
                // A STRING/BYTES result crosses as the value itself — the shim returns `String`/`Vec<u8>`
                // (the gate driver builds it from the recorded response text: a quoted "…" → String, a byte
                // list → Vec<u8>). The declared result type IS that Rust type, so pass the call through.
                Some(t) if t == "String" || t == "Vec<u8>" => call.clone(),
                // A UNIT result (H8): a pure effect op that crosses the boundary FOR ITS SIDE EFFECT only
                // (the host-call observation) and yields the unit value — the shim returns `()` and prints
                // its op. Call it for effect, then evaluate to `()`. This op has NO recorded host_response
                // (it returns nothing), so the driver generates its shim from the case's `(host-calls …)`
                // sequence instead (an op in host_calls but not host_responses → a Unit-result shim).
                Some(t) if t == "()" => format!("{{ {call}; () }}"),
                _ => {
                    return Err(Reject::unsupported(
                        "the Rust backend does not support a host call whose result is not a fixed-width integer, bool, float, unit, string, or bytes",
                    ));
                }
            };
            if bindings.is_empty() {
                Ok(marshalled)
            } else {
                // Wrap the arg-bindings + the call in a block so it's a single expression.
                Ok(format!("{{ {bindings}{marshalled} }}"))
            }
        }
        // A SEQUENCING block — evaluate each `stmt` FOR ITS SIDE EFFECT (discarding its value), then `tail`
        // as the block's value. Produced when a `do`'s non-final statement reaches a side effect selection
        // must emit (a host call whose result is discarded). Emit a Rust block `{ let _ = <stmt0>; …; <tail>
        // }`: `let _ = …` evaluates + drops each statement (a Unit host call leaves `()`; a value-returning
        // one is dropped), and the statements emit in written order so the host calls are observed in exactly
        // the order the program made them (the sequencing invariant the wasm backend also holds).
        Core::Seq { stmts, tail } => {
            let mut body = String::new();
            for &s in stmts.iter() {
                // DROP a non-final statement that reaches NO host call. `lower` only produces a `Seq`
                // (rather than folding to `tail`) because SOME statement reaches a side effect the backend
                // must emit (a host call). A statement whose value is DISCARDED and which reaches no host
                // call is DEAD — per the dead-init ruling (§283) its computation (incl. any trap, e.g. a
                // `(/ 100 0)` div-by-zero) is UNOBSERVED and must be elided, NOT emitted as `let _ = …`
                // (which would run it and spuriously trap — adv-56). Only a statement that reaches a host
                // call is kept, for its boundary-crossing observable effect. (A perform is a host call at
                // this tier; a handled perform folded away, so "reaches a host call" is the right test.)
                if !reaches_host_call(db, s) {
                    // (A) STRICT heap-collection construction (#5194 CASE2, #5328): a strict-construction
                    // arg computation `lower_let` decomposed out of a DEAD list/set/map ctor is marked in
                    // `db.strict_force_eval` and MUST be evaluated (its trap fires) — the (A)-overrides-§283
                    // rule (v-spec-oracle): a reached heap-collection ctor's args are strict, NOT deferrable.
                    // These are SCALAR-typed computations, so force-eval + discard (`let _ = …`) runs the
                    // trap (e.g. `(/ 5 d)` div-by-zero) with no build and no borrowed value touched → no
                    // reclaim. Mirrors the wasm `Core::Seq` emit's strict-force arm; without this the rust
                    // backend §283-elided the dead ctor and dropped the trap (breaker: gate-check-rust red).
                    if db.strict_force_eval.contains(&s) {
                        let sv = emit(db, s, env, ctx)?;
                        body.push_str(&format!("let _ = {sv}; "));
                    }
                    continue;
                }
                let sv = emit(db, s, env, ctx)?;
                body.push_str(&format!("let _ = {sv}; "));
            }
            let t = emit(db, tail, env, ctx)?;
            Ok(format!("{{ {body}{t} }}"))
        }
        // The `?`/try boundary block + break are the wasm backend's `block`/`br` shape (BRICK 3); the
        // Rust backend renders them in a later brick, so it declines for now.
        Core::Block { .. } | Core::Break { .. } => Err(Reject::unsupported(
            "the Rust backend does not support rendering this compound value",
        )),
        // Runtime structural equality over a COMPOUND value. On the wasm backend this is a value-heap
        // equality walk; on the Rust backend a sum/tuple/record maps to a native type that
        // `#[derive(PartialEq, Eq)]` — so when the operand type is `Eq`-derivable (a sum of Int/Bool/nested
        // comparable payloads, a tuple/record of such), emit a native `a == b` (the derived structural
        // equality, which agrees with the wasm heap walk). A non-`Eq` operand (a float-carrying sum, a
        // fn/collection payload) has no derived `==` and DECLINES (decline-don't-miscompile).
        Core::ValueEq { lhs, rhs } => {
            let ty = type_of(db, lhs);
            if ty_supports_native_eq(db, &ty) {
                let l = emit(db, lhs, env, ctx)?;
                let r = emit(db, rhs, env, ctx)?;
                Ok(format!("({l} == {r})"))
            } else if let Some(grounded) = super::enums::ground_free_for_eq(db, &ty)
                && let Some(rust_ty) = types::rust_type(&db.name_ctx(), &grounded)
            {
                // The operand type's ONLY block to native eq is a PHANTOM free var (a variant never
                // constructed — e.g. `Result Int64 ?e` with no `Err` built). Grounding it to `()` gives an
                // `Eq` type with a nameable Rust spelling; pin it via a typed `let` on the lhs so rustc can
                // instantiate the enum (a bare `Ok(5) == Ok(k)` leaves the phantom `E` un-inferable). Sound
                // because no value of the phantom type ever flows — see `enums::ground_free_for_eq`.
                let l = emit(db, lhs, env, ctx)?;
                let r = emit(db, rhs, env, ctx)?;
                Ok(format!(
                    "{{ let __eq_l: {rust_ty} = {l}; (__eq_l == {r}) }}"
                ))
            } else if ty_float_walkable(db, &ty) {
                // The type is NOT native-`Eq` only because it carries a FLOAT leaf (`f64`/`f32` is
                // `PartialEq` not `Eq`, and `==` gives the WRONG NaN/-0.0 answer), but every leaf is either
                // native-Eq OR a float — so a STRUCTURAL walk compares it: each non-float leaf by `==`, each
                // float leaf by the CANONICAL BYTE FORM (`FloatCompare`'s NaN-canonicalizing bit compare —
                // nan==nan, -0.0 != +0.0), matching the wasm heap walk. Bind both operands once (they may be
                // compound/non-Copy) and recurse. `emit_value_eq_walk` handles tuple/record/nominal shapes;
                // a sum/list/map at runtime is NOT reached here (it folds, or declines at `lower`).
                let l = emit(db, lhs, env, ctx)?;
                let r = emit(db, rhs, env, ctx)?;
                let mut helpers = Vec::new();
                let cmp = emit_value_eq_walk(db, &ty, "__eq_l", "__eq_r", &mut helpers)?;
                // Hoist any generated recursive-sum eq helper `fn`s INTO the block so they are in scope where
                // `cmp` (which may call them) runs. A `fn` defined in a block works for self/mutual recursion.
                Ok(format!(
                    "{{ {} let __eq_l = {l}; let __eq_r = {r}; {cmp} }}",
                    helpers.join(" ")
                ))
            } else {
                Err(Reject::unsupported(
                    "runtime structural equality over this compound is not supported by the Rust backend",
                ))
            }
        }
        // RUNTIME COMPOUND ORDERING (`value-cmp`) — the Rust twin of the wasm value-cmp walk. The operand
        // type is an ORDERABLE compound (lower's `is_orderable_compound` routes ONLY tuple/record/list/sum
        // with all-orderable leaves here — a float/char/bytes/set/map leaf declines at lower, never reaching
        // this), and Rust's DERIVED `Ord` on those reps is EXACTLY the blessed lexicographic order: a tuple
        // by field, a `Vec` (List) element-wise with a proper prefix less, a derived-`Ord` enum by
        // discriminant-then-payload — matching core-semantics §Compound Ordering Is Lexicographic and the
        // wasm walk. So emit the native `(l <op> r)`, mirroring `Core::StrCmp`'s native-String compare. A
        // diverging operand short-circuits like `Core::Compare`/`StrCmp`.
        Core::ValueCmp { op, lhs, rhs, .. } => {
            // Lt/Le/Gt/Ge → the BOOLEAN the op names (a native relational compare on the derived-Ord
            // compound). Compare → the three-way `Ordering` SUM (§331: the boolean ops and `compare`
            // surface the SAME total order). For the wasm twin the Ordering value is `res+1` over the
            // value-cmp result; here on the RUST backend the compound `l`/`r` have a derived `Ord`, so we
            // build the Ordering ctor from a NESTED-IF over the native `<`/`>` (the same shape v-inference's
            // scalar/String/BigInt compare uses — `(if l<r Less (if l>r Greater Equal))` — which the rust
            // backend already lowers to the emitted `enum Ordering` ctor paths). The compare NODE's result
            // type IS `Ordering`, so its ctor paths come from `sum_variant_path_of_ty(type_of(id), disc)`
            // (Less=disc 0, Equal=1, Greater=2 — sums.rs). No `.cmp()`→bool collapse; a real Ordering value.
            let is_compare = matches!(op, Prim::Compare);
            let sym = if is_compare {
                "" // unused for Compare (nested-if built below)
            } else {
                compare_sym(op)
                    .ok_or_else(|| Reject::decline("ValueCmp carries a non-compare prim"))?
            };
            if arith_operand_diverges(db, lhs) {
                return emit(db, lhs, env, ctx);
            }
            if arith_operand_diverges(db, rhs) {
                let l = emit(db, lhs, env, ctx)?;
                let r = emit(db, rhs, env, ctx)?;
                return Ok(format!("{{ let _ = {l}; {r} }}"));
            }
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            // SOUNDNESS (breaker/corpus-bugfix #42): the operands map to Rust reps whose DERIVED `Ord` is the
            // blessed lexicographic order — EXCEPT a built-in `Option`, which maps to std `Option` whose
            // derived order is `None < Some`, the REVERSE of Cadenza's declared `Some(disc0) < None(disc1)`.
            // So when the operand type contains a flip-order Option, the native `l < r`/`l.cmp(&r)` gives the
            // WRONG total order (and the flip propagates to a nested Option leaf). Route such a compare through
            // `emit_value_cmp_walk`, which orders every Option position `Some`-before-`None` (Cadenza) while
            // delegating Option-free subtrees to the native `.cmp()`. An Option-FREE operand keeps the native
            // path below — byte-identical to before (the common case). `Result` maps to std `Result` whose
            // `Ok < Err` already matches Cadenza, so it is NOT a flip and stays native.
            let opnd_ty = type_of(db, lhs);
            if ty_uses_flip_order_option(db, &opnd_ty) {
                let mut helpers = Vec::new();
                let walk = emit_value_cmp_walk(db, &opnd_ty, "__cl", "__cr", &mut helpers)?;
                let hs = helpers.join(" ");
                return if is_compare {
                    // The three-way `Ordering` the walk already produces IS the compare result — but the
                    // NODE's result type is Cadenza `Ordering`, whose emitted ctors are `Ordering::{Less,
                    // Equal,Greater}` (sums.rs disc 0/1/2). The walk yields a `core::cmp::Ordering`; map it to
                    // the emitted `Ordering` ctor via a match (the same 3 ctors the nested-if path uses).
                    let ord_ty = type_of(db, id);
                    let less = sum_variant_path_of_ty(db, &ord_ty, 0)?;
                    let equal = sum_variant_path_of_ty(db, &ord_ty, 1)?;
                    let greater = sum_variant_path_of_ty(db, &ord_ty, 2)?;
                    Ok(format!(
                        "{{ {hs} let __cl = {l}; let __cr = {r}; match {walk} {{ core::cmp::Ordering::Less => {less}, core::cmp::Ordering::Greater => {greater}, core::cmp::Ordering::Equal => {equal}, }} }}"
                    ))
                } else {
                    // A relational op (Lt/Le/Gt/Ge) → compare the walk's Ordering against the boolean the op
                    // names (the same mapping compare_sym encodes, but over the corrected Ordering).
                    let (test, negate) = match op {
                        Prim::Lt => ("core::cmp::Ordering::Less", false),
                        Prim::Gt => ("core::cmp::Ordering::Greater", false),
                        Prim::Ge => ("core::cmp::Ordering::Less", true),
                        Prim::Le => ("core::cmp::Ordering::Greater", true),
                        _ => return Err(Reject::decline("ValueCmp carries a non-compare prim")),
                    };
                    let eqop = if negate { "!=" } else { "==" };
                    Ok(format!(
                        "{{ {hs} let __cl = {l}; let __cr = {r}; ({walk} {eqop} {test}) }}"
                    ))
                };
            }
            if is_compare {
                // The compare node's result type is `Ordering`; build its ctor paths and the nested-if.
                // BIND l/r to locals first — each operand is referenced TWICE (`< ` and `> `), and a
                // compound operand may be a call/have effects, so re-emitting the expression would evaluate
                // it twice (wrong + a possible double-borrow). `.cmp(&)` needs a ref; a block with two lets
                // + the nested-if reads the bound values by ref.
                let ord_ty = type_of(db, id);
                let less = sum_variant_path_of_ty(db, &ord_ty, 0)?;
                let equal = sum_variant_path_of_ty(db, &ord_ty, 1)?;
                let greater = sum_variant_path_of_ty(db, &ord_ty, 2)?;
                Ok(format!(
                    "{{ let __cl = {l}; let __cr = {r}; if __cl < __cr {{ {less} }} else if __cl > __cr {{ {greater} }} else {{ {equal} }} }}"
                ))
            } else {
                Ok(format!("({l} {sym} {r})"))
            }
        }
        // RUNTIME DESCRIPTOR-GUIDED STRUCTURAL EQUALITY (`value-eq-shaped`) — the Rust twin of the wasm
        // element-wise walk, for a LIST(-containing) compound with a FLOAT/BYTES leaf that neither native
        // `==` (wrong NaN/-0.0) nor derived `Ord` (float is only `PartialOrd`) can compare. Bind both
        // operands once (they may be compound/non-Copy) and recurse with `emit_value_eq_walk`, which walks a
        // list element-wise (`.iter().zip().all()`) and compares a float leaf by canonical byte form —
        // matching the wasm `value-eq-shaped` descriptor walk.
        Core::ValueEqShaped { lhs, rhs, ty } => {
            let l = emit(db, lhs, env, ctx)?;
            let r = emit(db, rhs, env, ctx)?;
            let mut helpers = Vec::new();
            let cmp = emit_value_eq_walk(db, &ty, "__eq_l", "__eq_r", &mut helpers)?;
            Ok(format!(
                "{{ {} let __eq_l = {l}; let __eq_r = {r}; {cmp} }}",
                helpers.join(" ")
            ))
        }
    }
}

/// Whether `ty` is a compound whose leaves are each either native-`Eq` (via [`enums::ty_supports_eq`]) or a
/// FLOAT — so a structural walk ([`emit_value_eq_walk`]) can compare it (float leaves by the canonical byte
/// form, the rest by `==`). Only the shapes the walk emits qualify: a TUPLE/RECORD/NOMINAL/LIST of walkable
/// leaves, or a bare float. A sum/map/fn does NOT (a runtime sum eq folds or declines at `lower`; admitting
/// them here would emit an unhandled shape). Returns false when the type is ALREADY native-Eq (that path is
/// taken before this) or carries a non-Eq-non-float leaf (a function, an unknown var).
fn ty_float_walkable(db: &mut Db, ty: &Ty) -> bool {
    ty_float_walkable_seen(db, ty, &mut Vec::new())
}

/// Whether a float-carrying MONOMORPHIC sum can be given a hand-written `impl Ord` (so it is usable as a
/// `BTreeSet`/`BTreeMap` key) via [`emit_value_ord_walk`]. TRUE iff: it is a `Ty::Sum`; it is NOT already
/// native-`Ord` (that path derives `Ord` directly — this is only for the ELSE); every payload is
/// float-walkable (`ty_float_walkable` — the eq/ord walks render exactly this domain); it carries NO
/// flip-order `Option` (the ord-walk's native-`.cmp()` fast path would give the WRONG order for an Option
/// leaf — excluded here, declined rather than miscompiled); and it is MONOMORPHIC (no type args — a generic
/// helper signature is a follow-up). `Ast` (a `Float`+`List Ast` sum, no Option, monomorphic) qualifies.
pub(super) fn sum_is_custom_ord(db: &mut Db, ty: &Ty) -> bool {
    let Ty::Sum { args, .. } = ty else {
        return false;
    };
    if !args.is_empty() {
        return false; // generic — a generic __ord_ helper signature is a follow-up
    }
    // Native-Ord sums derive Ord (handled elsewhere); this predicate is only for the float-carrying ELSE.
    if super::enums::ty_supports_eq(db, ty) {
        return false;
    }
    if ty_uses_flip_order_option(db, ty) {
        return false; // an Option leaf's native .cmp() order is the reverse of Cadenza's — decline
    }
    ty_float_walkable(db, ty)
}

/// `ty_float_walkable` with a recursion guard for the SUM descent — a self-referential sum (e.g. `Ast` via
/// `Ast.List (List Ast)`) closes on `seen` so the walk terminates. `emit_value_eq_walk`'s `Ty::Sum` arm
/// renders EXACTLY this domain (the doc invariant that both backends route the same types).
fn ty_float_walkable_seen(db: &mut Db, ty: &Ty, seen: &mut Vec<crate::ast::StructId>) -> bool {
    match ty {
        // A bare float leaf — walkable (canonical-byte compare).
        Ty::Float(_) => true,
        // A native-Eq leaf — walkable (plain `==`). Checked here so a tuple element that is itself Eq
        // (an Int, a Bytes, a nested all-Eq tuple) passes without needing the float path.
        _ if super::enums::ty_supports_eq(db, ty) => true,
        // A tuple/record is walkable iff every element/field is. A nominal newtype walks its inner.
        Ty::Tuple(elems) => {
            let elems: Vec<Ty> = elems.to_vec();
            elems.iter().all(|e| ty_float_walkable_seen(db, e, seen))
        }
        Ty::Record(fields) => {
            let vals: Vec<Ty> = fields.values().cloned().collect();
            vals.iter().all(|v| ty_float_walkable_seen(db, v, seen))
        }
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            ty_float_walkable_seen(db, &inner, seen)
        }
        // A Qty erases to its inner magnitude (unit is compile-time); walk the inner.
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_float_walkable_seen(db, &inner, seen)
        }
        // A LIST — walkable iff its element is (a `List<Float>` compares element-wise via the `.iter().zip()`
        // walk, each float element by canonical byte form). This is what `Core::ValueEqShaped` routes here:
        // a list spine that native `==` (`Vec: PartialEq`) would compare with the wrong NaN/-0.0 answer.
        Ty::List(elem) => {
            let elem = (**elem).clone();
            ty_float_walkable_seen(db, &elem, seen)
        }
        // A MAP — walkable iff its KEY is an ord-key (so keys compare by `==`: an Int/String/… natively, a
        // float KEY via the `Eq` `__CdzF64` wrapper) AND its VALUE is walkable. Reached only when the Map is
        // NOT already native-`Eq` (a float / float-carrying VALUE — a float KEY alone keeps the Map native-`Eq`
        // via the wrapper, #7419, and takes the `==` fast-path). `emit_value_eq_walk`'s Map arm zips the sorted
        // `(k, v)` pairs: keys by `==`, the value by the value walk (a float value by canonical byte form —
        // `{5: NaN} == {5: NaN}`, matching wasm's map value-eq, NOT `BTreeMap`'s derived `PartialEq`).
        Ty::Map(k, v) => {
            let k = (**k).clone();
            let v = (**v).clone();
            types::ty_is_ord_key(db, &k) && ty_float_walkable_seen(db, &v, seen)
        }
        // A SUM whose payloads carry a Float and/or a List (so it is NOT already native-Eq — that path was
        // taken above) — walkable iff every variant's payload is. `emit_value_eq_walk` renders a Sum through
        // a generated recursive helper `fn __eq_<Ident>` (call-indirection), so a self-referential sum (e.g.
        // `Ast` via `Ast.List (List Ast)`, or a `Box`ed `Tree.Node Tree Tree`) is fine: a recursive back-edge
        // returns TRUE (the helper's runtime recursion terminates over the finite value), matching the wasm
        // `eq_shaped_walkable` (whose runtime walk is likewise iterative/recursive). A GENERIC recursive sum
        // is the one exception the emit still declines (a generic helper signature is a follow-up), but the
        // back-edge here can't see the args cheaply and a false-positive just yields a clean emit-time decline
        // downstream (reject-don't-miscompile), so admit it and let `emit_value_eq_walk` draw that line.
        Ty::Sum { decl, .. } => {
            if seen.contains(decl) {
                return true; // recursive back-edge — the helper `fn` breaks the cycle at runtime (see emit)
            }
            seen.push(*decl);
            let variant_count = db.type_decl_by_occ(*decl).map(|t| t.variants.len());
            let mut ok = variant_count.is_some();
            if let Some(vc) = variant_count {
                for disc in 0..vc as u32 {
                    // A nullary variant (no payload) is walkable; a payload variant's payload must be.
                    if let Some(payload_ty) = variant_payload_ty(db, ty, disc)
                        && !ty_float_walkable_seen(db, &payload_ty, seen)
                    {
                        ok = false;
                        break;
                    }
                }
            }
            seen.pop();
            ok
        }
        // A map, function, or unknown var — not walked by this slice.
        _ => false,
    }
}

/// Emit a boolean Rust expression comparing the two OWNED Rust expressions `l`/`r` (both of type `ty`) by
/// STRUCTURAL value equality — the recursive walk for a compound carrying a FLOAT leaf that cannot use a
/// derived `==`. Each leaf compares as: a native-`Eq` leaf → `(l == r)`; a FLOAT leaf → the canonical byte
/// form (NaN-canonicalizing bit compare, `nan==nan`, `-0.0 != +0.0`, byte-identical to `FloatCompare`'s
/// emit + the wasm heap walk); a TUPLE/RECORD → the `&&`-chain of its projected fields (`.0`/`.1`… in
/// rust_type's element order, which for a record is sorted-key order); a NOMINAL → its inner (the newtype is
/// transparent). `l`/`r` are already-bound identifiers (or field projections built on them), so re-reading
/// them per leaf is sound (a projection of a bound value; the enclosing bind is done once by the caller).
pub(super) fn emit_value_eq_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    emit_value_eq_walk_seen(db, ty, l, r, &mut Vec::new(), helpers)
}

/// The ORD twin of [`emit_value_eq_walk`]: build a `core::cmp::Ordering` expression comparing two values of a
/// FLOAT-CARRYING type (one that is NOT native-`Ord`-derivable — a float leaf makes the derive impossible)
/// in the blessed lexicographic order, with each float leaf ordered by its CANONICAL BIT FORM (NaN folded to
/// one form, matching the runtime's canonical-byte float order and `emit_value_eq_walk`'s float arm). Used to
/// give a float-carrying sum (`Ast`) a hand-written `impl Ord` so it can be a `BTreeSet`/`BTreeMap` key —
/// `enums.rs` wraps the generated `__ord_<Ident>` helper in the trait impl. Mirrors the eq-walk's shape
/// (native-`Ord` fast path, tuple/record lexicographic, list element-wise-then-length, sum via a recursive
/// helper). REQUIRES the type carry no flip-order `Option` (the admission gate ensures this): the native
/// `.cmp()` fast path for an Option-free native-Ord leaf is then the correct Cadenza order.
pub(super) fn emit_value_ord_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    emit_value_ord_walk_seen(db, ty, l, r, &mut Vec::new(), helpers)
}

fn emit_value_ord_walk_seen(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    // A NATIVELY-`Ord` leaf (Int/Bool/Bytes/String/BigInt/… and any all-`Ord`-DERIVING compound/sum) — a
    // plain `.cmp()`. Checked FIRST so an `Ord` sub-tree compares in one `.cmp()` rather than being walked;
    // it is also the ONLY spelling for a map/set leaf the walk does not descend. Gate on `ty_derives_eq`
    // (the DERIVE condition), NOT `ty_is_ord`: a CUSTOM-ord sum (`Ast`) satisfies `ty_is_ord` but its `.cmp()`
    // IS this very `__ord_` helper — short-circuiting to `l.cmp(&r)` would recurse forever. Such a sum must
    // fall through to the `Ty::Sum` walk arm. A flip-order Option would also be wrong here, but the admission
    // gate (`sum_is_custom_ord`) excludes it. The `&` handles a non-Copy compound.
    if super::enums::ty_supports_eq(db, ty) {
        // `ty_supports_eq` = the enum/compound DERIVES Eq (hence Ord) — a native `.cmp()` is the Cadenza
        // order. BigInt/Rational reach here too: `ty_derives_eq`'s tail arm returns `true` for them (they map
        // to `cdz_num::Big`/`Rational`, which derive `Ord`), so this one branch covers them — no separate
        // BigInt/Rational arm is needed (an earlier one was dead + its comment self-contradicting; dropped per
        // github-liaison's PR#1617 review). A custom-ord sum is NOT Eq-deriving (it carries a float), so it
        // does not take this path — it falls through to the `Ty::Sum` walk arm (whose `.cmp()` IS this
        // `__ord_` helper; short-circuiting here would recurse forever). The `&` handles a non-Copy compound.
        return Ok(format!("{l}.cmp(&{r})"));
    }
    match ty {
        // A FLOAT leaf — order by the canonical bit pattern (NaN folded to one form), mirroring the eq-walk's
        // float arm but with `.cmp()` on the bits (a total order over the canonicalized `u{32,64}`). This
        // matches the runtime's canonical-byte float order (so a float set/map key agrees with wasm).
        Ty::Float(ft) => {
            let (canon_nan, bits_ty) = if ft.ground_width() == 32 {
                ("0x7FC0_0000u32", "u32")
            } else {
                ("0x7FF8_0000_0000_0000u64", "u64")
            };
            let canon = |v: &str| {
                format!(
                    "({{ let __f = {v}; if __f.is_nan() {{ {canon_nan} }} else {{ __f.to_bits() as {bits_ty} }} }})"
                )
            };
            Ok(format!("{}.cmp(&{})", canon(l), canon(r)))
        }
        // A TUPLE — lexicographic: compare element 0, and only on `Equal` fall through (`.then_with`).
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let mut acc: Option<String> = None;
            for (i, e) in elems.iter().enumerate() {
                let part = emit_value_ord_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A RECORD — a tuple in sorted-key order; same lexicographic chain over `.i`.
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            let mut acc: Option<String> = None;
            for (i, e) in tys.iter().enumerate() {
                let part = emit_value_ord_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A LIST — `Vec<T>` compared element-wise lexicographically, then by length (the derived-`Vec` Ord
        // shape): the first non-Equal zipped element decides, else compare lengths. Built over bound refs.
        Ty::List(elem) => {
            let elem = (**elem).clone();
            let elem_cmp = emit_value_ord_walk_seen(db, &elem, "__le", "__re", seen, helpers)?;
            Ok(format!(
                "{l}.iter().zip({r}.iter()).map(|(__le, __re)| {elem_cmp}).find(|__o| *__o != core::cmp::Ordering::Equal).unwrap_or_else(|| {l}.len().cmp(&{r}.len()))"
            ))
        }
        // A NOMINAL newtype is transparent — walk the inner over the same operands (no projection).
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_ord_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A Qty erases to its inner magnitude — walk the inner (same operands).
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_ord_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A SUM whose payloads carry a Float/List (so it is NOT native-Ord — that path was taken first).
        // Compared through a generated recursive helper `fn __ord_<Ident>(l, r) -> Ordering` that matches
        // `(l, r)`: a same-variant pair compares payloads (lexicographic over the payload walk); a
        // mismatched pair compares the DECLARED discriminant ORDINALS (Cadenza declared order = the enum
        // declaration order, which for a monomorphic user sum matches the emitted enum's variant order). The
        // helper (call-indirection) makes a RECURSIVE sum (`Ast.List (List Ast)`) terminate at runtime —
        // exactly like the eq-walk's `__eq_<Ident>`. Monomorphic only (a generic re-entry declines).
        Ty::Sum { decl, args } => {
            let enum_ty = super::types::rust_type(&db.name_ctx(), ty)
                .ok_or_else(|| Reject::decline("sum ord: no rust type for the enum"))?;
            let name = db
                .type_decl_by_occ(*decl)
                .map(|t| t.name.clone())
                .ok_or_else(|| Reject::decline("sum ord: no decl name"))?;
            let fn_name = format!("__ord_{}", super::types::sum_ident(&name));
            let sum_ty = ty.clone();
            if seen.contains(&sum_ty) {
                if !args.is_empty() {
                    // (Internal: rendering this would need a generic helper fn; not built yet.)
                    return Err(Reject::unsupported(
                        "runtime ordering over a recursive generic sum is not supported by the Rust backend",
                    ));
                }
                return Ok(format!("{fn_name}(&{l}, &{r})"));
            }
            if !args.is_empty() {
                return Err(Reject::unsupported(
                    "runtime ordering over a generic sum is not supported by the Rust backend",
                ));
            }
            seen.push(sum_ty);
            let variant_count = match db.type_decl_by_occ(*decl).map(|t| t.variants.len()) {
                Some(n) => n,
                None => {
                    seen.pop();
                    return Err(Reject::decline("sum ord: no variant count"));
                }
            };
            // Same-variant arms compare payloads; the fallthrough compares declared ordinals. `__ord_pos`
            // maps a ref to its declared position (a small helper match, emitted alongside).
            let mut same_arms = Vec::with_capacity(variant_count + 1);
            let mut pos_arms = Vec::with_capacity(variant_count);
            let mut arm_err: Option<Reject> = None;
            for disc in 0..variant_count as u32 {
                let path = match sum_variant_path_of_ty(db, ty, disc) {
                    Ok(p) => p,
                    Err(e) => {
                        arm_err = Some(e);
                        break;
                    }
                };
                match variant_payload_ty(db, ty, disc) {
                    None => {
                        // Nullary — same-variant pair is `Equal`; position arm maps the bare ctor to its disc.
                        same_arms.push(format!("({path}, {path}) => core::cmp::Ordering::Equal,"));
                        pos_arms.push(format!("{path} => {disc}u32,"));
                    }
                    Some(payload_ty) => {
                        let deref = if super::enums::variant_is_recursive(db, ty, disc) {
                            "**"
                        } else {
                            "*"
                        };
                        let lp = format!("({deref}__lp)");
                        let rp = format!("({deref}__rp)");
                        match emit_value_ord_walk_seen(db, &payload_ty, &lp, &rp, seen, helpers) {
                            Ok(cmp) => {
                                same_arms.push(format!("({path}(__lp), {path}(__rp)) => {cmp},"));
                                pos_arms.push(format!("{path}(_) => {disc}u32,"));
                            }
                            Err(e) => {
                                arm_err = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
            seen.pop();
            if let Some(e) = arm_err {
                return Err(e);
            }
            // A same-discriminant pair is handled by an arm above; a mismatched pair compares declared
            // ordinals via the position helper. The `__pos` closure maps each operand to its declared disc.
            same_arms.push("_ => __pos(l).cmp(&__pos(r)),".to_string());
            if !helpers
                .iter()
                .any(|h| h.contains(&format!("fn {fn_name}(")))
            {
                helpers.push(format!(
                    "#[allow(unused)] fn {fn_name}(l: &{enum_ty}, r: &{enum_ty}) -> core::cmp::Ordering {{ \
                     fn __pos(v: &{enum_ty}) -> u32 {{ match v {{ {} }} }} \
                     match (l, r) {{ {} }} }}",
                    pos_arms.join(" "),
                    same_arms.join(" ")
                ));
            }
            Ok(format!("{fn_name}(&{l}, &{r})"))
        }
        _ => Err(Reject::unsupported(
            "runtime ordering over this compound is not supported by the Rust backend",
        )),
    }
}

/// [`emit_value_eq_walk`] with a `seen` set of sum decls currently being expanded (the recursion guard that
/// routes a self-referential sum through its helper `fn` instead of expanding inline) and a `helpers` sink
/// that collects the generated recursive `fn __eq_<Ident>(l, r) -> bool` definitions. A user SUM is compared
/// through such a helper (call-indirection), mirroring the render crate's `__render_<Ident>`: this is what
/// makes a RECURSIVE sum (via a `Box`ed variant OR through a `List`/tuple element as in `Ast.List (List
/// Ast)`) terminate — inlining a `match` per payload would expand UNBOUNDEDLY at compile time (a codegen
/// stack overflow), but the helper moves the recursion to Rust RUNTIME over the finite value. A self-
/// referential payload position, reached while the sum is on `seen`, emits a CALL to the same helper. The
/// caller hoists `helpers` into the enclosing block so the `fn`s are in scope where the returned `cmp` runs.
fn emit_value_eq_walk_seen(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    // Keyed on the FULL instantiated sum type (`Ty`, compared by decl+args), NOT the bare `StructId` decl —
    // mirrors the value-CMP walk. A NESTED distinct instantiation like `(Option (Option Float64))` (which
    // reaches this walk because the Float leaf is NOT native-`Eq`, so the `ty_supports_eq` `==` fast-path
    // above does not fire) re-enters the SAME `Option` decl but is a DIFFERENT type; a decl-only key
    // false-tripped the "recursive generic" decline. Keyed on the full type, only TRUE self-recursion
    // (identical decl+args) re-enters — nested instantiations expand inline (finite depth).
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    // A native-Eq leaf (Int/Bool/Bytes/String/BigInt/… and any all-Eq compound) — a plain `==`. Checked
    // FIRST so an Eq sub-tree compares in one `==` rather than being walked field-by-field (identical
    // result, smaller emit; and it is the ONLY path for a sum/list/map leaf, which the walk does not spell).
    if super::enums::ty_supports_eq(db, ty) {
        return Ok(format!("({l} == {r})"));
    }
    match ty {
        // A FLOAT leaf — the canonical byte form (mirror `FloatCompare`'s FEq emit). Canonicalize each side
        // to its integer bit pattern with NaN folded to one form, then integer-`==`.
        Ty::Float(ft) => {
            let (canon_nan, bits_ty) = if ft.ground_width() == 32 {
                ("0x7FC0_0000u32", "u32")
            } else {
                ("0x7FF8_0000_0000_0000u64", "u64")
            };
            let canon = |v: &str| {
                format!(
                    "({{ let __f = {v}; if __f.is_nan() {{ {canon_nan} }} else {{ __f.to_bits() as {bits_ty} }} }})"
                )
            };
            Ok(format!("({} == {})", canon(l), canon(r)))
        }
        // A TUPLE — the `&&`-chain of element comparisons, each projected `.i` off both operands.
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            let mut parts = Vec::with_capacity(elems.len());
            for (i, e) in elems.iter().enumerate() {
                parts.push(emit_value_eq_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?);
            }
            Ok(join_and(parts))
        }
        // A RECORD — a tuple in rust_type's SORTED-KEY order, so project `.i` over the sorted fields.
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            let mut parts = Vec::with_capacity(tys.len());
            for (i, e) in tys.iter().enumerate() {
                parts.push(emit_value_eq_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?);
            }
            Ok(join_and(parts))
        }
        // A LIST — a `Vec<T>`, compared element-wise: equal LENGTHS and every zipped element equal under the
        // element walk (a float element by canonical byte form). `.len()` first (a length mismatch decides
        // immediately, and short-circuits the zip), then `.iter().zip().all()` with the element comparison
        // built over the bound refs `__le`/`__re`. This is the rust twin of the wasm `value-eq-shaped` list
        // spine walk — element-wise so a concat-built and a push-built `[1.0, 2.0]` compare equal (§"Two
        // lists ... equal ... independent of how each was constructed"), and the float leaf uses the
        // canonical byte form (NOT `Vec`'s derived `PartialEq`, which would give the wrong NaN/-0.0 answer).
        Ty::List(elem) => {
            let elem = (**elem).clone();
            let elem_cmp = emit_value_eq_walk_seen(db, &elem, "__le", "__re", seen, helpers)?;
            Ok(format!(
                "({l}.len() == {r}.len() && {l}.iter().zip({r}.iter()).all(|(__le, __re)| {elem_cmp}))"
            ))
        }
        // A MAP reaches this walk ONLY when its VALUE is not native-`Eq` (a float value, or a compound
        // carrying one) — a `Map` whose value IS `Eq` took the `==` fast-path above. The KEY is always an
        // ord-key (hence `Eq`: a float key is the `__CdzF64` wrapper, canonical), so keys compare by `==`;
        // the VALUE is a RAW slot (`f64` for a float value), walked so a NaN/-0.0 value compares by the
        // canonical byte form (matching wasm's map value-eq — `{5: NaN} == {5: NaN}`), NOT `BTreeMap`'s
        // derived `PartialEq` (which would give the wrong NaN answer). Both `BTreeMap`s iterate in sorted
        // KEY order, so equal maps yield the same `(k, v)` sequence: equal lengths + every zipped pair with
        // equal key and value-walk-equal value. (No `Set` arm here — a `Set` is always native-`Eq`, closes
        // its element via the ord-wrapper, so it never reaches this walk.)
        Ty::Map(_k, v) => {
            let vty = (**v).clone();
            let val_cmp = emit_value_eq_walk_seen(db, &vty, "__lv", "__rv", seen, helpers)?;
            Ok(format!(
                "({l}.len() == {r}.len() && {l}.iter().zip({r}.iter()).all(|((__lk, __lv), (__rk, __rv))| __lk == __rk && ({val_cmp})))"
            ))
        }
        // A NOMINAL newtype is transparent — its Rust value IS the inner, so walk the inner over the same
        // operands (no projection; the newtype adds no Rust wrapper).
        Ty::Nominal { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_eq_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A Qty erases to its inner magnitude — walk the inner (same operands).
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            emit_value_eq_walk_seen(db, &inner, l, r, seen, helpers)
        }
        // A SUM whose payloads carry a Float/List (so it is NOT native-Eq — that path was taken first). It is
        // compared through a generated recursive helper `fn __eq_<Ident>(l: &Enum, r: &Enum) -> bool` that
        // `match`es `(l, r)` over the emitted enum's variants: each variant arm binds its payload on BOTH
        // sides and walks the payload (a float leaf by canonical byte form, a list element-wise); a nullary
        // variant → `true`; a mismatched variant pair → the `_ => false` catch-all. This EXACTLY mirrors the
        // render crate's `__render_<Ident>` helper (cdz-rust-render) and the wasm `value-eq-shaped` Shape::Sum
        // walk. Routing through a helper (call-indirection) is what makes a RECURSIVE sum TERMINATE: a
        // self-referential payload (via a `Box`ed variant OR through a `List`/tuple element, `Ast.List (List
        // Ast)`) would expand the emit UNBOUNDEDLY if inlined (a codegen stack overflow), but the helper moves
        // the recursion to Rust RUNTIME over the finite value — a self-reference reached while the decl is on
        // `seen` emits a CALL to the same helper, and a nullary leaf terminates the runtime walk.
        //
        // GENERIC instantiations (args non-empty) still DECLINE on re-entry: a helper for `Box<T0>` would need
        // a spelled generic signature (`fn __eq_Box<T0: ?>(…)`) with the right payload bound — a follow-up.
        // A non-recursive generic sum is native-Eq (took the `==` path); only a recursive generic one reaches
        // here, and it declines cleanly (todo, not a miscompile — wasm computes it).
        Ty::Sum { decl, args } => {
            let enum_ty = super::types::rust_type(&db.name_ctx(), ty)
                .ok_or_else(|| Reject::decline("sum eq: no rust type for the enum"))?;
            let name = db
                .type_decl_by_occ(*decl)
                .map(|t| t.name.clone())
                .ok_or_else(|| Reject::decline("sum eq: no decl name"))?;
            let fn_name = format!("__eq_{}", super::types::sum_ident(&name));
            let sum_ty = ty.clone();
            // On re-entry of THIS EXACT instantiated type (a true self-referential cycle — identical
            // decl+args), emit a CALL to its helper (the recursion base). A GENERIC self-recursive sum still
            // can't spell a generic helper signature → decline. But a NESTED DISTINCT instantiation
            // (`Option<Option<T>>` reaching `Option<T>`) is a DIFFERENT type, does NOT re-enter, and takes the
            // inline-match generic path below — the gap this closes (was falsely declined as recursive-generic).
            if seen.contains(&sum_ty) {
                if !args.is_empty() {
                    return Err(Reject::unsupported(
                        "runtime structural equality over a recursive generic sum is not supported by the Rust backend",
                    ));
                }
                return Ok(format!("{fn_name}(&{l}, &{r})"));
            }
            seen.push(sum_ty);
            let variant_count = match db.type_decl_by_occ(*decl).map(|t| t.variants.len()) {
                Some(n) => n,
                None => {
                    seen.pop();
                    return Err(Reject::decline("sum eq: no variant count"));
                }
            };
            let mut arms = Vec::with_capacity(variant_count + 1);
            let mut arm_err: Option<Reject> = None;
            for disc in 0..variant_count as u32 {
                let path = match sum_variant_path_of_ty(db, ty, disc) {
                    Ok(p) => p,
                    Err(e) => {
                        arm_err = Some(e);
                        break;
                    }
                };
                match variant_payload_ty(db, ty, disc) {
                    None => {
                        // Nullary variant — a bare `Enum::V` on both sides is equal (the discriminant matched).
                        arms.push(format!("({path}, {path}) => true,"));
                    }
                    Some(payload_ty) => {
                        // One payload field (a single type OR a tuple type — the walk handles both). A
                        // recursive variant boxes the field, so the bound ref derefs one extra level.
                        let deref = if super::enums::variant_is_recursive(db, ty, disc) {
                            "**"
                        } else {
                            "*"
                        };
                        // PARENTHESIZE the deref: the payload walk may append a method call (a `List` payload
                        // emits `{l}.len()`/`{l}.iter()`), and `.` binds tighter than prefix `*`, so a bare
                        // `*__lp.len()` parses as `*(__lp.len())` (deref of the `usize`, E0614). `(*__lp)` /
                        // `(**__lp)` binds the deref first. (Not hit before: a `List`-carrying sum payload was
                        // always declined as recursive; the helper now renders it, exercising this path.)
                        let lp = format!("({deref}__lp)");
                        let rp = format!("({deref}__rp)");
                        match emit_value_eq_walk_seen(db, &payload_ty, &lp, &rp, seen, helpers) {
                            Ok(cmp) => arms.push(format!("({path}(__lp), {path}(__rp)) => {cmp},")),
                            Err(e) => {
                                arm_err = Some(e);
                                break;
                            }
                        }
                    }
                }
            }
            seen.pop();
            if let Some(e) = arm_err {
                return Err(e);
            }
            // Mismatched-variant pair → not equal. (Only reached when the two discriminants differ; a matched
            // pair took its arm above.)
            arms.push("_ => false,".to_string());
            // A GENERIC instantiation is NOT routed through a helper (no generic signature to spell) — emit
            // the inline `match` as before (a non-recursive generic sum works; a recursive one already
            // declined above on re-entry). Only a MONOMORPHIC user sum generates + calls a helper `fn`.
            if !args.is_empty() {
                return Ok(format!("(match (&{l}, &{r}) {{ {} }})", arms.join(" ")));
            }
            // Emit the helper `fn` once (a re-entry on the same decl returned a call above, so a given decl's
            // helper is pushed exactly once per top-level walk). `#[allow(unused)]` — a mutually-referenced
            // helper may be defined but only reached via another. The helper takes `&Enum` refs (the caller
            // passes `&value`, and a boxed self-reference deref-then-re-borrows via the call `&{l}`).
            if !helpers
                .iter()
                .any(|h| h.contains(&format!("fn {fn_name}(")))
            {
                helpers.push(format!(
                    "#[allow(unused)] fn {fn_name}(l: &{enum_ty}, r: &{enum_ty}) -> bool {{ match (l, r) {{ {} }} }}",
                    arms.join(" ")
                ));
            }
            Ok(format!("{fn_name}(&{l}, &{r})"))
        }
        // Any other shape should have been excluded by `ty_float_walkable` before we got here.
        _ => Err(Reject::unsupported(
            "runtime structural equality over this compound is not supported by the Rust backend",
        )),
    }
}

/// Join boolean parts with `&&`, yielding `true` for an empty list (an empty tuple/record is always equal to
/// itself) and the sole part unparenthesized for a singleton. A multi-part chain is parenthesized so it
/// composes as one boolean sub-expression inside a larger `&&`.
fn join_and(parts: Vec<String>) -> String {
    match parts.len() {
        0 => "true".to_string(),
        1 => parts.into_iter().next().unwrap(),
        _ => format!("({})", parts.join(" && ")),
    }
}

/// Whether `ty` contains (at any depth) a built-in `Option` whose Rust std-`Option` DERIVED ORDER
/// DISAGREES with the Cadenza declared variant order — the soundness trap `emit_value_cmp_walk` exists to
/// fix. Cadenza declares `Some` (disc 0) `< None` (disc 1), but Rust's `std::option::Option` derives
/// `None < Some` — the REVERSE. So a native `l < r` / `l.cmp(&r)` on an `Option`-typed (or Option-containing)
/// value gives the WRONG total order (`compare (Some 3) None` → std `Greater`, Cadenza `Less`). `Result`
/// maps to std `Result` whose `Ok < Err` MATCHES Cadenza's declared `Ok < Err`, so it needs no correction;
/// only `Option` flips. A NON-flip type (no std-Option anywhere) keeps the native compare (byte-identical to
/// before — the overwhelmingly common case). A USER `(type Option …)` emits its own decl-order enum (correct
/// native Ord), so it is NOT a flip — `is_builtin_std_sum` distinguishes it. (breaker/corpus-bugfix #42.)
fn ty_uses_flip_order_option(db: &mut Db, ty: &Ty) -> bool {
    ty_uses_flip_order_option_seen(db, ty, &mut Vec::new())
}

fn ty_uses_flip_order_option_seen(
    db: &mut Db,
    ty: &Ty,
    seen: &mut Vec<crate::ast::StructId>,
) -> bool {
    match ty.strip_nominal() {
        Ty::Tuple(elems) => {
            let elems = elems.clone();
            elems
                .iter()
                .any(|e| ty_uses_flip_order_option_seen(db, e, seen))
        }
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            tys.iter()
                .any(|t| ty_uses_flip_order_option_seen(db, t, seen))
        }
        Ty::List(elem) => {
            let elem = (**elem).clone();
            ty_uses_flip_order_option_seen(db, &elem, seen)
        }
        Ty::Qty { inner, .. } => {
            let inner = (**inner).clone();
            ty_uses_flip_order_option_seen(db, &inner, seen)
        }
        s @ Ty::Sum { decl, .. } => {
            let decl_occ = *decl;
            // The std-mapped `Option` builtin is the ONLY flip. Check via the emit's own recognizer.
            let is_flip_option = db
                .type_decl_by_occ(decl_occ)
                .map(|d| {
                    let d = d.clone();
                    super::enums::is_builtin_std_sum(db, &d) && d.name == "Option"
                })
                .unwrap_or(false);
            if is_flip_option {
                return true;
            }
            // RECURSION GUARD: a self-referential sum (e.g. `Ast` carrying `List Ast`) would otherwise loop
            // forever through its payloads. Skip a decl already on the descent path — if it were flip-order
            // it'd have returned true at its first (Option) visit; re-entering it adds no new Option.
            if seen.contains(&decl_occ) {
                return false;
            }
            seen.push(decl_occ);
            // Otherwise recurse into the variant payloads — a user sum / Result may CARRY an Option leaf.
            let s = s.clone();
            let vcount = db
                .type_decl_by_occ(decl_occ)
                .map(|d| d.variants.len())
                .unwrap_or(0);
            let found = (0..vcount as u32).any(|disc| {
                variant_payload_ty(db, &s, disc)
                    .map(|p| ty_uses_flip_order_option_seen(db, &p, seen))
                    .unwrap_or(false)
            });
            seen.pop();
            found
        }
        _ => false,
    }
}

/// Emit a Rust `core::cmp::Ordering` expression comparing `l`/`r` (both of type `ty`) in the CADENZA
/// DECLARED total order — the correction for the std-`Option` order flip ([`ty_uses_flip_order_option`]).
/// Used by `Core::ValueCmp` ONLY when `ty` contains a flip-order `Option`; an Option-free type keeps the
/// native `l < r` / `l.cmp(&r)` (byte-identical). The walk is lexicographic (matching core-semantics
/// §Compound Ordering Is Lexicographic + the wasm value-cmp walk), delegating an Option-FREE subtree to the
/// native `.cmp()` (correct there — only Option's derived Ord disagrees) and handling an `Option` position
/// by an EXPLICIT `Some`-before-`None` match so the order is `Some(_) < None` (Cadenza), overriding std's
/// `None < Some`. `helpers` collects generated recursive `fn`s (a self-referential Option-carrying sum),
/// mirroring `emit_value_eq_walk`.
fn emit_value_cmp_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    emit_value_cmp_walk_seen(db, ty, l, r, &mut Vec::new(), helpers)
}

fn emit_value_cmp_walk_seen(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    // The recursion guard keys on the FULL instantiated sum type (`Ty`, compared by decl + args), NOT the
    // bare `StructId` decl: a NESTED distinct instantiation like `(Option (Option Int64))` re-enters the
    // SAME `Option` decl but is a DIFFERENT type, so a decl-only key would false-trip the "recursive
    // generic" decline. Keyed on the full type, only a TRULY self-referential type (identical decl+args)
    // re-enters — the finite value's own cycle — and nested instantiations expand inline (finite depth).
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    // An Option-FREE subtree compares correctly under the native derived `Ord` — emit `l.cmp(&r)` and stop
    // walking (smaller emit, and it is the ONLY spelling for a Map/Set/other leaf the walk does not descend).
    // The ref-`&` handles a non-Copy compound; a Copy scalar coerces fine. This is what keeps a compare with
    // NO Option byte-identical to the pre-fix native path (the walk only diverges at an actual Option).
    if !ty_uses_flip_order_option(db, ty) {
        return Ok(format!("{l}.cmp(&{r})"));
    }
    match ty.strip_nominal().clone() {
        // A TUPLE — lexicographic: compare field 0, and only on `Equal` fall through to the next (`.then_with`).
        Ty::Tuple(elems) => {
            let mut acc: Option<String> = None;
            for (i, e) in elems.iter().enumerate() {
                let part = emit_value_cmp_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A RECORD — a tuple in sorted-key order; same lexicographic chain over `.i`.
        Ty::Record(fields) => {
            let tys: Vec<Ty> = fields.values().cloned().collect();
            let mut acc: Option<String> = None;
            for (i, e) in tys.iter().enumerate() {
                let part = emit_value_cmp_walk_seen(
                    db,
                    e,
                    &format!("{l}.{i}"),
                    &format!("{r}.{i}"),
                    seen,
                    helpers,
                )?;
                acc = Some(match acc {
                    None => part,
                    Some(prev) => format!("{prev}.then_with(|| {part})"),
                });
            }
            Ok(acc.unwrap_or_else(|| "core::cmp::Ordering::Equal".to_string()))
        }
        // A LIST — `Vec<T>` compared element-wise lexicographically, then by length (the derived `Vec` Ord
        // shape): zip + find the first non-Equal element compare, else compare lengths. Built over bound refs.
        Ty::List(elem) => {
            let elem_cmp = emit_value_cmp_walk_seen(db, &elem, "__le", "__re", seen, helpers)?;
            Ok(format!(
                "{l}.iter().zip({r}.iter()).map(|(__le, __re)| {elem_cmp}).find(|__o| *__o != core::cmp::Ordering::Equal).unwrap_or_else(|| {l}.len().cmp(&{r}.len()))"
            ))
        }
        Ty::Qty { inner, .. } => emit_value_cmp_walk_seen(db, &inner, l, r, seen, helpers),
        // A SUM. The Option case (the flip) is ordered `Some`-before-`None` (Cadenza) by the declared-ordinal
        // match; any other sum carrying an Option leaf compares by declared discriminant then payload — the
        // correct Cadenza order, computed WITHOUT trusting std's derived Ord.
        Ty::Sum { .. } => emit_sum_cmp_walk(db, &ty.strip_nominal().clone(), l, r, seen, helpers),
        // Unreachable: an Option-free shape (scalar/float/…) took the native `.cmp()` early-return above, and
        // only Tuple/Record/List/Qty/Sum can CONTAIN an Option. Decline defensively rather than miscompile.
        _ => Err(Reject::decline(
            "value-cmp walk reached an unexpected Option-containing shape",
        )),
    }
}

/// The sum arm of [`emit_value_cmp_walk_seen`]: compare two sum values in CADENZA DECLARED variant order
/// (discriminant ascending by declaration, then payload lexicographically) via a generated `match (l, r)`.
/// This is what overrides std `Option`'s `None < Some`: the arms are emitted in DECLARATION order (`Some`
/// disc 0 first, `None` disc 1), and a lower-disc-vs-higher-disc pair yields `Less`/`Greater` by declared
/// position — NOT by std's derived discriminant. Routed through a helper `fn __cmp_<Ident>` for a recursive
/// sum (like `emit_value_eq_walk`'s `__eq_` helper) so it terminates.
fn emit_sum_cmp_walk(
    db: &mut Db,
    ty: &Ty,
    l: &str,
    r: &str,
    seen: &mut Vec<Ty>,
    helpers: &mut Vec<String>,
) -> Result<String, Reject> {
    let sum_ty = ty.strip_nominal().clone();
    match &sum_ty {
        Ty::Sum { .. } => {}
        _ => return Err(Reject::decline("value-cmp: not a sum type")),
    };
    let enum_ty = super::types::rust_type(&db.name_ctx(), ty)
        .ok_or_else(|| Reject::decline("value-cmp: no rust type for the sum"))?;
    // The helper fn name is mangled by the FULL INSTANTIATED type (via `rust_type`), not the bare sum name:
    // a nested `(Option (Option Int64))` needs a distinct `fn __cmp_*` for the outer `Option<Option<i64>>`
    // and the inner `Option<i64>` (different signatures) — a bare `__cmp_Option` would collide (the dedup
    // guard would suppress the 2nd, leaving an ill-typed call). `cmp_helper_name` hashes `enum_ty`, so each
    // instantiation gets a unique, valid ident.
    let fn_name = cmp_helper_name(&enum_ty);
    // RECURSION GUARD (PR#890): a TRULY self-referential sum (`T = (Node (Tuple (Option Int64) T)) | (Leaf)`)
    // re-enters its OWN type and would inline-recurse UNBOUNDED in codegen (a compiler stack overflow); route
    // it through a helper `fn` (the recursion base) instead. The key is the FULL instantiated type, so a
    // NESTED distinct instantiation (`Option<Option<i64>>` containing `Option<i64>`) does NOT trip this — the
    // two are different types → the inner expands inline (finite), no false "recursive generic" decline (the
    // gap this closes). Only an identical decl+args re-entry (a real cycle in the finite value) routes to the
    // helper by call-indirection.
    if seen.contains(&sum_ty) {
        return Ok(format!("{fn_name}(&{l}, &{r})"));
    }
    let decl_occ = match &sum_ty {
        Ty::Sum { decl, .. } => *decl,
        _ => unreachable!("checked Ty::Sum above"),
    };
    seen.push(sum_ty.clone());
    let variant_count = match db.type_decl_by_occ(decl_occ).map(|t| t.variants.len()) {
        Some(n) => n,
        None => {
            seen.pop();
            return Err(Reject::decline("value-cmp: no variant count"));
        }
    };
    // Build the declared-order compare: compare the discriminant-ORDINAL first (declared position), and on an
    // equal-variant pair compare payloads. `__ord` maps a ref to its DECLARED position; the same-variant arms
    // compare payloads; the fallthrough compares ordinals.
    let mut ord_arms = Vec::with_capacity(variant_count);
    let mut same_arms = Vec::with_capacity(variant_count + 1);
    let mut arm_err: Option<Reject> = None;
    for disc in 0..variant_count as u32 {
        let path = match sum_variant_path_of_ty(db, ty, disc) {
            Ok(p) => p,
            Err(e) => {
                arm_err = Some(e);
                break;
            }
        };
        let has_payload = variant_payload_ty(db, ty, disc).is_some();
        let ord_pat = if has_payload {
            format!("{path}(..)")
        } else {
            path.clone()
        };
        ord_arms.push(format!("{ord_pat} => {disc}u32,"));
        match variant_payload_ty(db, ty, disc) {
            None => same_arms.push(format!("({path}, {path}) => core::cmp::Ordering::Equal,")),
            Some(payload_ty) => {
                let deref = if super::enums::variant_is_recursive(db, ty, disc) {
                    "**"
                } else {
                    "*"
                };
                let lp = format!("({deref}__lp)");
                let rp = format!("({deref}__rp)");
                match emit_value_cmp_walk_seen(db, &payload_ty, &lp, &rp, seen, helpers) {
                    Ok(cmp) => same_arms.push(format!("({path}(__lp), {path}(__rp)) => {cmp},")),
                    Err(e) => {
                        arm_err = Some(e);
                        break;
                    }
                }
            }
        }
    }
    seen.pop();
    if let Some(e) = arm_err {
        return Err(e);
    }
    // Emit the helper `fn __cmp_<Ident>(l, r) -> Ordering` ONCE (call-indirection, so a recursive payload
    // reaches this decl via a CALL, terminating codegen), then return a call. Mirrors `emit_value_eq_walk`'s
    // `__eq_<Ident>` helper. `#[allow]` for the generated fn's lints. Guard against a duplicate emit if the
    // same decl's helper was already pushed (a sibling occurrence in the same walk).
    let helper = format!(
        "#[allow(clippy::all)] fn {fn_name}(__cl: &{enum_ty}, __cr: &{enum_ty}) -> core::cmp::Ordering {{ let __ord = |__v: &{enum_ty}| -> u32 {{ match __v {{ {} }} }}; match (__cl, __cr) {{ {} _ => __ord(__cl).cmp(&__ord(__cr)), }} }}",
        ord_arms.join(" "),
        same_arms.join(" "),
    );
    if !helpers
        .iter()
        .any(|h| h.contains(&format!("fn {fn_name}(")))
    {
        helpers.push(helper);
    }
    Ok(format!("{fn_name}(&{l}, &{r})"))
}

/// The `fn __cmp_*` helper name for a sum's value-cmp walk, mangled by its FULL rendered Rust type
/// (`enum_ty`, e.g. `Option<Option<i64>>`) rather than the bare sum name. Two DISTINCT instantiations of one
/// generic sum (`Option<Option<i64>>` vs `Option<i64>` in a nested compare) need DISTINCT helpers — same
/// bare name + different signatures would collide (the dedup guard suppresses the 2nd → an ill-typed call).
/// Hex-encoding the rendered type gives a unique, valid, collision-free ident (the same injective hex idiom
/// `types::sum_ident` uses for lossy names); the leading `__cmp_` keeps it in the generated-helper namespace
/// (user idents with a leading `__` are escaped by `sanitize_ident`, so no clash with a user fn).
fn cmp_helper_name(enum_ty: &str) -> String {
    let mut s = String::with_capacity(enum_ty.len() * 2 + 6);
    s.push_str("__cmp_");
    for b in enum_ty.bytes() {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Whether the sub-tree at `id` reaches an OBSERVABLE side effect — a `Core::HostCall`, OR a `Core::Call`
/// (a callee might itself perform a host call, which the shallow host-import walk doesn't descend into). Used
/// by the `Core::Seq` emit to decide whether a DISCARDED non-final statement must be run: a statement that
/// reaches no host call and makes no call is DEAD (its value is discarded, its trap unobserved per the
/// dead-init ruling §283) and is ELIDED rather than emitted `let _ = …` (which would run it and spuriously
/// trap — adv-56). CONSERVATIVE: a statement with any call is KEPT (it might perform), so only a provably
/// pure+callless statement (e.g. `(/ 100 d)`) is dropped — exactly the dead-init shape.
fn reaches_host_call(db: &mut Db, id: StructId) -> bool {
    let mut imports = Vec::new();
    crate::backend::wasm::host::collect_host_imports(db, id, &mut imports);
    !imports.is_empty() || crate::layout::body_has_call(db, id)
}

/// The crate-root shim fn ident a `Core::HostCall` emits a call to, derived from the CANONICAL host-op key
/// (`effects::canonical_host_op_key` — kebab-normalized effect + verbatim op). The gate driver derives the
/// SAME ident from the recorded response key (kebab-normalizing its effect part), so the emitted
/// `crate::<ident>()` names exactly the generated fn — no casing drift. `__cdz_host_` prefixes the reserved
/// generated-helper namespace; the key's `.`/`-`/other non-ident chars map to `_` for a valid Rust ident.
pub(crate) fn host_shim_ident(op_key: &str) -> String {
    let mut s = String::with_capacity(op_key.len() + 11);
    s.push_str("__cdz_host_");
    for c in op_key.chars() {
        if c == '_' || c.is_ascii_alphanumeric() {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

/// Whether a rendered Rust type is a FIXED-WIDTH INTEGER — the only host-call result the H1 slice renders
/// (the shim returns `i64`, cast to this width). A bool/float/string/bytes/compound result declines (a later
/// host-call increment).
fn int_rust_ty(rust_ty: &str) -> bool {
    matches!(
        rust_ty,
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64"
    )
}

/// Whether a runtime `(= a b)` over type `ty` can emit a native Rust `==` — the operand type maps to a
/// Rust type that derives `Eq`/`PartialEq`. Delegates to `enums::ty_supports_eq` (which handles sums,
/// built-in Option/Result, tuples, records, nominals, and rejects floats/fns/collections), so the `==`
/// this emits type-checks against the emitted enum's derives.
fn ty_supports_native_eq(db: &mut Db, ty: &Ty) -> bool {
    super::enums::ty_supports_eq(db, ty)
}

/// Render an integer constant at the node's OWN solved type. Used only where the node stands in a
/// context that already fixes its width (a bare literal whose own `type_of` is definite). A literal
/// used as an OPERAND / BRANCH / ARM BODY of a construct is instead grounded to that construct's width
/// via [`emit_const_int_at`] — see [`emit_grounded`] — because a bare literal's own type is the default
/// (`Int64`), which unification does not thread the context width back onto.
fn emit_const_int(db: &mut Db, id: StructId, v: &IntValue) -> Result<String, Reject> {
    let it = int_ty_of(db, id);
    emit_const_int_at(&db.name_ctx(), it, v)
}

/// Whether a solved type is BIGINT-VALUED — a value that emits as a `cdz_num::Big`. That is a bare
/// `Ty::BigInt` OR a `Ty::Qty { inner: BigInt }` (a quantity over a BigInt magnitude: the `Ty::Qty`
/// wrapper is compile-time-only and `lower` erases it, so the magnitude emits as a `Big`). A bare
/// `Ty::Int` is NOT (it emits a fixed-width int literal). Used by the `BigIntBinOp`/`BigIntCmp` guards to
/// admit a BigInt-magnitude quantity while still declining a constant that reaches the op typed plain
/// `Int`. STRIPS nominals at BOTH levels — the outer type (an erased newtype over `BigInt` still emits a
/// `Big`, cf. `add3eca3a`) and the `Qty` magnitude (`Qty` over a newtype-wrapped `BigInt`) — so it is
/// STRICTLY WIDER than the wasm backend's bare `Ty::Qty { inner, .. } if matches!(*inner, Ty::BigInt)`,
/// which does not peel nominals. Keep that in mind for a rust-vs-wasm BigInt-handling comparison.
fn is_bigint_valued(ty: &Ty) -> bool {
    match ty.strip_nominal() {
        Ty::BigInt => true,
        Ty::Qty { inner, .. } => matches!(inner.strip_nominal(), Ty::BigInt),
        _ => false,
    }
}

/// A `cdz_num::Big` constructor EXPRESSION for a CONSTANT integer `v` — in-i64 range → `Big::from_i64`,
/// beyond → `Big::from_sign_magnitude_bytes(&[sign, LE-magnitude…])` (the runtime's canonical leaf form,
/// `IntValue.magnitude` is BIG-endian so reversed). Shared by the `Core::ConstInt`-typed-BigInt arm and
/// the `Core::ConstRational` num/den materialization.
fn const_big_expr(v: &IntValue) -> String {
    if let Some(n) = v.to_i64() {
        format!("cdz_num::Big::from_i64({n})")
    } else {
        let sign = if v.negative { 1u8 } else { 0u8 };
        let mut bytes = vec![sign];
        bytes.extend(v.magnitude.iter().rev().copied()); // BE magnitude → LE
        let elems = bytes
            .iter()
            .map(|b| b.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        format!("cdz_num::Big::from_sign_magnitude_bytes(&[{elems}])")
    }
}

/// Emit a fixed-width int NODE widened to a `cdz_num::Big` BY VALUE (through `i128`, so an unsigned
/// operand `>= 2^63` keeps its true sign — the unsigned-safe path `BigIntOfI64` uses). Shared by the
/// Rational-of-ints ops (`Rational.of`/`Rational.of-int`), whose operands are `∀a.(Int a)`.
fn emit_int_as_big(db: &mut Db, node: StructId, env: &Env, ctx: &Ctx) -> Result<String, Reject> {
    let v = emit(db, node, env, ctx)?;
    Ok(format!(
        "{{ let mut __buf = [0u8; 17]; \
         let __n = cdz_num::Big::i128_to_sign_magnitude_bytes_into(({v}) as i128, &mut __buf) \
         .expect(\"i128 fits 17 bytes\"); \
         cdz_num::Big::from_sign_magnitude_bytes(&__buf[..__n]) }}"
    ))
}

/// Render an integer constant as `<bits><utype> as <target>` (or just `<bits><utype>` when the target
/// IS the unsigned bit type) at the GIVEN integer type `it` — the width/signedness of the CONTEXT the
/// literal appears in, not necessarily the literal's own defaulted type. Mirrors the wasm backend
/// (`emit_operand`/`emit_branch` ground a bare literal to the op/branch width): the value must fit that
/// width (else CDZ0302 — never truncate), and it is written as the two's-complement bit pattern so a
/// negative signed value and a large unsigned value share one spelling.
fn emit_const_int_at(ncx: &crate::ty::NameCtx, it: IntTy, v: &IntValue) -> Result<String, Reject> {
    let signed = it.ground_signed();
    let width = it.ground_width();
    if !v.fits_width(signed, width) {
        return Err(Reject::coded(
            Code::IntOutOfRange,
            "integer literal does not fit its width",
        ));
    }
    let target = types::rust_type(ncx, &Ty::Int(it)).ok_or_else(|| {
        Reject::decline("integer literal width has no native Rust representation")
    })?;
    let ubits = types::unsigned_bits_type(it).ok_or_else(|| {
        Reject::decline("integer literal width has no native Rust representation")
    })?;
    // A SIGNED UNUSUAL width (not a machine boundary — `Int4`/`Int12`, stored in the next-larger primitive):
    // the `<bits>u8 as i8` bit-pattern cast reinterprets at the STORAGE width (8 bits), NOT the declared
    // 4-bit width — so a 4-bit-negative value like `(Int 4).wrap 8` (= -8, its bit-3 sign set) would emit
    // `8u8 as i8` = +8 (WRONG), because bit 3 is not the i8 sign bit. The value `v` is ALREADY the correct
    // signed narrow value (lower folds `.wrap` via `IntValue::wrap_to`, which sign-extends), so emit its
    // TRUE SIGNED DECIMAL (`-8i8`) — unambiguous and in range for the storage type. (This is the const-fold
    // twin of the runtime `Convert(Wrap)` sign-extend; a plain bit-pattern cast only round-trips at machine
    // widths, where bit N-1 IS the storage sign bit.)
    if signed && !matches!(width, 8 | 16 | 32 | 64) {
        return Ok(format!("{}{target}", int_value_signed_decimal(v)));
    }
    // The unsigned bit pattern of the value at its width: the low `width` bits of its two's-complement
    // representation, as an unsigned magnitude. `wrap_to(false, width)` computes exactly that, and the
    // result is a non-negative `IntValue` whose decimal is the unsigned literal.
    let bits = v.wrap_to(false, width);
    let literal = int_value_decimal(&bits);
    if target == ubits {
        // The target is itself the unsigned bit type (a `UIntN`): write the literal directly.
        Ok(format!("{literal}{ubits}"))
    } else {
        // A signed MACHINE-width target: write the bit pattern in the unsigned type and cast, so the sign
        // is set from the bit pattern (`128u8 as i8` = -128), never a decimal minus. (For a machine width,
        // bit N-1 IS the storage sign bit, so this reinterprets correctly.)
        Ok(format!("({literal}{ubits} as {target})"))
    }
}

/// Whether an arithmetic OPERAND provably diverges — the RUST-emit-local, TRANSITIVE companion of
/// `body_diverges`. It is `body_diverges` (a bare/`Seq`/`Let`/`If`/`Match`-forwarded `Core::Trap`) PLUS
/// the case `body_diverges` deliberately omits for its wasm purpose: a `Core::Arith` (or `Compare`) node
/// whose OWN operand diverges. Cadenza evaluates an op's operands lhs-then-rhs before the op, so if either
/// operand diverges the whole op is dead — but the node stays a live `Core::Arith` in Core (lower_arith
/// propagates only `Poison`, not `Trap`), and `body_diverges`' `_ => false` arm never looks inside it. This
/// helper looks inside, so `emit_arith`'s diverging-operand guard fires at ANY nesting depth (the fix for
/// the nested `(+ (+ (trap) 1) 2)` residue of the direct guard). It does NOT touch shared Core / the wasm
/// backend — it is a pure rust-emit predicate. Recursion is bounded by the finite operand tree.
fn arith_operand_diverges(db: &mut Db, id: StructId) -> bool {
    if crate::backend::common::diverge::body_diverges(db, id) {
        return true;
    }
    match core_of(db, id) {
        // An arithmetic/comparison node is dead if either operand diverges (operands run before the op).
        Core::Arith { lhs, rhs, .. }
        | Core::Compare { lhs, rhs, .. }
        | Core::StrCmp { lhs, rhs, .. } => {
            arith_operand_diverges(db, lhs) || arith_operand_diverges(db, rhs)
        }
        _ => false,
    }
}

/// Render a runtime arithmetic op as a Rust expression, honoring the numeric model's traps:
///  - `+`/`-`/`*` → `<lhs>.checked_add(<rhs>).unwrap_or_else(|| <trap>)` — trap (panic) on overflow;
///  - `/`/`%` → `checked_div`/`checked_rem` — trap on ÷0 and `MIN / -1`;
///  - `&`/`|`/`^` → the total bitwise operator;
///  - `<<`/`>>` → a guarded block: count `>= N` traps; `<<` also round-trips to trap on overflow;
///    `>>` is arithmetic (signed) / logical (unsigned) via the value type's own `>>`.
#[allow(clippy::too_many_arguments)]
fn emit_arith(
    db: &mut Db,
    id: StructId,
    op: Prim,
    lhs: StructId,
    rhs: StructId,
    env: &Env,
    ctx: &Ctx,
    // The CONSUMING op's width, when this arith is a nested OPERAND of an enclosing narrow op and its OWN
    // width is DEFERRED (no anchor). The rust twin of the wasm backend's `emit_operand_into` `ot`
    // (select.rs): a nested `+`/`-`/`*` whose operands are all deferred-width types as `Int(Deferred)`,
    // which grounds to the i64 DEFAULT — so the inner op computes AND range-checks at i64, then the caller
    // truncates the i64 result `as iN`, SILENTLY WRAPPING an inner overflow (`(+ (+ (if c 100 10) (if d
    // 100 10)) 5) : Int8`, inner 100+100=200 → `200 as i8` = -56) instead of TRAPPING. Emitting the inner
    // op at the consuming width makes it compute AND range-check (`checked_*`) at the narrow width, so the
    // inner overflow traps — matching wasm. `None` for a top-level / fixed-width op (the common path).
    width_override: Option<IntTy>,
) -> Result<String, Reject> {
    // A DIVERGING OPERAND (a `(trap …)` value, or any provably-diverging sub-expression — reaching here via
    // a `let`-binding `(let ((x (trap))) (+ x 1))` folded to a `Core::Trap` operand, or an inlined call-arg
    // `(f (trap))` substituting the trap for the param) makes the WHOLE arithmetic dead: the operand's
    // `panic!` aborts before the op runs. Emitting the op verbatim yields `(panic!(…)).checked_add(1)` — a
    // method call on Rust's `!`, which E0599s ("no method `checked_add` for type `!`"): `!` coerces to a
    // value but is not a method receiver. So emit ONLY the diverging computation (the op is unreachable),
    // matching the wasm backend where the trap's `unreachable` aborts before the add is ever executed.
    // Cadenza evaluates lhs THEN rhs: if lhs diverges, rhs never runs → emit lhs alone; if lhs is fine but
    // rhs diverges, lhs still runs (for effect) then rhs aborts → `{ let _ = <lhs>; <rhs> }`.
    //
    // The check is TRANSITIVE (`arith_operand_diverges`), not just `body_diverges`: a diverging operand can
    // be NESTED inside another arith — `(+ (+ (trap) 1) 2)` — where the outer lhs is a live `Core::Arith`
    // (its own lhs traps). `body_diverges` (correctly, for its wasm purpose) does NOT treat an `Arith` node
    // as diverging, and `lower_arith` keeps such a node live (it propagates only `Poison`, not `Trap`), so a
    // bare `body_diverges` here MISSES the nested shape and the outer takes the normal path, emitting
    // `<inner>.checked_add(2)` where `<inner>` is `panic!("unreachable")` — the exact E0599 the direct guard
    // kills, one level deeper. Recursing into arith operands catches it: the outer sees lhs diverges, emits
    // only `emit(lhs)`, which re-enters `emit_arith` on the inner and (its own lhs diverging) yields the bare
    // panic — no method call on `!` at any depth.
    if arith_operand_diverges(db, lhs) {
        return emit(db, lhs, env, ctx);
    }
    if arith_operand_diverges(db, rhs) {
        let l = emit(db, lhs, env, ctx)?;
        let r = emit(db, rhs, env, ctx)?;
        return Ok(format!("{{ let _ = {l}; {r} }}"));
    }
    // A FLOAT arithmetic op (`+.`/`-.`/`*.`/`/.`) → the native Rust `+`/`-`/`*`/`/` on `f64`/`f32`. IEEE,
    // never traps (no `checked_*`/overflow panic, unlike the integer arith below) — matches the wasm
    // machine op. Both operands share the op's float type, but a bare LITERAL operand defaults to Float64
    // when unpinned (`(*. x 2.0)` with `x: Float32`) → `x * <f64>` is rustc E0277 (`f32 * f64`), so GROUND
    // each operand to the op's float width (`emit_grounded_float` emits a `ConstFloat` at that width). The
    // width is the op's solved float type (the operands + result share it); default to 64 if not solved.
    if op.is_float_arith() {
        let sym = match op {
            Prim::FAdd => "+",
            Prim::FSub => "-",
            Prim::FMul => "*",
            Prim::FDiv => "/",
            _ => unreachable!("guarded by is_float_arith"),
        };
        let fwidth = match type_of(db, id) {
            Ty::Float(ft) => ft.ground_width(),
            _ => match type_of(db, lhs) {
                Ty::Float(ft) => ft.ground_width(),
                _ => crate::ty::DEFAULT_FLOAT_WIDTH,
            },
        };
        let l = emit_grounded_float(db, lhs, fwidth, env, ctx)?;
        let r = emit_grounded_float(db, rhs, fwidth, env, ctx)?;
        return Ok(format!("({l} {sym} {r})"));
    }
    // Both operands share the OP's integer type (its result width == operand width). Ground a bare
    // literal operand to it so `(+ a 1)` over a narrow `a` emits `<narrow>::checked_add(1<narrow>)`,
    // not `checked_add((1u64 as i64))` (Rust E0308) — the analogue of the wasm backend's `emit_operand`.
    // A `width_override` (this arith is a nested operand of a narrow op) takes effect ONLY when the op's
    // OWN width is DEFERRED — a genuine FIXED inner width differing from the context is a CDZ0301 fault
    // that aborts before emit, so a fixed inner width is kept as-is. Exactly the wasm `emit_operand_into`
    // decision (`if own.width_is_fixed() { own } else { ot }`, select.rs).
    let own_it = int_ty_of(db, id);
    let it = match width_override {
        Some(ot) if !own_it.width_is_fixed() => ot,
        _ => own_it,
    };
    // SAFETY GUARD (unusual-width arithmetic): an UNUSUAL width (`UInt48`, `Int12` — 1..=64 but not an
    // aliased boundary) is STORED in the next-larger machine primitive (`types::int_type`), so a `checked_*`
    // on that primitive would trap at `2^machine`, NOT the type's `2^N` — a WRONG overflow (`(UInt48).max +
    // 1` = 2^48 must trap, but `u64::checked_add` wouldn't). Emitting it is a silent miscompile. DECLINE
    // runtime arithmetic on an unusual width (defense-in-depth: no corpus case runs it — the only unusual-
    // width `+` is a compile-time CDZ0304 reject — but the storage-width map makes the type representable, so
    // this guard is what keeps a future runtime unusual-width arith from miscompiling). The value/wrap/render
    // surface (which the storage map serves) has no such hazard. An unusual-width `2^N` range-check is a
    // later slice.
    // An UNUSUAL width (`UInt48`, `Int12` — 1..=64 but not an aliased boundary 8/16/32/64) is STORED in the
    // next-larger machine primitive (`types::int_type` → `storage_width_type`), so a `checked_*` on that
    // primitive traps at the STORAGE width's `2^machine`, NOT the type's `2^N` (`(UInt48).max + 1` = 2^48
    // must trap, but `u64::checked_add` wouldn't). So `+`/`-`/`*` compute the native op on the storage type
    // (which never wraps at `2^machine` for in-range operands) and then RANGE-CHECK the result against the
    // TYPE's own `[min_N, max_N]`, panicking "integer overflow" out of range — the rust twin of the wasm
    // narrow-width `emit_range_check` (select.rs). `/`/`%`/bitwise/shift over an unusual width can't exceed
    // the operands' range (a quotient/remainder/mask/shift stays within), so they need no width check and
    // fall through to the normal emit below. Handled here as a dedicated arm so the storage-width map stays
    // safe for arith too. A NON-unusual width (8/16/32/64 with an exact primitive) skips this entirely.
    if let Width::Fixed(w) = it.width
        && (1..=64).contains(&w)
        && !matches!(w, 8 | 16 | 32 | 64)
        && matches!(op, Prim::Add | Prim::Sub | Prim::Mul)
    {
        let l = emit_grounded(db, lhs, it, env, ctx)?;
        let r = emit_grounded(db, rhs, it, env, ctx)?;
        // Provably-in-range elision — the SAME Core-tier decision the wasm backend + the normal path use.
        let grounded = crate::ty::IntTy::fixed(it.ground_signed(), it.ground_width());
        if crate::lower::provably_no_overflow(db, op, lhs, rhs, grounded, id) {
            let native = match op {
                Prim::Add => "wrapping_add",
                Prim::Sub => "wrapping_sub",
                Prim::Mul => "wrapping_mul",
                _ => unreachable!(),
            };
            return Ok(format!("({l}).{native}({r})"));
        }
        // The type's own bounds. An unusual width is 1..=63 (never 64 here), so BOTH bounds fit i64/i128.
        let signed = it.ground_signed();
        let (min_n, max_n): (i128, i128) = if signed {
            let half = 1i128 << (w - 1);
            (-half, half - 1)
        } else {
            (0, (1i128 << w) - 1)
        };
        let store = super::types::rust_type(&db.name_ctx(), &Ty::Int(it))
            .ok_or_else(|| Reject::decline("unusual-width arith: no storage type"))?;
        return Ok(match op {
            // ADD/SUB — the storage type is STRICTLY WIDER than the type's N bits, so the true result of
            // two in-range operands (each < 2^N ≤ 2^(storage-1)) NEVER overflows the storage width: a
            // `wrapping_*` on the storage prim equals the true result exactly, and the type-bound check then
            // catches a result that leaves `[min_N, max_N]`. Bind it, compare against the type's bounds (as
            // storage-type literals, no cast mismatch), panic the classifying "integer overflow" if outside.
            Prim::Add | Prim::Sub => {
                let native = if matches!(op, Prim::Add) {
                    "wrapping_add"
                } else {
                    "wrapping_sub"
                };
                let cond = if signed {
                    format!("__uw < {min_n}{store} || __uw > {max_n}{store}")
                } else {
                    format!("__uw > {max_n}{store}")
                };
                format!(
                    "{{ let __uw = ({l}).{native}({r}); if {cond} {{ panic!(\"integer overflow in {}\") }} __uw }}",
                    op_name(op),
                )
            }
            // MUL — a `wrapping_mul` on the STORAGE prim is UNSOUND: two in-range operands can multiply PAST
            // the storage width (e.g. `UInt48` `2^32 * 2^32 = 2^64` wraps `u64::wrapping_mul` to 0, which
            // falsely passes the `[0, 2^48-1]` check → a silent wrong value instead of a trap — Copilot/
            // github-liaison PR#756). Compute the product in a WIDER intermediate (`i128`, always wider than
            // the ≤64-bit storage, so the product is EXACT — a 63×63-bit product fits i128's 127 bits), range-
            // check the i128 against the type's bounds, then cast the in-range result back to the storage
            // type. `i128` holds every unsigned-N value too (N ≤ 63, and a `uN as i128` is non-negative), so
            // the single/two-sided bound test is correct for both signednesses.
            Prim::Mul => {
                let cond = if signed {
                    format!("__wide < {min_n}i128 || __wide > {max_n}i128")
                } else {
                    format!("__wide > {max_n}i128")
                };
                format!(
                    "{{ let __wide = ({l} as i128) * ({r} as i128); if {cond} {{ panic!(\"integer overflow in {}\") }} __wide as {store} }}",
                    op_name(op),
                )
            }
            _ => unreachable!(),
        });
    }
    let l = emit_grounded(db, lhs, it, env, ctx)?;
    let r = emit_grounded(db, rhs, it, env, ctx)?;
    match op {
        Prim::Add | Prim::Sub | Prim::Mul => {
            // GUARD ELISION — Core-tier parity with the wasm backend's `select.rs:12542` fast path. When
            // interval arithmetic proves the result stays in the type (the SAME `lower::arith_provably_in_range`
            // predicate, defined in lower.rs at the CORE tier — so ONE decision drives BOTH backends), the
            // overflow trap cannot fire, so emit the plain MODULAR op (`wrapping_*`, the analogue of the
            // elided wasm path's bare `m.add()`) with NO `checked_*`/panic. SOUND: provably-in-range ⇒ the
            // true result never leaves the type ⇒ the wrapping result IS the true result, byte-identical to
            // the checked form on every in-range input, and never traps (exactly like the elided wasm path).
            // Until this, the rust backend consulted the predicate NOWHERE, so a Core-tier elision (range
            // analysis today, a discharged no-overflow proof next) was silently wasm-only.
            //
            // PARITY: pass the GROUNDED result type, exactly as the wasm backend does. wasm feeds the
            // predicate `IntTy::fixed(m.signed, m.width)` where `m = Machine::of(int_ty_of(db, id))`, and
            // `Machine::of` grounds a Deferred/Var width→64 and a deferred sign→signed (`ground_width`/
            // `ground_signed`). `arith_provably_in_range` internally rejects a NON-ground `IntTy`
            // (`resolved_int_bounds` returns `None` on Deferred/Var), so passing `it` raw would make rust
            // KEEP a guard wasm elides on a deferred-width node — a correct-but-divergent elision decision.
            // Grounding here mirrors `Machine::of` so BOTH backends make the identical decision. (For a
            // concrete narrow/wide type `it` is already ground, so this is a no-op on the common path.)
            let grounded = crate::ty::IntTy::fixed(it.ground_signed(), it.ground_width());
            // Consult the both-backend elision DECISION (`provably_no_overflow` = range analysis OR a
            // discharged proof), not the bare predicate — so a verification-licensed node elides here too
            // once v-verification's b3 fills `discharged_no_overflow`. Behavior-neutral today (the stub
            // returns false, so this ≡ `arith_provably_in_range` alone).
            if crate::lower::provably_no_overflow(db, op, lhs, rhs, grounded, id) {
                let wrapping = match op {
                    Prim::Add => "wrapping_add",
                    Prim::Sub => "wrapping_sub",
                    Prim::Mul => "wrapping_mul",
                    _ => unreachable!(),
                };
                return Ok(format!("({l}).{wrapping}({r})"));
            }
            let method = match op {
                Prim::Add => "checked_add",
                Prim::Sub => "checked_sub",
                Prim::Mul => "checked_mul",
                _ => unreachable!(),
            };
            // `checked_*` returns `None` exactly when the true result leaves the N-bit type — the
            // numeric model's overflow trap. Panic on `None` (an aborting trap, the native `unreachable`
            // analogue); the message names the op so a trap is legible.
            Ok(format!(
                "({l}).{method}({r}).unwrap_or_else(|| panic!(\"integer overflow in {}\"))",
                op_name(op),
            ))
        }
        Prim::Div => {
            // `/` traps on a zero divisor AND (for a SIGNED type) on `MIN / -1` — the two cases the numeric
            // model traps for division (`MIN / -1` overflows: the quotient +2^(N-1) is out of range),
            // mirroring the wasm `i64.div_s` native trap. But those are DISTINCT trap KINDS the corpus grades
            // separately (`divide by zero` vs `overflow`), and the gate's `trap_kind` classifies by the panic
            // MESSAGE — a single "by zero or overflow" message contains BOTH substrings and is misread as
            // div-by-zero (checked first), so a `MIN / -1` case that must grade `overflow` graded wrong. So
            // GUARD the conditions explicitly and panic with a KIND-SPECIFIC message: `r == 0` → "divide by
            // zero"; `l == <T>::MIN && r == -1` → "overflow". An UNSIGNED type has NO MIN/-1 overflow (and
            // `r == -1` would not type-check), so it emits only the zero guard. Otherwise the plain `/`
            // (neither condition holds). Each operand binds once so a side-effecting operand runs once.
            // The `MIN/-1` overflow guard exists SOLELY for `MIN ÷ -1`; it is DEAD when either operand
            // provably rules that pair out — matching the wasm backend's `select.rs:13275` consult of the
            // SAME two Core-tier predicates (the Div member of the both-backend guard-elision family):
            //   • the DIVISOR provably is NOT `-1` (`!divisor_can_be_neg_one` — a positive constant, an
            //     unsigned/nonneg/masked value, or a flow-refined range excluding -1); OR
            //   • the DIVIDEND is provably NON-NEGATIVE (`value_provably_nonneg`) — `MIN` is negative, so a
            //     nonneg dividend can never be `MIN`, and then `MIN ÷ -1` cannot occur.
            // The zero-divisor guard always stays (only the signed MIN/-1 overflow guard is elidable).
            let overflow_possible = crate::lower::divisor_can_be_neg_one(db, rhs)
                && !crate::lower::value_provably_nonneg(db, lhs);
            let overflow_guard = match types::int_type_is_signed(it) && overflow_possible {
                // `MIN / -1` overflows the DECLARED width (the quotient +2^(N-1) is out of range). The guard
                // must test the DECLARED-width minimum, NOT the STORAGE slot's `<T>::MIN`: an odd width
                // (Int24, Int48) is stored in the next-larger machine prim (i32/i64), so `<slot>::MIN`
                // (i32::MIN = -2^31) is NOT the type's min (-2^23 for Int24) — a `l == i32::MIN` guard NEVER
                // fires for an Int24 value, so `MIN(-8388608) / -1` computed the out-of-range +8388608 and it
                // escaped into downstream unchecked math (adv-67, HIGH differential: rust returned it, wasm
                // trapped). Compute the declared min `-(1 << (N-1))` from the width `w` and compare against
                // THAT (as the checked +/-/* unusual-width path already re-checks the declared range). For an
                // aliased width (8/16/32/64) the declared min == slot min, so this is behavior-identical there.
                true => match (types::rust_type(&db.name_ctx(), &Ty::Int(it)), it.width) {
                    (Some(t), Width::Fixed(w)) if (1..=64).contains(&w) => {
                        // The declared minimum as a literal in the storage type `t`. TWO cases:
                        //  • ALIASED width (8/16/32/64): the slot IS the declared width, so `{t}::MIN` is the
                        //    exact declared min. MUST use it — the computed `-(1{t} << (w-1))` would OVERFLOW
                        //    the slot at w==bits (`1i32 << 31` = 2^31 doesn't fit i32 → rustc "arithmetic
                        //    operation will overflow"). This is the original behavior, preserved for the
                        //    aliased widths (where it was already correct).
                        //  • ODD width (Int24, Int48 — stored in the next-larger slot): `{t}::MIN` is the
                        //    SLOT's min (i32::MIN for Int24), NOT the declared min (-2^23), so the guard would
                        //    never fire (adv-67). Compute `-(1{t} << (w-1))` — the declared min fits the
                        //    strictly-wider slot (w < slot bits), no overflow.
                        let decl_min = if crate::ty::ALIASED_INT_WIDTHS.contains(&w) {
                            format!("{t}::MIN")
                        } else {
                            format!("(-(1{t} << {}))", w - 1)
                        };
                        format!(
                            "else if l == {decl_min} && r == -1 {{ panic!(\"{} overflow\") }} ",
                            op_name(op)
                        )
                    }
                    // No storage type / non-fixed width — no guard (the operand type wasn't a fixed int).
                    _ => String::new(),
                },
                false => String::new(),
            };
            Ok(format!(
                "{{ let (l, r) = ({l}, {r}); \
                 if r == 0 {{ panic!(\"{op} by zero\") }} \
                 {overflow_guard}else {{ l / r }} }}",
                op = op_name(op),
            ))
        }
        Prim::Rem => {
            // `%` traps ONLY on a zero divisor — NOT on `MIN % -1`. `x % -1` is 0 for every x, including
            // `MIN % -1 = 0` (numeric-model.md §Modulo by -1 is always zero: modulo forms no quotient, so
            // it has no overflow — the check that makes `/` trap must NOT apply to `%`). Rust's
            // `checked_rem` WRONGLY returns `None` at `MIN % -1` (it conflates the remainder with the
            // division overflow), so it cannot be used here — it would panic where the value must be 0.
            // Guard only the zero divisor explicitly, then `wrapping_rem`, which yields 0 at `MIN % -1`
            // (it performs no overflow check), matching the wasm backend's `i64.rem_s`. Evaluate each
            // operand once into a block-local binding so a side-effecting operand runs exactly once.
            Ok(format!(
                "{{ let (l, r) = ({l}, {r}); \
                 if r == 0 {{ panic!(\"{} by zero\") }} else {{ l.wrapping_rem(r) }} }}",
                op_name(op),
            ))
        }
        Prim::BitAnd | Prim::BitOr | Prim::BitXor => {
            let sym = match op {
                Prim::BitAnd => "&",
                Prim::BitOr => "|",
                _ => "^",
            };
            Ok(format!("({l} {sym} {r})"))
        }
        // WRAPPING arithmetic → Rust's own `wrapping_add`/`wrapping_mul` — two's-complement wraparound,
        // never panics (the native mirror of the wasm backend's raw `i64.add`/`i64.mul`). `it` is the
        // aliased width N, so the operands are the N-bit type and the wrap is modulo 2^N.
        Prim::WrappingAdd | Prim::WrappingSub | Prim::WrappingMul => {
            let method = match op {
                Prim::WrappingAdd => "wrapping_add",
                Prim::WrappingSub => "wrapping_sub",
                _ => "wrapping_mul",
            };
            Ok(format!("({l}).{method}({r})"))
        }
        // A runtime shift, honoring the numeric model's trapping semantics exactly (mirroring the wasm
        // backend's `emit_shift` — `numeric-model.md` §A Shift Is Not Exempt From Overflow Is Defined):
        //   - COUNT GUARD: a count outside `0..N` traps. The count is range-checked at its FULL i64 width
        //     (`(0..N).contains(&c64)`) BEFORE the narrowing `as u32`, catching BOTH a too-large count and
        //     a negative one. Checking the untruncated width matters: a count that is a multiple of 2^32
        //     has low-32-bits 0, so an `as u32`-then-compare guard would see 0, skip the trap, and shift
        //     by a masked amount — a silent wrong VALUE where wasm traps;
        //   - `<<` is exact `*2^count`, so it TRAPS on overflow: shift, then round-trip `(r >> count)`
        //     must recover the value — Rust's `>>` is arithmetic for a signed type / logical for an
        //     unsigned one, so the inverse is exact and the check catches a dropped high bit;
        //   - `>>` is arithmetic (signed) / logical (unsigned) — Rust's native `>>` on the value's type
        //     already IS that, so the count guard is the only trap.
        // `it` is the op's aliased width N (a non-aliased width already declined at `rust_type`), so the
        // Rust value type IS the N-bit native type — no wider-slot round-trip like wasm needs. Emitted as
        // a block that binds the value + count once (so a computed operand is evaluated once) then guards.
        Prim::Shl | Prim::Shr => {
            let width = it.ground_width();
            let vty = types::rust_type(&db.name_ctx(), &Ty::Int(it)).ok_or_else(|| {
                Reject::decline("shift value width has no native Rust representation")
            })?;
            // The count expression: its own solved type (a shift count is not rigidly the value's type).
            // Range-checked at full i64 width, then cast to u32 for the shift-count position.
            let count_it = int_ty_of(db, rhs);
            let count = emit_grounded(db, rhs, count_it, env, ctx)?;
            if matches!(op, Prim::Shr) {
                // `>>`: guard the count, then the native shift (arithmetic/logical by `vty`'s sign).
                Ok(format!(
                    "{{ let v: {vty} = {l}; let c64 = ({count}) as i64; \
                     if !(0..{width}).contains(&c64) {{ panic!(\"shift count out of range\") }} \
                     let c = c64 as u32; v >> c }}"
                ))
            } else {
                // `<<` GUARD ELISION — Core-tier parity with the wasm backend's `emit_shift` fast path
                // (`select.rs`). When the shift provably cannot overflow, BOTH the count guard and the
                // overflow round-trip are dead, so emit the bare `v << count`. Two sound cases, mirroring
                // wasm exactly:
                //   • CONSTANT count `k` with `0 <= k < width` AND `lower::shl_provably_in_range(lhs, k)`
                //     (`(<< (& x 15) 2)` = [0,60]) — the fixed shift fits, and `k < width` means no count
                //     guard is needed.
                //   • RUNTIME count whose range is known AND `lower::shl_provably_in_range_dynamic(lhs,
                //     rhs)` (`(<< (& x 15) (& k 3))`) — the dynamic predicate requires `chi < width`, so
                //     the count is provably in range too (both guards dead).
                // Until this, the rust `<<` emit consulted neither predicate, so this Core-tier elision
                // was silently wasm-only (the same gap the arith consult closed for `+`/`-`/`*`).
                let const_count = match core_of(db, rhs) {
                    Core::ConstInt(v) => v.to_i64(),
                    _ => None,
                };
                let elide = const_count.is_some_and(|k| {
                    (0..width as i64).contains(&k)
                        && crate::lower::shl_provably_in_range(db, lhs, k as u32)
                }) || (const_count.is_none()
                    && crate::lower::shl_provably_in_range_dynamic(db, lhs, rhs));
                if elide {
                    // Provably in range: the modular `<<` IS the true value (no dropped bit), and the count
                    // is provably `< width`, so no guard is needed — byte-identical to the guarded form on
                    // every reachable input, exactly like the elided wasm path.
                    return Ok(format!("{{ let v: {vty} = {l}; v << ({count}) }}"));
                }
                // `<<`: guard the count, shift, then detect an overflow. The round-trip `(r >> c) != v`
                // catches a bit dropped at the SLOT width — CORRECT for an ALIASED width (8/16/32/64) where
                // the slot IS the declared width. But for an ODD/unusual width (UInt4, Int24 — stored in a
                // strictly-wider slot), an overflow of the DECLARED width fits the slot losslessly, so the
                // round-trip passes and the out-of-range value ESCAPES (adv-67b, HIGH: `UInt4 3<<3` = 24 >
                // max 15, `24>>3`==3 round-trips clean → 24 escaped, poisoning a CHAMP Set). So for an odd
                // width ADD a DECLARED-RANGE check on `r` (the same `[min_N, max_N]` bound the checked +/-/*
                // unusual-width path uses). An aliased width keeps ONLY the round-trip (slot==declared).
                let odd_width =
                    (1..=64).contains(&width) && !crate::ty::ALIASED_INT_WIDTHS.contains(&width);
                let range_check = if odd_width {
                    let signed = it.ground_signed();
                    let (min_n, max_n): (i128, i128) = if signed {
                        let half = 1i128 << (width - 1);
                        (-half, half - 1)
                    } else {
                        (0, (1i128 << width) - 1)
                    };
                    // `r` is the slot type `vty`; compare via i128 to avoid any slot-width truncation in the
                    // literal bounds (an odd width is < 64 bits, so both bounds fit i128 comfortably).
                    format!(
                        "if ((r as i128) < {min_n} || (r as i128) > {max_n}) {{ panic!(\"integer overflow in left shift\") }} "
                    )
                } else {
                    String::new()
                };
                Ok(format!(
                    "{{ let v: {vty} = {l}; let c64 = ({count}) as i64; \
                     if !(0..{width}).contains(&c64) {{ panic!(\"shift count out of range\") }} \
                     let c = c64 as u32; \
                     let r = v << c; \
                     if (r >> c) != v {{ panic!(\"integer overflow in left shift\") }} {range_check}r }}"
                ))
            }
        }
        _ => Err(Reject::decline(
            "not a runtime integer arithmetic operation",
        )),
    }
}

/// Render a scalar `match` as Rust's `match`. The scrutinee is rendered once (Rust binds it as the
/// matchee); each arm is `pattern [if guard] => body`. A literal probe becomes the literal pattern
/// written in the scrutinee's type; a wildcard OR a bare-name BINDER becomes `_` (a binder resolves to
/// the scrutinee occurrence in `resolve`, so a body reference to the binder already re-reads the
/// scrutinee — no Rust binding pattern is needed). A guarded arm emits Rust's own pattern guard `if
/// <cond>`, which Rust evaluates ONLY after the pattern matches and falls through on false — exactly
/// the core's guard semantics (short-circuit + fall-through), so no manual nesting is needed.
///
/// EXHAUSTIVENESS maps across: `lower` admits a runtime match only if it is exhaustive by its UNGUARDED
/// arms (a guard does not count — `numeric-model`/CDZ0210), which is Rust's rule too. An integer match
/// therefore carries an unguarded wildcard/binder arm → a Rust `_` catch-all; a Bool match carries
/// `true`+`false`. Arms AFTER an unguarded catch-all are unreachable in both models, so emission stops
/// at the first unguarded `_` (mirroring the wasm probe chain, and avoiding Rust's unreachable-arm
/// lint) — leaving a `match` Rust sees as exhaustive.
fn emit_match(
    db: &mut Db,
    match_id: StructId,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    emit_match_impl(db, match_id, scrutinee, arms, env, ctx, false)
}

/// Render a scalar `match`, with `tail` selecting whether the arm bodies are in TAIL position (inside a
/// self-loop): when `tail`, each arm body goes through [`emit_tail`] (a self-call iterates the loop, any
/// other value `break`s it); otherwise each arm body is an ordinary expression grounded to the match's
/// result width. The scrutinee, patterns, and guards are identical either way.
#[allow(clippy::too_many_arguments)]
fn emit_match_impl(
    db: &mut Db,
    match_id: StructId,
    scrutinee: StructId,
    arms: &[crate::core::MatchArm],
    env: &Env,
    ctx: &Ctx,
    tail: bool,
) -> Result<String, Reject> {
    // The match's RESULT integer type, if any — a bare-literal arm body is grounded to it so a
    // default-Int64 literal arm beside a narrow-width arm does not yield a mismatched type (Rust E0308),
    // the same reconciliation the wasm backend applies to a `ConstInt` arm body via `emit_operand`.
    let result_ty = type_of(db, match_id);
    let result_it = match &result_ty {
        Ty::Int(it) => Some(*it),
        _ => None,
    };
    // The match's RESULT FLOAT width, if any — the float twin: a bare `ConstFloat` arm body defaults to
    // Float64, so under an outer `Float32` result (`(: (match n (0 0.5) (_ 1.5)) Float32)`) an ungrounded
    // arm renders `f64::from_bits(…)` in an `-> f32` match → rustc E0308 (corpus-bugfix: the match-arm
    // sibling of the `if`-branch float grounding `emit_branch` already does). Ground each arm literal to
    // this width. `float_width_of_ty` strips a nominal/Qty wrapper (a `(Qty Float32 …)` result grounds f32).
    let result_fw = float_width_of_ty(&result_ty);
    let scrut = emit(db, scrutinee, env, ctx)?;
    let mut out = format!("match ({scrut}) {{ ");
    for arm in arms {
        let pat = match arm.probe {
            crate::core::Probe::Int(ref v) => int_pattern(db, scrutinee, v)?,
            crate::core::Probe::Bool(x) => (if x { "true" } else { "false" }).to_string(),
            // A string-literal probe only ever FOLDS (a constant scrutinee); a runtime string match
            // declines at `is_scalar` before a `Core::Match` is built, so no `Probe::Str` reaches a
            // runtime match emit on either backend.
            crate::core::Probe::Str(_) => {
                return Err(crate::diag::Reject::unsupported(
                    "a runtime string-literal match is not supported by the Rust backend",
                ));
            }
            // A byte-string-literal probe reaching a scalar `Core::Match` emit is unreachable in practice
            // (a runtime Bytes match desugars to a `value-eq` if-chain in `lower`, and a Bytes is not
            // `is_scalar`, so no `Probe::Bytes` survives to a scalar match); decline defensively like `Str`.
            crate::core::Probe::Bytes(_) => {
                return Err(crate::diag::Reject::unsupported(
                    "a runtime byte-string-literal match is not supported by the Rust backend",
                ));
            }
            // A runtime char-literal probe (Char-rep 3/N): the scrutinee is a native rust `char`
            // (`rust_type(Ty::Char) = char`), so the arm pattern is a char LITERAL — the SAME
            // `rust_char_literal` `Core::ConstChar` emits, so the pattern's escaping exactly matches the
            // scrutinee value. `is_scalar` (2/N) now routes a runtime char scrutinee into a `Core::Match`,
            // reaching this arm (was a decline).
            crate::core::Probe::Char(c) => rust_char_literal(c),
            // A `ListLen` probe only ever FOLDS (a constant list payload); a runtime list payload declines
            // at `build_lit_test` before a decision tree is emitted, so it never reaches a runtime match.
            crate::core::Probe::ListLen { .. } => {
                return Err(crate::diag::Reject::unsupported(
                    "a runtime list-pattern match is not supported by the Rust backend",
                ));
            }
            // A `MapHasKeys` probe only ever FOLDS (a constant map sub-value); a runtime map declines at
            // `build_lit_test`, so it never reaches a runtime match emit.
            crate::core::Probe::MapHasKeys { .. } => {
                return Err(crate::diag::Reject::unsupported(
                    "a runtime map-pattern match is not supported by the Rust backend",
                ));
            }
            crate::core::Probe::Wild => "_".to_string(),
        };
        let guard = match arm.guard {
            Some(g) => format!(" if {}", emit(db, g, env, ctx)?),
            None => String::new(),
        };
        let b = if tail {
            // Tail arm: `emit_tail` produces `break v;` / a self-loop `continue` — a statement, so the
            // arm is `pat => { <stmt> }` (braces make a statement a valid match-arm body).
            format!("{{ {} }}", emit_tail(db, arm.body, env, ctx)?)
        } else {
            match (result_it, result_fw) {
                (Some(it), _) => emit_grounded(db, arm.body, it, env, ctx)?,
                (None, Some(w)) => emit_grounded_float(db, arm.body, w, env, ctx)?,
                (None, None) => emit(db, arm.body, env, ctx)?,
            }
        };
        out.push_str(&format!("{pat}{guard} => {b}, "));
        // An UNGUARDED wildcard is the unconditional catch-all — every later arm is unreachable (as in
        // `lower`/wasm). Stop here so the emitted `match` is exhaustive with no unreachable arm.
        if arm.guard.is_none() && matches!(arm.probe, crate::core::Probe::Wild) {
            break;
        }
    }
    out.push('}');
    Ok(out)
}

/// Emit a runtime LIST match `(match xs ((list) …) ((list a .. rest) …) …)` → an `if`/`else if` chain
/// over the scrutinee's `.len()`. Non-tail form (the arm bodies are ordinary values). See
/// [`emit_list_match_impl`].
fn emit_list_match(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    emit_list_match_impl(db, scrutinee, arms, env, ctx, false)
}

/// Emit a runtime LIST match as a length-tested `if`/`else if` chain over the scrutinee's `.len()`.
///
/// Each [`ListArm`]'s condition tests the scrutinee length: `LenEq(n)` → `len == n`, `LenGe(lead)` →
/// `len >= lead`, `Any` → an unconditional `else`. A `guard` ANDs a boolean onto the length test (a false
/// guard falls through to the next arm — the natural `else` chain). The scrutinee is a pure occurrence
/// (a param/local, per `lower`), so each element/rest binder in an arm body re-reads it via `SumPayload`
/// (`Elem(i)` → `xs[i]`, `RestFrom(k)` → `xs[k..].to_vec()`), materializing it identically each time.
/// `lower` proved exhaustiveness (every length ≥ 0 is covered), so the chain always ends in a catch-all;
/// a defensive trailing `else { panic!("unreachable") }` makes the emitted `if` total for Rust (a chain
/// with no bare `Any`/`LenGe(0)` tail — e.g. only guarded arms — would otherwise be a non-exhaustive
/// `if` with no `else`, an E0317 "missing else"). When `tail`, each arm body goes through [`emit_tail`]
/// (a self-call iterates the enclosing loop); otherwise each is an ordinary value grounded to the match's
/// result width.
#[allow(clippy::too_many_arguments)]
fn emit_list_match_impl(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[crate::core::ListArm],
    env: &Env,
    ctx: &Ctx,
    tail: bool,
) -> Result<String, Reject> {
    use crate::core::ListArmCond;
    // The scrutinee's Rust value, evaluated ONCE into a local so `.len()` and every binder read the same
    // list. (The scrutinee is pure, but binding it once keeps the emitted chain readable and avoids
    // re-emitting a possibly-large expression per length test.) The binder is fresh per match nesting.
    let scrut = emit(db, scrutinee, env, ctx)?;
    let lv = format!("__lm{}", scrutinee.0);
    // Register `(scrutinee, lv)` so every arm read of the scrutinee — the guard's element-discriminant
    // probe and each element/rest binder (`emit_scrutinee` → this local) — reads the ONE bound `let lv`,
    // NOT a re-emit of the scrutinee expression. Critical when the scrutinee is a non-Copy PROJECTION (a
    // tuple field `(__msN).0` that is a `Vec`, from a recursive fold that rebuilds a list): `let lv =
    // (__msN).0` MOVES the Vec out, so a re-emitted `(__msN).0` in a guard/body is a borrow-of-moved-value
    // (rustc E0382) — the mutually-recursive-fold-over-a-rebuilt-list no-build (breaker, 2026-07-25; wasm
    // has no ownership so it computed). Routing reads through `lv` (element reads `.clone()` off it, the
    // rebuild arm moves it as its last use) keeps every read valid. This is the list-match twin of the
    // `MatchSum` materialize-once registration. `scrut` above is emitted with the ORIGINAL ctx so the
    // binding RHS still projects the parent; only the arm reads consult the local.
    let mut arm_ctx = ctx.clone();
    arm_ctx.scrut_locals.push((scrutinee, lv.clone()));
    let ctx = &arm_ctx;
    // The match's result integer type — a bare-literal arm body is grounded to it (as in `emit_match_impl`).
    let result_it = arm_result_it(db, arms);
    let mut chain = String::new();
    let mut first = true;
    let mut has_catch_all = false;
    for arm in arms {
        // The length test. `Any` (a bare binder / `_`) is the unconditional catch-all; render it as the
        // final `else` (no condition). A guard ANDs onto the length test.
        let len_cond = match arm.cond {
            ListArmCond::LenEq(n) => Some(format!("{lv}.len() == {n}")),
            // `LenGe(0)` is unconditional (every list has length ≥ 0) — treat like `Any`.
            ListArmCond::LenGe(0) => None,
            ListArmCond::LenGe(lead) => Some(format!("{lv}.len() >= {lead}")),
            ListArmCond::Any => None,
        };
        let cond = match (len_cond, arm.guard) {
            (Some(c), Some(g)) => Some(format!("{c} && {}", emit(db, g, env, ctx)?)),
            (Some(c), None) => Some(c),
            // An unconditional length (Any/LenGe(0)) WITH a guard is still conditional on the guard.
            (None, Some(g)) => Some(emit(db, g, env, ctx)?),
            (None, None) => None,
        };
        let body = if tail {
            // `emit_tail` returns a STATEMENT form — `break v;` or a self-loop `{ … continue; }` block — so
            // it drops directly inside the arm's own `{ … }` (added at the `chain.push_str` below / the
            // catch-all/bare-binder wraps). Do NOT wrap it in an extra `{ }` here: a self-loop arm would then
            // be `if c { { { … continue; } } }`, whose inner brace pair rustc's `unused_braces` lint flags as
            // "unnecessary braces around block return value" — a warning the gate's -D warnings turns into a
            // NO-BUILD (breaker #18: a nested match on a recursive-call result in a tail arm). The single arm
            // brace is sufficient; emit_tail's own block (for the continue case) is the value, not doubly
            // wrapped.
            emit_tail(db, arm.body, env, ctx)?
        } else {
            match result_it {
                Some(it) => emit_grounded(db, arm.body, it, env, ctx)?,
                None => emit(db, arm.body, env, ctx)?,
            }
        };
        match cond {
            Some(c) => {
                let kw = if first { "if" } else { "else if" };
                chain.push_str(&format!("{kw} {c} {{ {body} }} "));
                first = false;
            }
            None => {
                // An unconditional arm — the catch-all `else`. Every later arm is unreachable (as in
                // `lower`), so stop here.
                if first {
                    // No preceding condition: the whole match is just this arm's body (a bare-binder match).
                    return Ok(format!("{{ let {lv} = {scrut}; {body} }}"));
                }
                chain.push_str(&format!("else {{ {body} }}"));
                has_catch_all = true;
                break;
            }
        }
    }
    // A chain with no unconditional tail (only `==`/`>=`/guarded arms) needs a defensive `else` so the
    // `if` is a total expression (Rust E0317). `lower` guarantees exhaustiveness, so this is unreachable.
    if !has_catch_all {
        chain.push_str("else { panic!(\"unreachable\") }");
    }
    Ok(format!("{{ let {lv} = {scrut}; {chain} }}"))
}

/// The result INTEGER type shared by a list-match's arms (for grounding a bare-literal arm body), read off
/// the first arm's body type. `None` if it is not an integer type (no width grounding needed).
fn arm_result_it(db: &mut Db, arms: &[crate::core::ListArm]) -> Option<IntTy> {
    let first = arms.first()?;
    match type_of(db, first.body) {
        Ty::Int(it) => Some(it),
        _ => None,
    }
}

/// An integer literal PATTERN in the scrutinee's Rust type — the literal written so it matches a value
/// of that type. Uses the same bit-pattern spelling as a constant, but a pattern cannot contain an
/// `as` cast, so a value that would need reinterpretation (a signed negative, or an unsigned value
/// above the signed max) is written as its signed decimal / plain unsigned decimal directly.
fn int_pattern(db: &mut Db, scrutinee: StructId, v: &IntValue) -> Result<String, Reject> {
    let it = int_ty_of(db, scrutinee);
    let target = types::rust_type(&db.name_ctx(), &Ty::Int(it)).ok_or_else(|| {
        Reject::decline("match scrutinee width has no native Rust representation")
    })?;
    // A pattern is written as a plain decimal in the target type (`5i64`, `-1i8`). `int_value_decimal`
    // gives the signed decimal (with a leading `-` for a negative), which is a valid Rust integer
    // pattern for the signed target; for an unsigned target the value is non-negative so it is a plain
    // decimal. This is exact for every in-range value (range already checked at type time).
    Ok(format!("{}{target}", int_value_signed_decimal(v)))
}

/// The Rust identifier a `let` binding is emitted under — its source name, made a valid identifier,
/// de-collided against names already in scope by appending a numeric suffix. Determinism matters: the
/// body's `LocalRef` to this binding must resolve to the same identifier, so it is inserted into the
/// environment by the caller right after this returns.
fn local_name(db: &Db, binder: StructId, env: &Env) -> String {
    let base = db
        .ast
        .as_name(binder)
        .map(super::sanitize_ident)
        .unwrap_or_else(|| "tmp".to_string());
    // De-collide: if the base is already bound (a shadowing `let`, or a param of the same name), append
    // a suffix until unique. The binder occurrence is unique, so this always terminates.
    if !env.values().any(|n| n == &base) {
        return base;
    }
    let mut n = 1;
    loop {
        let cand = format!("{base}_{n}");
        if !env.values().any(|v| v == &cand) {
            return cand;
        }
        n += 1;
    }
}

/// The signed decimal string of an integer value (a leading `-` for a negative) — for a Rust literal
/// or pattern in a signed context.
fn int_value_signed_decimal(v: &IntValue) -> String {
    let mag = int_value_decimal(v);
    if v.negative && mag != "0" {
        format!("-{mag}")
    } else {
        mag
    }
}

/// The decimal string of an integer value's MAGNITUDE (unsigned, no sign) — the big-endian magnitude
/// bytes rendered in base 10. Empty magnitude is `0`. Done by repeated division so it needs no bignum
/// dependency (the value is arbitrary-precision; a width-bounded value here is small, but the routine
/// is general).
fn int_value_decimal(v: &IntValue) -> String {
    if v.magnitude.is_empty() || v.magnitude.iter().all(|&b| b == 0) {
        return "0".to_string();
    }
    // Repeatedly divide the big-endian magnitude by 10, collecting remainder digits.
    let mut digits = Vec::new();
    let mut cur = v.magnitude.clone();
    while !cur.iter().all(|&b| b == 0) {
        let mut rem: u16 = 0;
        for byte in cur.iter_mut() {
            let acc = (rem << 8) | (*byte as u16);
            *byte = (acc / 10) as u8;
            rem = acc % 10;
        }
        digits.push(b'0' + rem as u8);
    }
    digits.reverse();
    String::from_utf8(digits).expect("ascii digits")
}

/// The Rust comparison operator symbol for a comparison prim, or `None` for a non-comparison prim.
fn compare_sym(op: Prim) -> Option<&'static str> {
    Some(match op {
        Prim::Lt => "<",
        Prim::Gt => ">",
        Prim::Le => "<=",
        Prim::Ge => ">=",
        Prim::Eq => "==",
        _ => return None,
    })
}

/// Whether a binding of the node's type must be `.clone()`d when READ, because its emitted Rust type is
/// NON-COPY (move-only) and a second by-value use would be an E0382 move error. Only a `List` (→ `Vec<T>`)
/// and a compound that CONTAINS a list are non-Copy in the types this backend emits today; every scalar,
/// `Bool`, `Unit`, all-scalar tuple/record, and enum whose payloads are all Copy is `Copy`/read-as-is.
/// Conservative by construction: it returns `true` ONLY for a type provably non-Copy, so every pre-list
/// Copy case stays byte-identical (no spurious `.clone()` → no needless-clone lint under `-D warnings`).
/// A `Nominal` newtype erases to its inner type; a `Sum`/`Tuple`/`Record` is non-Copy iff any component is.
fn needs_clone_on_read(db: &mut Db, id: StructId) -> bool {
    ty_is_non_copy(&type_of(db, id))
}

/// Whether `ty`'s emitted Rust representation is non-Copy (move-only). A `List` maps to `Vec` (non-Copy);
/// a compound is non-Copy iff any element/field/payload is. Everything else this backend emits is Copy.
fn ty_is_non_copy(ty: &Ty) -> bool {
    match ty {
        // `Vec<T>`/`BTreeMap<K,V>`/`BTreeSet<T>`/`String` are heap-owned values — non-Copy (move-only), so a
        // binding of one read in more than one position clones (the clone-on-read discipline). `Big`
        // (`cdz_num::Big`) owns a limb `Vec`, so it is likewise non-Copy → clone-on-read.
        Ty::List(_)
        | Ty::Map(_, _)
        | Ty::Set(_)
        | Ty::String
        // A `Symbol` maps to Rust's `String` (types::rust_type — a Symbol value IS its text; the
        // `Symbol.of`/`Symbol.to-string` retag is the identity on the `String`), so it is likewise
        // move-only and MUST clone-on-read. Missing this arm regressed adv-54's rust-async landing:
        // a Symbol-typed binding read more than once (e.g. `(def back (Symbol.to-string (Symbol.of s)))`,
        // then compared `(= (Symbol.of back) sym)`) emitted a bare move on the first read and E0382'd
        // (`borrow of moved value`) on the second — the exact failure the wasm StrSlice/StrToBytes
        // keep-binding fix exposed on the rust backend.
        | Ty::Symbol
        | Ty::Bytes
        | Ty::BigInt
        | Ty::Rational => true,
        // A compound is non-Copy iff any component is (a tuple/record of scalars stays Copy).
        Ty::Tuple(elems) => elems.iter().any(ty_is_non_copy),
        Ty::Record(fields) => fields.values().any(ty_is_non_copy),
        // A newtype erases to its inner type — inherit its Copy-ness.
        Ty::Nominal { inner, .. } => ty_is_non_copy(inner),
        // A sum's emitted enum `#[derive(Clone)]`s but is NEVER `#[derive(Copy)]` (the derive list adds
        // only Clone/PartialEq/Eq — see `enums::emit_one_enum`), so an enum VALUE is move-only in Rust
        // regardless of whether its payloads happen to be Copy. Reading a sum binding therefore clones it,
        // so a value used in more than one position (e.g. matched twice, or matched then passed) does not
        // E0382-move. This also correctly covers a sum whose payload CONTAINS a `Vec` (a non-generic
        // `(KCall (Tuple Int64 (List Core)))`), which the type-args check alone would miss. Over-cloning a
        // single-use enum is sound; the emitted enums carry `#[allow(clippy::all)]` so no needless-clone
        // lint fires.
        Ty::Sum { .. } => true,
        // A function value is `Rc<dyn Fn>` — Clone (so a multiply-used closure clones on read) but NOT
        // Copy, so a closure read in more than one position must clone, like any other heap value.
        Ty::Fn(_, _) => true,
        _ => false,
    }
}

/// Render `s` as a Rust STRING LITERAL (`"…"`) with a valid escape for every character — so the emitted
/// source compiles regardless of the string's content. Escapes `\`, `"`, the common whitespace controls
/// (`\n`/`\r`/`\t`), and any other control/non-printable char via `\u{..}`; a printable non-ASCII char
/// (a UTF-8 letter like `é`) passes through verbatim (a Rust string literal is UTF-8, so this preserves
/// the exact scalar content — matching cdz-run's raw-passthrough String render). Includes the surrounding
/// quotes.
fn rust_string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // A CONTROL scalar → an explicit `\u{..}` escape (valid in a Rust string literal). `is_control`
            // covers C0 (0x00-0x1F), DEL (0x7F), AND C1 (0x80-0x9F) — the earlier `< 0x20 || == 0x7f` guard
            // missed the C1 range, emitting a raw control byte into the literal. Matches
            // `cadenza-syntax::render_char`'s `is_control` branch. A printable char (ASCII or a higher
            // UTF-8 scalar like `é`) is emitted verbatim — valid in a UTF-8 Rust literal.
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Render `c` as a Rust CHAR LITERAL (`'…'`) with a valid escape for every scalar — so the emitted
/// source compiles for any char. Escapes `'`, `\`, the whitespace controls, and any other control/
/// non-printable scalar via `\u{..}`; a printable scalar (incl a UTF-8 letter) is emitted verbatim.
fn rust_char_literal(c: char) -> String {
    match c {
        '\\' => "'\\\\'".to_string(),
        '\'' => "'\\''".to_string(),
        '\n' => "'\\n'".to_string(),
        '\r' => "'\\r'".to_string(),
        '\t' => "'\\t'".to_string(),
        // `is_control` covers C0 + DEL + C1 (0x80-0x9F) — the earlier `< 0x20 || == 0x7f` missed C1,
        // emitting a raw control char into the Rust char literal. Matches `cadenza-syntax::render_char`.
        c if c.is_control() => format!("'\\u{{{:x}}}'", c as u32),
        c => format!("'{c}'"),
    }
}

/// A human op name for a trap panic message.
fn op_name(op: Prim) -> &'static str {
    match op {
        Prim::Add => "addition",
        Prim::Sub => "subtraction",
        Prim::Mul => "multiplication",
        Prim::Div => "division",
        Prim::Rem => "remainder",
        _ => "arithmetic",
    }
}

/// The integer type of the node at `id`, defaulting to `Int64` for a non-integer type — the same
/// read-off `select.rs` does (`int_ty_of`). A non-integer node never reaches an integer-typed emit
/// path, so the default is defensive.
/// Peel a `Ty::Qty` to its inner type — a quantity erases to its inner numeric's machine slot, so a
/// value-form width reader (the `ConstFloat`/`ConstFloatNan` f32-vs-f64 emit) must see through the wrapper.
/// A non-quantity type passes through unchanged. (The float twin of `int_ty_of`'s Qty peel.)
/// The FLOAT width (32/64) a `Ty` grounds to after strip_nominal → peel `Ty::Qty` → strip_nominal, or
/// `None` when it is not a float under those erasures. The float twin of `int_ty_of`'s width read: the
/// bare `peel_qty` alone (a RAW `Ty::Qty` match, no strip_nominal) MISSES a NOMINAL-over-Qty wrapper
/// (`(type Len (Q (Qty Float32 …)))` stored as a heap value = `Ty::Nominal { inner: Qty { Float32 } }`),
/// so it defaults to f64 → `f64::from_bits` into an `f32` slot → rustc E0308 / invalid wasm (the reviewer-
/// flagged gap the integer `int_ty_of` already fixed). Strip → peel → strip sees the inner float in every
/// wrapping. Returns `None` for a non-float so a caller can DISTINGUISH "not a float" (fall through).
fn float_width_of_ty(ty: &Ty) -> Option<u32> {
    let inner = match ty.strip_nominal() {
        Ty::Qty { inner, .. } => inner.strip_nominal(),
        other => other,
    };
    match inner {
        Ty::Float(ft) => Some(ft.ground_width()),
        _ => None,
    }
}

/// The FLOAT width of the node at `id` (strip_nominal → peel `Ty::Qty` → strip_nominal), defaulting to
/// `DEFAULT_FLOAT_WIDTH`. The float twin of `int_ty_of`'s width read — used by the `ConstFloat`/
/// `ConstFloatNan` emit to ground a wrapped f32 const to f32 (see `float_width_of_ty`).
fn float_width_of(db: &mut Db, id: StructId) -> u32 {
    float_width_of_ty(&type_of(db, id)).unwrap_or(crate::ty::DEFAULT_FLOAT_WIDTH)
}

fn int_ty_of(db: &mut Db, id: StructId) -> IntTy {
    // STRIP_NOMINAL then PEEL `Ty::Qty` then STRIP_NOMINAL again — mirroring the wasm backend's `int_ty_of`
    // EXACTLY (the cross-backend narrow-width lockstep). Two erasures compose:
    //  - `strip_nominal`: an ERASED newtype over an int (`(type W (Wrap UInt8))`) has its inner int's width,
    //    so a `(W.Wrap 5)` grounds to the INNER (u8), not the i64 default.
    //  - PEEL `Ty::Qty`: a quantity over an int (`(Qty Int8 u)`) erases to its inner int's width (the unit is
    //    compile-time), so a `(Qty.of (Int8.of n) u)` magnitude grounds to the INNER (i8).
    // The leading strip is what the reviewer flagged as missing: a NOMINAL-over-Qty (`(type Len (Q (Qty Int8
    // u)))` stored as a heap value) is `Ty::Nominal { inner: Qty }` — WITHOUT the leading strip the raw
    // `Ty::Qty` match misses it and it defaults to i64, so a `Map.insert` value rendered `n as i64` into an
    // `i8`-typed `BTreeMap<_, i8>` slot → Rust E0308. Strip → peel → strip handles nominal-over-Qty-over-
    // nominal, so `int_ty_of` sees the true narrow inner in every wrapping. (Verified: closes the
    // nominal-over-Qty map-value E0308; the bare-Qty case v-quantity fixed still resolves to the inner.)
    let solved = type_of(db, id);
    let inner = match solved.strip_nominal() {
        Ty::Qty { inner, .. } => inner.strip_nominal(),
        other => other,
    };
    match inner {
        Ty::Int(it) => *it,
        _ => IntTy {
            sign: Sign::Fixed(true),
            width: Width::Fixed(crate::ty::DEFAULT_INT_WIDTH),
        },
    }
}

/// The Rust path `<Enum>::<Variant>` for the sum value at `id` (a `Ty::Sum`) whose runtime discriminant
/// is `disc`. The enum name is the sum's declared name (sanitized); the variant name is the declaration's
/// `disc`-th variant (the discriminant IS the declaration-order position). Both are `sum_ident`-sanitized
/// so they match the emitted `enum` declaration. Declines if the node is not a sum or the disc is out of
/// range (a compiler bug — a `SumNew` always carries a sum type + an in-range disc).
fn sum_variant_path(db: &mut Db, id: StructId, disc: u32) -> Result<String, Reject> {
    let ty = type_of(db, id);
    sum_variant_path_of_ty(db, &ty, disc)
}

/// Emit a nullary variant's constructor from its bare path (`Enum::Variant`, from `sum_variant_path`).
///
/// A MONOMORPHIC sum keeps the bare path (`Shape::Circle`). A GENERIC sum needs a TYPE ANNOTATION: a bare
/// `Option::None` gives rustc nothing to infer the type parameter from in a position with no expected type
/// (an `if`/`match` branch typed before its sibling `Some` arm). When the node's type args are SOLVED, emit
/// a turbofish — `Option::<(Vec<Term>, Term)>::None`. When they are UNSOLVED (a bare `Ty::Var` — the None's
/// own type is `Option<?>`, the concrete arg living only in the surrounding context this local emit can't
/// see), we cannot spell the annotation, and a bare `Option::None` would be E0282 "type annotations needed"
/// (an uncompilable artifact). So DECLINE — decline-don't-miscompile. (A later increment that threads the
/// expected type from the enclosing `def` result / match subject into the branch emit would lift this; the
/// wasm backend has the type at the value-encode boundary, so it does not hit this.)
fn nullary_variant_path(ncx: &crate::ty::NameCtx, ty: &Ty, disc: u32, bare: &str) -> String {
    let _ = disc; // the disc already selected `bare`; kept for call-site symmetry with sum_variant_path.
    let Ty::Sum { args, .. } = ty.strip_nominal() else {
        return bare.to_string();
    };
    if args.is_empty() {
        return bare.to_string(); // monomorphic sum — bare path, no annotation needed.
    }
    // Generic sum: build the turbofish from the SOLVED args — a pure improvement over the bare path when
    // every arg has a native rep. If ANY arg is unsolved (`Ty::Var`) or unrepresentable, `rust_type`
    // returns `None`: fall back to the BARE path. This is INFALLIBLE — it NEVER declines (returns `String`,
    // not `Result`). rustc infers the bare form in most contexts; the one residual case it CANNOT (a
    // nullary generic variant in a branch typed before its type-fixing sibling, args living only in the
    // enclosing context) is caught EARLIER by `Core::If`'s generic-sum result annotation, not by declining
    // here — a FALSE decline regressed 22 cases rustc DOES infer (annotate-when-known, never decline).
    // [Reconciles PR#467: the `Result` return + a "decline" doc were leftovers from an abandoned
    //  decline-when-unsolved attempt; the behavior always fell back to bare, so the type is now `String`.]
    let mut params = Vec::with_capacity(args.len());
    for a in args.iter() {
        match types::rust_type(ncx, a) {
            Some(p) => params.push(p),
            None => return bare.to_string(),
        }
    }
    match bare.rsplit_once("::") {
        Some((enum_path, variant)) => format!("{enum_path}::<{}>::{variant}", params.join(", ")),
        None => bare.to_string(),
    }
}

/// The Rust `<Enum>::<Variant>` path for the `disc`-th variant of the sum TYPE `ty` — the type-keyed core
/// of [`sum_variant_path`] (which reads the type off a node). Split out so a nested switch can name a
/// variant of a sub-value's type. Declines if `ty` is not a sum, its enum is not representable (a
/// recursive/unrepresentable sum has no Rust type), or the disc is out of range.
/// A compact, unique-per-CONTENT tag for a sum-switch path — encodes each `PathStep`'s KIND AND INDEX,
/// not just the path's length. Used to de-collide `emit_sum_switch`'s payload binder names: two sibling
/// switches on the same scrutinee at the same depth but different path content (`[Elem(0)]` vs `[Elem(1)]`
/// — the bottom-up-fold tuple-of-recursive-results idiom) must get DISTINCT binder names, else the two
/// `E.Lit` binders alias and `(+ x y)` reads `p+p` (a wrong-value miscompile). Length alone collides them;
/// content does not. `Payload`→`p`, `Elem(i)`→`e{i}`, `RestFrom(k)`→`r{k}`, joined so no two distinct
/// paths share a tag (indices are decimal, kind letters delimit).
fn sum_path_tag(path: &[crate::core::PathStep]) -> String {
    let mut tag = String::new();
    for step in path {
        match step {
            crate::core::PathStep::Payload => tag.push('p'),
            crate::core::PathStep::Elem(i) => {
                tag.push('e');
                tag.push_str(&i.to_string());
            }
            crate::core::PathStep::RestFrom(k) => {
                tag.push('r');
                tag.push_str(&k.to_string());
            }
            crate::core::PathStep::TupleRestFrom(k) => {
                tag.push('t');
                tag.push_str(&k.to_string());
            }
        }
    }
    tag
}

pub(super) fn sum_variant_path_of_ty(db: &mut Db, ty: &Ty, disc: u32) -> Result<String, Reject> {
    let decl_occ = match ty.strip_nominal() {
        Ty::Sum { decl, .. } => *decl,
        _ => return Err(Reject::decline("sum construction node is not a sum type")),
    };
    // The sum's enum must have EMITTED — a recursive/unrepresentable sum has no Rust type, so naming
    // `<Enum>::<Variant>` here would reference an undeclared type. This catches a construct/match of such
    // a sum ANYWHERE IN A BODY (not just a signature): the fold can inline a helper that builds a
    // non-representable sum as a discarded intermediate (`(. (tuple (NLit 5) 9) 1)` keeps only the Int64,
    // but still constructs `Node::NLit`), which the signature-level `sum_representable` guard cannot see.
    if !super::enums::sum_representable(db, ty) {
        // Name the PRECISE reason (a recursive newtype vs a genuine recursive/unrepresentable sum) so the
        // decline points whoever picks up the gap at the right fix — `unrepresentable_reason` walks `ty`
        // (the UNSTRIPPED value type, so a recursive newtype still reads as `Ty::Nominal`) and returns the
        // newtype phrasing for the `(type Lst (Mk (Option (Tuple Int64 Lst))))` shape.
        return Err(Reject::decline(super::enums::unrepresentable_reason(
            db, ty,
        )));
    }
    let decl = db
        .type_decl_by_occ(decl_occ)
        .ok_or_else(|| Reject::decline("sum declaration not found"))?;
    let enum_name = types::sum_ident(&decl.name);
    let variant = decl
        .variants
        .get(disc as usize)
        .ok_or_else(|| Reject::decline("sum discriminant out of range"))?;
    let vname = types::sum_ident(&variant.name);
    Ok(format!("{enum_name}::{vname}"))
}

/// The payload type of a sum's variant 0 (the shape a `Payload` path step descends into) — `None` for a
/// nullary or unresolvable variant. Substitutes the sum's actual type args into the variant's generic
/// payload (`Option Int64`'s `Some` payload is `Int64`, not `?0`). The rust-backend twin of the wasm
/// backend's `sum_single_payload_ty`; used by `ty_at_sum_path` to walk a nested switch's subject type.
fn sum_disc0_payload_ty(db: &mut Db, sum: &Ty) -> Option<Ty> {
    variant_payload_ty(db, sum, 0)
}

/// The payload type of a sum's variant `disc` at THIS instantiation — `None` for a nullary or
/// unresolvable variant. Generalizes [`sum_disc0_payload_ty`] to ANY discriminant: a nested switch on a
/// variant at disc ≥ 1 (`(type W (A Int64) (V (Option Int64)))` matched `(W.V (Some n))`) must read the
/// payload of the ACTUAL entered variant (`V` → `Option Int64`), not variant 0's (`A` → `Int64`). Reading
/// variant 0 unconditionally made a nested constructor match on a non-first variant resolve to the wrong
/// sub-value type and decline (`sum construction node is not a sum type`). Substitutes the sum's actual
/// type args (`Option a`'s `V` payload at `W Int64` → `Option Int64`).
pub(super) fn variant_payload_ty(db: &mut Db, sum: &Ty, disc: u32) -> Option<Ty> {
    let stripped = sum.strip_nominal().clone();
    let Ty::Sum { decl, .. } = &stripped else {
        return None;
    };
    let ctor = {
        let td = db.type_decl_by_occ(*decl)?;
        td.variants.get(disc as usize)?.ctor?
    };
    crate::infer::payload_ty_at_instantiation(db, ctor, &stripped)
}

/// Emit a sum MATCH → a Rust `match` on the scrutinee. The `root` continuation is normally a
/// [`SumCont::Switch`] on the scrutinee's own discriminant (`path` empty); each `SumArm` becomes
/// `<Enum>::<Variant>(binder) => <cont>` (a nullary variant → `<Enum>::<Variant> => …`, no binding) and a
/// `disc: None` arm is the `_` default. The arm BINDS its variant's payload to a fresh identifier and
/// threads a `SumBind` (keyed by the scrutinee + the arm's path `[Payload]`) into the continuation's
/// `Ctx`, so a `Core::SumPayload` in the body resolves to that identifier. A LEAF continuation is the arm
/// body; a NESTED switch (the decision tree recursing into a deeper sub-value) and a GUARDED arm (a
/// sum-scrutinee guard) are DECLINED for now — the common single-level match (Option, a flat user sum)
/// lands first; nested constructor patterns and sum guards follow.
///
/// A disc-fold can collapse the root to a nested `Switch` on a deeper path (a statically-known scrutinee
/// discriminant), or to a `Guarded`/`Leaf` — those non-`Switch` roots are declined here (they need the
/// deeper-path/guard rendering not yet built); the reached-directly `Leaf` root already folds in `lower`.
fn emit_sum_match(
    db: &mut Db,
    scrutinee: StructId,
    root: &crate::core::SumCont,
    // The match's solved integer RESULT type, if any — each `Leaf` continuation body is grounded to it (a
    // narrow sum-payload arm widened to a wider unified result, a default-Int64 literal arm narrowed).
    // `None` for a non-integer result (no width to reconcile).
    result_it: Option<IntTy>,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // The root is normally a Switch on the scrutinee's own discriminant (path empty). A non-root-Switch
    // continuation (a Guarded arm or a bare Leaf as the ROOT) is a shape this slice does not yet render —
    // decline. A `Switch` root (empty or a disc-folded deeper path) recurses through `emit_sum_switch`.
    // A `Switch` root recurses through `emit_sum_switch`. A NON-Switch root arises when the disc-fold in
    // `lower` collapses the root `Switch` on a STATICALLY-KNOWN discriminant (a constant `SumNew`
    // scrutinee) to the selected arm's continuation — a `LitTest` (`(match (Cons x Nil) ((Cons 0 t) …))`
    // where the `Cons` tag is known but the payload `x` is runtime), a `Guarded`, or a bare `Leaf`. Those
    // continuations are exactly what `emit_sum_cont` renders (it reads a sub-value via `emit_sum_payload`,
    // which folds against the constant scrutinee's payload nodes), so route them there rather than
    // declining. Before this, a constant-disc recursive/literal match declined on Rust while wasm compiled
    // it — the last non-Switch-root gap.
    match root {
        crate::core::SumCont::Switch { path, arms } => {
            emit_sum_switch(db, scrutinee, path, arms, result_it, env, ctx)
        }
        crate::core::SumCont::Guarded { .. }
        | crate::core::SumCont::Leaf(_)
        | crate::core::SumCont::LitTest { .. } => {
            emit_sum_cont(db, scrutinee, root, result_it, env, ctx)
        }
    }
}

/// Emit a `Switch` on the sub-value of `scrutinee` at `sw_path` — a Rust `match` dispatching on each
/// arm's discriminant. The switched-on VALUE is the scrutinee itself for the root (`sw_path == []`) or the
/// payload the enclosing arm bound for a NESTED switch (`sw_path` reads it via `emit_sum_payload`, which
/// resolves the parent arm's `__pay` binding). Each arm binds its own payload (`__pay{i}` at `sw_path +
/// [Payload]`) and recurses on its continuation: a `Leaf` emits the body, a nested `Switch` emits an inner
/// `match` (a nested constructor pattern like `(Some (Ok n))` — the outer switches Some/None, the Some
/// arm's continuation switches Ok/Err of the payload). Guarded / literal-payload continuations are still
/// declined (a later slice). This is what lets a RUNTIME nested sum match render on the Rust backend, the
/// two-compiler companion of the wasm decision-tree walk.
#[allow(clippy::too_many_arguments)]
fn emit_sum_switch(
    db: &mut Db,
    scrutinee: StructId,
    sw_path: &[crate::core::PathStep],
    arms: &[crate::core::SumArm],
    result_it: Option<IntTy>,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // ERASED-NEWTYPE ALIGNMENT: a `Payload` step over a NOMINAL newtype sub-value is a runtime no-op (the
    // tag erases; the value IS the inner). `lower` drops such steps from the BODY's `SumPayload` read
    // paths (`erase_nominal_steps`), but the decision-tree's switch/bind paths keep them, so a switch on a
    // sum WRAPPED in an erased newtype (`(type W (V (Result …)))` matched `(W.V (Result.Ok n))`) carries a
    // leading nominal `[Payload]` the erased body read does not — the bind (`sw_path+[Payload]`) and the
    // read (erased) then disagree by one step ("sum payload has no bound match arm"). Erase the switch
    // path the same way here so the subject reads the inner sum directly and every bind path this switch
    // mints aligns with the erased body reads. (wasm tolerates the raw path via its runtime-rep
    // coincidence; the Rust backend's path-keyed binds need the alignment.)
    let sw_path_owned = erase_nominal_switch_path(db, scrutinee, sw_path);
    let sw_path = &sw_path_owned[..];
    // The value this switch dispatches on: the scrutinee (root, empty path) or the sub-value at `sw_path`
    // (a nested switch — read the enclosing arm's payload binding). `emit_sum_payload` folds a constant
    // scrutinee or reads the bound `__pay` name.
    let subject = if sw_path.is_empty() {
        emit_scrutinee(db, scrutinee, env, ctx)?
    } else {
        emit_sum_payload(db, scrutinee, scrutinee, sw_path, env, ctx)?
    };
    // The SOLVED TYPE of the value this switch dispatches on. At the root (`sw_path == []`) it is the
    // scrutinee's own type; at a nested switch it is the sub-value type an ENCLOSING arm recorded in
    // `sum_path_types` when it descended into this variant (`variant_payload_ty` of the entered disc). A
    // recorded hint is authoritative — it carries which variant was entered, which the flattened path
    // cannot; only if none is recorded (the root, or a path with no hint) do we walk the type from the
    // scrutinee via `ty_at_sum_path` (which then falls back to the disc-0 payload for a `Payload` step).
    let subject_ty = lookup_sum_path_type(ctx, sw_path)
        .unwrap_or_else(|| ty_at_sum_path(db, scrutinee, sw_path));
    // GROUNDED-DECL CONSISTENCY GUARD (fold-miscompile hazard, coordinated with v-inference). When
    // inference's SCC return-type-fixpoint grounds `subject_ty` (e.g. the `(tuple (fold a) (fold b))`
    // elements of the bottom-up-fold idiom), a resolved `Ty::Sum` reaches here where before it was
    // unresolved (and `sum_variant_path_of_ty` declined). Cadenza sums are STRUCTURAL (a sum's identity IS
    // its shape — type-system.md), so a same-shape sum IS the arms' type; the only real failure is a
    // grounding to a DIFFERENT-shaped sum (fewer/other variants) that happens to have an in-range disc for
    // SOME arms but does not cover the whole match — which would resolve a WRONG `Enum::Variant` for the
    // arms it can't represent (the transiently-observed 20-not-12 fold miscompile). So: if `subject_ty` IS
    // a resolved sum, assert its variant count COVERS every arm's disc (max disc + 1); otherwise DECLINE
    // rather than mis-resolve. A structurally-correct grounding always covers the arms (they were checked
    // against it); an unresolved `subject_ty` (Any/var) still declines downstream in `sum_variant_path_of_ty`
    // exactly as today. This makes v-inference's grounding correct-or-declines, never a wrong-variant emit.
    if let Ty::Sum { decl, .. } = subject_ty.strip_nominal() {
        let variant_count = db
            .type_decl_by_occ(*decl)
            .map(|d| d.variants.len())
            .unwrap_or(0);
        let max_disc = arms.iter().filter_map(|a| a.disc).max();
        if let Some(md) = max_disc
            && (md as usize) >= variant_count
        {
            return Err(Reject::decline(
                "a sum match whose grounded subject type has fewer variants than the match's discriminants \
                 (a different-shaped grounding) is declined rather than resolving a wrong variant",
            ));
        }
    }
    let mut out = format!("match {subject} {{ ");
    for (i, arm) in arms.iter().enumerate() {
        match arm.disc {
            Some(disc) => {
                // `<Enum>::<Variant>(binder) => cont`. The payload binder is a fresh `__pay_{path}_{i}` the
                // arm's `SumPayload { scrutinee, sw_path + [Payload] (…) }` resolves to; a nullary variant
                // binds nothing. The bind's path is FROM THE ROOT scrutinee (`sw_path + [Payload]`), so a
                // deeper `SumPayload` (a binder in this arm's body, or a nested switch's subject) resolves.
                let vpath = sum_variant_path_of_ty(db, &subject_ty, disc)?;
                let arity = variant_arity_of_ty(db, &subject_ty, disc);
                let (pat_tail, arm_ctx) = if arity == 0 {
                    (String::new(), ctx.clone())
                } else {
                    // The binder name MUST be unique across NESTED matches, not just within one switch: a
                    // path-length+arm-index name (`__pay_{len}_{i}`) COLLIDES when two matches on DIFFERENT
                    // scrutinees nest at the same relative path — e.g. `(match (lookup m k1) ((Some a) (match
                    // (lookup m k2) ((Some b) (+ a b)) …)))`, where both `Some` binders are `__pay_0_0`, so
                    // the inner shadows the outer and `a` silently reads `b` (a wrong value, not a build
                    // error). Include the SCRUTINEE id (unique per match node) so nested matches get distinct
                    // identifiers; the bind is still resolved by `(scrutinee, path)`, this only de-collides
                    // the emitted name.
                    //
                    // But the scrutinee id + path LENGTH is STILL not enough: two SIBLING switches on the
                    // SAME scrutinee at the SAME depth but DIFFERENT path CONTENT collide. The bottom-up-fold
                    // idiom `(match (tuple (fold a) (fold b)) ((tuple (E.Lit x) (E.Lit y)) (+ x y)) …)` mints
                    // two switches on the one tuple-match node — one at path `[Elem(0)]`, one at `[Elem(1)]`,
                    // both len 1, both the `E.Lit` arm (i=0) → both `__pay_{s}_1_0`, so `x` and `y` alias and
                    // `(+ x y)` becomes `p.checked_add(p)` (the 20-not-12 miscompile v-inference's grounding
                    // exposed). Key the name off the path CONTENT (`sum_path_tag`) — `Elem(0)` vs `Elem(1)`
                    // yield distinct tags — so sibling switches never collide.
                    let name = format!("__pay_{}_{}_{i}", scrutinee.0, sum_path_tag(sw_path));
                    let mut payload_path = sw_path.to_vec();
                    payload_path.push(crate::core::PathStep::Payload);
                    // A RECURSIVE variant's field is a `Box<…>` (the enum boxes it), so the bind is boxed —
                    // a read derefs. The switched variant's type is THIS switch's subject type.
                    let boxed = super::enums::variant_is_recursive(db, &subject_ty, disc);
                    let mut c = ctx.clone();
                    c.sum_binds.push(SumBind {
                        scrutinee,
                        path: payload_path.clone(),
                        name: name.clone(),
                        boxed,
                    });
                    // RECORD the entered variant's payload type at the bind path, so a NESTED switch on this
                    // arm's payload (a disc-≥1 variant carrying a sum) resolves its subject to the ACTUAL
                    // payload type, not variant-0's. This is what the flattened path alone cannot supply.
                    if let Some(pty) = variant_payload_ty(db, &subject_ty, disc) {
                        c.sum_path_types.push((payload_path, pty));
                    }
                    (format!("({name})"), c)
                };
                let cont = emit_sum_cont(db, scrutinee, &arm.cont, result_it, env, &arm_ctx)?;
                out.push_str(&format!("{vpath}{pat_tail} => {cont}, "));
            }
            None => {
                // The default (wildcard) tail. Its continuation is emitted in the OUTER ctx (no payload
                // bound — a wildcard arm binds nothing of the switched variant).
                let cont = emit_sum_cont(db, scrutinee, &arm.cont, result_it, env, ctx)?;
                out.push_str(&format!("_ => {cont}, "));
            }
        }
    }
    out.push('}');
    Ok(out)
}

/// Emit an arm's CONTINUATION as a Rust EXPRESSION:
///  - `Leaf` → the arm body;
///  - nested `Switch` → an inner `match` ([`emit_sum_switch`], a nested constructor pattern);
///  - `Guarded { cond, body, els }` → `if <cond> { <body> } else { <els-cont> }` — the variant already
///    matched (the enclosing switch bound its payload into `ctx`), so `cond`/`body` see the payload binder;
///    a false guard FALLS THROUGH to the `els` continuation (the rest of the sub-matrix), mirroring the
///    wasm backend's guarded `if`;
///  - `LitTest { path, probe, then_, els }` → `if (<sub-value at path> == <literal>) { <then-cont> } else
///    { <els-cont> }` — a payload-literal refinement (`(Some 0)`); the sub-value is read via
///    `emit_sum_payload` (folds a constant / reads the bound name), compared to the literal, and a mismatch
///    falls through to `els` (the binding arm). Both mirror the wasm `emit_sum_cont`'s desugar to an `if`.
#[allow(clippy::too_many_arguments)]
fn emit_sum_cont(
    db: &mut Db,
    scrutinee: StructId,
    cont: &crate::core::SumCont,
    // The enclosing match's integer RESULT type — a `Leaf`/`Guarded`-body leaf is GROUNDED to it, so a
    // narrow sum-payload arm (a `UInt8` payload read) widens to the unified result and a default-Int64
    // literal arm narrows, keeping every `if`/`match` branch AND the fn return type at one width (else
    // rustc E0308). `None` for a non-integer result.
    result_it: Option<IntTy>,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // Ground a Leaf body to the match's result width (mirrors `emit_match_impl`'s per-arm grounding).
    let leaf = |db: &mut Db, b: StructId, ctx: &Ctx| match result_it {
        Some(it) => emit_grounded(db, b, it, env, ctx),
        None => emit(db, b, env, ctx),
    };
    match cont {
        crate::core::SumCont::Leaf(b) => leaf(db, *b, ctx),
        crate::core::SumCont::Switch { path, arms } => {
            emit_sum_switch(db, scrutinee, path, arms, result_it, env, ctx)
        }
        crate::core::SumCont::Guarded { cond, body, els } => {
            let c = emit(db, *cond, env, ctx)?;
            // The guard's BODY is a result leaf → ground it; the `els` continuation recurses (grounded there).
            let then_ = leaf(db, *body, ctx)?;
            let els = emit_sum_cont(db, scrutinee, els, result_it, env, ctx)?;
            Ok(format!("if {c} {{ {then_} }} else {{ {els} }}"))
        }
        crate::core::SumCont::LitTest {
            path,
            probe,
            then_,
            els,
        } => {
            // ERASED-NEWTYPE ALIGNMENT (the LitTest twin of `emit_sum_switch`'s `erase_nominal_switch_path`):
            // a literal-payload refinement over a single-variant newtype scrutinee (`(match (W.Wrap n)
            // ((W.Wrap 0) 100) …)`, `W = (Wrap UInt8)`) carries a probe `path` of `[Payload]`, but the newtype
            // tag erases (`(W.Wrap n)` → `n`) so the value IS the inner and mints no bind. Drop the erased
            // `Payload` step so the path becomes `[]` — `emit_sum_payload`'s empty-path arm then reads the
            // scrutinee value directly, and the width lookup below resolves the inner (narrow) type. Without
            // this the raw `[Payload]` found no bind and declined "sum payload has no bound match arm".
            let path_owned = erase_nominal_switch_path(db, scrutinee, path);
            let path = &path_owned[..];
            let subject = emit_sum_payload(db, scrutinee, scrutinee, path, env, ctx)?;
            // A LIST-LENGTH probe (`(Call (list _ .. rest))` — a list PATTERN in a sum-variant payload) tests
            // the subject list's `vec-len`, not a value equality: `== len` for a fixed-arity `(list p0…p{n-1})`,
            // `>= len` for a rest pattern (`.. rest`). The subject at `path` is the payload's `Vec<T>` (read via
            // `emit_sum_payload`); mirror `emit_list_match_impl`'s length test. The leading-element binders
            // (`SumPayload{…,Elem(i)}` → `xs[i]`) and the rest binder (`SumPayload{…,RestFrom(len)}` →
            // `xs[len..].to_vec()`) already resolve through `emit_sum_payload`'s list arms, so only the length
            // TEST needed rendering. This is the sum-decision-tree meeting the list matcher — the compiler-AST
            // shape (a `(Call (List Node))` node dispatched by its child count).
            if let crate::core::Probe::ListLen { len, at_least } = probe {
                let op = if *at_least { ">=" } else { "==" };
                let then_ = emit_sum_cont(db, scrutinee, then_, result_it, env, ctx)?;
                let els = emit_sum_cont(db, scrutinee, els, result_it, env, ctx)?;
                return Ok(format!(
                    "if ({subject}).len() {op} {len} {{ {then_} }} else {{ {els} }}"
                ));
            }
            // The literal to compare against, in the sub-value's own type (`5i64`, `true`) so the Rust
            // comparison types. A string probe never reaches a RUNTIME test (it declines at `is_scalar`
            // before a decision tree is built), matching the scalar-match path. For an INT probe both sides
            // of the `==` MUST share one width: the `subject` (read via `emit_sum_payload`) can come back
            // WIDENED to i64 (an erased-newtype scrutinee built through a narrowing wrap — `(W.V (Int8.wrap
            // n))` with `n: Int64` — reads the value as `(n as i64)`), while the literal is emitted at the
            // narrow logical width (`3i8`) → `i64 == i8` E0308. So key BOTH sides off the SAME `target`
            // width: emit the literal at `target`, and cast the subject to `target` too (`(<subj>) as i8`).
            // The cast is sound — the sub-value is logically that narrow type (the newtype's inner width), so
            // narrowing recovers the true value, matching the wasm decision-tree's width-normalized compare.
            let (lit, subject) = match probe {
                crate::core::Probe::Int(v) => {
                    // The sub-value's integer type gives the literal's suffix; a `Payload`/`Elem` path ends
                    // at an Int leaf. Prefer an arm-recorded path type (the entered-variant type, exact for
                    // a disc-≥1 payload), falling back to a scrutinee-rooted walk. `strip_nominal` so an
                    // erased newtype's inner Int width drives the literal (a `Ty::Nominal { inner: UInt8 }`
                    // read at the now-empty path must compare at `u8`, not default to `i64` → E0308).
                    let sub = lookup_sum_path_type(ctx, path)
                        .unwrap_or_else(|| ty_at_sum_path(db, scrutinee, path));
                    // A BIGINT sub-value probes by BigInt EQUALITY, not an integer cast: the payload is a
                    // `cdz_num::Big` (arbitrary-precision heap), so `(<Big>) as i64` is a non-primitive cast
                    // (rustc E0605 — corpus-bugfix's runtime-BigInt-literal-probe build-fail, the rust twin of
                    // FINDING #22). Compare against the materialized `Big` literal (`const_big_expr`, in-i64 →
                    // `Big::from_i64`, beyond → sign-magnitude bytes); `Big` derives `PartialEq`, so `==` types.
                    if matches!(sub.strip_nominal(), Ty::BigInt) {
                        let then_ = emit_sum_cont(db, scrutinee, then_, result_it, env, ctx)?;
                        let els = emit_sum_cont(db, scrutinee, els, result_it, env, ctx)?;
                        return Ok(format!(
                            "if ({subject}) == {} {{ {then_} }} else {{ {els} }}",
                            const_big_expr(v)
                        ));
                    }
                    let it = match sub.strip_nominal() {
                        Ty::Int(it) => *it,
                        _ => IntTy {
                            sign: Sign::Fixed(true),
                            width: Width::Fixed(crate::ty::DEFAULT_INT_WIDTH),
                        },
                    };
                    let target =
                        types::rust_type(&db.name_ctx(), &Ty::Int(it)).ok_or_else(|| {
                            Reject::decline(
                                "a literal-payload width has no native Rust representation",
                            )
                        })?;
                    // Cast the subject to the SAME width as the literal so both sides of `==` agree (fixes
                    // the widened-subject E0308); a subject already at `target` casts to itself (a no-op the
                    // compiler folds).
                    (
                        format!("{}{target}", int_value_signed_decimal(v)),
                        format!("(({subject}) as {target})"),
                    )
                }
                crate::core::Probe::Bool(b) => {
                    ((if *b { "true" } else { "false" }).to_string(), subject)
                }
                crate::core::Probe::Char(c) => {
                    // A CHAR sub-value is a SCALAR (`Ty::Char` -> native rust `char`, the same as a
                    // top-level char scrutinee), so a nested char literal-payload probe renders as a
                    // value-equality compare `(<subject>) == '<c>'` — the identical `rust_char_literal`
                    // escaping the scalar-match arm and `Core::ConstChar` emit use, matching the wasm
                    // decision-tree's scalar compare. Only Bytes/ListLen/MapHasKeys/Wild remain non-scalar.
                    (rust_char_literal(*c), subject)
                }
                crate::core::Probe::Str(s) => {
                    // A STRING/SYMBOL sub-value probes by CONTENT equality. A Symbol payload inside a sum
                    // variant (`(type W (Mk Symbol))`, `(match w ((Mk #"go") …))`) reaches the decision tree
                    // here (unlike a top-level String scrutinee, which declines earlier at `is_scalar`): the
                    // `subject` read via `emit_sum_payload` is a Rust `String` (both `Ty::Symbol` and
                    // `Ty::String` map to `String`, and a Symbol IS its text at run time), so a content probe
                    // is `<subject>.as_str() == "<lit>"` — `.as_str()` auto-derefs a `String`/`&String`, and
                    // `str == str` is the byte-content compare the wasm decision-tree gives. Only String/Symbol
                    // render; any other sub-value at a `Str` probe (defensive — the lowering only emits `Str`
                    // for a String/Symbol leaf) declines as before.
                    let sub = lookup_sum_path_type(ctx, path)
                        .unwrap_or_else(|| ty_at_sum_path(db, scrutinee, path));
                    if matches!(sub.strip_nominal(), Ty::Symbol | Ty::String) {
                        let then_ = emit_sum_cont(db, scrutinee, then_, result_it, env, ctx)?;
                        let els = emit_sum_cont(db, scrutinee, els, result_it, env, ctx)?;
                        return Ok(format!(
                            "if ({subject}).as_str() == {} {{ {then_} }} else {{ {els} }}",
                            rust_string_literal(s)
                        ));
                    }
                    return Err(Reject::decline(
                        "a non-scalar literal-payload probe is not rendered by the Rust backend",
                    ));
                }
                // A nested byte-string-literal payload probe (`(Some b"AB")` over a runtime `Some`) is
                // rendered on wasm (the `value-eq` byte-leaf compare) but not yet by the Rust backend —
                // decline cleanly. (Top-level runtime Bytes dispatch desugars to a `value-eq` if-chain in
                // `lower` and works on BOTH backends; only this nested-sum payload probe is Rust-deferred.)
                crate::core::Probe::Bytes(_)
                | crate::core::Probe::ListLen { .. }
                | crate::core::Probe::MapHasKeys { .. }
                | crate::core::Probe::Wild => {
                    return Err(Reject::decline(
                        "a non-scalar literal-payload probe is not rendered by the Rust backend",
                    ));
                }
            };
            let then_ = emit_sum_cont(db, scrutinee, then_, result_it, env, ctx)?;
            let els = emit_sum_cont(db, scrutinee, els, result_it, env, ctx)?;
            Ok(format!(
                "if ({subject}) == {lit} {{ {then_} }} else {{ {els} }}"
            ))
        }
    }
}

/// The TYPE of the sub-value reached by walking `sw_path` from `scrutinee`'s type — a `Payload` step
/// descends a variant's payload (the sum's disc-0 single payload at its instantiation, or a nominal's
/// inner), an `Elem(i)` a tuple element. Returns `Ty::Any` on an unwalkable path (the caller then reads
/// arity 0 / declines). Enough for the nested-sum-switch subject: a nested switch's `path` ends at a sum
/// sub-value, so the walk reaches a `Ty::Sum` whose declaration names the variant.
/// Drop `Payload` steps that land on an ERASED NOMINAL newtype from a switch/bind path — the Rust-backend
/// twin of `lower::erase_nominal_steps` (which does this for the BODY's `SumPayload` read paths). A newtype
/// tag erases at runtime (the value IS the inner), so its `Payload` step is a no-op; keeping it in a switch
/// path would put the switch one level too shallow and mint bind paths one step deeper than the erased body
/// reads. Walking the scrutinee's TYPE, a `Payload` over a `Ty::Nominal` advances to its inner and is
/// dropped; a `Payload` over a real sum is KEPT (and the type advances through the sum's disc-0 payload,
/// enough to detect a nominal deeper in the path); an `Elem(i)` is kept (advancing through a tuple/list).
/// A boxed non-newtype path has no nominal `Payload` step, so it is returned unchanged (no regression).
fn erase_nominal_switch_path(
    db: &mut Db,
    scrutinee: StructId,
    sw_path: &[crate::core::PathStep],
) -> Vec<crate::core::PathStep> {
    let mut cur = type_of(db, scrutinee);
    let mut out = Vec::with_capacity(sw_path.len());
    for step in sw_path {
        match step {
            crate::core::PathStep::Payload => match &cur {
                // A nominal newtype's payload step erases — advance to the inner, drop the step.
                Ty::Nominal { inner, .. } => cur = (**inner).clone(),
                // A real (boxed-sum) payload step — keep it, advance through the sum's payload shape.
                _ => {
                    out.push(*step);
                    cur = sum_disc0_payload_ty(db, &cur).unwrap_or(Ty::Any);
                }
            },
            crate::core::PathStep::Elem(i) => {
                out.push(*step);
                cur = match cur.strip_nominal() {
                    Ty::Tuple(elems) => elems.get(*i).cloned().unwrap_or(Ty::Any),
                    Ty::List(elem) => (**elem).clone(),
                    // A RECORD sub-value descends by SORTED-SLOT `Elem(i)` (a record erases to a tuple in
                    // sorted-field order, so field-slot `i` = `fields.values().nth(i)` — the same index
                    // space `Core::Record`/`Core::Proj` use). DORMANT until `resolve` emits an `Elem` under
                    // a record head (nested-record match binders, v-inference); today no such `Elem` is
                    // produced, so this arm cannot fire on current trunk — it fills the `Ty::Any` gap ahead
                    // of that feature so a narrow-width nested-record field resolves its real type (not the
                    // default the `_ => Ty::Any` fallback would give). Mirrors the `Ty::Tuple` arm.
                    Ty::Record(fields) => fields.values().nth(*i).cloned().unwrap_or(Ty::Any),
                    _ => Ty::Any,
                };
            }
            // A list-rest step is not a nominal `Payload` (never erased) — keep it, type stays the list.
            crate::core::PathStep::RestFrom(_) => {
                out.push(*step);
                cur = match cur.strip_nominal() {
                    Ty::List(_) => cur.strip_nominal().clone(),
                    _ => Ty::Any,
                };
            }
            // A tuple-rest step never appears in a sum-SWITCH path (a rest binder's own path does not go
            // through the switch); keep it, advance to the trailing sub-tuple for a well-defined cursor.
            crate::core::PathStep::TupleRestFrom(k) => {
                out.push(*step);
                cur = match cur.strip_nominal() {
                    Ty::Tuple(elems) => Ty::Tuple(elems.get(*k..).unwrap_or(&[]).to_vec().into()),
                    _ => Ty::Any,
                };
            }
        }
    }
    out
}

/// Resolve the solved type of the sub-value at `path` from the arm-recorded `sum_path_types` hints —
/// the entered-variant type an enclosing switch recorded when it descended. Longest-prefix match: find the
/// deepest recorded path that is a prefix of `path`, then walk the remaining `Elem`/nominal-`Payload`
/// steps from its type (a tuple-payload destructure). `None` if no recorded path is a prefix (the root, or
/// a genuinely un-hinted path) — the caller then falls back to a scrutinee-rooted type walk.
fn lookup_sum_path_type(ctx: &Ctx, path: &[crate::core::PathStep]) -> Option<Ty> {
    let (best_path, best_ty) = ctx
        .sum_path_types
        .iter()
        .filter(|(p, _)| path.starts_with(p))
        .max_by_key(|(p, _)| p.len())?;
    let rest = &path[best_path.len()..];
    let mut ty = best_ty.clone();
    for step in rest {
        ty = match step {
            crate::core::PathStep::Elem(i) => match ty.strip_nominal() {
                Ty::Tuple(elems) => elems.get(*i).cloned()?,
                Ty::List(elem) => (**elem).clone(),
                // Record sub-value at sorted-slot `i` (DORMANT — see the `map_switch_path_to_payload_path`
                // Elem arm; no `Elem`-under-record is emitted on current trunk, so this can't fire until
                // v-inference's nested-record-binder resolve change).
                Ty::Record(fields) => fields.values().nth(*i).cloned()?,
                _ => return None,
            },
            // A nominal-newtype Payload peels a layer (a no-op); a sum Payload beyond a recorded hint only
            // arises through a nested switch, which records its OWN hint — so here it is not resolvable.
            crate::core::PathStep::Payload => match &ty {
                Ty::Nominal { inner, .. } => (**inner).clone(),
                _ => return None,
            },
            crate::core::PathStep::RestFrom(_) => return None,
            crate::core::PathStep::TupleRestFrom(_) => return None,
        };
    }
    Some(ty)
}

/// The payload arity of variant `disc` of the sum TYPE `ty` — the type-keyed twin of
/// [`variant_payload_arity_at`], reading the arity off the (possibly hint-supplied) subject type rather
/// than re-walking from the scrutinee. `strip_nominal` first so an erased-newtype-wrapped sum reads the
/// inner sum's variant arity.
pub(super) fn variant_arity_of_ty(db: &mut Db, ty: &Ty, disc: u32) -> usize {
    let decl_occ = match ty.strip_nominal() {
        Ty::Sum { decl, .. } => *decl,
        _ => return 0,
    };
    match db.type_decl_by_occ(decl_occ) {
        Some(decl) => decl
            .variants
            .get(disc as usize)
            .map(|v| v.payloads.len())
            .unwrap_or(0),
        None => 0,
    }
}

fn ty_at_sum_path(db: &mut Db, scrutinee: StructId, sw_path: &[crate::core::PathStep]) -> Ty {
    let mut ty = type_of(db, scrutinee);
    // A parallel CONSTANT-VALUE cursor: the `Core` node the sub-value currently is, when the scrutinee is
    // a compile-time-known value (a folded `SumNew`/`Tuple`). A `Payload` step over a `Ty::Sum` must
    // descend the ENTERED variant's payload — but the flattened path does not carry which discriminant the
    // enclosing arm selected. When the value is a constant `SumNew { disc }`, its `disc` IS the entered
    // variant, so read THAT variant's payload type (not variant 0's). This is what lets a nested match on a
    // variant at disc ≥ 1 (`(type W (A Int64) (V (Option Int64)))` matched `(W.V (Some n))`, folded to a
    // known `W.V`) resolve its inner switch's subject to `Option Int64` (V's payload), not `Int64` (A's).
    // A non-constant scrutinee falls back to variant 0 — a fully-runtime nested match on a disc-≥1 variant
    // is not reachable here (a sum-typed value can't cross the export boundary, so `f` folds or declines).
    let mut val: Option<Core> = Some(crate::lower::core_of(db, scrutinee));
    for step in sw_path {
        // The disc of the current constant value, if it is a `SumNew` — the entered variant at a `Payload`.
        let cur_disc = match &val {
            Some(Core::SumNew { disc, .. }) => Some(*disc),
            _ => None,
        };
        ty = match step {
            crate::core::PathStep::Payload => match ty.strip_nominal() {
                Ty::Sum { .. } => {
                    let disc = cur_disc.unwrap_or(0);
                    match variant_payload_ty(db, &ty, disc) {
                        Some(t) => t,
                        None => return Ty::Any,
                    }
                }
                Ty::Nominal { inner, .. } => (**inner).clone(),
                _ => return Ty::Any,
            },
            crate::core::PathStep::Elem(i) => match ty.strip_nominal() {
                Ty::Tuple(elems) => match elems.get(*i) {
                    Some(t) => t.clone(),
                    None => return Ty::Any,
                },
                Ty::List(elem) => (**elem).clone(),
                // Record sub-value at sorted-slot `i` (DORMANT — see the `map_switch_path_to_payload_path`
                // Elem arm; no `Elem`-under-record is emitted on current trunk, so this can't fire until
                // v-inference's nested-record-binder resolve change).
                Ty::Record(fields) => match fields.values().nth(*i) {
                    Some(t) => t.clone(),
                    None => return Ty::Any,
                },
                _ => return Ty::Any,
            },
            // A rest sublist keeps the list type (the Rust backend declines a runtime list match; total here).
            crate::core::PathStep::RestFrom(_) => match ty.strip_nominal() {
                Ty::List(_) => ty.clone(),
                _ => return Ty::Any,
            },
            // A tuple rest binder — the trailing sub-tuple `(Tuple T_k …)`.
            crate::core::PathStep::TupleRestFrom(k) => match ty.strip_nominal() {
                Ty::Tuple(elems) => Ty::Tuple(elems.get(*k..).unwrap_or(&[]).to_vec().into()),
                _ => return Ty::Any,
            },
        };
        // Advance the value cursor alongside the type: a `Payload` enters a `SumNew`'s sole payload (a
        // multi-payload variant's payloads become the following `Elem`s), an `Elem` a `Tuple`/`SumNew`
        // element. Anything else drops the cursor to `None` (fall back to variant 0 / structural type).
        val = match (step, val.take()) {
            (crate::core::PathStep::Payload, Some(Core::SumNew { payloads, .. }))
                if payloads.len() == 1 =>
            {
                Some(crate::lower::core_of(db, payloads[0]))
            }
            (crate::core::PathStep::Elem(i), Some(Core::SumNew { payloads, .. })) => {
                payloads.get(*i).map(|&p| crate::lower::core_of(db, p))
            }
            (crate::core::PathStep::Elem(i), Some(Core::Tuple { elems })) => {
                elems.get(*i).map(|&e| crate::lower::core_of(db, e))
            }
            _ => None,
        };
    }
    ty
}

/// The payload ARITY of the `disc`-th variant of the sum the value at `id` has — how many payload types
/// the variant declares (0 = nullary). Read from the declaration's variant. A single-payload variant is
/// 1 (its payload may itself be a tuple); a multi-payload variant is its payload count. Used to decide
/// whether a match arm's pattern binds a payload (`(p)`) or not.
fn variant_payload_arity(db: &mut Db, id: StructId, disc: u32) -> usize {
    let decl_occ = match type_of(db, id) {
        Ty::Sum { decl, .. } => decl,
        _ => return 0,
    };
    match db.type_decl_by_occ(decl_occ) {
        Some(decl) => decl
            .variants
            .get(disc as usize)
            .map(|v| v.payloads.len())
            .unwrap_or(0),
        None => 0,
    }
}

/// Emit a `Core::SumPayload { scrutinee, path }` → the Rust identifier the enclosing sum-match arm bound
/// the payload to (looked up in `ctx.sum_binds` by `(scrutinee, path)`). A payload deeper than the
/// arm's direct payload — a `PathStep::Elem(i)` after the `Payload` (a tuple-payload destructure) —
/// reads a tuple field off that binding (`(<bound>).i`). Declines if no binding is in scope (a sum
/// pattern shape this slice does not yet render — e.g. a nested switch's payload).
/// Walk `path` through the CONSTANT value tree rooted at `root`, returning the single `Core` node it
/// selects — or `None` if the path lands between nodes (a multi-payload `Payload`, or a step over a
/// non-constant node). A `Payload` over a single-payload `SumNew` enters its sole payload (`(W.V x)` →
/// `x` — the disc-fold-flattened nested-match subject read, several `Payload`s deep); an `Elem(i)` indexes
/// a `SumNew`'s / `Tuple`'s / `ListNew`'s element. The value-tree twin of `lower::fold_sum_path`, but
/// returning the NODE (for `emit`) rather than its folded `Core`.
fn fold_const_sum_path(
    db: &mut Db,
    root: StructId,
    path: &[crate::core::PathStep],
) -> Option<StructId> {
    let mut cur = root;
    let mut i = 0;
    while i < path.len() {
        let step = &path[i];
        match (step, crate::lower::core_of(db, cur)) {
            (crate::core::PathStep::Payload, Core::SumNew { payloads, .. })
                if payloads.len() == 1 =>
            {
                cur = payloads[0];
                i += 1;
            }
            // A MULTI-payload variant's payload IS the tuple of its payloads (no single node). A following
            // `Elem(j)` indexes payload `j` DIRECTLY — consume BOTH steps. A bare `Payload` ending here has
            // no single node (`None` — the caller renders the payload tuple).
            (crate::core::PathStep::Payload, Core::SumNew { payloads, .. }) => {
                match path.get(i + 1) {
                    Some(crate::core::PathStep::Elem(j)) => {
                        cur = *payloads.get(*j)?;
                        i += 2;
                    }
                    _ => return None,
                }
            }
            (crate::core::PathStep::Elem(j), Core::Tuple { elems }) => {
                cur = *elems.get(*j)?;
                i += 1;
            }
            (crate::core::PathStep::Elem(j), Core::ListNew { elems }) => {
                cur = *elems.get(*j)?;
                i += 1;
            }
            // A `RestFrom`, or a step over a non-constant node.
            _ => return None,
        }
    }
    Some(cur)
}

/// Whether a match SCRUTINEE is expensive/unsafe to RE-EMIT per payload read, so it must be materialized
/// into a `let` once (see `Ctx::scrut_locals`). A pure LOCAL/param read (`Core::LocalRef`) — or a CONSTANT
/// `SumNew` (which `emit_sum_payload` folds against its payload nodes, no re-emit) — is trivial: repeating
/// it is a cheap, side-effect-free re-read, so no binding is needed (keeps the emitted code + the many
/// passing simple matches unchanged). Anything else (a `Core::Call` — the exponential case — an arithmetic
/// expression, another match, a construction) is materialized: re-emitting it K times is wasted work at
/// best and a `2^depth` blow-up (a recursive-call scrutinee) at worst.
fn scrutinee_needs_materialize(db: &mut Db, scrutinee: StructId) -> bool {
    !matches!(
        crate::lower::core_of(db, scrutinee),
        Core::LocalRef { .. } | Core::SumNew { .. }
    )
}

/// Emit the SCRUTINEE expression — reading its pre-bound `let` local (`Ctx::scrut_locals`) if the enclosing
/// `Core::MatchSum` materialized it once, else emitting it directly. Every `emit_sum_payload` scrutinee read
/// routes through here so a materialized scrutinee is read from its ONE local instead of re-emitted (the
/// exponential-blow-up fix).
fn emit_scrutinee(
    db: &mut Db,
    scrutinee: StructId,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    if let Some((_, local)) = ctx.scrut_locals.iter().find(|(s, _)| *s == scrutinee) {
        return Ok(local.clone());
    }
    emit(db, scrutinee, env, ctx)
}

fn emit_sum_payload(
    db: &mut Db,
    id: StructId,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    // EMPTY PATH: the sub-value at the empty path IS the scrutinee value itself. This arises for an
    // ERASED single-variant newtype scrutinee (`(match (W.Wrap n) ((W.Wrap 0) 100) …)` where
    // `W = (Wrap UInt8)`): the newtype tag erases (`(W.Wrap n)` → `n`), so the switch collapses in
    // `lower` to a `LitTest` whose probe path `[Payload]` erases to `[]` (see the LitTest arm) — the
    // literal-0 compare and the binding arm both read the inner value directly off the scrutinee. The
    // scrutinee is pure (a param/local read), so emitting it is sound; clone a non-Copy read used more
    // than once (the same clone-on-read discipline the bind path below uses).
    if path.is_empty() {
        let expr = emit_scrutinee(db, scrutinee, env, ctx)?;
        if needs_clone_on_read(db, id) {
            return Ok(format!("({expr}).clone()"));
        }
        return Ok(expr);
    }
    // CONSTANT-SCRUTINEE FOLD: the scrutinee is a compile-time `SumNew { disc, payloads }` (a constant
    // sum value, e.g. `(match (V.P 3 4) ((V.P a b) …))` where the front-end did NOT fold the match to the
    // arm body). Then the arm's payload binders read directly off the constant's payload NODES, no runtime
    // re-match: a `[Payload]` reads the sole payload (a single-payload variant) or the whole tuple (a
    // multi-payload variant — its payloads ARE the tuple), and a trailing `Elem(i)` reads payload `i`. This
    // is what lets a CONSTANT multi-payload match `(V.P a b)` render on Rust (the runtime-scrutinee case
    // already works via `emit_sum_match`'s binding, and a single-payload constant already folds in `lower`
    // to a bare value; only a constant MULTI-payload match reached here unresolved — the wasm backend emits
    // runtime `sum-payload`/`arr-get` reads of the constant, the Rust one folds to the payload node).
    if matches!(crate::lower::core_of(db, scrutinee), Core::SumNew { .. }) {
        // Walk the path through the CONSTANT value tree, descending each step into the node it selects. A
        // `Payload` over a single-payload `SumNew` enters its sole payload (a `(W.V …)` → the inner value —
        // this is what lets a disc-fold-FLATTENED nested match, whose single switch sits at a deep path like
        // `[Payload, Payload]` with NO enclosing binds, read its subject directly off the constant); a
        // `Payload` over a MULTI-payload variant yields the tuple of all payloads; an `Elem(i)` indexes a
        // tuple/multi-payload node. If the whole path resolves to a single node, emit it; a path that lands
        // "between" nodes (a multi-payload `Payload` not at the end) falls through to the bind lookup.
        if let Some(node) = fold_const_sum_path(db, scrutinee, path) {
            return emit(db, node, env, ctx);
        }
        // A `[…, Payload]` ending on a MULTI-payload variant is the tuple of its payloads (no single node).
        if let Some((last, prefix)) = path.split_last()
            && matches!(last, crate::core::PathStep::Payload)
            && let Some(parent) = fold_const_sum_path(db, scrutinee, prefix)
            && let Core::SumNew { payloads, .. } = crate::lower::core_of(db, parent)
            && payloads.len() != 1
        {
            let mut parts = Vec::with_capacity(payloads.len());
            for &p in payloads.iter() {
                parts.push(emit(db, p, env, ctx)?);
            }
            return Ok(format!("({})", parts.join(", ")));
        }
    }
    // The binding covers the arm's direct payload (path prefix `[Payload]`); any trailing `Elem(i)` steps
    // index into it (a tuple payload). Find a bind whose path is a prefix of `path`.
    for b in ctx.sum_binds.iter().rev() {
        if b.scrutinee == scrutinee && path.starts_with(&b.path) {
            let rest = &path[b.path.len()..];
            // A BOXED bind (a recursive variant's `Box<…>` field) is DEREFERENCED to reach the payload —
            // `(*name)` — the twin of the construct site's `Box::new`. An `Elem(i)` then indexes the
            // deref'd tuple (`(*name).i`); the whole payload is `(*name)`. A non-boxed bind reads `name`
            // directly (Rust auto-derefs a `Box` for a field access, but the explicit `*` is uniform and
            // correct whether the following step is a field index or the value itself).
            let mut expr = if b.boxed {
                format!("(*{})", b.name)
            } else {
                b.name.clone()
            };
            // Walk the bind's payload TYPE alongside `rest` so an `Elem(i)` renders correctly per container:
            // a TUPLE element is a field access `.i`, but a LIST element is an INDEX `[i]` (a `(Some (list x
            // .. r))` binder reads element 0 of the payload `Vec`, not a `.0` tuple field → E0609 otherwise).
            // The bind's path type is the entered-variant's payload (`ty_at_sum_path` at `b.path`, or a
            // recorded hint); `None`/unknown falls back to the tuple `.i` form (the pre-existing behavior).
            let mut cur_ty = lookup_sum_path_type(ctx, &b.path)
                .unwrap_or_else(|| ty_at_sum_path(db, scrutinee, &b.path));
            for step in rest {
                match step {
                    crate::core::PathStep::Elem(i) => {
                        match cur_ty.strip_nominal() {
                            Ty::List(elem) => {
                                cur_ty = (**elem).clone();
                                expr = format!("({expr})[{i}]");
                            }
                            Ty::Tuple(elems) => {
                                cur_ty = elems.get(*i).cloned().unwrap_or(Ty::Any);
                                expr = format!("({expr}).{i}");
                            }
                            // Unknown/other — keep the historical tuple-field form.
                            _ => {
                                cur_ty = Ty::Any;
                                expr = format!("({expr}).{i}");
                            }
                        }
                    }
                    crate::core::PathStep::Payload => {
                        return Err(Reject::unsupported(
                            "a nested sum payload is not supported by the Rust backend",
                        ));
                    }
                    crate::core::PathStep::TupleRestFrom(_) => {
                        // A tuple REST binder's runtime read (a trailing sub-tuple gather) is not yet
                        // lowered by the Rust backend — decline (slice 1: const folds; wasm/rust runtime
                        // are follow-up slices). A graceful not-yet, never a miscompile.
                        return Err(Reject::unsupported(
                            "a runtime tuple rest binder is not supported by the Rust backend (a constant tuple-rest is)",
                        ));
                    }
                    crate::core::PathStep::RestFrom(k) => {
                        // A list REST binder — the tail sublist from index `k` (`xs[k..].to_vec()`), an owned
                        // independent `Vec`. Only valid over a list-typed payload; other shapes decline.
                        match cur_ty.strip_nominal() {
                            Ty::List(_) => {
                                return Ok(format!("({expr})[{k}..].to_vec()"));
                            }
                            _ => {
                                return Err(Reject::decline(
                                    "a list rest binder over a non-list payload is not rendered by the Rust backend",
                                ));
                            }
                        }
                    }
                }
            }
            // A read of a BOXED payload field MOVES it out of the `Box` — `(*name).i` extracts a non-`Copy`
            // field by value, so a field used more than once (a `let`-bound tail read in both an `if`
            // condition and a branch; two accessed fields) is a use-after-move → rustc E0382. The wasm
            // backend re-reads the heap slot each time with no move discipline, so it never sees this. CLONE
            // the projection so each read is an owned copy that leaves the box intact (the emitted enums all
            // `#[derive(Clone)]`, so the field type — a scalar, a nested recursive enum, a tuple of these —
            // is `Clone`). A `Copy` scalar field's `.clone()` is a plain copy; a recursive field's is a deep
            // copy — both avoid the move. Only a BOXED bind needs this: a non-boxed bind reads a `Copy`
            // scalar / a value already bound by the match pattern, which does not move out of a box.
            // A BOXED bind ALWAYS clones (a `(*name).i` extraction moves out of the box). A non-boxed bind
            // clones only when the READ value's type is NON-COPY (a `Vec` payload field, or a tuple field
            // that is a list): reading such a field by value moves it, so a payload used in more than one
            // position (a list field passed to a call AND measured with `.len()`) would E0382. A Copy field
            // (the common scalar case) reads in place with no clone — byte-identical to before.
            if b.boxed || needs_clone_on_read(db, id) {
                expr = format!("({expr}).clone()");
            }
            return Ok(expr);
        }
    }
    // A TOP-LEVEL TUPLE-PATTERN read off a RUNTIME tuple scrutinee — `(match (if … (tuple …) (tuple …))
    // ((tuple a b) …))` — where the scrutinee is neither a constant `Core::Tuple` (folded above) nor a
    // bound `__pay` (a top-level tuple match mints no `Switch` arm, so no bind): the binders `a`/`b` read
    // `[Elem(0)]`/`[Elem(1)]` DIRECTLY off the scrutinee. Emit the scrutinee value and index it (`(<t>).i`)
    // — the runtime-tuple twin of the constant fold. Gate on the path being pure `Elem` steps over a tuple-
    // typed scrutinee (a `Payload`/`RestFrom` here is a different shape). Without this, a tuple built by a
    // runtime `if` (or returned from a branchy fn) and matched declined "no bound match arm" (wasm reads it
    // via `arr-get`, which needs no bind).
    // Also covers a RECORD scrutinee: a record is emitted as a Rust tuple in SORTED field-name order
    // (`types::rust_type` / `Core::Record`), so a record-field probe path `[Elem(sorted_slot)]` reads the
    // j-th tuple position `.{slot}` — identical to a tuple element. This is what lets a record-match LITERAL
    // FIELD probe `((record (x 3) (y b)) …)` render on Rust: `pattern_constraints` emits a `lit_test` at
    // `[Elem(slot)]` over the record scrutinee, whose subject read reaches here with no bind → before, it
    // fell through to "sum payload has no bound match arm" (the record LitTest declined on Rust though it
    // computed on wasm). The direct field read is the record twin of the runtime-tuple direct read.
    if path
        .iter()
        .all(|s| matches!(s, crate::core::PathStep::Elem(_)))
        && matches!(
            type_of(db, scrutinee).strip_nominal(),
            Ty::Tuple(_) | Ty::Record(_)
        )
    {
        let mut expr = emit_scrutinee(db, scrutinee, env, ctx)?;
        // Walk the scrutinee TYPE alongside `path` so each `Elem(i)` renders per its CONTAINER: a TUPLE or
        // RECORD element is a field access `.i` (a record erases to a sorted-field tuple), but a LIST element
        // is an INDEX `[i]` — a `Vec` has no `.i` field (rustc E0609). A tuple whose slot is a `(List …)`
        // and whose pattern binds a list element (`(match t ((tuple a (list h .. r)) …))`, `t: (Tuple Int64
        // (List Int64))`) reaches here with a pure-`Elem` path `[Elem(1), Elem(0)]`; the old loop rendered
        // BOTH as `.i` → `((t).1).0` on a `Vec` (no field 0). Mirrors `emit_sum_payload`'s per-container walk.
        let mut cur_ty = type_of(db, scrutinee);
        for step in path {
            if let crate::core::PathStep::Elem(i) = step {
                match cur_ty.strip_nominal() {
                    Ty::List(elem) => {
                        cur_ty = (**elem).clone();
                        expr = format!("({expr})[{i}]");
                    }
                    Ty::Tuple(elems) => {
                        cur_ty = elems.get(*i).cloned().unwrap_or(Ty::Any);
                        expr = format!("({expr}).{i}");
                    }
                    // A record is a Rust tuple in SORTED field-name order; `Elem(i)` is the i-th sorted slot.
                    Ty::Record(fields) => {
                        cur_ty = fields.values().nth(*i).cloned().unwrap_or(Ty::Any);
                        expr = format!("({expr}).{i}");
                    }
                    // Unknown/other — keep the historical tuple-field form.
                    _ => {
                        cur_ty = Ty::Any;
                        expr = format!("({expr}).{i}");
                    }
                }
            }
        }
        // CLONE a non-Copy field read (a `Vec`/`String`/sum/nested-tuple field): reading it by value MOVES
        // it out of the tuple/record, so a binder used in more than one position — e.g. a rebuilt-list field
        // `xs2` that is BOTH a list-match scrutinee AND re-referenced in a catch-all `(Ast.List xs2)` — is a
        // use-after-move (rustc E0382; the mutually-recursive-fold-over-a-rebuilt-list no-build, breaker
        // 2026-07-25). The scrutinee tuple is bound once (`let __msN = …`) and cloning leaves it intact for a
        // sibling field read (`(__msN).1`). A Copy field (the common scalar case) reads in place — no clone,
        // byte-identical to before. Mirrors the boxed / list-element clone-on-read discipline above.
        if needs_clone_on_read(db, id) {
            expr = format!("({expr}).clone()");
        }
        return Ok(expr);
    }
    // A LIST-PATTERN binder off a runtime LIST scrutinee — a `MatchList` arm's leading-element binder
    // (`[Elem(i)]` → `xs[i]`) or rest binder (`[RestFrom(k)]` → the tail sublist `xs[k..].to_vec()`). The
    // scrutinee is pure (a param/local), so re-emitting it per binder is sound; each read is INDEPENDENT
    // (matching the wasm `vec-get`/`vec-split` per binder). A leading element of a non-Copy type is
    // `.clone()`d (a `Vec` element used by value would move out of the borrowed list); the rest
    // `.to_vec()` already produces an owned, independent `Vec`.
    if matches!(type_of(db, scrutinee).strip_nominal(), Ty::List(_)) {
        match path {
            // A leading `RestFrom(k)` — the tail sublist from index `k`, an owned `Vec` slice copy
            // (persistent value semantics; the source list is left intact for a sibling element binder).
            [crate::core::PathStep::RestFrom(k)] => {
                let xs = emit_scrutinee(db, scrutinee, env, ctx)?;
                return Ok(format!("({xs})[{k}..].to_vec()"));
            }
            // A leading `Elem(i)` — the i-th element, then ANY trailing steps index INTO that element (a
            // NESTED element pattern: `(list (tuple a b) .. rest)` binds `a` at `[Elem(0), Elem(0)]` — list
            // index 0, then tuple field 0). Walk the trailing steps against the element TYPE so each renders
            // per its container (a tuple field `.j`, a nested-list index `[j]`). Before, only a SINGLE
            // `[Elem(i)]` resolved; a nested `[Elem(i), Elem(j)]` fell through to the decline (surfacing when
            // a self-recursive arm reads such a binder). Clone a non-Copy final read (the element/subfield
            // moves out of the borrowed list otherwise).
            [crate::core::PathStep::Elem(i), rest @ ..] => {
                let xs = emit_scrutinee(db, scrutinee, env, ctx)?;
                let mut expr = format!("({xs})[{i}]");
                let mut cur_ty = match type_of(db, scrutinee).strip_nominal() {
                    Ty::List(elem) => (**elem).clone(),
                    _ => Ty::Any,
                };
                for step in rest {
                    match step {
                        crate::core::PathStep::Elem(j) => match cur_ty.strip_nominal() {
                            Ty::Tuple(elems) => {
                                cur_ty = elems.get(*j).cloned().unwrap_or(Ty::Any);
                                expr = format!("({expr}).{j}");
                            }
                            // A RECORD maps to a Rust tuple in SORTED FIELD-NAME order (see `types::rust_type`
                            // / `Core::Record`), so an `Elem(j)` is the j-th sorted field → `.{j}`, exactly
                            // like a tuple. Advance the type through the sorted field values (`BTreeMap`
                            // iterates sorted) so a deeper step resolves.
                            Ty::Record(fields) => {
                                cur_ty = fields.values().nth(*j).cloned().unwrap_or(Ty::Any);
                                expr = format!("({expr}).{j}");
                            }
                            Ty::List(elem) => {
                                cur_ty = (**elem).clone();
                                expr = format!("({expr})[{j}]");
                            }
                            // Any OTHER element shape is not a positional projectable (`Elem` is only valid
                            // over a tuple/record/list) — emitting `.{j}` would be an uncompilable field
                            // access on, say, a scalar/sum/map. DECLINE with a clear message (Copilot PR#522
                            // — the old catch-all `.{j}` risked invalid Rust and dropped type-tracking to Any).
                            _ => {
                                return Err(Reject::decline(
                                    "a nested list-element `Elem` step over a non-tuple/record/list type is not rendered by the Rust backend",
                                ));
                            }
                        },
                        // A `Payload` step over a SUM element — a sum-constructor list-element binder
                        // (`(list (A.I x) .. rest)`, path `[Elem(i), Payload]`): the leading `Elem(i)` read
                        // `(xs)[i]` (a value of the element sum `A`), and this `Payload` binds the matched
                        // variant's payload. The desugar's discriminant guard already established WHICH
                        // variant `(xs)[i]` is (the arm only runs when it matched `A::I`), but that disc is
                        // not on this path. RECOVER it from the SumPayload node's OWN solved type: the binder
                        // is typed at its variant's payload type, so the variant is the one whose payload type
                        // equals `type_of(id)` — UNIQUE for the common heterogeneous sum (`A (I Int64) (N
                        // String)`: I≠N). Emit `match (xs)[i] { A::I(__p) => __p, _ => unreachable!() }` — a
                        // SINGLE-variant match (no or-pattern, so no heterogeneous-payload E0308) with a
                        // defensive `_` (the guard proved this variant, so `_` is dead). AMBIGUOUS (two
                        // variants share the exact payload type) or NO match → decline (can't pick soundly
                        // without the guard's disc — a genuine lower.rs-threading case, still deferred).
                        crate::core::PathStep::Payload => {
                            // A `Payload` step over a SUM element. The sub-value's variant is not on this path
                            // (the desugar's disc guard is elsewhere); RECOVER it from the UNIQUE variant whose
                            // single payload type agrees with the sub-value's SOLVED type at this point in the
                            // walk. Terminal (`rest == [Payload]`) binds the payload directly; a DEEPER walk
                            // (`[Payload, Elem(j), …]` — a list element that is a sum whose payload is a
                            // TUPLE/record, `(list (Pt (tuple a b)))`) extracts the payload then projects into
                            // it. Compute the payload sub-type this Payload lands at so the loop continues.
                            let sum_ty = cur_ty.strip_nominal().clone();
                            // The solved type AT THE PAYLOAD: terminal → the binder's own `type_of(id)`; deeper
                            // → recover from the sum's variant payload (the trailing steps project into it).
                            let n = match &sum_ty {
                                Ty::Sum { decl, .. } => db
                                    .type_decl_by_occ(*decl)
                                    .map(|d| d.variants.len())
                                    .unwrap_or(0),
                                _ => 0,
                            };
                            // Find the UNIQUE variant. When TERMINAL, match against the binder's type; when
                            // DEEPER, we can't use `type_of(id)` (that's the final leaf, past the tuple), so a
                            // sum with a SINGLE non-nullary variant is unambiguous, else decline (needs the
                            // guard disc threaded — the same deferred lower.rs case as the ambiguous terminal).
                            let terminal = rest.len() == 1;
                            let target = if terminal {
                                Some(type_of(db, id))
                            } else {
                                None
                            };
                            // Collect EVERY variant whose payload matches (terminal → agrees with the binder's
                            // type; deeper → any payload-bearing variant). One match = unambiguous. SEVERAL
                            // (a HOMOGENEOUS multi-variant sum — `(Op (Add Int64) (Mul Int64))` matched by
                            // `(Op.Add n)`, where Add and Mul share Int64) is the case the arm's disc-test
                            // GUARD already resolved: the body only runs for the guard-proven variant. Since
                            // every candidate shares the payload type (all agree with `target`), we can bind
                            // the payload with an OR-PATTERN over all of them — `match x { Add(__pv) | Mul(__pv)
                            // => __pv, _ => panic!() }` — which is type-correct (one shared binder type) and
                            // value-correct (only the guarded variant reaches here; the others are dead arms).
                            // This is the disc-threading the decline deferred, realized WITHOUT the disc: the
                            // guard supplies it at run time and the shared payload type makes the or-pattern
                            // sound. (A DEEPER walk keeps the single-candidate requirement — its candidates were
                            // only checked to HAVE a payload, not to share a type, so an or-pattern could
                            // mis-bind; that stays deferred.)
                            let mut candidates: Vec<u32> = Vec::new();
                            for d in 0..n as u32 {
                                let Some(pt) = variant_payload_ty(db, &sum_ty, d) else {
                                    continue;
                                };
                                let matches = match &target {
                                    Some(t) => pt.agrees_with(t),
                                    // Deeper walk: the variant is the one WITH a payload; unique iff exactly one.
                                    None => true,
                                };
                                if matches {
                                    candidates.push(d);
                                }
                            }
                            // A multi-candidate OR-PATTERN is sound ONLY for the terminal case (payloads proven
                            // type-equal via `target`) and when every candidate boxes its payload the same way
                            // (a `(*__pv)` deref and a bare `__pv` can't share one or-pattern arm). Otherwise a
                            // single candidate is required; zero or an unresolvable multi stays a decline.
                            let same_boxing = candidates
                                .iter()
                                .map(|&d| super::enums::variant_is_recursive(db, &sum_ty, d))
                                .collect::<std::collections::BTreeSet<_>>()
                                .len()
                                <= 1;
                            if candidates.is_empty()
                                || (candidates.len() > 1 && !(terminal && same_boxing))
                            {
                                return Err(Reject::decline(
                                    "a sum-constructor list-element payload whose variant is not uniquely determined needs the guard discriminant threaded (deferred)",
                                ));
                            }
                            let boxed =
                                super::enums::variant_is_recursive(db, &sum_ty, candidates[0]);
                            let bind = if boxed { "(*__pv)" } else { "__pv" };
                            // The or-pattern head: every candidate variant path, binding the shared `__pv`.
                            let mut vpaths = Vec::with_capacity(candidates.len());
                            for &d in &candidates {
                                vpaths.push(format!(
                                    "{}(__pv)",
                                    sum_variant_path_of_ty(db, &sum_ty, d)?
                                ));
                            }
                            let pat = vpaths.join(" | ");
                            // Extract the payload from the matched variant(s). `.clone()` the borrowed list
                            // element into the match so the payload can move out. This becomes the new `expr`;
                            // the loop then projects any trailing `Elem(j)` steps into it (a TUPLE/record payload).
                            expr = format!(
                                "match ({expr}).clone() {{ {pat} => {bind}, _ => panic!(\"unreachable\") }}"
                            );
                            // Advance `cur_ty` to the extracted payload's type so a following `Elem(j)` resolves
                            // (a tuple field `.j`, etc.). Terminal walks stop here (no more steps). All candidates
                            // share the payload type, so candidate[0]'s is authoritative.
                            cur_ty =
                                variant_payload_ty(db, &sum_ty, candidates[0]).unwrap_or(Ty::Any);
                        }
                        // A `RestFrom` beyond the leading list index is a shape this slice does not render.
                        _ => {
                            return Err(Reject::decline(
                                "a nested list-element binder beyond a tuple projection is not rendered by the Rust backend",
                            ));
                        }
                    }
                }
                // Clone a non-Copy final read (a `Vec`/`String`/nested list/tuple element read by value would
                // move out of the borrowed list; a `Copy` scalar reads in place). `id` is this `SumPayload`
                // node, so its solved type drives the decision.
                if needs_clone_on_read(db, id) {
                    return Ok(format!("{expr}.clone()"));
                }
                return Ok(expr);
            }
            _ => {}
        }
    }
    Err(Reject::decline(
        "sum payload has no bound match arm (unsupported pattern shape)",
    ))
}

/// Emit `Option.expect`/`Result.expect` → `match <scrut> { <Enum>::<Present>(p) => p, _ => panic!("…") }`.
/// The present variant is `disc_present` (Some/Ok = 0), which carries exactly one payload (the shape the
/// `expect` field is added for); its binding IS the expression's value. Any other variant panics — a Rust
/// panic is a Cadenza trap, the native mirror of the wasm `unreachable` (core-semantics.md §Requiring The
/// Value Of An Optional Traps On Absence). The scrutinee is pure (param/local/call), so matching it inline
/// evaluates it once, observably as the wasm path's single materialization.
fn emit_sum_expect(
    db: &mut Db,
    scrutinee: StructId,
    disc_present: u32,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    let vpath = sum_variant_path(db, scrutinee, disc_present)?;
    if variant_payload_arity(db, scrutinee, disc_present) != 1 {
        return Err(Reject::decline(
            "expect's present variant does not carry exactly one payload",
        ));
    }
    let scrut = emit(db, scrutinee, env, ctx)?;
    // The payload binds to `__expect` and is the match's value directly (a single-payload present arm).
    // The absent-case panic message is `"unreachable"` (NOT `"expect"`): requiring the value of an absent
    // optional is a trap whose canonical KIND is `unreachable` — the SAME kind the wasm backend produces
    // (its `SumExpect` absent branch is an `unreachable`) and the SAME literal `Core::Trap` emits. The gate
    // classifies a trap by its reason (`trap_kind`); `"expect"` classifies as nothing, so a `(trap
    // "unreachable")` expect-on-absent case graded todo on rust though it correctly halts. Matching the
    // literal makes rust agree with wasm.
    Ok(format!(
        "match {scrut} {{ {vpath}(__expect) => __expect, _ => panic!(\"unreachable\") }}"
    ))
}

/// Emit an `if`/`match` branch producing the construct at `construct_id`'s RESULT type. When that
/// result is an integer, a bare-literal branch is GROUNDED to its width (via [`emit_grounded`]) so a
/// default-Int64 literal branch opposite a narrow branch does not mismatch the block's type; a
/// non-integer result (e.g. Bool branches) emits normally. Mirrors the wasm backend's `emit_branch`.
fn emit_branch(
    db: &mut Db,
    branch: StructId,
    construct_id: StructId,
    env: &Env,
    ctx: &Ctx,
) -> Result<String, Reject> {
    let cty = type_of(db, construct_id);
    if let Ty::Int(it) = cty {
        return emit_grounded(db, branch, it, env, ctx);
    }
    // A FLOAT `if`/scalar-`match` result: a bare-literal branch (`(if b x 0.0)` with `x: Float32`) defaults
    // its `ConstFloat` to Float64, so emitting it as-is renders `f64::from_bits(…)` in an `-> f32` branch →
    // rustc E0308. Ground the branch literal to the construct's float width — the float twin of the `Ty::Int`
    // grounding above. `float_width_of_ty` strips a nominal/Qty wrapper (so a `(Qty Float32 …)`-typed result
    // grounds to f32); `None` (not a float) falls through to the plain emit.
    if let Some(w) = float_width_of_ty(&cty) {
        return emit_grounded_float(db, branch, w, env, ctx);
    }
    // A COLLECTION result (`Map`/`Set`/`List`) whose element/key types are FULLY SOLVED: thread the
    // construct's type as `expected_ty` so an EMPTY-collection branch annotates its `BTreeMap<K,V>` /
    // `BTreeSet` / `Vec` from the solved construct type. Without this, an `(if b <Map String Int64>
    // Map.empty)` emits the empty else-branch as `BTreeMap<i64, i64>` — its own KEY is an open var that
    // `MapNew` grounds to the default `i64` (no `expected_ty` to read) — while the then-branch is
    // `BTreeMap<String, i64>`, so rustc rejects `if`/`else` with incompatible types (E0308; the
    // runtime-map REST-binder read / `Map.empty` if-branch case v-wasmtime-migration hit). The empty
    // `MapNew`/`SetOf` emit already consults `expected_ty` for exactly this — feed it the construct type.
    // Only when the construct type has NO free var (an exact type — a wrong annotation errors LOUD at
    // rustc, never a silent miscompile); a call-arg inside the branch overrides this with its own
    // `expected_ty`, so a nested unrelated empty collection is unaffected. The collection twin of the
    // generic-sum `if`-result annotation in the `Core::If` arm.
    if matches!(cty, Ty::Map(..) | Ty::Set(_) | Ty::List(_)) && !cty.has_free_var() {
        let mut bctx = ctx.clone();
        bctx.expected_ty = Some(cty.clone());
        return emit(db, branch, env, &bctx);
    }
    emit(db, branch, env, ctx)
}

/// The shared integer type of a comparison's two operands — the width/signedness both must be rendered
/// at. A bare literal defaults to `Int64`, so the DEFINITE side (the non-literal operand) supplies the
/// real width: prefer whichever operand has a concrete `Ty::Int`. `None` when neither is an integer (a
/// Bool comparison — no width to reconcile, the operands emit as-is). Mirrors `select.rs`'s
/// `operand_int_ty`, but returns `None` for the non-integer case rather than a Bool-as-i32 stand-in
/// (Rust compares `bool` with `==` directly, needing no width).
fn operand_int_ty(db: &mut Db, lhs: StructId, rhs: StructId) -> Option<IntTy> {
    // Prefer the operand whose type is NOT a bare defaulted literal: a param/computed operand carries
    // the real width, a literal defaults. Both concrete-and-equal is the common case; if one is a
    // literal (deferred width) the other pins the width through unify, so either read gives the same
    // ground width — but reading the non-literal side first is robust to the literal's default.
    let pick = |id: StructId, db: &mut Db| match type_of(db, id) {
        Ty::Int(it) => Some(it),
        _ => None,
    };
    // If lhs is a literal and rhs is definite (or vice versa), take the definite side.
    let lhs_lit = matches!(core_of(db, lhs), Core::ConstInt(_));
    if lhs_lit {
        pick(rhs, db).or_else(|| pick(lhs, db))
    } else {
        pick(lhs, db).or_else(|| pick(rhs, db))
    }
}
