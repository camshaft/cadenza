//! `infer` — the query that fills the type column: for a node's `StructId`, its solved [`Ty`].
//!
//! One concern: type determination. [`type_of`] solves one node's type, reading its resolved form
//! (via [`crate::resolve::resolved_of`], which fills the resolved column on demand) and, for a
//! compound node, its children's types (each a lazy `type_of`). It memoizes into `db.types` and is
//! the ONLY module that fills it (`reference-compiler.md` §Types Are Solved Once And Read
//! Downstream). Asking one node's type solves only the nodes that answer reaches — the query stays
//! demand-driven.
//!
//! A literal's type is a signed integer of DEFERRED width and sign (numeric-literal polymorphism —
//! inference or the backend grounds it), a boolean is `Bool`, an `if` is the join of its branches, and
//! a poison is `Ty::Any` (compatible with everything, so a "no" never cascades into a spurious
//! mismatch). Polymorphism is real Hindley-Milner: an operator's scheme is instantiated with fresh
//! variables and its operands UNIFIED (via [`crate::unify`]) — a generic operation, a recursive def's
//! parameters ([`solve_recursive_params`]), and an annotation constraint all solve through the one
//! unify seam, not a per-node coarse rule.
//!
//! Because a node's type is solved from its structure and its uses, an UNANNOTATED program that has a
//! valid typing is accepted with no author-written types — inference supplies what the structure
//! already determines:
//!
//= spec/capabilities/type-system.md#an-unannotated-program-is-accepted-when-it-has-a-valid-typing
//# An unannotated program that has a valid typing MUST be accepted without requiring the author to write that typing, so that inference relieves the author of restating what the structure already determines.
//!
//! [`type_errors`] is a SEPARATE query — a read over the (demand-filled) type column that reports
//! type-agreement faults (an `if` whose condition is not `Bool`, or whose branches disagree). Keeping
//! the fault check apart from the value fill is what lets filling a type never reject and a later
//! rewrite preserve both the value and its checks (`reference-compiler.md` §A Meaning-Preserving
//! Rewrite Preserves Value And Checks).

use crate::arena::Slot;
use crate::ast::{CompoundCtor, StructId};
use crate::db::Db;
use crate::diag::{Code, Fix, Reject};
use crate::resolve::{resolved_of, resolved_ref};
use crate::resolved::Resolved;
use crate::ty::{NameCtx, Scheme, Ty};
use crate::unify::{Fresh, Subst};
use tracing::trace;

mod application;
use application::*;
mod node;
use node::*;
/// The solved type of the node at `id`, filling the column on demand (memoized). Works backward:
/// reads the resolved form and, for a compound node, its children's types.
pub fn type_of(db: &mut Db, id: StructId) -> Ty {
    if let Slot::Filled(t) = db.types.get(id) {
        trace!(target: "rcdzc::infer", node = id.0, "memo hit");
        return t.clone();
    }
    // Recursive-descent DEPTH GUARD (shared with `collect` and `core_of`). `compute` re-enters
    // `type_of` for sub-expressions, so pathologically deep input would recurse until the native stack
    // overflows and the process ABORTS. Past the limit, type as `Any` (unknown, compatible with
    // everything) — the fault walk's own guard reports the resource-limit decline, and a compiler must
    // never crash on well-formed input. Not memoized (like the provisional `Any` below): a node typed
    // at a shallower depth still gets its real type. See `db::DESCENT_DEPTH_LIMIT`.
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::infer", node = id.0, "type depth limit hit → Any (resource limit)");
        return Ty::Any;
    }
    db.descent_depth += 1;
    let t = compute(db, id);
    db.descent_depth -= 1;
    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(&db.name_ctx()), "solved type");
    // Do NOT memoize a provisional `Any`, OR a type that still CONTAINS A FREE VARIABLE: a node typed
    // here may depend on a recursive-def parameter (or a reference to one) whose CONNECTED solve (A2) has
    // not run yet — caching the stale answer would freeze it even after the solve fills the real type. For
    // a bare `Any` this is obvious; the subtler case is a PARTIALLY-solved type like `(Box ?0)` — a
    // generic sum read off a param whose instantiation the A2 solve pins LATER. A payload read
    // `(match acc ((Box.Full m) …))` computes `m`'s type by walking `acc`'s type down the payload path; if
    // `acc` is still `(Box ?0)` when first demanded, the walk yields `?0` (a `Ty::Var`) — NOT `Any`, so the
    // old guard memoized it, and the later-solved `acc = (Box Int64)` never reached `m` (the value-heap
    // layout then declined "projecting an element of type ?0"). A type with a free var is likewise cheap to
    // recompute, so leave it unmemoized and let the solved type win. Every FULLY-GROUND type is memoized as
    // before (the solve-once discipline holds for real types; `has_free_var` treats a deferred int
    // width/sign as ground — those default, they are not undetermined).
    // Skip caching a provisional NESTED-`Any` type born from a SELF-NESTED-GENERIC-PRODUCER re-entrancy.
    // `(from-list (list (inner) (inner)))` — where each `(inner)` is itself a `from-list` call — types
    // its argument `(list (inner) (inner))` WHILE `from-list`'s own param/scheme solve is on the stack;
    // the re-entry guard (which returns `None`/`Any` to break the solve cycle) collapses each `(inner)`
    // to `Any`, so the list types `(List Any)`. That is a NESTED `Any` (not a bare `Any`/`Var`, so the
    // ordinary guard below memoizes it) — and cached, the stale `(List Any)` wins on every later CLEAN
    // read, freezing the outer producer's result element undetermined → a spurious CDZ0201 monomorphize
    // decline. Leaving it UNcached lets a later read — after the producer's scheme completes — recompute
    // the grounded `(List (Iter Int64))`, so the outer call monomorphizes and the program runs.
    //
    // SCOPED to a node EXTERNAL to every in-flight def's body (`node_external_to_inflight_solves`): the
    // poisoned `(List Any)` sits in a CALLER (`main`) re-entering `from-list`'s solve, not inside
    // `from-list`. This is load-bearing — it must NOT touch a MONOMORPHIC recursive def's OWN self-call
    // result, typed INSIDE that def's body. A bottom-up fold's tuple scrutinee `(tuple (fold a) (fold b))`
    // (`fold : E -> E`) is typed inside `fold`'s body while `fold`'s solve is on the stack → INTERNAL →
    // still cached, so `(fold a)`'s concrete `E` stays resolved. That matters: the rust sum-match emit
    // reads `type_of(scrutinee element)` and REQUIRES the concrete `Ty::Sum{E}` (else it lowers a wrong
    // variant path); a blunter "skip any nested-`Any` mid-solve" gate grounded those to a wrong shape and
    // turned that fold's clean `todo`-decline into a rust MISCOMPILE.
    //
    // Also gated on `has_any_in_data_element` — the `Any` must be a collapsed DATA-CONTAINER element
    // (`(List Any)`), NOT one under a function arrow (`(-> Int64 (-> Any Any))`, a curried closure's
    // not-yet-solved domain/result). An arrow's `Any` is a legitimate intermediate the transformer-closure
    // tie grounds, and a module-member generic monomorphization caches that arrow signature — skipping it
    // regressed `across_def_flavors` to a "closure parameter has no machine representation" CDZ0203.
    let skip_reentrant_nested_any = t.has_any_in_data_element()
        && ((!db.solving_params.is_empty() && node_external_to_inflight_solves(db, id))
            // A data-element `Any` computed WHILE A SCHEME SOLVE IS IN FLIGHT (a mutual-recursion SCC) is
            // PROVISIONAL — a call to an in-flight sibling types `Any`, so a field projected off it
            // collapses (`parse-if`'s next-index → `(Tuple Int64 Any Tree)`). Caching that node FREEZES the
            // hole: the emit path then reads the frozen `Any` (the mutual recursion never terminates, the
            // rust target declines "no native representation") even though a clean re-solve would ground it
            // to `Int64` once the sibling's scheme settles. The companion `def_scheme` defer (which returns
            // `None` for a data-`Any` result under an in-flight sibling) re-grounds the SCHEME on a later
            // clean demand — but the emit path also reads per-NODE `type_of`, so those nodes must likewise
            // not freeze. Skip caching regardless of external-ness while a scheme solve is on the stack;
            // scoped to `solving_schemes` (a fixpoint that RE-GROUNDS), never `solving_params` alone (a
            // monomorphic fold whose internal self-call `Any` must stay cached — the `across_def_flavors` /
            // bottom-up-fold cases the `node_external` arm protects).
            || !db.solving_schemes.is_empty());
    if !matches!(t, Ty::Any) && !ty_has_free_var(db, &t) && !skip_reentrant_nested_any {
        db.types.fill(id, t.clone());
    }
    t
}

/// STAGE 2a of the configurable-overflow build (operator-greenlit #5290, ruling B): the single resolved
/// [`crate::db::OverflowMode`] for an unqualified `+`/`-`/`*` arithmetic `node`. Ruling B = ONE authoritative
/// mode per node, so const-fold + backend codegen (2b) + the oracle all read the SAME trap-vs-wrap decision
/// and cannot drift. Precedence (numeric-model §"Overflow Behavior Is Configurable By Policy", #5313): the
/// governing MODULE `(pragma overflow …)` — from the load-time `db.overflow_specs` map (stage 1, #5353),
/// selected by the operand's concrete SIGNEDNESS — then the GLOBAL `Project.cdz` manifest default, then
/// `Trap`.
///
/// Signedness is read from the node's SOLVED type, so this MUST be called POST-monomorphization (once the
/// operand's concrete type is fixed — a homogeneous arithmetic op shares one sign across both operands and
/// the result, so the node's own `Ty::Int` sign IS the operand signedness). An unconstrained bare literal
/// (`Sign::Deferred`/`Var`) resolves as SIGNED — the `Int64` default. A node absent from `overflow_specs` (an
/// op written outside any `(pragma overflow …)` module, or a named `Int64.wrapping-*` form which is never
/// keyed) falls straight through to the global/`Trap` level.
pub fn overflow_mode_of(db: &mut Db, node: StructId) -> crate::db::OverflowMode {
    use crate::db::OverflowMode;
    let spec = db.overflow_specs.get(&node).copied();
    // A `Ty::Int` with a FIXED unsigned sign selects the pragma's `unsigned` mode; everything else (signed,
    // or a still-deferred/var sign that will default signed, or a non-int node) selects `signed`.
    let unsigned = matches!(
        type_of(db, node),
        Ty::Int(it) if it.sign == crate::ty::Sign::Fixed(false)
    );
    let module_mode = spec.and_then(|s| if unsigned { s.unsigned } else { s.signed });
    module_mode
        .or_else(|| db.global_overflow_default(unsigned))
        .unwrap_or(OverflowMode::Trap)
}

/// `Ty::has_free_var`, but MEMOIZED per shared compound `Rc` — for the `type_of` memoization guard above,
/// which runs on EVERY node's solved type. A wide `Ty::Record`/`Ty::Tuple` (an N-field record annotation)
/// is referenced from N nodes, each of which had the guard walk the whole O(N) payload → O(N²). The payload
/// is immutable and its `Rc` is SHARED across those nodes (a memoized `typeval_of` / a solved param type
/// hands back the same `Rc`), so the verdict caches by the `Rc`'s address (`Db::ty_has_free_var`, the
/// fix-50 key). Scalars and thin wrappers recurse directly (already O(1)); only the wide `Rc`-backed
/// payloads — `Record`/`Tuple`/`Sum`/`Nominal` — are cached, since those are the ones that make the walk
/// superlinear. Identical result to `Ty::has_free_var`, just without the repeated deep walk.
pub(crate) fn ty_has_free_var(db: &mut Db, t: &Ty) -> bool {
    match t {
        // The `Rc`-backed compounds: cache the whole-payload verdict by the `Rc`'s address so N references
        // to the same shared type pay ONE walk, not N.
        Ty::Record(fields) => {
            let ptr = std::rc::Rc::as_ptr(fields) as *const () as usize;
            if let Some(&v) = db.ty_has_free_var.get(&ptr) {
                return v;
            }
            let tys: Vec<Ty> = fields.values().cloned().collect();
            #[cfg(test)]
            crate::db::TY_HAS_FREE_VAR_ELEMS_WALKED.with(|c| c.set(c.get() + tys.len() as u64));
            let v = tys.iter().any(|f| ty_has_free_var(db, f));
            db.ty_has_free_var.insert(ptr, v);
            v
        }
        Ty::Tuple(elems) => {
            let ptr = std::rc::Rc::as_ptr(elems) as *const () as usize;
            if let Some(&v) = db.ty_has_free_var.get(&ptr) {
                return v;
            }
            let tys: Vec<Ty> = elems.to_vec();
            #[cfg(test)]
            crate::db::TY_HAS_FREE_VAR_ELEMS_WALKED.with(|c| c.set(c.get() + tys.len() as u64));
            let v = tys.iter().any(|e| ty_has_free_var(db, e));
            db.ty_has_free_var.insert(ptr, v);
            v
        }
        // Thin wrappers / leaves — already O(1) or O(depth-of-thin-nesting); recurse directly (no shared
        // `Rc` to key on, and no wide fan-out to amortize). `Sum`/`Nominal` `args` is a `Vec` (no shared
        // pointer identity) and holds only type ARGUMENTS (small — an instantiation's type params, not an
        // N-wide payload), so a direct walk is fine — the wide case is the `Record`/`Tuple` above.
        Ty::Var(_) => true,
        Ty::Fn(p, r) => ty_has_free_var(db, p) || ty_has_free_var(db, r),
        Ty::Cont { resume, answer } => ty_has_free_var(db, resume) || ty_has_free_var(db, answer),
        Ty::List(elem) | Ty::Set(elem) => ty_has_free_var(db, elem),
        Ty::Map(k, v) => ty_has_free_var(db, k) || ty_has_free_var(db, v),
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => {
            // `args` is a shared `Rc<[Ty]>`; clone the handle (a refcount bump) so the recursive `&mut db`
            // calls don't hold a borrow of `t`, then walk its elements — no per-call deep Vec copy.
            let args = args.clone();
            args.iter().any(|a| ty_has_free_var(db, a))
        }
        Ty::Qty { inner, .. } => ty_has_free_var(db, inner),
        Ty::Int(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::Type
        | Ty::Any
        | Ty::Bytes
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Float(_) => false,
    }
}

/// Whether the solved type of `id` is a `Ty::Nominal` — a cheap KIND check that does NOT clone the type.
/// `type_of` returns a `Ty` BY VALUE (a deep clone of a nested type), so a caller that only needs the
/// outermost constructor — e.g. `lower::const_at_path`, which tests each `Payload` step for a nominal
/// newtype (a run-time-erased box) once per step, per match-tree level — paid an O(depth) clone per check,
/// compounding to O(depth³) on a deeply-nested pattern. This computes/memoizes as `type_of` does, then
/// BORROWS the memoized slot to read only the discriminant. (A type with a free var / `Any` is not
/// memoized, so borrow the freshly-computed value in that case — it is cheap and never `Nominal` here.)
pub fn type_is_nominal(db: &mut Db, id: StructId) -> bool {
    if let Slot::Filled(t) = db.types.get(id) {
        return matches!(t, Ty::Nominal { .. });
    }
    // Not yet memoized — compute (this fills the slot for a ground type). Re-borrow after, or fall back to
    // inspecting the just-computed value for the unmemoized (free-var / `Any`) case.
    let t = type_of(db, id);
    matches!(t, Ty::Nominal { .. })
}

/// Solve one node's type. A poison is typed `Any` (compatible with everything) so a "no" never
/// induces a spurious mismatch upward. An integer literal is typed with a DEFERRED width, which
/// inference (or, failing that, the backend) grounds later.
fn compute(db: &mut Db, id: StructId) -> Ty {
    match resolved_of(db, id) {
        // A bare integer literal is polymorphic in its width until something fixes it — UNLESS the module
        // it is WRITTEN in declares `(pragma default-integer <T>)`, which fixes the type an otherwise-
        // unconstrained literal STARTS as (`numeric-model.md` §A Module May Declare Its Default Integer
        // Literal Type). `default_int_literals` (a load-time per-node map) records which literals it
        // applies to — keyed by the ORIGINAL node, so it survives the β-copy that reparents an inlined
        // literal. The default is the literal's starting type in unification, NOT a coercion: a mix with
        // another numeric type still rejects CDZ0301 (no silent promotion), and an explicit annotation
        // still wins (the `Annot` node fixes its own type regardless of the inner literal's).
        //
        // The map is keyed by literals WRITTEN in the pragma module (definition-site scoped), so an
        // importer's literals are unaffected; the default only chooses the literal's starting type and
        // introduces no conversion (no-silent-promotion is unchanged); and an explicit annotation/constraint
        // takes precedence over the default.
        //= spec/capabilities/numeric-model.md#a-declared-default-applies-at-the-definition-site
        //# The default integer literal type in force for a literal MUST be the one declared by the module in which the literal is written, not one declared by any module that imports it, so that importing a module never changes the type its literals take.
        //= spec/capabilities/numeric-model.md#a-declared-default-fixes-a-type-not-a-conversion
        //# Declaring a default integer literal type MUST only determine the type an otherwise-unconstrained integer literal takes, and MUST NOT introduce any implicit conversion between numeric types, so that every no-silent-promotion rule applies unchanged to a literal whatever its declared default type.
        //= spec/capabilities/numeric-model.md#a-declared-default-fixes-a-type-not-a-conversion
        //# An explicit type annotation or other constraint on an integer literal MUST take precedence over the module's declared default integer literal type.
        // A bare integer literal: a `(pragma default-fraction Rational)` module grounds it to `Rational`
        // (exact-by-default) — checked FIRST since an exact-fraction default is a stronger statement than
        // an integer-width default; then a `default-integer` width; else the deferred integer default.
        // A bare integer literal written as a constructor argument whose DECLARED payload type is `BigInt`
        // GROUNDS to `Ty::BigInt` (operator-approved contextual grounding — lossless, and an explicit
        // context overrides the Int64 default, so NOT a promotion). Marked at load in
        // `bigint_ctor_arg_literals` (bare/un-suffixed only — a `42N` is an annotation node, never marked),
        // consulted here FIRST like the fraction/int defaults. A `Core::ConstInt` typed `BigInt` already
        // emits as a BigInt leaf, so no lowering change is needed.
        Resolved::Int(_) if db.bigint_ctor_arg_literals.contains(&id) => Ty::BigInt,
        // A bare integer literal that is a direct COMPARISON operand beside a concretely-`BigInt` sibling
        // GROUNDS to `Ty::BigInt` — contextual literal typing (a constraint, so it precedes the module
        // defaults below, exactly as the ctor-arg grounding does), NOT a promotion (see
        // `literal_comparison_bigint_context`; scoped to comparison, not arithmetic). This is why `(= n 5)` /
        // `(< n 5)` type-check when `n : BigInt`: the `5` adopts its peer's `BigInt` rather than defaulting to
        // `Int64` and clashing CDZ0301. A `Core::ConstInt` typed `BigInt` already emits as a BigInt leaf.
        Resolved::Int(_) if literal_comparison_bigint_context(db, id) => Ty::BigInt,
        Resolved::Int(_) => module_default_fraction_ty(db, id)
            .or_else(|| module_default_int_ty(db, id))
            .unwrap_or_else(Ty::int),
        Resolved::Bool(_) => Ty::Bool,
        Resolved::Str(_) => Ty::String,
        Resolved::Bytes(_) => Ty::Bytes,
        // A char literal (`#\a`) is the monomorphic `Ty::Char`.
        Resolved::Char(_) => Ty::Char,
        // A symbol literal (`#"meter"`) is the monomorphic `Ty::Symbol` (DISTINCT from `Ty::String`).
        Resolved::SymbolConst(_) => Ty::Symbol,
        // A `(bin …)` in value position CONSTRUCTS a byte sequence → `Ty::Bytes`.
        Resolved::Bin { .. } => Ty::Bytes,
        // A `bin` PATTERN binder: an integer segment decodes an `Int`, a `bytes` segment a `Bytes`.
        Resolved::BinField {
            segs, seg_index, ..
        } => match segs.get(seg_index).map(|s| &s.kind) {
            // A fixed-width integer segment decodes to a GENERAL integer (deferred width → grounds `Int64`),
            // the established binding semantics — EXCEPT when the segment's value range does not FIT `Int64`,
            // in which case it MUST carry its concrete `(signed, width)` type. The sole such width is `u64`:
            // its range `[0, 2^64-1]` exceeds `Int64`, so a top-bit-set value (a genuine `UInt64 > Int64.max`)
            // is NOT representable as a signed `Int64`. Collapsing a `u64` binder to `Ty::int()` silently
            // makes it behave as a signed `Int64`, so downstream `%`/`/` pick `rem_s`/`div_s` and `Int64.of`'s
            // range-check trusts the sign — the value arithmetics/narrows as its WRAPPED NEGATIVE (a SILENT
            // wrong value on BOTH backends). Every other width (`u8`/`u16`/`u32` — all-nonneg, so signed and
            // unsigned ops agree — and `i8`..`i64`, whose signed decode is already correct) fits `Int64` and
            // stays the general-integer decode, so a wider signed CONSUMER (`(_ -9)` catch-all, `Int64` return
            // slot) keeps working. `width` is in BYTES; the concrete type carries it in BITS. (A `bits` field
            // is sub-byte, carries no explicit signedness → stays deferred.)
            Some(crate::resolved::SegKind::Int { width, signed }) => {
                let it = crate::ty::IntTy::fixed(*signed, (*width as u32) * 8);
                if it.fits_within(crate::ty::IntTy::i64()) {
                    Ty::int()
                } else {
                    Ty::Int(it)
                }
            }
            Some(crate::resolved::SegKind::Bits { .. }) => Ty::int(),
            Some(crate::resolved::SegKind::Bytes { .. }) => Ty::Bytes,
            // A `utf8` segment decodes its bytes to a well-formed `String` (a non-match on ill-formed).
            Some(crate::resolved::SegKind::Utf8 { .. }) => Ty::String,
            None => Ty::Any,
        },
        // A MAP PATTERN binder: a VALUE binder (`key = Some`) has the map's VALUE type; the REST binder
        // (`key = None`) has the whole MAP type (the scrutinee minus the named keys — same `Map<K,V>`).
        // Both read off the scrutinee's solved `Ty::Map(k, v)`.
        Resolved::MapField {
            scrutinee,
            path,
            key,
            value_steps,
            value_heads,
            ..
        } => {
            // Walk the access PATH from the scrutinee down to the nested MAP (empty for a direct map
            // scrutinee), then read the value type (a value binder) or the map type (the rest binder).
            let map_ty = map_field_map_ty(db, scrutinee, &path);
            match map_ty {
                Ty::Map(k, v) => {
                    if key.is_some() {
                        // A value binder holds the value type — but when the binder is NESTED inside a value
                        // sub-pattern (`(map ("a" (tuple x y)))`), walk the value type down `value_steps` to
                        // the nested binder (`(tuple Int64 Int64)` at `Elem(0)` → `Int64`), exactly as a
                        // nested payload binder walks its scrutinee. Empty `value_steps` = the value IS the
                        // binder (the common case).
                        if value_steps.is_empty() {
                            (*v).clone()
                        } else {
                            walk_payload_ty(
                                db,
                                (*v).clone(),
                                &value_steps,
                                &value_heads,
                                &Subst::new(),
                            )
                        }
                    } else {
                        Ty::Map(k, v) // the rest binder holds the map type
                    }
                }
                _ => Ty::Any,
            }
        }
        // A RECORD sub-pattern binder NESTED inside a tuple/list/variant match pattern — walk the scrutinee
        // type down `path` to the nested `Ty::Record`, then read the FIELD `key`'s type. The record analogue
        // of the `MapField` value-binder arm: `record_field_at_path` reaches the `Ty::Record` (the same
        // `Elem`/`Payload` descent `SumPayload` walks), and looking `key` up in its field map gives the
        // binder's type. `Ty::Any` (poison-safe) if the path-walk lands on a non-record or the field is
        // absent — the fault surfaces at the match, never a miscompile here.
        Resolved::RecordField {
            scrutinee,
            path,
            key,
            heads,
        } => match record_field_at_path(db, scrutinee, &path, &heads) {
            Ty::Record(fields) => fields.get(&key).cloned().unwrap_or(Ty::Any),
            _ => Ty::Any,
        },
        // A float literal's width is DEFERRED — it grounds to `Float64` unless an annotation or a float
        // operator's signature fixes it (`(: 3.5 Float32)`), mirroring a bare integer literal's width.
        // A bare decimal literal: a `(pragma default-fraction Rational)` module grounds it to the EXACT
        // rational its digits denote (`0.5` → `1/2`) — exact-by-default, checked FIRST (an exact-fraction
        // default is a stronger statement than a float-width default); else a `(pragma default-float <T>)`
        // width; else the deferred float default (`Float64`).
        //
        // The final `.unwrap_or_else` here (a decimal → `Ty::float`) and its twin on the integer arm above
        // (a `Resolved::Int` → `Ty::int`) realize the NO-fraction-default fallthrough: when no default
        // fraction is in force, a literal takes the numeric model's default for its WRITTEN form — an
        // integer literal the default integer type, a decimal literal the default floating-point type.
        //= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-fraction-literal-type
        //# When a module declares no default fraction literal type, a numeric literal with no other constraint MUST take the numeric model's default numeric type for its written form (an integer literal the default integer type, a decimal literal the default floating-point type).
        Resolved::Float(_) => module_default_fraction_ty(db, id)
            .or_else(|| module_default_float_ty(db, id))
            .unwrap_or_else(Ty::float),
        Resolved::Unit => Ty::Unit,
        // A name IS its bound value's type — follow the ref (a lazy `type_of` on the value occurrence).
        // EXCEPT when the ref is to an annotated let-binder `((: n T) v)` whose declared type `T` and the
        // initializer's inferred type disagree: that mismatch is ALREADY reported once at the binder
        // (CDZ0203, `check_binding_pattern`), and a body use should type against what the author DECLARED,
        // not the wrong value — exactly as an annotated PARAMETER does (`Resolved::Param` prefers
        // `param_annot_ty`). Without this the body sees the initializer's type and can emit a SECOND
        // diagnostic whose fix CONTRADICTS the first (rename the value's field vs rename the body's use), a
        // cascade rustc suppresses by binding the name at its declared type. Agreeing annotations are
        // untouched (the helper returns `None`), so a well-typed program is byte-identical.
        Resolved::Ref { value } => {
            annotated_let_binder_ty(db, value).unwrap_or_else(|| type_of(db, value))
        }
        // A `let`'s type is its body's type (the bindings are compile-time structure that folds away).
        Resolved::Let { body, .. } => type_of(db, body),
        // A VARIANT CONSTRUCTOR record carrying `(meta variant)` is a sum value/constructor, not a plain
        // data record. Its type is the constructor's `(meta t)`: for a NULLARY variant that is the sum
        // itself (a bare `None` is a VALUE of the sum — `Ty::Sum`), for a PAYLOAD variant it is the
        // curried arrow `(-> P Sum)` (a function value, applied to construct). Reading `(meta t)` as the
        // scheme and taking its type is the same path an operator value would take. This case comes
        // BEFORE the type-value check so a nullary variant is not misread as `Ty::Type`.
        Resolved::Record { .. } if crate::eval::variant_disc_of(db, id).is_some() => {
            let mut fresh = crate::unify::Fresh::new();
            match crate::eval::scheme_of(db, id, &mut fresh) {
                Some(scheme) => scheme.ty,
                None => Ty::Any,
            }
        }
        // An OPERATOR record used as a bare VALUE (not as an application head) — it carries a `(meta
        // apply)` primitive AND a `(meta t)` scheme, so its type is that scheme instantiated. This is what
        // types `Map.empty` (a nullary value operator, `∀k v. (Map k v)`) when it flows into an argument
        // position (`(Map.insert Map.empty …)`), and a bare operator passed as a HOF argument (its arrow
        // type). An APPLIED operator never reaches here — `apply_type` reads its scheme directly off the
        // head — so this only fires for a bare use. Checked before the type-value branch (an op record is
        // not a type-value: it has a `(meta apply)`, so `typeval_of` declines it) and the plain-record
        // branch (which would wrongly type it as `(record (apply …) (t …))`).
        Resolved::Record { .. }
            if crate::eval::meta_apply_of(db, id).is_some()
                && crate::eval::variant_disc_of(db, id).is_none() =>
        {
            let mut fresh = crate::unify::Fresh::new();
            match crate::eval::scheme_of(db, id, &mut fresh) {
                Some(scheme) => crate::unify::instantiate(&scheme, &mut fresh),
                None => Ty::Any,
            }
        }
        // A record that IS a type — a ground-type record (`Bool`), a built integer module, or any
        // record carrying a `(meta t)` — is a type VALUE, so its type is `Type`. Otherwise a plain
        // data record's type is the record of its fields' types (each a lazy `type_of`).
        Resolved::Record { fields } => {
            if crate::eval::typeval_of(db, id).is_some() {
                Ty::Type
            } else {
                // Each field is an INDEPENDENT type position: freshen its free vars into a disjoint block
                // off a SHARED counter so sibling fields never share a var. Two bare `None()` fields each
                // `type_of` to `Option(?0)` (the nullary-variant scheme var, memoized per node), so without
                // this they'd share `?0` and cross-contaminate when the record unifies against an expected
                // type (`?0 := Bytes` for one field then `?0 := Outcome` for its sibling — a spurious
                // mismatch). Mirrors the `Apply(RecordNew)` build in `compound_ctor_type`.
                let mut fresh = crate::unify::Fresh::new();
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    let t = crate::unify::freshen_free(&type_of(db, value), &mut fresh);
                    field_tys.insert(label.clone(), t);
                }
                Ty::Record(std::rc::Rc::new(field_tys))
            }
        }
        // Member access — the field's type is the type of the field's VALUE, found by reducing the
        // operand to a record and PROJECTING the field named by the key (the one projection, via the
        // evaluator, so it works off a literal record, a `let`-bound one, OR a module a type constructor
        // built like `(Int 64)`). A non-record operand or an absent field has no field type — typed
        // `Any` here so it does not cascade; the actual fault (CDZ0201) is reported by `type_errors`.
        //= spec/capabilities/core-semantics.md#member-access-projects-a-record-field
        //# Member access MUST project the field named by its key from the record it is applied to, evaluating to the value that field holds.
        Resolved::Member { operand, key } => match crate::eval::member_value(db, operand, &key) {
            crate::eval::Member::Field(value) => type_of(db, value),
            // The operand does not reduce to a compile-time-visible record (a call result, an `if`
            // selection), but its TYPE may be a record carrying the field — a RUNTIME record. Its
            // field type is that field's type in the record type (the runtime read's result type),
            // mirroring how a tuple projection reads its element type off `Ty::Tuple`.
            // A NOMINAL newtype over a record is erased at run time to that record, so a field read sees
            // through the tag (`strip_nominal`) to the inner record's field type — `(. u x)` on `u :
            // UserId` (a `(type UserId (Mk (Record (x Int64) …)))`) types as the inner `x`'s type.
            _ => match type_of(db, operand).strip_nominal() {
                Ty::Record(fields) => fields.get(&key).cloned().unwrap_or(Ty::Any),
                _ => Ty::Any,
            },
        },
        // A tuple's type is the tuple of its elements' types, in position order (each a lazy `type_of`).
        // Arity + element types ARE the type.
        Resolved::Tuple { elems } => Ty::Tuple(elems.iter().map(|&e| type_of(db, e)).collect()),
        // A list's type is `List <elem>` where `<elem>` is the JOIN of the element types (like a match's
        // arm-join — every element shares one type; a deferred/`Any` element yields the others). An empty
        // list is `List Any` (a deferred element, solved by unification against its use). The homogeneity
        // CHECK (a mixed list is CDZ0203) is `type_errors`' job; this fills the value column.
        Resolved::List { elems } => {
            let mut elem_ty = Ty::Any;
            for &e in elems.iter() {
                elem_ty = elem_ty.join(&type_of(db, e));
            }
            Ty::List(Box::new(elem_ty))
        }
        // A set literal's type is `Set <elem>` where `<elem>` is the JOIN of the element types (homogeneous
        // — a mixed set is CDZ0203, checked in `type_errors`). An empty `("set")` is `Set Any` (deferred).
        // Mirrors the `List` arm; the elem type flows to `Core::SetOf` at lowering.
        Resolved::Set { elems } => {
            let mut elem_ty = Ty::Any;
            for &e in elems.iter() {
                elem_ty = elem_ty.join(&type_of(db, e));
            }
            Ty::Set(Box::new(elem_ty))
        }
        // A map literal's type is `Map <key> <value>` where `<key>` is the JOIN of the entry key types
        // and `<value>` the JOIN of the entry value types (each homogeneous — a mixed-key or mixed-value
        // map is CDZ0201, the CHECK is `type_errors`' job; this fills the value column). An empty `(map)`
        // is `Map Any Any` (deferred, solved by unification against a use). A map's KEY SET is NOT part
        // of its type — only the key TYPE is (`Map<K,V>`).
        Resolved::Map { entries } => {
            let mut key_ty = Ty::Any;
            let mut val_ty = Ty::Any;
            for &(k, v) in entries.iter() {
                key_ty = key_ty.join(&type_of(db, k));
                val_ty = val_ty.join(&type_of(db, v));
            }
            Ty::Map(Box::new(key_ty), Box::new(val_ty))
        }
        // (Rc<[Ty]> collects directly from the element iterator — a refcounted immutable slice.)
        // A tuple projection's type is the operand tuple's element type AT `index`. An operand that is
        // not a tuple, or an index outside its arity, has no element type — typed `Any` here so it does
        // not cascade; the actual fault (CDZ0201) is reported by `type_errors`.
        Resolved::Proj { operand, index } => match type_of(db, operand) {
            Ty::Tuple(elems) => elems.get(index).cloned().unwrap_or(Ty::Any),
            _ => Ty::Any,
        },
        // A sum-variant pattern's payload binder — its type is the variant's PAYLOAD type AT THE
        // SCRUTINEE'S INSTANTIATION. The ctor `(. Sum V)`'s scheme is `∀a. a → Option a`; instantiating
        // it gives `?0 → Option ?0`, and unifying the RESULT `Option ?0` against the scrutinee's solved
        // type `Option Int64` solves `?0 = Int64` — so `(match (s : Option Int64) ((Some x) …))` types
        // `x` as `Int64`, not a free var. For a MONOMORPHIC sum the scheme has no vars, so the payload is
        // read directly. `Any` if the head is not a single-payload variant (a fault the match reports).
        Resolved::SumPayload {
            scrutinee,
            steps,
            heads,
        } => {
            // Walk the scrutinee's solved type down the access PATH. A `Payload` step descends into a
            // variant's payload: the next sub-value's type is that variant's payload AT THE CURRENT
            // instantiation (`payload_ty_at_instantiation` unifies the head's `(-> payload Sum)` result
            // against the current type). `heads` supplies the variant head at each `Payload` step, in
            // order (a queue — one head per Payload step). An `Elem(i)` step descends into a tuple element
            // (a variant whose payload is a tuple, destructured by a nested `(tuple …)` pattern): the next
            // type is the tuple's i-th element. A nested `(Some (Some y))` on `Option (Option Int64)`
            // walks two Payload steps; `(Exp.Add (tuple a b))` walks `[Payload, Elem(0/1)]`.
            //
            // A scrutinee that is a TUPLE CONSTRUCTOR — `(match (tuple (fold a) (fold b)) ((tuple fa fb)
            // …))`, where `fa`/`fb` read its elements — types via the CONSTRUCTOR's element occurrences,
            // NOT the aggregate `type_of((tuple …))`. Aggregate typing reads a RECURSIVE-call element
            // (`(fold a)`, during `fold`'s own solve) as `Any` → `(Tuple Any Any)` → the binder `fa` reads
            // `Any` and the value-heap emit declines; typing each element occurrence on its own reaches the
            // recursive callee's cached `def_scheme` (`fold : E → E`), so `fa : E`. `tuple_constructor_ty`
            // builds `(Tuple <elem-tys>)` from the constructor when the scrutinee is one, else `None`.
            let start =
                tuple_constructor_ty(db, scrutinee).unwrap_or_else(|| type_of(db, scrutinee));
            project_path_type(db, start, &steps, &heads)
        }
        Resolved::If { cond, then_, else_ } => {
            // Reading the children's types is the backward demand: each is a lazy `type_of`.
            let _cond_ty = type_of(db, cond);
            let then_ty = type_of(db, then_);
            let else_ty = type_of(db, else_);
            // The if's type is the join of its branches — `Any` yields the other, and a branch that
            // fixed a deferred integer width contributes it. The cond-is-Bool and branches-agree
            // CHECKS are `type_errors`' job; this fills the value column.
            //
            // RIGID-BIASED join: when the two branches are the SAME sum built two ways whose element vars
            // DIFFER, and one element is a RIGID param var (this def's own parameter element, marked in
            // `scheme_rigid_vars` — the recursive-generic element tie), prefer THAT var. The plain `join`
            // picks a branch arbitrarily for two `Ty::Var`s (`(Var, t) => t`), so a recursive-generic
            // transformer's STOP branch — `take-while`'s `(if (p h) (Iter.Cons h (rec …)) (Iter.Nil))`
            // where the `Iter.Nil` else-branch carries a FRESH element var and the then-branch carries the
            // param's rigid element — could pick the fresh var, severing the result-element↔param tie so
            // the scheme generalized a disconnected result var and monomorphization declined CDZ0201 at ≥2
            // types (v-iterators' take-while, the bare-nullary-leaf-on-one-path family). Biasing the join
            // toward the rigid element keeps the result tied. Only reorders the two operands (still a
            // `join`, same result set); a non-sum / no-rigid / equal-var join is byte-identical.
            let ty = rigid_biased_join(db, &then_ty, &else_ty);
            ground_open_var_arms_to_collection(db, &[then_, else_], &ty);
            ty
        }
        // A boolean connective is a Bool. Reading the operands' types is the backward demand; the
        // operands-are-Bool CHECK is `type_errors`' job (this fills the value column).
        Resolved::And { lhs, rhs, .. } => {
            let _l = type_of(db, lhs);
            let _r = type_of(db, rhs);
            Ty::Bool
        }
        Resolved::Not { operand } => {
            let _o = type_of(db, operand);
            Ty::Bool
        }
        // `(try e)` UNWRAPS a fallible value: its type is the SUCCESS PAYLOAD of the operand's
        // `Option(a)`/`Result(a, b)` type (the `a` yielded on the normal path). The short-circuit path
        // does not contribute to the node's value type — it exits to the boundary. Reading the operand's
        // type is the backward demand; the operand-is-fallible and boundary-agreement CHECKS (CDZ0230 /
        // CDZ0203) are `type_errors`' job. Falls back to `Any` when the operand is not a recognized
        // fallible sum, so an ill-formed `?` stays a soft `Any` here and the fault surfaces in `collect`.
        Resolved::Try { operand } => {
            let ot = type_of(db, operand);
            match fallible_shape(db, &ot) {
                Some((_, payload, _)) => payload,
                None => Ty::Any,
            }
        }
        // A match's type is the JOIN of its arm bodies (like an `if` over N branches) — every arm must
        // produce the same type, and a branch that fixed a deferred width contributes it. The
        // arms-agree and exhaustiveness CHECKS are `type_errors`' job; this fills the value column.
        Resolved::Match { scrutinee, arms } => {
            let _scrut = type_of(db, scrutinee);
            let mut ty = Ty::Any;
            for (_, body) in &arms {
                let bt = type_of(db, *body);
                ty = ty.join(&bt);
            }
            let bodies: Vec<StructId> = arms.iter().map(|(_, b)| *b).collect();
            ground_open_var_arms_to_collection(db, &bodies, &ty);
            ty
        }
        // `nan` — the canonical NaN Float VALUE (a bare prim naming a value). Types as `Ty::Float` (a
        // bare `nan` grounds to Float64), so `(= nan nan)` type-checks like any float equality.
        Resolved::Prim(crate::resolved::Prim::FloatNan) => Ty::float(),
        // `Infinity` — the positive-infinity Float VALUE. Types as `Ty::Float` exactly as `nan` (a bare
        // `Infinity` grounds to Float64), so `(< Infinity x)` / `(= Infinity Infinity)` type-check.
        Resolved::Prim(crate::resolved::Prim::FloatInf) => Ty::float(),
        // A bare built-in operation value standing alone has no scalar type yet (it is not a runtime
        // value until functions/closures exist). Typed `Any`; applying it is what has a type.
        Resolved::Prim(_) => Ty::Any,
        // Application — the ONE generic rule: read the head's TYPE (its `(meta t)` scheme), instantiate
        // it with fresh variables, and unify each argument's type into the curried parameter
        // positions; the result is the instantiated return type. This is HM application — the SAME
        // rule for every operator (and every function later), with NO per-operator arm. A type
        // constructor's application yields a type value, typed `Any` at the value level. `apply_type`
        // returns the result type (unification FAULTS are surfaced separately by `type_errors`).
        Resolved::Apply { head, args } => apply_type(db, head, &args),
        // A type annotation `(: expr T)`: the node's type is the annotation type `T`, with `expr`'s
        // type UNIFIED into it — so a deferred width in `expr` (a bare literal) is GROUNDED by `T`
        // (`(: 5 Int64)` types as `Int64`), and a genuine conflict (`(: true Int64)`) is a fault the
        // `type_errors` side reports (here we return `T`, the asserted type, so the value column stays
        // definite). If `T` is not a type expression this stage reduces, fall back to `expr`'s type.
        //
        // `T` is not a syntactic marker stripped before evaluation: `ty_expr` is REDUCED to a type VALUE
        // (`typeval_of` runs the SAME evaluator that reduces any value — `(Int 8)`/`(-> A B)` etc. fold to
        // a `Ty` through the one `Meta.apply` channel), and that value is what unifies into `expr`'s type.
        // The annotation carries its type AS a value, exactly as §Types Are First-Class Values requires.
        //= spec/capabilities/core-semantics.md#types-are-first-class-values
        //# A type annotation `(: <expr> <Type>)` MUST carry its type as a value, not as a syntactic marker erased before evaluation.
        // `(const e)` is SEE-THROUGH for typing — it types AS its inner expression (no annotation to
        // unify, no type of its own); the force-eval / reject-if-not-const semantics are downstream in
        // lowering (v-compiler-primitives). `type_of((const e)) == type_of(e)`.
        Resolved::ConstBlock { expr } => type_of(db, expr),
        Resolved::Annot { expr, ty_expr } => match crate::eval::typeval_of(db, ty_expr) {
            Some(annot_ty) => {
                let expr_ty = type_of(db, expr);
                // A QUANTITY annotation is a pure DIMENSION CHECK — it must NOT rebrand the value's unit.
                // `(: (Qty.of 1 kilometer) (Qty Int64 meter))` checks that km and meter share a dimension
                // (the check lives in `check_application`), but the value STAYS `1 km` downstream — the
                // annotation names the dimension, it does NOT normalize/coerce the magnitude to the
                // annotation's unit. Returning `annot_ty` here (the general grounding behavior below)
                // REBRANDED it to `(Qty Int64 meter)`, so `1 km` was silently reinterpreted as `1 meter`
                // and then combined with real km quantities WITHOUT the mixed-unit conversion (a
                // high-severity miscompile: `(: (1 km) meter) + 2 km` gave 2001 m, not 3000 m — the units
                // safety promise inverted). So when both sides are quantities of the SAME dimension, keep
                // the EXPRESSION's UNIT (its own scale intact) — but GROUND the inner numeric type from the
                // ANNOTATION, exactly as any annotation grounds a deferred width: `(: (Qty.of 5 kilometer)
                // (Qty UInt8 meter))` keeps `kilometer` (no rebrand) yet its inner becomes `UInt8` (so a
                // deferred literal width is fixed + range-checked by `nested_literal_width_faults`, and the
                // emitted value carries the annotated width, not a defaulted Int64). Returning `expr_ty`
                // WHOLESALE kept the unit but left the inner ungrounded (a completeness gap: an out-of-range
                // magnitude slipped the check and the ABI type stayed Int64). Keep expr's unit + annot's
                // inner; a cross-dimension conflict is `check_application`'s CDZ0501, an inner-numeric
                // conflict its CDZ0203.
                if let (
                    Ty::Qty {
                        inner: ai,
                        unit: au,
                    },
                    Ty::Qty {
                        inner: ei,
                        unit: eu,
                    },
                ) = (&annot_ty, &expr_ty)
                    && au.same_dimension(eu)
                {
                    // Unify the two inner numeric types (a deferred literal width grounds to the
                    // annotation's); on a genuine inner clash keep the annotation's inner (the CDZ0203 is
                    // reported by `check_application`), and always keep the EXPRESSION's unit `eu`.
                    let mut subst = Subst::new();
                    let inner = if crate::unify::unify(&mut subst, ai, ei, &db.name_ctx()).is_ok() {
                        subst.apply(ai)
                    } else {
                        (**ai).clone()
                    };
                    return Ty::Qty {
                        inner: Box::new(inner),
                        unit: eu.clone(),
                    };
                }
                let mut subst = Subst::new();
                let _ = crate::unify::unify(&mut subst, &annot_ty, &expr_ty, &db.name_ctx());
                subst.apply(&annot_ty)
            }
            None => type_of(db, expr),
        },
        // The type of an un-typeable node: compatible with everything, so it cannot cascade.
        Resolved::Poison(_) => Ty::Any,
        // EFFECT CONTROL FORMS. A `handle` evaluates to the value its FOLDED body produces (each perform
        // resolved to its arm's resume value, state threaded away), so its type is the type of that
        // rewritten body — reduce the handler and type the result. This is what lets a state-threading
        // body whose surface uses a nested `(do …)` (which resolve does not model as an expression, so
        // the ORIGINAL body types as `Any`) still get a definite type from its reduced form. If the fold
        // declines (a case the tail path cannot serve), fall back to the original body's type — harmless,
        // since lowering will decline it anyway.
        Resolved::Handle { init, arms, body } => {
            match crate::effects::reduce_handle(db, init, &arms, body) {
                Some(rewritten) => type_of(db, rewritten),
                None => type_of(db, body),
            }
        }
        // A `host` evaluates to its body's value (E2 handles the boundary). A `resume`'s value is handed
        // back at the perform site; outside the tail-rewrite it has no independent type, so `Any`.
        Resolved::Host { body, .. } => type_of(db, body),
        Resolved::Resume { .. } => Ty::Any,
        // A lambda/def parameter used as a value — a formal. If its binder is ANNOTATED (`(: a T)`),
        // its type is that annotation `T` — so the body type-checks against a definite parameter type
        // (`(: a Bool)` used as an integer operand is caught). Otherwise, for the parameter of a
        // RECURSIVE def (which cannot inline, so its type must be inferred rather than flowing from a
        // call site), the CONNECTED def-body solve (`solve_recursive_params`, A2) infers it from its
        // uses; a still-`None` result means either a non-recursive param (typed `Any` — it inlines at
        // its call site, where the argument's type flows in via the fold) or an unconstrained one.
        Resolved::Param { binder } => param_annot_ty(db, binder)
            .or_else(|| crate::effects::handle_arm_param_ty(db, binder))
            .or_else(|| crate::effects::handle_arm_state_ty(db, binder))
            .or_else(|| solved_param_ty(db, binder))
            .or_else(|| lambda_param_ty_from_context(db, binder))
            .unwrap_or(Ty::Any),
        // A TYPE value is a value, so it has a type — `Type` (the type of types). A bare type value
        // (a `(typeval …)` node, OR a value the evaluator reduces to a type) types as `Type`; this is
        // what makes a type first-class (it can be passed, returned, checked).
        Resolved::TypeVal(_) => Ty::Type,
        // A lambda's type is its function type `param → … → result` — each parameter's type (its
        // annotation, or `Any` for a bare param) curried onto the body's type. Typing a lambda as its
        // arrow type (rather than the opaque `Any` it used to be) is what lets a HIGHER-ORDER call
        // check the passed function against a `(-> A B)` parameter annotation: unifying the argument's
        // `Fn(A', B')` against the declared `Fn(A, B)` catches a RESULT-type mismatch (`(-> Int Bool)`
        // vs an `Int → Int` lambda) — structurally, so a nested/curried arrow is checked all the way
        // down. A bare-param lambda contributes `Any` in its parameter slot, so it still unifies with
        // any expected parameter type (no over-rejection); only a definite result disagreement faults.
        Resolved::Lambda { params, body } => {
            let result = type_of(db, body);
            params.iter().rev().fold(result, |acc, &p| {
                let pt = type_of(db, crate::eval::param_name_occ(db, p));
                Ty::Fn(Box::new(pt), Box::new(acc))
            })
        }
    }
}

/// GROUND an open-`Ty::Var` control-flow-JOIN arm body to a DETERMINED-COLLECTION join type — the empty-
/// collection-in-a-match/if-fallback miscompile fix (breaker ms13 family). When one arm body of a `match`
/// (or branch of an `if`) types as a bare `Ty::Var` (e.g. the `Some ys` arm binding a never-populated
/// `Map.empty`'s value var `?v`) while a SIBLING arm supplies a determined collection (the empty `(list)`
/// fallback grounds to `(List Any)`, or a `(List Int64)` sibling), the `join` already yields the determined
/// collection at the MATCH NODE (`(Var, t) => t`), and the let-binder + downstream use read that grounded
/// type. But the per-ARM emit reads each arm body's OWN `type_of` — and a bare `Ty::Var` has NO machine
/// valtype (`lir::valtype_of(Ty::Var) = None`) while the join is a collection HANDLE (i32), so the Var-arm
/// emits a scalar where the binder demands a handle → INVALID WASM ("expected i32, found i64") + rust E0308.
/// (Confirmed: the join node IS already `(List Any)`; the sole gap is the ungrounded ARM node — RUN-probed
/// ms13/mG on trunk 0e12d9bac.) GROUND each such arm's node type to the join, so the arm emits the same
/// collection handle as its siblings — the fix gets the program COMPILING (operator's steer), not a decline.
///
/// NARROW + SAFE: fires ONLY when (a) the join `ty` is a determined collection — `List`/`Set`/`Map` whose
/// element(s) are NOT themselves `Var`/`Any` (a genuinely-solved element, so the grounding is to a real
/// machine type, never one guess feeding another); AND (b) an arm body's own `type_of` applies to a bare
/// `Ty::Var`. It writes the arm NODE's memo directly (the type column), so the emit-time per-arm read sees
/// the grounded type. It does NOT run while a SCHEME solve is in flight (`solving_schemes` non-empty) — a
/// provisional mid-fixpoint Var must stay unfrozen to re-ground cleanly, exactly as the `type_of` memo guard
/// skips a re-entrant nested-`Any`. A non-collection join, or no Var arm, is a no-op (byte-identical).
fn ground_open_var_arms_to_collection(db: &mut Db, bodies: &[StructId], ty: &Ty) {
    // (a) The join must be a COLLECTION KIND — `List`/`Set`/`Map`. The CONTAINER is what fixes the machine
    // slot: every collection is an i32 heap HANDLE (`lir::valtype_of` returns `I32` for `List`/`Set`/`Map`
    // REGARDLESS of the element type), so grounding a bare-`Ty::Var` arm to the join gives it the correct
    // handle valtype whether the join element is solved (`(List Int64)`), an `Any` (a lone empty `(list)` →
    // `(List Any)`), or itself a `Var` (an empty `(Set.of (list))` → `(Set ?e)` / `Map.empty` → `(Map ?k
    // ?v)`). The Var-arm otherwise has NO valtype (`valtype_of(Ty::Var) = None`) and emits a scalar where
    // the binder demands a handle → invalid wasm. So admit ANY collection kind — the element need not be
    // solved (breaker ej2/ej3: Set/Map empty-literal siblings ground to a `Var`-element collection, not an
    // `Any`-element one like the list case, so requiring a non-Var element left those two kinds broken).
    //
    // We ground the arm to the collection's own ELEMENT-ERASED shell (element(s) → `Ty::Any`), NOT the raw
    // `ty`: the raw join may carry a FREE `Ty::Var` element (`(Set ?e)`), and the `type_of` memo invariant
    // (this fn's caller at the memoize guard) DELIBERATELY never caches a free-var-bearing type so a later
    // connected solve can re-ground it — filling one here would FREEZE that var and, since the fill order
    // vs other demands varies, make the solve ORDER-DEPENDENT (a flaky freeze). The i32 handle slot is the
    // same for any element, so erasing the element to `Any` yields a fully-GROUND collection type that is
    // memo-safe AND gives the arm the correct handle valtype. (A determined-element join like `(List Int64)`
    // erases to `(List Any)`, still an i32 handle — the arm only needs the CONTAINER, not the element.)
    let shell = match ty {
        Ty::List(_) => Ty::List(Box::new(Ty::Any)),
        Ty::Set(_) => Ty::Set(Box::new(Ty::Any)),
        Ty::Map(_, _) => Ty::Map(Box::new(Ty::Any), Box::new(Ty::Any)),
        _ => return,
    };
    // (b) A provisional mid-scheme-fixpoint Var must not be frozen — it re-grounds on a later clean demand
    // (the same discipline the `type_of` memo guard applies to a re-entrant nested-`Any`). Only ground when
    // no scheme solve is on the stack.
    if !db.solving_schemes.is_empty() {
        return;
    }
    for &body in bodies {
        if matches!(type_of(db, body), Ty::Var(_)) {
            db.types.fill(body, shell.clone());
        }
    }
}

/// The CONCRETE integer type a bare literal takes from its BINARY-OPERATOR context, if any — the type
/// its SIBLING operand fixes. An integer binary op (`is_binop`: arith/bitwise/comparison) constrains its
/// two operands to one type (`+ : ∀a. (Int a) → (Int a) → (Int a)`; a comparison relates two of one
/// type), so a bare literal beside a `UInt64`-typed operand takes `UInt64` — the constraint
/// numeric-model.md §"An Explicit … Or Other Constraint On An Integer Literal MUST Take Precedence"
/// requires. Per-node `type_of` does NOT thread this back to the literal (a bare literal always solves to
/// a DEFERRED `Ty::int()`; the shared width is reconciled at selection via `operand_int_ty`), so the
/// well-formedness range check must consult the context itself to avoid fitting a `UInt64` literal
/// against the i64 default. Returns the sibling's `IntTy` only when it is CONCRETELY fixed (a deferred
/// sibling — two bare literals — imposes nothing, and both then default to Int64). Keyed on the
/// operator's PRIM (`is_binop`), never a name — no key outside the prelude.
fn literal_binop_context_ty(db: &mut Db, id: StructId) -> Option<crate::ty::IntTy> {
    // CLIMB the binary-operator spine from the literal upward. At each step `child` is the node we ascended
    // from and `parent` its enclosing binop; `sibling` is the operator's OTHER operand. A CONCRETELY-fixed
    // integer sibling (a `UInt8` variable, an annotated operand, a nested op whose own operands fix it —
    // `(% a b)` over two `UInt8`) fixes the shared width, the same "prefer a concrete width over a deferred
    // literal" rule `operand_int_ty` applies at selection.
    //
    // When the immediate sibling is itself DEFERRED (an `if`/`match` whose bare-literal branches default to
    // Int64, another bare literal), the width may still be fixed TRANSITIVELY by an ancestor: in `(+ (* 10000
    // (if …)) (% a b))` the literal's own sibling (the `if`) is deferred, but the enclosing `*`'s sibling
    // under the `+` is the `UInt8` `(% a b)`. An integer ARITH op unifies its two operands to ONE width (its
    // result width IS its operand width — `+ : ∀a. (Int a) → (Int a) → (Int a)`), so that `UInt8` flows down
    // through the `*` to the literal, which must then fit it (numeric-model.md §"An Explicit … Or Other
    // Constraint On An Integer Literal MUST Take Precedence"). This is the wasm selection path's downward
    // width threading, surfaced at the SHARED well-formedness layer so `cdz check` and BOTH backends inherit
    // one verdict — rather than the rust backend silently emitting a truncating `as u8` cast where wasm
    // rejects (CDZ0302). Climb only through ARITH ops: a COMPARISON's result is `Bool`, so a width fixed
    // above it does NOT flow to its operands (the chain breaks there).
    let mut child = id;
    loop {
        let parent = db.parent_of(child)?;
        // Borrow the resolved form (a dispatch/tag test that only READS `head`/`args`) instead of cloning
        // the whole `Resolved` per climb step via `resolved_of` — the `resolved_ref` borrow family the
        // `a_wide_match_resolves_in_a_bounded_number_of_clones` guard documents. A binop is exactly two
        // operands (any other arity returns `None` below), so extract only the Copy `StructId` head + the
        // two operands, dropping the borrow before the `&mut db` calls below — no `Vec` clone at all.
        let (head, arg0, arg1) = {
            let Resolved::Apply { head, args } = resolved_ref(db, parent) else {
                return None;
            };
            if args.len() != 2 {
                return None;
            }
            (*head, args[0], args[1])
        };
        // The head must be an integer binary operator; the child must be one of its (exactly two) operands.
        let prim = crate::eval::meta_apply_of(db, head)?;
        if !prim.is_binop() {
            return None;
        }
        let sibling = if arg0 == child {
            arg1
        } else if arg1 == child {
            arg0
        } else {
            return None;
        };
        // A CONCRETELY-fixed sibling fixes the shared width. A deferred/var sibling imposes no direct
        // constraint here.
        if let Ty::Int(it) = type_of(db, sibling)
            && it.width_is_fixed()
        {
            return Some(it);
        }
        // The sibling is deferred. Keep climbing only through an ARITH op (whose enclosing width flows down
        // to its operands); a COMPARISON breaks the chain (its result is `Bool`, not the operand width).
        if !prim.is_arith() {
            return None;
        }
        child = parent;
    }
}

/// True iff a bare FLOAT literal at `id` is grounded to `Float32` by an arith-operand CONTEXT — the float
/// twin of `literal_binop_context_ty`'s integer climb. Climbs the binary-arith spine from the literal; a
/// concretely `Float32` sibling anywhere up the ARITH chain fixes the shared width to Float32 (a float
/// arith op unifies its operands to one width, `+. : ∀a.(Float a)→(Float a)→(Float a)`), so the literal
/// must fit Float32. A COMPARISON breaks the chain (its result is `Bool`); a non-`Float32` fixed sibling
/// (e.g. Float64) does NOT ground it narrow. Used to surface an inf-materializing overflow at `check`.
fn literal_binop_float32_context(db: &mut Db, id: StructId) -> bool {
    let mut child = id;
    loop {
        let Some(parent) = db.parent_of(child) else {
            return false;
        };
        let (head, arg0, arg1) = {
            let Resolved::Apply { head, args } = resolved_ref(db, parent) else {
                return false;
            };
            if args.len() != 2 {
                return false;
            }
            (*head, args[0], args[1])
        };
        let Some(prim) = crate::eval::meta_apply_of(db, head) else {
            return false;
        };
        if !prim.is_binop() {
            return false;
        }
        let sibling = if arg0 == child {
            arg1
        } else if arg1 == child {
            arg0
        } else {
            return false;
        };
        // A concretely `Float32` sibling fixes the shared width to Float32.
        if let Ty::Float(ft) = type_of(db, sibling)
            && ft.ground_width() == 32
        {
            return true;
        }
        // Keep climbing only through an ARITH op (a comparison's result is `Bool`, breaking the chain).
        if !prim.is_arith() {
            return false;
        }
        child = parent;
    }
}

/// True iff a bare INTEGER literal at `id` is a direct COMPARISON operand beside a concretely-`BigInt`
/// sibling, so it grounds to `Ty::BigInt`. A comparison (`= < > <= >= compare`) relates its two operands to
/// ONE type and yields `Bool`, so a bare literal compared against a `BigInt` takes `BigInt` — the comparison
/// twin of a bare literal grounding to a constructor's declared `BigInt` payload (`bigint_ctor_arg_literals`)
/// and of `literal_binop_context_ty`'s WIDTH grounding, here extended to the unbounded integer (a DISTINCT
/// `Ty`, not an `IntTy` width). This is contextual literal TYPING (lossless, operator-approved seq-257), NOT
/// a numeric promotion: only a BARE LITERAL grounds; a non-literal `Int64` VALUE compared to a `BigInt` still
/// mismatches CDZ0301 (that unify is untouched). So `(= n 5)` / `(< n 5)` with `n : BigInt` type-check (the
/// `5` is BigInt), while `(= n m)` with `m : Int64` still declines.
///
/// SCOPED to COMPARISON, deliberately NOT arithmetic: `(+ (BigInt.of 1) 1) → CDZ0301` is a pinned spec case
/// (numeric-model.md §"Numeric Types Do Not Silently Promote", `06-numeric-model` "a BigInt operation does
/// not silently promote a fixed-width operand"), and seq-257's operator examples are all comparisons. A
/// comparison's result is `Bool` (no promoted result value to speak of); extending the grounding to
/// arithmetic would REVERSE that documented case, so it is left to a separate operator ruling.
fn literal_comparison_bigint_context(db: &mut Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    let (head, arg0, arg1) = {
        let Resolved::Apply { head, args } = resolved_ref(db, parent) else {
            return false;
        };
        if args.len() != 2 {
            return false;
        }
        (*head, args[0], args[1])
    };
    // The parent must be a binary COMPARISON with `id` as one of its two operands.
    let Some(prim) = crate::eval::meta_apply_of(db, head) else {
        return false;
    };
    if !prim.is_comparison() {
        return false;
    }
    let sibling = if arg0 == id {
        arg1
    } else if arg1 == id {
        arg0
    } else {
        return false;
    };
    // The sibling grounds the literal only when it is CONCRETELY `BigInt`. A sibling that is itself a bare
    // integer literal imposes nothing (two bare literals both default to Int64) — and skipping it before
    // reading its type is also the termination guard: `type_of` on a bare-literal sibling would re-enter
    // THIS check from the sibling (`(= 5 6)`: 5 reads 6, 6 reads 5, …). A `BigInt`-suffixed `5N` is an
    // ANNOTATION node (`as_int` is `None`), so it is still consulted.
    db.ast.as_int(sibling).is_none() && matches!(type_of(db, sibling), Ty::BigInt)
}

/// The CDZ0302 fault if the value at `value` is an integer LITERAL that does not fit the NARROW integer
/// type the type-expression `ty_expr` denotes, else `None`. The literal analogue of "Annotations
/// Constrain" (numeric-model.md §A Bare Integer Literal Is Grounded By Its Annotation, Subject To A Range
/// Check): a bare literal has no intrinsic width, so an annotation FIXES its type subject only to a range
/// check — a literal outside the width is REJECTED, never truncated. Shared by the value annotation
/// `(: value T)` and the annotated LET BINDER `((: name T) value)` so both range-check identically. Only
/// a literal + a fixed-width integer type can fault here; a non-literal value's agreement is a separate
/// unify (CDZ0203), and a deferred/Var width imposes no bound.
fn literal_width_fault(db: &mut Db, value: StructId, ty_expr: StructId) -> Option<Reject> {
    let annot_ty = crate::eval::typeval_of(db, ty_expr)?;
    // A FLOAT literal annotated to a NARROWER float width it cannot hold — `(: 1.0e300 Float32)`. The
    // value is finite as `Float64` (the literal's default) but overflows `Float32` to `±inf`, a value with
    // no written form (numeric-model.md §A Floating-Point Literal That Denotes No Representable Value Is
    // Malformed) — the float analogue of an out-of-range integer literal (CDZ0302). Only `Float32` is
    // narrow enough to catch here (a `Float64` overflow is a malformed bare literal, caught earlier); a
    // non-literal value imposes no compile-time bound. `is_finite_f64` guards the default `Float64`; this
    // is its `Float32` sibling, promised by that method's own doc-comment ("`(: 1e40 Float32)` … caught at
    // the annotation").
    if let Ty::Float(ft) = &annot_ty
        && ft.ground_width() == 32
        && let crate::ast::Struct::Atom(lid) = db.ast.get(value)
        && let crate::ast::Leaf::Float(dec) = db.ast.leaf(*lid).clone()
        && !dec.fits_f32()
    {
        // The mechanical repair: retype the annotation to `Float64` — the wider float holds the value (a
        // `Float64` is the literal's own default, so it is finite there), the float twin of the integer
        // width-widen / BigInt fix. Rewrite the whole `ty_expr` so either spelling — a bare `Float32` or a
        // `(Float 32)` compound — becomes the bare `Float64`. Heuristic (the author may instead have meant a
        // smaller literal), but the retype clears the overflow in one shot and type-checks.
        return Some(
            Reject::coded(
                Code::IntOutOfRange,
                "float literal does not fit the annotated type Float32 (it overflows the Float32 \
                 range to infinity — the largest finite Float32 is about 3.4e38)",
            )
            .at(ty_expr)
            .with_fix(Fix::replace_heuristic(ty_expr, "Float64")),
        );
    }
    let Ty::Int(it) = &annot_ty else { return None };
    let crate::ty::Width::Fixed(w) = it.width else {
        return None;
    };
    // The SENTINEL width 0 (a `reduce_ctor` clamp of an out-of-range/malformed width like `(UInt 65)`) is
    // an ill-formed TYPE, not a literal-range problem — reporting "literal does not fit UInt0" misleads
    // (it names the clamped sentinel and blames the literal). The ill-formed-width check
    // (`out_of_range_int_width`, applied at the param/value annotation) reports the REAL fault naming the
    // written width, so skip the literal-fit report here and let that fire.
    if w == 0 {
        return None;
    }
    // A value that ALREADY has a distinct numeric type — an EXPLICITLY-SUFFIXED literal `999N`
    // (`Ty::BigInt`) / `1R` (`Ty::Rational`) — is NOT a bare literal being grounded by the annotation: it
    // carries its own type, so `(: 999N Int64)` (or passing `999N` to an `Int64` parameter) is a genuine
    // type MISMATCH (BigInt ≠ Int64), reported by the CDZ0203 unify. The width fit-check sees through the
    // suffix's `(: 999 BigInt)` desugar to the inner `Resolved::Int` and would ALSO fire CDZ0302 ("literal
    // does not fit Int64") — double-reporting the same slip with a misleading second framing (a BigInt is
    // the wrong TYPE, not an out-of-range Int64 literal). Skip it; the mismatch path is the correct, sole
    // diagnostic. A BARE literal (no suffix) still range-checks: it types as the `Int64` default, so it is
    // a grounding, exactly as before.
    if matches!(type_of(db, value), Ty::BigInt | Ty::Rational) {
        return None;
    }
    // The bound value's CONSTANT integer value, if it has one: a bare literal `200`, OR a value that
    // FOLDS to a constant (`(+ 100 100)` → 200) — the same computed constants the value annotation
    // `(: (+ 100 100) Int8)` range-checks. `core_of` performs the fold; a runtime value (a param, a call
    // result) does not fold to `ConstInt` and imposes no compile-time bound (it is checked by its own
    // type / traps at run time).
    let v = match resolved_of(db, value) {
        Resolved::Int(v) => v,
        _ => match crate::lower::core_of(db, value) {
            crate::core::Core::ConstInt(v) => v,
            _ => return None,
        },
    };
    if !v.fits_width(it.ground_signed(), w) {
        return Some(int_out_of_range_reject(
            &annot_ty,
            it.ground_signed(),
            w,
            &v,
            ty_expr,
            &db.name_ctx(),
        ));
    }
    None
}

/// The CDZ0302 message for an ILL-FORMED integer width (`crate::eval::IntWidthFault`), shared by the value-
/// and parameter-annotation checks so both phrase it identically. An OVER-CEILING/zero width names the
/// WRITTEN width (`(UInt 65)` → "`UInt65` is not a valid integer type …"); a MALFORMED (negative /
/// non-natural) width has NO width number to name, so it states the constraint the width violated (a width
/// must be a compile-time NATURAL in 1..=64) — the case that used to slip past `cdz check` entirely.
pub(crate) fn ill_formed_int_width_message(fault: &crate::eval::IntWidthFault) -> String {
    match *fault {
        crate::eval::IntWidthFault::OverCeiling { signed, width } => format!(
            "`{}{width}` is not a valid integer type: a width must be in 1..=64 (a fixed-size integer \
             wider than 64 bits is reserved to the big-integer layer, and 0 is not a width)",
            if signed { "Int" } else { "UInt" }
        ),
        crate::eval::IntWidthFault::Malformed { signed } => format!(
            "an integer type's width must be a compile-time natural number in 1..=64 — this `{}` type's \
             width is not a natural number (a negative, fractional, or non-numeric width is not a width)",
            if signed { "Int" } else { "UInt" }
        ),
    }
}

/// The CDZ0302 REPAIR for an ill-formed integer width at `pos` — the actionable half of
/// [`ill_formed_int_width_message`] (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
/// A Fix). Only the OVER-CEILING case (`(UInt 65)`, `(Int 128)` — a fixed width strictly greater than 64)
/// has a single confident target: the message itself says such a width is "reserved to the big-integer
/// layer", so the repair is the unbounded `BigInt`, which holds any magnitude — the type-level twin of the
/// literal-range fix's `BigInt` continuation (`int_out_of_range_reject`). Every other ill-formed width has
/// NO single correct target — a `0` width or a `Malformed` (negative/non-numeric) width could mean the
/// author dropped or mistyped the number, so guessing one would be a false suggestion (worse than none, per
/// the `suggest` module) — those carry the message alone. Heuristic: the author may instead have meant a
/// specific in-range width, but `BigInt` clears the fault in one shot and always type-checks.
pub(crate) fn ill_formed_int_width_fix(
    fault: &crate::eval::IntWidthFault,
    pos: StructId,
) -> Option<Fix> {
    match *fault {
        crate::eval::IntWidthFault::OverCeiling { width, .. } if width > 64 => {
            Some(Fix::replace_heuristic(pos, "BigInt"))
        }
        _ => None,
    }
}

/// The FIRST ill-formed integer width ANYWHERE in the type-expression `ty_expr` — the top-level type OR a
/// NESTED type-argument position (`(Option (UInt 65))`, `(List (Int -8))`, `(Tuple Int8 (Int -8))`,
/// `(Map (Int -8) v)`). A top-level `int_width_fault` catches `(: 5 (Int -8))`, but a width nested in a
/// compound annotation reduces to a valid-looking `Ty` (the ctor clamps the bad width to sentinel `Int0`),
/// so the top-level check + `typeval_of` both wave it through — it slipped past `cdz check` while the emit
/// path caught it (a check-vs-emit gap). Recurse every type-ctor ARGUMENT position (the tail elements of a
/// `(head arg…)` type form; the head `List`/`Option`/`Map`/`->`/`Int`/… is the ctor, not a nested type)
/// and return the first arg that is itself an ill-formed-width integer type. Reuses `eval::int_width_fault`
/// per position, so the message + code match the top-level check exactly. A record `(Record (f T)…)` field
/// TYPE is a tail element of its `(f T)` pair — descended too (skipping the label). Returns `(pos, fault)`.
fn nested_ill_formed_int_width(
    db: &mut Db,
    ty_expr: StructId,
) -> Option<(StructId, crate::eval::IntWidthFault)> {
    // This position itself — an `(Int W)`/`(UInt W)` with an ill-formed width.
    if let Some(fault) = crate::eval::int_width_fault(db, ty_expr) {
        return Some((ty_expr, fault));
    }
    // Otherwise descend its type-argument positions. A type form is `(head arg…)`; the head is the ctor
    // (a name/prim), never a nested type, so skip child 0. A `(name Type)` record-field pair's TYPE is its
    // second child (skip the label at child 0 via the same skip-first rule, recursively).
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_ill_formed_int_width(db, child) {
            return Some(found);
        }
    }
    None
}

/// The `(Float W)` companion of [`nested_ill_formed_int_width`]: the position of an ill-formed float width
/// (outside the admitted IEEE set `{32,64}`, or non-natural) at `ty_expr` OR nested in one of its
/// type-argument positions (`(List (Float 8))`, `(Option (Float 16))`, a record field). `None` if every
/// float width in the type expression is admitted. Same skip-first descent as the integer helper (child 0
/// of a `(head arg…)` form is the ctor, never a nested type). Every ill-formed float width shares one
/// message, so this returns only the POSITION (to anchor the reject); the message is a constant.
fn nested_ill_formed_float_width(db: &mut Db, ty_expr: StructId) -> Option<StructId> {
    if crate::eval::is_ill_formed_float_width(db, ty_expr) {
        return Some(ty_expr);
    }
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_ill_formed_float_width(db, child) {
            return Some(found);
        }
    }
    None
}

pub(crate) const FLOAT_WIDTH_MESSAGE: &str =
    "a floating-point width must be one of the admitted IEEE widths (32 or 64)";

/// The UNBOUND-WIDTH companion of [`nested_ill_formed_int_width`]/[`nested_ill_formed_float_width`]: the
/// position of a width constructor `(Int W)`/`(UInt W)`/`(Float W)` whose width argument `W` is an UNBOUND
/// NAME (`(: a (Int hello))`), at `ty_expr` OR nested in one of its type-argument positions (`(List (Int
/// hello))`). `None` when no width position holds an unbound name. An unbound name in a WIDTH slot is not a
/// type (so the nested-type-var walk skips it) and reads as a non-constant width (so `int_width_fault`
/// waves it through as if it were a bound width variable), which let it slip past `cdz check` silently —
/// this closes that gap. Same skip-first descent as the sibling width walkers (child 0 of a `(head arg…)`
/// form is the ctor, never a nested type). A BOUND width variable (`(Int a)` with `a` a `Type`/width param)
/// is valid and returns `None` — `eval::unbound_width_arg` distinguishes the two by the arg's resolution.
fn nested_unbound_width(db: &mut Db, ty_expr: StructId) -> Option<(StructId, &'static str)> {
    if let Some(found) = crate::eval::unbound_width_arg(db, ty_expr) {
        return Some(found);
    }
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_unbound_width(db, child) {
            return Some(found);
        }
    }
    None
}

/// The CDZ0101 message for an UNBOUND NAME in a width position — `(: a (Int hello))` / `(Float hi)`. Names
/// the specific mistake (a width is a compile-time integer literal, not a name) and the repair: write the
/// literal, or the sized type directly. `example` is a ctor-appropriate sized type (`Int64`/`UInt64`/
/// `Float64`), so a `Float` width names a float example rather than the misleading `Int64`. The
/// width-position analogue of the lowercase-type-var guidance, but a width is not a type, so the fix is a
/// literal like `64`, not "leave the parameter unannotated".
pub(crate) fn unbound_width_message(name: &str, example: &str) -> String {
    format!(
        "unbound name `{name}` — a width must be a compile-time integer literal like `64`, not a name \
         (write the width literal, or the sized type `{example}` directly)"
    )
}

/// The CDZ0302 REPAIR for an ill-formed FLOAT width at `pos` — the actionable half of
/// [`FLOAT_WIDTH_MESSAGE`] (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix),
/// the float twin of [`ill_formed_int_width_fix`]. A CONCRETE natural width outside the admitted IEEE set
/// `{32, 64}` snaps to the nearest admitted width: a below-32 width (`(Float 8)`, `(Float 16)`) retypes to
/// `Float32`, and any wider non-admitted width (`(Float 48)`, `(Float 128)`) retypes to `Float64` — the
/// widest admitted precision (32 itself is admitted, so it never reaches here). A MALFORMED (negative /
/// non-numeric) width has NO width
/// number and no single confident target, so it carries the message alone (a false suggestion is worse
/// than none). Heuristic: the author may have meant a specific admitted width, but the snap clears the
/// fault in one shot and type-checks. `db` reads the concrete width off the annotation via
/// `eval::out_of_set_float_width`.
pub(crate) fn ill_formed_float_width_fix(db: &mut Db, pos: StructId) -> Option<Fix> {
    let w = crate::eval::out_of_set_float_width(db, pos)?;
    // A ZERO width (`(Float 0)`) reads as a dropped/mistyped number with no confident target — like the
    // integer twin, it stays message-only (a false suggestion is worse than none).
    if w == 0 {
        return None;
    }
    let target = if w < 32 { "Float32" } else { "Float64" };
    Some(Fix::replace_heuristic(pos, target))
}

/// The RUNTIME-WIDTH companion of [`nested_ill_formed_int_width`]/[`nested_ill_formed_float_width`]: the
/// position of a width-indexed numeric type `(Int n)`/`(UInt n)`/`(Float n)` whose width is RUNTIME DATA
/// (a parameter/ref) at `ty_expr` OR nested in one of its type-argument positions (`(List (Int n))`,
/// `(Option (Float n))`, a record field). `None` if no width in the type expression is runtime data. Same
/// skip-first descent as the ill-formed-width helpers. `is_runtime_width_type` (eval.rs) checks only the
/// TOP-LEVEL ctor, so a runtime width NESTED in a compound slipped past `cdz check` (rc=0) AND compiled —
/// a runtime value determining a type, which the type system forbids (numeric-model.md §An Integer/
/// Floating-Point Type Is Indexed By A Compile-Time Width). This closes that nested gap.
pub(crate) fn nested_runtime_width_type(db: &mut Db, ty_expr: StructId) -> Option<StructId> {
    if crate::eval::is_runtime_width_type(db, ty_expr) {
        return Some(ty_expr);
    }
    let crate::ast::Struct::List(children) = db.ast.get(ty_expr) else {
        return None;
    };
    for &child in children.clone().iter().skip(1) {
        if let Some(found) = nested_runtime_width_type(db, child) {
            return Some(found);
        }
    }
    None
}

/// The CDZ0302 out-of-range range-check EXTENDED through a COMPOUND value's payload/elements. The scalar
/// `literal_width_fault` above catches a top-level `(: 999 Int8)`, but a NESTED narrow-width literal — the
/// payload of `(: (Some 999) (Option Int8))`, an element of `(: (tuple 999) (Tuple Int8))`, a list element
/// of `(: (list 999) (List Int8))` — slipped through: the annotation's `Int8` propagates into the outer
/// value's type but the literal itself stays a deferred `Int64` (its own `type_of` reads `Int64`), so the
/// scalar fit-check never fires and `cdz check` ACCEPTED a value the declared type cannot hold (the emit
/// path DID catch it — a check-vs-emit gap). This walks the ANNOTATION's expected `Ty` (from `typeval_of`)
/// paired with the value's payload/element NODES, so each nested literal is fit-checked against the width
/// the annotation gives it — the range-check analogue of the annotation-descends-into-compound-payload
/// type check. Descends Sum (single-payload variant), Tuple, and List; a non-compound / mismatched shape
/// (reported by the ordinary type check) or a runtime (non-literal) payload adds nothing. Returns the FIRST
/// out-of-range nested literal's reject (anchored at that literal, so `cdz fix` targets it).
fn nested_literal_width_faults(db: &mut Db, value: StructId, ty_expr: StructId) -> Option<Reject> {
    let expected = crate::eval::typeval_of(db, ty_expr)?;
    nested_literal_width_faults_against(db, value, &expected)
}

/// The `&Ty`-typed core of [`nested_literal_width_faults`] — takes the already-resolved expected `Ty`
/// instead of an annotation NODE, so a collection builder-chain arm (a `Map.insert`/`Set.insert` operand)
/// can RECURSE into the operand collection against the SAME `Ty::Map`/`Ty::Set` without a type-expr node
/// to re-resolve. The public entry point resolves `ty_expr` once and delegates here.
fn nested_literal_width_faults_against(
    db: &mut Db,
    value: StructId,
    expected: &Ty,
) -> Option<Reject> {
    match expected {
        // A NARROW-INT annotation on a non-literal that `literal_width_fault` could not check directly — a
        // runtime `(if c 10000 0)` / `(match …)` annotated `(: … UInt8)`: the value is neither a
        // `Resolved::Int` nor a folding constant, so the scalar check above found nothing, yet each live
        // branch of the conditional carries the annotation's narrow width. Route it through
        // `width_fault_against_ty` (which descends a runtime `if` into both branches + range-checks the
        // narrow int). A bare out-of-range literal in a branch (`(: (if c 10000 0) UInt8)`) then rejects at
        // `check` as the emit path already does — closing the same check-vs-emit gap the compound arms close.
        Ty::Int(_) => width_fault_against_ty(db, value, expected),
        // A narrow `Float32` annotation on a non-literal `literal_width_fault` could not check directly — a
        // runtime `(if c 1.0e300 0.0)` / `(match …)` annotated `(: … Float32)`. Route it through
        // `width_fault_against_ty` (which descends the conditional's branches + applies the Float32-overflow
        // check to each branch literal). Without this, an overflowing branch literal slipped `cdz check`
        // while the emit path produced an INVALID module — the float sibling of the narrow-int gap.
        Ty::Float(_) => width_fault_against_ty(db, value, expected),
        // A single-payload variant `(Some 999)` : `(Option Int8)` — drill the payload arg against the
        // payload type at this sum's instantiation. (A multi-payload variant boxes its payloads as a tuple;
        // its single ctor arg is that tuple, handled by the Tuple arm once drilled — kept simple here to the
        // single-payload numeric case, the common one.)
        Ty::Sum { .. } => {
            let Resolved::Apply { head, args } = resolved_of(db, value) else {
                return None;
            };
            if crate::eval::variant_disc_of(db, head).is_none() || args.len() != 1 {
                return None;
            }
            let want = payload_ty_at_instantiation(db, head, expected)?;
            width_fault_against_ty(db, args[0], &want)
        }
        // A user-declared NOMINAL type — a newtype `(type W (W Int8))` (`inner` = the payload type) or a
        // multi-payload `(type P (P Int8 Int64))` (`inner` = a `Tuple` of the payloads). Its constructor
        // `(W 999)` / `(P 999 5)` resolves as `Apply(ctor, [payload args])`; without this arm a bare
        // over-range payload literal escaped the fit-check → wasm SILENTLY TRUNCATED it (999 → -25; rust
        // E0308) — the nominal face of the Option/Record/Map cases. Descend each ctor arg against the
        // matching `inner` type: a Tuple `inner` zips positionally (multi-payload), else the single arg
        // against `inner`. (A user MULTI-VARIANT sum is `Ty::Sum` and takes the Sum arm above.)
        Ty::Nominal { inner, .. } => {
            let Resolved::Apply { head, args } = resolved_of(db, value) else {
                return None;
            };
            crate::eval::variant_disc_of(db, head)?;
            match &**inner {
                Ty::Tuple(elem_tys) => elem_tys
                    .iter()
                    .zip(args.iter())
                    .find_map(|(t, &a)| width_fault_against_ty(db, a, t)),
                single => args
                    .first()
                    .and_then(|&a| width_fault_against_ty(db, a, single)),
            }
        }
        // A tuple `(tuple 999 …)` : `(Tuple Int8 …)` — each element against its element type.
        Ty::Tuple(elem_tys) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::TupleNew)?;
            elem_tys
                .iter()
                .zip(elems.iter())
                .find_map(|(t, &e)| width_fault_against_ty(db, e, t))
        }
        // A list `(list 999 …)` : `(List Int8)` — each element against the element type (homogeneous).
        Ty::List(elem_ty) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::ListNew)?;
            let elem_ty = (**elem_ty).clone();
            elems
                .iter()
                .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
        }
        // A record `(record (x 999) …)` : `(Record (: x Int8) …)` — each field value against its DECLARED
        // field type. Without this a bare `999` in an `Int8` field escaped the fit-check → wasm silently
        // TRUNCATED it (999 → -25) while rust rejected E0308 (a backend-divergent SILENT MISCOMPILE, the
        // record face of the Option/Tuple payload cases). The record value's fields are keyed by symbol
        // (`Resolved::Record`); pair each declared field type with its value node by name.
        Ty::Record(field_tys) => {
            // A record literal resolves either as a folded `Resolved::Record` OR (the common case) an
            // `Apply(RecordNew, [(key value)…])` name-alias — read the field value nodes by symbol from
            // whichever shape, like `positional_value_nodes` unifies the Tuple/List Apply cases.
            let fields = match resolved_of(db, value) {
                Resolved::Record { fields } => (*fields).clone(),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::RecordNew) =>
                {
                    crate::resolve::read_record_fields(db, &args).ok()?
                }
                _ => return None,
            };
            field_tys.iter().find_map(|(sym, t)| {
                fields
                    .get(sym)
                    .and_then(|&v| width_fault_against_ty(db, v, t))
            })
        }
        // A map `(map (k v) …)` : `(Map Int8 Int64)` — each KEY literal against the key type + each VALUE
        // literal against the value type. Both positions escaped the fit-check: `(: (map (1 999)) (Map
        // Int64 Int8))` silently TRUNCATED the value (999 → -25 on lookup), and `(: (map (999 1)) (Map Int8
        // Int64))` accepted an out-of-range key. Descend the entry key/value nodes (paired, in order) each
        // against its declared side. The map value's entries are `(key value)` occurrence pairs.
        Ty::Map(key_ty, val_ty) => {
            let (key_ty, val_ty) = ((**key_ty).clone(), (**val_ty).clone());
            // A map literal resolves as a folded `Resolved::Map { entries }` OR an `Apply(MapNew, [(k v)…])`
            // name-alias; read the `(key value)` node pairs from whichever shape (each Apply arg is a
            // two-element `(key value)` list, as `resolve_map` reads them). A map BUILT by a `Map.insert`
            // chain (`Apply(MapInsert, [map, key, val])`, bottoming at `Map.empty`) is NOT a literal — its
            // key/value literals escaped this check entirely, so an out-of-range literal fed through
            // `(Map.insert Map.empty k 200) : (Map Int64 Int8)` compiled clean AND ran to a truncated -56
            // (a silent miscompile — the builder-chain face of the map-literal case). Walk the insert chain
            // too: range-check this insert's key/value args, then recurse into the operand map.
            match resolved_of(db, value) {
                Resolved::Map { entries } => entries.to_vec().iter().find_map(|&(k, v)| {
                    width_fault_against_ty(db, k, &key_ty)
                        .or_else(|| width_fault_against_ty(db, v, &val_ty))
                }),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::MapNew) =>
                {
                    // Each `MapNew` arg is a map ENTRY. Read `(key, value)` from the native `(= k v)`
                    // FieldPair leaf (M2, what the reader emits for a `#map`/`(map (= k v))` entry), the
                    // transitional name-head `(= k v)`, OR the legacy 2-element `(k v)` pair. Before this
                    // only the 2-element pair was read, so a native FieldPair entry `(map (= 1 999))` fed
                    // through the name-alias `MapNew` path was skipped → its out-of-range value/key literal
                    // escaped CDZ0302 and silently truncated (the map face of the native-leaf descent gap).
                    args.iter()
                        .filter_map(|&entry| {
                            db.ast
                                .field_pair_parts(entry)
                                .or_else(|| db.ast.field_pair(entry))
                                .or_else(|| match db.ast.get(entry) {
                                    crate::ast::Struct::List(items) if items.len() == 2 => {
                                        Some((items[0], items[1]))
                                    }
                                    _ => None,
                                })
                        })
                        .collect::<Vec<_>>()
                        .iter()
                        .find_map(|&(k, v)| {
                            width_fault_against_ty(db, k, &key_ty)
                                .or_else(|| width_fault_against_ty(db, v, &val_ty))
                        })
                }
                // `(Map.insert <map> <key> <val>)` — check this entry's key + value, then recurse the
                // operand map (`Map.empty` bottoms out as a non-insert with no literal → None).
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::MapInsert)
                        && args.len() == 3 =>
                {
                    width_fault_against_ty(db, args[1], &key_ty)
                        .or_else(|| width_fault_against_ty(db, args[2], &val_ty))
                        .or_else(|| nested_literal_width_faults_against(db, args[0], expected))
                }
                _ => None,
            }
        }
        // A set BUILT by `Set.of (list …)` or a `Set.insert` chain : `(Set Int8)` — each element literal
        // against the element type. Previously there was NO `Ty::Set` arm at all, so an out-of-range set
        // element escaped the fit-check on both `check` and `emit` (the set face of the map builder-chain
        // silent miscompile). `Set.of list` (single list arg) descends the list elements; `Set.insert set
        // elem` checks the inserted element then recurses the operand set (`Set.empty` bottoms out).
        Ty::Set(elem_ty) => {
            let elem_ty = (**elem_ty).clone();
            match resolved_of(db, value) {
                // A native `#set(e…)` / `("set" e…)` LITERAL resolves to `Resolved::Set { elems }` (the
                // first-class set ctor). Before this arm it fell through to `_ => None`, so an out-of-range
                // set-literal element `(: #set(200) (Set Int8))` escaped CDZ0302 and silently truncated
                // (the set-literal face of the native-leaf descent gap; the `Set.of`/`Set.insert` builder
                // chains below were already covered).
                Resolved::Set { elems } => elems
                    .to_vec()
                    .iter()
                    .find_map(|&e| width_fault_against_ty(db, e, &elem_ty)),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::SetOf)
                        && args.len() == 1 =>
                {
                    positional_value_nodes(db, args[0], crate::resolved::Prim::ListNew)?
                        .iter()
                        .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
                }
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::SetInsert)
                        && args.len() == 2 =>
                {
                    width_fault_against_ty(db, args[1], &elem_ty)
                        .or_else(|| nested_literal_width_faults_against(db, args[0], expected))
                }
                _ => None,
            }
        }
        // A quantity `(Qty.of 300 kilometer)` : `(Qty UInt8 meter)` — drill the MAGNITUDE against the
        // annotation's INNER numeric type. A quantity annotation checks the dimension (not the scale, so
        // km may be annotated at meter), but it STILL grounds + range-checks the inner numeric type exactly
        // as a bare `(: 300 UInt8)` does — otherwise a quantity-wrapped literal slips its width entirely
        // (the annotation's `Ty::Qty` arm in `type_of` keeps the expression's own type to avoid the scale
        // rebrand, so the inner width never grounds/checks; this restores the check at the same choke point
        // the compound-payload cases use). The magnitude is the `Qty.of` value occurrence; range-check it
        // against the annotation's `inner`. This covers both a same-unit and a same-dimension different-
        // scale annotation uniformly. `Unit.in`'s bare-number result is not a `Ty::Qty` and is unaffected.
        Ty::Qty { inner, .. } => {
            let magnitude = crate::eval::qty_value_occ(db, value)?;
            width_fault_against_ty(db, magnitude, inner)
        }
        _ => None,
    }
}

/// Range-check the value node `value` (a literal, a folded constant, or a compound to recurse into)
/// against the EXPECTED type `want` — the `Ty`-typed core of the nested width check. A narrow-integer
/// `want` fit-checks a constant `value` (the same fold `literal_width_fault` runs); any other `want`
/// recurses through the compound if `value` is one. Returns the first out-of-range literal's reject.
fn width_fault_against_ty(db: &mut Db, value: StructId, want: &Ty) -> Option<Reject> {
    // A RUNTIME `(Option.expect s "…")` / `(Result.expect …)` in a narrow-width context: `expect` PROJECTS
    // the sum's payload, so the annotation's `want` is the payload type — descend into the sum argument
    // against `Option<want>` (the payload arg substituted to `want`). Without this, `(: (Option.expect (if c
    // (Some 10000) None) "x") UInt8)` was a SILENT MISCOMPILE: a CONSTANT `(Some 10000)` FOLDS (`expect`
    // reduces to the payload `10000`, caught by the scalar check below), but a RUNTIME sum (here an `if`
    // returning `Some`) does NOT fold → the value stays a runtime `SumExpect` call, no literal to check, and
    // EMIT truncated `10000` to `16` (a `wrap` on the projected payload) with NO diagnostic on either side.
    // Rebuilding the sum's type with `want` as its payload arg + descending routes the `if`-branch `(Some
    // 10000)` through the Sum arm of `nested_width_fault_by_ty`, which range-checks `10000` against `want`.
    if let Resolved::Apply { head, args } = resolved_of(db, value)
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::SumExpect)
        && let Some(&sum_arg) = args.first()
        && let Ty::Sum {
            decl,
            args: sum_args,
        } = type_of(db, sum_arg)
        && !sum_args.is_empty()
    {
        // `expect` projects the present variant's payload, whose type is the sum's FIRST type arg — the
        // `Some a` of `Option a` (1 arg) AND the `Ok a` of `Result a e` (2 args, payload is arg 0). So
        // substitute `want` for arg 0 only, leaving any others (Result's error type) as-is.
        let mut new_args: Vec<Ty> = sum_args.iter().cloned().collect();
        new_args[0] = want.clone();
        let payload_sum = Ty::Sum {
            decl,
            args: std::rc::Rc::from(new_args),
        };
        return width_fault_against_ty(db, sum_arg, &payload_sum);
    }
    // A RUNTIME conditional `(if c a b)` in a narrow-width context: the WHOLE `if` carries the expected
    // type `want`, so BOTH of its live branches must fit `want` — each branch is a value the annotation's
    // width applies to. Without this a bare out-of-range literal in a branch (`(: (if c 10000 0) UInt8)`,
    // or the same reaching a narrow PARAMETER) slipped through `cdz check` because a runtime `if` folds to
    // neither a `Resolved::Int` nor a `Core::ConstInt` (the narrow-int block below then reads `v = None`
    // and returns), while the EMIT path DID reject it (CDZ0302) — a check-vs-emit gap. Descend into each
    // branch here so `check` catches it at the same choke point the compound-payload cases use. A CONSTANT-
    // condition `if` is NOT descended: `core_of` folds it to its taken branch (handled by the constant path
    // below), so a `Core::If` result marks a genuine runtime conditional with both branches live — checking
    // both is sound (neither is dead), whereas descending a folded `if` would falsely reject a dead untaken
    // out-of-range branch.
    if matches!(
        crate::lower::core_of(db, value),
        crate::core::Core::If { .. }
    ) && let Resolved::If { then_, else_, .. } = resolved_of(db, value)
    {
        return width_fault_against_ty(db, then_, want)
            .or_else(|| width_fault_against_ty(db, else_, want));
    }
    // A RUNTIME `(match s (p0 b0) …)` in a narrow-width context — the same rule as the `if` above, one
    // body per arm: the whole `match` carries `want`, so EVERY arm body must fit it, and a bare
    // out-of-range literal in any arm (`(: (match n (0 10000) (_ 0)) UInt8)`, or reaching a narrow param)
    // slipped `cdz check` while emit rejected CDZ0302. A `Core::Match` after `core_of` marks a genuine
    // RUNTIME match (all arms live) — a CONSTANT-scrutinee match folds to its selected arm (handled by the
    // constant path below), so a folded-away non-selected out-of-range arm is not falsely rejected. Descend
    // each arm's BODY against `want` (a pattern binder in the body is fine — the width check reads the
    // body's constant leaves, exactly as an if-branch).
    if matches!(
        crate::lower::core_of(db, value),
        crate::core::Core::Match { .. }
    ) && let Resolved::Match { arms, .. } = resolved_of(db, value)
    {
        return arms
            .iter()
            .find_map(|&(_pattern, body)| width_fault_against_ty(db, body, want));
    }
    // A FLOAT literal that overflows a narrow `Float32` `want` — the float analogue of the narrow-int
    // block below, reached here (not only by `literal_width_fault`'s direct-literal check) so it fires
    // through the runtime `if`/`match` descent above: `(: (if c 1.0e300 0.0) Float32)` PASSED `cdz
    // check` while the emit path produced an INVALID module (the branch literal is `±inf` in Float32, a
    // malformed value with no written form). Only `Float32` is narrow enough to overflow a finite `Float64`
    // literal; the retype-to-`Float64` fix is not offered here (no `ty_expr` for a nested/branch position,
    // like the nested-int case). Reuses `dec.fits_f32()` — the same predicate `literal_width_fault` runs.
    if let Ty::Float(ft) = want
        && ft.ground_width() == 32
    {
        // The overflowing float `Decimal`, whether `value` is a DIRECT float-literal atom OR a value that
        // FOLDS to a constant float — a CONST-condition `(if true 1.0e300 0.5)` reduces via `core_of` to
        // `Core::ConstFloat(1.0e300)`, materializing the malformed `inf` that a runtime `if` (handled by the
        // descent above) would reject; without reading the fold, the const-fold path slipped `check` and
        // COMPILED + ran to `inf`. (A runtime `if` is a `Core::If`, taken by the descent arm above, not here.)
        let dec = match db.ast.get(value) {
            crate::ast::Struct::Atom(lid) => match db.ast.leaf(*lid).clone() {
                crate::ast::Leaf::Float(dec) => Some(dec),
                _ => None,
            },
            _ => match crate::lower::core_of(db, value) {
                crate::core::Core::ConstFloat(dec) => Some(dec),
                _ => None,
            },
        };
        if let Some(dec) = dec
            && !dec.fits_f32()
        {
            return Some(
                Reject::coded(
                    Code::IntOutOfRange,
                    "float literal does not fit the annotated type Float32 (it overflows the Float32 \
                     range to infinity — the largest finite Float32 is about 3.4e38)",
                )
                .at(value),
            );
        }
    }
    if let Ty::Int(it) = want
        && let crate::ty::Width::Fixed(w) = it.width
        && w != 0
        && !matches!(type_of(db, value), Ty::BigInt | Ty::Rational)
    {
        let v = match resolved_of(db, value) {
            Resolved::Int(v) => Some(v),
            _ => match crate::lower::core_of(db, value) {
                crate::core::Core::ConstInt(v) => Some(v),
                _ => None,
            },
        };
        if let Some(v) = v
            && !v.fits_width(it.ground_signed(), w)
        {
            // Anchor at the offending literal node. The width came from a SOLVED element/payload `Ty` (an
            // enclosing compound annotation, or — via the list arms — a SIBLING literal's annotation), NOT a
            // written sub-annotation on THIS literal, so there is no type-node to retype. `int_out_of_range_reject`
            // would attach `Fix::replace_heuristic(<literal>, "Int16")` — rewriting the VALUE `-41` into a TYPE
            // name (`(list (: 1 UInt64) Int8)`), a source-corrupting suggestion. So build the reject WITHOUT a
            // fix: the message names the valid range, which is the actionable fact (the direct-annotation
            // callers `(: v T)` / `((: name T) v)` DO have a type-node and keep their retype fix). See the
            // `int_out_of_range_reject` doc.
            return Some(
                Reject::coded(
                    Code::IntOutOfRange,
                    int_out_of_range_message(want, it.ground_signed(), w, &db.name_ctx()),
                )
                .at(value),
            );
        }
        return None;
    }
    // Not a narrow int `want` — recurse if the value is itself a compound whose expected shape is `want`.
    // (A nested `(Some (tuple 999))` : `(Option (Tuple Int8))` descends Sum → Tuple.)
    nested_width_fault_by_ty(db, value, want)
}

/// The `Ty`-driven twin of `nested_literal_width_faults`'s descent (which is `ty_expr`-driven at the top):
/// descend a compound `value` against an already-solved expected `Ty`. Sum/Tuple/List, single-payload.
fn nested_width_fault_by_ty(db: &mut Db, value: StructId, want: &Ty) -> Option<Reject> {
    match want {
        Ty::Sum { .. } => {
            let Resolved::Apply { head, args } = resolved_of(db, value) else {
                return None;
            };
            if crate::eval::variant_disc_of(db, head).is_none() || args.len() != 1 {
                return None;
            }
            let inner = payload_ty_at_instantiation(db, head, want)?;
            width_fault_against_ty(db, args[0], &inner)
        }
        Ty::Tuple(elem_tys) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::TupleNew)?;
            elem_tys
                .iter()
                .zip(elems.iter())
                .find_map(|(t, &e)| width_fault_against_ty(db, e, t))
        }
        Ty::List(elem_ty) => {
            let elems = positional_value_nodes(db, value, crate::resolved::Prim::ListNew)?;
            let elem_ty = (**elem_ty).clone();
            elems
                .iter()
                .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
        }
        // A RECORD value against a `(Record …)` expected type — each declared field's type applies to its
        // value node, keyed by symbol. The `Ty`-driven twin of the `ty_expr`-driven Record arm in
        // `nested_literal_width_faults_against`: without it a narrow FIELD literal fed through a compound
        // op ARGUMENT — `(Send.put (record (small 999) (big 5)))` for `(-> (Record (small UInt8) …) …)` —
        // escaped the fit-check (the tuple/list element arms above already reach their elements, but a
        // record row was not descended), so `999` inhabited the `UInt8` field and the arm OBSERVED it
        // (breaker nc-t3, the record face of the nw-class op-arg soundness gap). Read the field value nodes
        // by symbol from whichever record shape (a folded `Resolved::Record` or a `RecordNew` name-alias),
        // exactly as the `ty_expr` descent does.
        Ty::Record(field_tys) => {
            let fields = match resolved_of(db, value) {
                Resolved::Record { fields } => (*fields).clone(),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::RecordNew) =>
                {
                    crate::resolve::read_record_fields(db, &args).ok()?
                }
                _ => return None,
            };
            field_tys.iter().find_map(|(sym, t)| {
                fields
                    .get(sym)
                    .and_then(|&v| width_fault_against_ty(db, v, t))
            })
        }
        // A MAP value against a `(Map K V)` expected type — each entry key literal against `K`, each value
        // literal against `V`. The `Ty`-driven twin of the `ty_expr` Map arm; the map face of the same
        // compound-op-argument gap (a `(-> (Map … UInt8) …)` op arg with an out-of-range value literal).
        Ty::Map(key_ty, val_ty) => {
            let (key_ty, val_ty) = ((**key_ty).clone(), (**val_ty).clone());
            match resolved_of(db, value) {
                Resolved::Map { entries } => entries.to_vec().iter().find_map(|&(k, v)| {
                    width_fault_against_ty(db, k, &key_ty)
                        .or_else(|| width_fault_against_ty(db, v, &val_ty))
                }),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::MapNew) =>
                {
                    // Read `(key, value)` from a native `(= k v)` FieldPair entry (M2) as well as the legacy
                    // 2-element `(k v)` pair — see the `ty_expr`-driven twin in
                    // `nested_literal_width_faults_against`.
                    args.iter()
                        .filter_map(|&entry| {
                            db.ast
                                .field_pair_parts(entry)
                                .or_else(|| db.ast.field_pair(entry))
                                .or_else(|| match db.ast.get(entry) {
                                    crate::ast::Struct::List(items) if items.len() == 2 => {
                                        Some((items[0], items[1]))
                                    }
                                    _ => None,
                                })
                        })
                        .collect::<Vec<_>>()
                        .iter()
                        .find_map(|&(k, v)| {
                            width_fault_against_ty(db, k, &key_ty)
                                .or_else(|| width_fault_against_ty(db, v, &val_ty))
                        })
                }
                _ => None,
            }
        }
        // A SET value against a `(Set E)` expected type — each element literal against `E`. The `Ty`-driven
        // twin of the `ty_expr` Set arm; without it a native `#set(e…)` literal (`Resolved::Set`) fed through
        // a compound-op ARGUMENT with an out-of-range element escaped the fit-check (the set face of the
        // compound-op-argument narrow-width soundness gap, mirroring the Map/Record arms above).
        Ty::Set(elem_ty) => {
            let elem_ty = (**elem_ty).clone();
            match resolved_of(db, value) {
                Resolved::Set { elems } => elems
                    .to_vec()
                    .iter()
                    .find_map(|&e| width_fault_against_ty(db, e, &elem_ty)),
                Resolved::Apply { head, args }
                    if crate::eval::meta_apply_of(db, head)
                        == Some(crate::resolved::Prim::SetOf)
                        && args.len() == 1 =>
                {
                    positional_value_nodes(db, args[0], crate::resolved::Prim::ListNew)?
                        .iter()
                        .find_map(|&e| width_fault_against_ty(db, e, &elem_ty))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The CDZ0302 reject for an integer literal `v` that overflows the `(signed, width)` type `annot_ty`,
/// carrying — when possible — a retype fix: replace the annotation `ty_expr` with the SMALLEST aliased
/// width ({8,16,32,64}) that DOES fit `v`, the rustc-style "value doesn't fit; use a type that holds it"
/// repair (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Two shapes.
/// SAME-SIGNEDNESS WIDEN: a magnitude too large for the width (`(: 999 Int8)` → `Int16`, `(: 70000
/// UInt8)` → `UInt32`) takes the smallest wider width of the SAME sign. SIGN FLIP: a NEGATIVE literal in
/// an UNSIGNED type (`(: -5 UInt8)` → `Int8`) takes the smallest SIGNED width holding `v` — no unsigned
/// type can EVER hold a negative value, so the fit is UNAMBIGUOUS (rustc makes exactly this suggestion);
/// this is NOT a speculative signedness guess, since a negative literal has no unsigned reading, so the
/// signed type is forced, not chosen.
/// A value beyond `Int64`/`UInt64` (no aliased width fits) retypes to `BigInt` — the unbounded integer
/// type holds any magnitude. Replacing the whole `ty_expr` rewrites either spelling — a bare `Int8` or a
/// `(Int 8)` compound — to the bare `Int16` (or `BigInt`).
/// Heuristic: the retype clears the range fault, but whether the author meant a wider/signed type (vs. a
/// different literal) is theirs to confirm. Shared by both CDZ0302 literal-range sites (the value
/// annotation `(: v T)` and the let-binder/param `((: name T) v)`), so both carry the fix.
///
/// WARNING: `ty_expr` MUST be a written TYPE node (the annotation being retyped) — the fix replaces its spelling
/// with a type name. NEVER pass a VALUE node here: a literal whose width came from a solved/inferred `Ty`
/// (a nested compound payload, or a sibling literal's annotation) has no type-node to retype, and
/// rewriting the literal into a type name corrupts the source. Those sites build the reject directly
/// (message + `.at(value)`, no fix) — see `width_fault_against_ty`'s narrow-int arm.
fn int_out_of_range_reject(
    annot_ty: &Ty,
    signed: bool,
    w: u32,
    v: &crate::ast::IntValue,
    ty_expr: StructId,
    ncx: &NameCtx,
) -> Reject {
    let reject = Reject::coded(
        Code::IntOutOfRange,
        int_out_of_range_message(annot_ty, signed, w, ncx),
    );
    // A NEGATIVE literal annotated with an UNSIGNED type cannot fit ANY unsigned width — the value is
    // negative, so only a SIGNED type reads it. Offer the smallest signed width that holds it (forced, not
    // guessed). Otherwise widen within the SAME signedness (the ordinary magnitude-too-large case).
    let (fix_signed, search_from) = if !signed && v.negative {
        (true, 0) // any signed width may fit; search all aliased widths
    } else {
        (signed, w) // widen: strictly larger widths of the same sign
    };
    match crate::ty::ALIASED_INT_WIDTHS
        .iter()
        .copied()
        .filter(|&aw| aw > search_from)
        .find(|&aw| v.fits_width(fix_signed, aw))
    {
        Some(fit) => {
            let stem = if fix_signed { "Int" } else { "UInt" };
            reject.with_fix(Fix::replace_heuristic(ty_expr, format!("{stem}{fit}")))
        }
        // No fixed width (8/16/32/64) holds `v` — the literal overflows even `Int64`/`UInt64`. The UNBOUNDED
        // integer type `BigInt` holds ANY magnitude (a literal grounds to it losslessly), so it is the
        // forced retype when no aliased width fits — the rustc-gold "use a type that holds it" repair
        // continued past the fixed widths. Heuristic (the author may instead have meant a different literal),
        // but the retype clears the range fault in one shot and always type-checks.
        None => reject.with_fix(Fix::replace_heuristic(ty_expr, "BigInt")),
    }
}

/// The CDZ0302 message for an integer literal that overflows the annotated type: names the type and,
/// when the width is a well-formed one whose range renders exactly, appends `(the valid range is
/// min..=max)`. A malformed width (no exact range) falls back to the type-name-only message.
fn int_out_of_range_message(annot_ty: &Ty, signed: bool, w: u32, ncx: &NameCtx) -> String {
    match int_width_range(signed, w) {
        Some(range) => format!(
            "integer literal does not fit the annotated type {} (the valid range is {range})",
            annot_ty.render_name(ncx),
        ),
        None => format!(
            "integer literal does not fit the annotated type {}",
            annot_ty.render_name(ncx)
        ),
    }
}

/// The inclusive value range a `(signed, width)` integer type holds, rendered `min..=max` (rustc's
/// "the range is `-128..=127`" phrasing) — a signed N-bit holds `-(2^(N-1)) ..= 2^(N-1) - 1`, an
/// unsigned N-bit `0 ..= 2^N - 1`. Names the concrete bounds a CDZ0302 out-of-range literal missed, so
/// the message says WHICH range rather than only the type name. Returns `None` for a width the `i128`/
/// `u128` arithmetic can't hold exactly (`w == 0`, or `> 127` signed / `> 128` unsigned — only a
/// MALFORMED width, since a well-formed integer type is `1..=64`); the caller then omits the range
/// clause rather than the helper panicking on a shift overflow.
pub(crate) fn int_width_range(signed: bool, w: u32) -> Option<String> {
    if w == 0 {
        return None;
    }
    if signed {
        if w > 127 {
            return None;
        }
        let max = (1i128 << (w - 1)) - 1;
        let min = -(1i128 << (w - 1));
        Some(format!("{min}..={max}"))
    } else {
        if w > 128 {
            return None;
        }
        // `1u128 << 128` overflows; `w == 128` max is `u128::MAX` directly.
        let max = if w == 128 {
            u128::MAX
        } else {
            (1u128 << w) - 1
        };
        Some(format!("0..={max}"))
    }
}

/// The GERUND naming the additive/relational operation a CDZ0501 dimensional fault arose in — "adding",
/// "subtracting", "comparing" — so the message reads as an action ("adding quantities of incompatible
/// dimension"). `prim` is the operator's [`crate::resolved::Prim`] (the `is_additive` set: `+`/`-` plus
/// the comparisons); anything else (or an unrecognized head) falls back to the neutral "combining".
/// The prelude OPERATION-MODULE name for a collection or text type — `List`/`Map`/`Set`/`String`/`Bytes`
/// — whose fields are its operations (reached by member access `(. List at)`). Used to redirect a NAMED
/// member access on such a value (`(. xs foo)`, which is not a field read — these are not records) to the
/// module operation form. `None` for a type with no such operation module (a record has real fields, a
/// tuple is positional, a scalar/sum has no member-access operations). The module name matches the type's
/// own render (`List`/`Map`/`Set`/`String`/`Bytes`), so `(. <Module> <op>)` names a real prelude module.
fn collection_or_text_module(ty: &Ty) -> Option<&'static str> {
    match ty {
        Ty::List(_) => Some("List"),
        Ty::Map(..) => Some("Map"),
        Ty::Set(_) => Some("Set"),
        Ty::String => Some("String"),
        Ty::Bytes => Some("Bytes"),
        _ => None,
    }
}

/// A `(match <value> …)` TEMPLATE for a SUM value member-accessed by name — `(. o foo)` on an `(Option
/// …)`, `(. p x)` on a user sum. A sum's payload is not a field: it is reached by MATCHING each variant.
/// Spells one arm per variant with a `…` body, so the reader sees the shape to write — `(match <value>
/// ((Some x) …) ((None) …))`. Each arm binds a fresh `x0`/`x1`/… per payload slot (the arity from the
/// variant's payload count) so a payload-carrying variant shows its binders. `None` for a sum whose decl
/// is unknown (no variant set to spell) or a sum with no variants. Reads the variant set off the type's
/// declaration (`ty::Sum { decl }`), so it names THIS sum's real variants.
fn sum_match_hint(db: &mut Db, ty: &Ty) -> Option<String> {
    let Ty::Sum { decl, .. } = ty else {
        return None;
    };
    let decl = *decl;
    let variants: Vec<(String, usize)> = db
        .type_decl_by_occ(decl)?
        .variants
        .iter()
        .map(|v| (v.name.clone(), v.payloads.len()))
        .collect();
    if variants.is_empty() {
        return None;
    }
    let arms: Vec<String> = variants
        .iter()
        .map(|(name, arity)| {
            if *arity == 0 {
                format!("(({name}) …)")
            } else {
                let binders: Vec<String> = (0..*arity).map(|i| format!("x{i}")).collect();
                format!("(({name} {}) …)", binders.join(" "))
            }
        })
        .collect();
    Some(format!("(match <value> {})", arms.join(" ")))
}

fn additive_op_gerund(prim: Option<crate::resolved::Prim>) -> &'static str {
    use crate::resolved::Prim;
    match prim {
        Some(Prim::Add) => "adding",
        Some(Prim::Sub) => "subtracting",
        Some(Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq | Prim::Compare) => "comparing",
        _ => "combining",
    }
}

/// The INNER-VALUE argument node of a `(Qty.of <value> <unit>)` application — the `<value>` a coercion fix
/// retypes when two quantities of one dimension have mismatched inner numeric types (`(Qty.of 5 m) +
/// (Qty.of 3.0 m)` → retype the `5`). `None` when `node` is not a directly-written `Qty.of` application
/// (a quantity bound to a variable / returned from a call has no inner-value node to edit here).
fn qty_of_value_arg(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyOf)
                && args.len() == 2 =>
        {
            Some(args[0])
        }
        _ => None,
    }
}

/// The NOUN for the same operation — "addition", "subtraction", "comparison" — used in the "… requires
/// equal dimensions" clause of a CDZ0501 message. Falls back to "this operation".
fn additive_op_noun(prim: Option<crate::resolved::Prim>) -> &'static str {
    use crate::resolved::Prim;
    match prim {
        Some(Prim::Add) => "addition",
        Some(Prim::Sub) => "subtraction",
        Some(Prim::Lt | Prim::Gt | Prim::Le | Prim::Ge | Prim::Eq | Prim::Compare) => "comparison",
        _ => "this operation",
    }
}

/// `""` for exactly one, `"s"` otherwise — the plural suffix for a count in a diagnostic.
fn plural_s(n: u32) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// "N open bit(s)" — the count of bit-field bits accumulated since the last byte boundary, for a
/// CDZ0220 byte-alignment message.
fn open_bits_phrase(bits: u32) -> String {
    let open = bits % 8;
    format!("{open} open bit{}", plural_s(open))
}

/// The actionable "add N more bit(s) to reach K byte(s)" hint for a CDZ0220 byte-alignment fault — how
/// far the running bit-cursor is from the next byte boundary, and the byte count once closed. `bits` is
/// the total accumulated bit-field width; only called when it is NOT a whole number of bytes. When there
/// is a lower byte boundary above zero (`bits >= 8`), the alternative "drop M bits" is offered too; a
/// sub-byte total (`bits < 8`) omits it (dropping would reach zero bytes — not useful advice).
fn bits_to_byte_boundary_hint(bits: u32) -> String {
    let pad = (8 - (bits % 8)) % 8;
    let bytes = bits.div_ceil(8);
    let over = bits % 8;
    let add = format!(
        "add {pad} more bit{} to reach {bytes} byte{}",
        plural_s(pad),
        plural_s(bytes)
    );
    if bits >= 8 {
        format!(
            "{add} (or drop {over} bit{} to the previous boundary)",
            plural_s(over)
        )
    } else {
        add
    }
}

/// The declared type of an annotated parameter whose NAME occurrence is `binder`, if any. A parameter
/// is annotated when its name sits in a `(: name T)` binder (the name's parent is that form); the type
/// is `T` reduced to a `Ty` by the evaluator (`typeval_of`). `None` for a bare (unannotated) parameter
/// or an unreducible annotation type — in which case the parameter's type is left open (`Any`).
///
/// This binder is the BIDIRECTIONAL boundary where first-class types meet inference: the parameter's type
/// is SYNTHESIZED by reducing the annotation type-value (`typeval_of` — monomorphization from the concrete
/// type supplied, e.g. `(: t Type)` → `Ty::Type` for a type-valued parameter), not solved by unification —
/// so a first-class computable type is reconciled with principal-type inference rather than contradicting it.
//= spec/capabilities/type-system.md#inference-and-first-class-types-meet-at-a-bidirectional-boundary
//# A position that binds a type-valued parameter MUST be a bidirectional-checking boundary, at which a type is either synthesized by monomorphization from the concrete type-value supplied or checked against an explicit annotation, rather than solved by unification, so that first-class computable types are reconciled with principal-type inference instead of contradicting it.
fn param_annot_ty(db: &mut Db, binder: StructId) -> Option<Ty> {
    let parent = db.parent_of(binder)?;
    let tail = db.ast.as_form(parent, ":")?;
    // The binder must be the NAME position (first) of the `(: name T)`, not the type position.
    if tail.first().copied() != Some(binder) {
        return None;
    }
    let ty_expr = *tail.get(1)?;
    crate::eval::typeval_of(db, ty_expr)
}

/// The "type position holds a non-type" message for a `ty_expr` that `typeval_of` rejected and whose own
/// `collect` surfaced no fault (so it is bound / well-formed, not an unbound-name typo). When `ty_expr` is
/// a bare NAME, it is a bound VALUE (a def / parameter / prelude value — a type name would have made
/// `typeval_of` succeed), so name it: "`helper` is a value, not a type — a type belongs here (annotate
/// `(: value Int64)`)". This is the type-position analogue of the apply-position category message (M76):
/// a bound name misused as a type gets NAMED, not the opaque "found a non-type". A NON-name operand (a
/// literal `5`, a compound `(+ 1 2)`) keeps the generic phrasing — naming a literal adds nothing. `lead`
/// prefixes the sentence ("a parameter's annotation" / "the type position of an annotation").
fn non_type_annotation_message(db: &mut Db, ty_expr: StructId, lead: &str) -> String {
    let Some(name) = db.ast.as_name(ty_expr).map(str::to_string) else {
        // A NON-name operand (a literal `5`, a compound `(+ 1 2)`) — naming a literal adds nothing.
        return format!("{lead} requires a type, but found a non-type");
    };
    // Compute each classifier ONCE (each projects the resolved binding's metadata), then branch on the
    // result — a bare name is at most ONE of these categories, so the checks are mutually exclusive.
    if let Some((ctor, placeholder)) = bare_type_ctor_needs_argument(db, ty_expr) {
        // A bare type-CONSTRUCTOR name used with NO argument — `(: xs List)`, `(: m Map)`. `List` IS a type
        // (constructor), so "is a value, not a type" misleads; name the missing argument + the fix, the
        // bare-name twin of the `(List Int64 Int64)` wrong-arity message.
        format!(
            "`{ctor}` is a type constructor — it needs a type argument here, e.g. `({ctor} {placeholder})`"
        )
    } else if let Some((default, widths)) = bare_width_ctor_default_type(db, ty_expr) {
        // A bare WIDTH-FAMILY value ctor — `Int` / `UInt` / `Float`. It builds a SIZED type from a width
        // (`(Int 64)` ≡ `Int64`), so a bare `Int` is a value; name the concrete sized default the author
        // almost certainly meant + list the other widths, the rustc "perhaps you meant `i32`" analogue.
        format!(
            "`{name}` is a width constructor, not a type — {lead} requires a sized type; use `{default}` \
             (or another width: {widths})"
        )
    } else {
        format!(
            "`{name}` is a value, not a type — {lead} requires a type (e.g. annotate `(: value Int64)`)"
        )
    }
}

/// When the bare NAME at `ty_expr` denotes a TYPE CONSTRUCTOR that requires at least one type argument —
/// a prelude collection/quantity ctor (`List`/`Set`/`Map`/`Qty`) or a USER GENERIC sum (`(type Box (W a)
/// …)`, ≥1 type parameter) — return `(name, placeholder)` for a "needs a type argument" message, where
/// `placeholder` echoes the ctor's argument shape (`List Elem`, `Map Key Value`, `Box a`). `None` when the
/// name is not such a constructor — a genuine value, or a monomorphic/nullary type that stands alone
/// (`Int64`, a `(type C (R) (G))`). Used to turn a bare `(: xs List)` from the misleading "is a value" into
/// the accurate missing-argument message (the bare-name twin of `type_ctor_arity_message`, which handles
/// only the APPLIED `(List)` / `(List T T)` forms).
pub(crate) fn bare_type_ctor_needs_argument(
    db: &mut Db,
    ty_expr: StructId,
) -> Option<(String, String)> {
    let name = db.ast.as_name(ty_expr)?.to_string();
    // A PRELUDE collection/quantity constructor — identified by its `(meta apply)` prim, placeholder names
    // matching `type_ctor_arity_message_here`.
    if let Some(placeholder) = match crate::eval::meta_apply_of(db, ty_expr) {
        Some(crate::resolved::Prim::ListCtor) | Some(crate::resolved::Prim::SetCtor) => {
            Some("Elem")
        }
        Some(crate::resolved::Prim::MapCtor) => Some("Key Value"),
        Some(crate::resolved::Prim::QtyCtor) => Some("T u"),
        _ => None,
    } {
        return Some((name, placeholder.to_string()));
    }
    // A USER GENERIC sum — a bare `Box` for `(type Box (W a) …)`. `typeval_of` a bare generic sum name
    // yields a `Ty::Sum`/`Ty::Nominal` whose decl carries the type parameters; ≥1 param means the bare name
    // is missing its argument(s). (A monomorphic sum — 0 params — stands alone, so `None`.)
    let tv = crate::eval::typeval_of(db, ty_expr)?;
    let decl = match &tv {
        Ty::Sum { decl, .. } | Ty::Nominal { decl, .. } => *decl,
        _ => return None,
    };
    let td = db.type_decl_by_occ(decl)?;
    if td.params.is_empty() {
        return None;
    }
    Some((td.name.clone(), td.params.join(" ")))
}

/// When the bare NAME at `ty_expr` is one of the WIDTH-FAMILY *value* constructors — `Int` / `UInt` /
/// `Float` — return `(default, widths)`: the concrete default-width TYPE a user almost certainly meant
/// (`Int64` / `UInt64` / `Float64`) plus a short list of the other sized widths in that family. These
/// prelude names are value constructors that BUILD a sized type from a width literal (`(Int 64)` ≡ the
/// `Int64` type), so a bare `(: a Int)` resolves to a VALUE, not a type — the near-universal newcomer
/// reflex (`int`/`float` name a type in most other languages). Naming the sized default + offering it as
/// a Replace fix turns the opaque "`Int` is a value, not a type" into a one-shot repair, the direct
/// analogue of rustc's "help: perhaps you meant `i32`" for a bare `int`. `None` for any other name (an
/// ordinary value, or a type constructor handled by `bare_type_ctor_needs_argument`). Keyed on the
/// `(meta apply)` prim of the resolved binding — NOT the spelling — so a shadowing `(let ((Int …)) …)`
/// never mis-fires (it resolves to the local, whose prim is not a width ctor).
fn bare_width_ctor_default_type(
    db: &mut Db,
    ty_expr: StructId,
) -> Option<(&'static str, &'static str)> {
    // A bare NAME only — a compound `(Int 64)` is already a valid type and never reaches this reject path.
    db.ast.as_name(ty_expr)?;
    match crate::eval::meta_apply_of(db, ty_expr) {
        Some(crate::resolved::Prim::IntCtor) => Some(("Int64", "`Int32`, `Int16`, `Int8`")),
        Some(crate::resolved::Prim::UIntCtor) => Some(("UInt64", "`UInt32`, `UInt16`, `UInt8`")),
        Some(crate::resolved::Prim::FloatCtor) => Some(("Float64", "`Float32`")),
        _ => None,
    }
}

/// Validate a NON-type-denoting annotation type expression `ty_expr` (one `typeval_of` rejected), pushing
/// each fault. The SHARED core of the three annotation sites — a parameter annotation, a value annotation
/// `(: value T)`, and a let-binder annotation — so all three name a bad type the same way. A RECORD-bearing
/// type (`(Record (name Type)…)`, or a container carrying one) uses the record-aware position split
/// (`push_payload_type_positions` skips field LABELS + descends into each field's TYPE; `validate_type_position`
/// keeps only a genuinely-unknown type name), so a `(Record (x Nonesuch))` names only `Nonesuch`, not the
/// label `x` (the naive value-`collect` mis-resolves labels as unbound names — M125). Otherwise: collect the
/// operand's own faults (an unbound name → CDZ0101), and if none surfaced (a well-formed non-type — a
/// literal, a compound), add the "expected a type" CDZ0203 with `lead` naming the site.
fn validate_non_type_annotation(
    db: &mut Db,
    ty_expr: StructId,
    lead: &str,
    at_parameter: bool,
    out: &mut Vec<Reject>,
) {
    if crate::compile::is_record_bearing(db, ty_expr) || db.ast.head_name(ty_expr) == Some("Record")
    {
        let mut positions: Vec<(StructId, Vec<String>)> = Vec::new();
        crate::compile::push_payload_type_positions(db, ty_expr, &[], &mut positions);
        for (pos, params) in &positions {
            crate::compile::validate_type_position(db, *pos, params, lead, out);
        }
        return;
    }
    // A name that IS a DECLARED TYPE whose `typeval_of` failed only because that type's OWN declaration is
    // MALFORMED — e.g. `(: c C)` / `(: x (Box Int64))` where `(type C (Red) (Red))` / `(type Box (W a) (W
    // b))` has a duplicate variant — is NOT "a value, not a type" / "found a non-type": the name genuinely
    // denotes a type, just a broken one. The duplicate-variant (or other declaration-site) reject is the
    // primary, actionable "no"; adding a use-site "not a type" here is a MISLEADING consequent (it blames
    // the annotation, not the real defect). Suppress it — defer to the declaration-site error. Covers BOTH
    // a BARE name `C` and an APPLIED generic `(Box …)` (its head names the declared generic). A well-formed
    // type reduces via `typeval_of` and never reaches this branch, so this fires only for a malformed one.
    // The DECLARED-TYPE HEAD of the annotation, when its type is intrinsically MALFORMED — a bare name `C`,
    // or an applied `(Box …)` whose head is a declared generic. For the applied form we additionally
    // require the HEAD ALONE to fail `typeval_of`: that isolates a broken DECLARATION (`(type Box (W a) (W
    // b))` — the bare `Box` does not reduce) from a WELL-FORMED generic merely MISAPPLIED (`(Box 5)` — the
    // bare `Box` reduces fine; the `5` argument is the real, reportable defect via `non_type_argument_message`
    // below, which we must NOT suppress). A bare-name annotation has no arguments, so its failing
    // `typeval_of` already means the declaration is broken — no extra check needed there.
    let malformed_type_head = if let Some(name) = db.ast.as_name(ty_expr) {
        db.type_decl_by_name(name).is_some()
    } else if let crate::ast::Struct::List(kids) = db.ast.get(ty_expr)
        && let Some(&head) = kids.first()
        && let Some(name) = db.ast.as_name(head)
    {
        db.type_decl_by_name(name).is_some() && crate::eval::typeval_of(db, head).is_none()
    } else {
        false
    };
    if malformed_type_head {
        return;
    }
    // A bare LOWERCASE name in a type-annotation position that resolves to NOTHING — `(: x a)`. A user
    // coming from ML/Haskell reads `a` as a TYPE VARIABLE (and it IS one in a VARIANT PAYLOAD `(type Box (B
    // a))` / an effect-op type, where a lowercase name is a declared type parameter). But an annotation's
    // type position is NOT a binding site for a fresh type variable — there is no `∀a.` to scope it — so `a`
    // there is a genuinely unbound name. The bare "unbound name `a`" is technically right but unhelpful: it
    // does not tell the ML user how to get the polymorphism they wanted. Cadenza's generics come from an
    // UNANNOTATED parameter (`(def (id x) x)` is already `∀a. a → a`), so name that route. Only for a bare
    // lowercase name that (a) is not a declared type (uppercase/prelude types took the branches above), and
    // (b) resolves to no value — an uppercase unbound name, or one that is a real value, is a different
    // fault and keeps its own message. Gives the CDZ0101 (still an unbound name) with the actionable hint.
    if let Some(name) = db.ast.as_name(ty_expr).map(str::to_string)
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && matches!(resolved_of(db, ty_expr), Resolved::Poison(_))
    {
        out.push(lowercase_type_var_reject(
            &name,
            ty_expr,
            lead,
            at_parameter,
        ));
        return;
    }
    // A bare UPPERCASE name in a type position that resolves to NOTHING and is not a declared type —
    // `(: 5 Widget)`, `(: x Widget)`, a variant payload `(type Box (Mk Widget))`. The generic `collect`
    // below gives the terse "unbound name `Widget`", which does not convey that a TYPE is what is missing
    // here (rustc distinguishes "cannot find type `T`" from "cannot find value"). Say so — it is the
    // uppercase companion of the lowercase-type-var guidance just above. GATED on there being NO near
    // suggestion: a typo of a real type (`Strng` → `String`, `Colr` → `Color`) must keep the more useful
    // did-you-mean the ordinary unbound path (`enrich_unbound` → `nearest_unbound_suggestion`, whose pool
    // includes type names) produces — so defer to it whenever a candidate is in range, and take this branch
    // only for a genuinely-unknown type. (Lowercase already returned above; a name that is a real VALUE
    // resolves to a `Ref`, not `Poison`, so it never reaches here.)
    if let Some(name) = db.ast.as_name(ty_expr).map(str::to_string)
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && matches!(resolved_of(db, ty_expr), Resolved::Poison(_))
        && crate::resolve::nearest_unbound_suggestion(db, ty_expr, &name).is_none()
    {
        out.push(unknown_type_reject(&name, ty_expr, lead));
        return;
    }
    // A COMPOUND type expression carrying a lowercase type-var in a nested position — `(List b)`,
    // `(Tuple a b)`, `(-> a b)`, `(Map k v)`, `(Record (x a))`. The bare-name branch above only catches a
    // top-level `(: x a)`; a nested `b` fell to the generic `collect` below, which reported the terse
    // "unbound name `b`" WITHOUT the "not a type variable here — leave the parameter unannotated" guidance
    // an ML/Haskell user needs (they read `(List b)` as `List<some type var b>`). Walk the type expr's
    // nested positions and emit the SAME rich message for each lowercase-unbound leaf, so a nested type-var
    // gets fix-parity with the top-level one. If any fired, we are done (the enriched rejects replace what
    // `collect` would have said); otherwise fall through to the ordinary path.
    let before_tv = out.len();
    enrich_nested_lowercase_type_vars(db, ty_expr, lead, at_parameter, out);
    if out.len() != before_tv {
        return;
    }
    let before = out.len();
    collect(db, ty_expr, out);
    if out.len() == before {
        // A type-CONSTRUCTOR form with a well-formed NON-TYPE in an argument position — `(List 5)`, `(Tuple
        // Int64 5)`, `(-> Int64 5)` — names the SPECIFIC offending element + anchors THERE, rather than the
        // flat "requires a type, but found a non-type" over the whole form (which never says which sub-part
        // is wrong). Falls back to the flat message when no single argument is the culprit (a bare literal
        // `(: x 5)`, or a head that is not a recognized type constructor).
        if let Some((arg, msg)) = non_type_argument_message(db, ty_expr) {
            out.push(Reject::coded(Code::TypeMismatch, format!("{lead}: {msg}")).at(arg));
        } else {
            let mut reject = Reject::coded(
                Code::TypeMismatch,
                non_type_annotation_message(db, ty_expr, lead),
            )
            .at(ty_expr);
            // A bare width ctor (`Int`/`UInt`/`Float`) carries a one-shot Replace to the sized default —
            // applying `Int`→`Int64` clears the fault. Heuristic (the author may have wanted a narrower
            // width), but the default retype type-checks in one edit.
            if let Some((default, _)) = bare_width_ctor_default_type(db, ty_expr) {
                reject = reject.with_fix(Fix::replace_heuristic(ty_expr, default));
            }
            out.push(reject);
        }
    }
}

/// The CDZ0101 reject for a LOWERCASE name used as a would-be type variable in a type-annotation position
/// — the ML/Haskell reflex (`a` read as `∀a`), which Cadenza has no `∀`-binder to scope. Names the actual
/// route to the polymorphism the author wanted: leave the parameter UNANNOTATED. Shared by the top-level
/// bare-name case and the NESTED-position walk (`enrich_nested_lowercase_type_vars`), so `(: x a)` and
/// `(: x (List b))` read identically. `lead` names the site ("a parameter's annotation", etc.).
///
/// At a PARAMETER site the message additionally names the explicit-`Type`-parameter idiom — `(def (f (: t
/// Type) (: x (List t))) …)` — because a lowercase name in a USER-GENERIC signature (`(: it (Iter a))`) is
/// exactly the case where a documenting annotation is wanted and dropping it is unsatisfying: generics are
/// type-valued parameters (spec §"Generics Are Type-Valued Parameters"), so binding the element type as a
/// preceding `(: t Type)` parameter gives the annotated generic signature with no `∀`-binder. Keyed off
/// `lead` naming a parameter; the value/let-binder sites (where there is no parameter list to add a `Type`
/// binder to) keep the drop-the-annotation / concrete-type guidance only.
fn lowercase_type_var_reject(name: &str, at: StructId, lead: &str, at_parameter: bool) -> Reject {
    // A parameter annotation can be made generic BOTH ways — drop it, or bind the type as an explicit
    // preceding `(: t Type)` parameter (the composable idiom for a user-generic signature). A value/binder
    // annotation has no parameter list, so only the drop / concrete-type routes apply. The parameter-ness
    // is passed EXPLICITLY by the caller (which knows the site) — NOT sniffed from the human-readable
    // `lead` string, so a reword of `lead` can never silently drop/add this guidance (Copilot PR #438).
    let type_param_route = if at_parameter {
        " — or take the type as an explicit `Type` parameter, `(def (f (: t Type) (: x (List t))) …)`, \
         which keeps a documenting generic signature"
    } else {
        ""
    };
    Reject::coded(
        Code::Unbound,
        format!(
            "unbound name `{name}` — a lowercase name in a type position is not a type variable here \
             ({lead} names an existing type). Cadenza has no `∀`-binder in an annotation; write a \
             GENERIC parameter by leaving it UNANNOTATED — `(def (f x) …)` is already polymorphic in `x` \
             — or annotate a concrete type{type_param_route}"
        ),
    )
    .at(at)
}

/// The CDZ0101 reject for an UPPERCASE name in a type position that names no declared type — `Widget` in
/// `(: 5 Widget)` / `(List Widget)` / a variant payload. Says a TYPE is what is missing (rustc's "cannot
/// find type `T`"), not the terse "unbound name". Shared by the top-level bare-name case and the
/// NESTED-position walk so `(: x Widget)` and `(: x (List Widget))` read identically — the uppercase twin
/// of [`lowercase_type_var_reject`]. Callers GATE on there being no near suggestion (a typo of a real type
/// keeps its did-you-mean); this helper just builds the message. `lead` names the site.
pub(crate) fn unknown_type_reject(name: &str, at: StructId, lead: &str) -> Reject {
    Reject::coded(
        Code::Unbound,
        format!(
            "unknown type `{name}` — no type by that name is declared ({lead} names an existing type); \
             declare it with `(type {name} …)`, or use a type that is in scope"
        ),
    )
    .at(at)
}

/// Walk a COMPOUND type-annotation expression's NESTED type positions and emit the rich per-leaf reject:
/// [`lowercase_type_var_reject`] for a lowercase would-be type variable, [`unknown_type_reject`] for an
/// uppercase name that names no declared type — so a `(List b)` / `(Tuple a Widget)` / `(Map k v)` leaf
/// gets the SAME guidance the top-level `(: x a)` / `(: x Widget)` does, instead of the terse "unbound
/// name" the generic `collect` gives. Only a bare name resolving to `Poison` is enriched (a name that IS a
/// value keeps its own fault); the uppercase branch further requires NO near suggestion, so a nested typo
/// of a real type (`(List Strng)`) keeps its did-you-mean. Recurses through the tail elements of a `(head
/// …)` form (the type-argument positions), NOT the head (`List`/`Map`/`->` are the known ctors); a
/// record-bearing type never reaches here (the caller's record branch returns first), so there are no
/// field LABELS to skip.
fn enrich_nested_lowercase_type_vars(
    db: &mut Db,
    node: StructId,
    lead: &str,
    at_parameter: bool,
    out: &mut Vec<Reject>,
) {
    // A bare NAME node is a leaf — enrich it if it resolves to nothing (a lowercase would-be type var, or
    // an uppercase unknown type with no near suggestion), then stop (a name has no tail to recurse).
    if let Some(name) = db.ast.as_name(node).map(str::to_string) {
        if matches!(resolved_of(db, node), Resolved::Poison(_)) {
            if name.starts_with(|c: char| c.is_ascii_lowercase()) {
                out.push(lowercase_type_var_reject(&name, node, lead, at_parameter));
            } else if name.starts_with(|c: char| c.is_ascii_uppercase())
                && crate::resolve::nearest_unbound_suggestion(db, node, &name).is_none()
            {
                out.push(unknown_type_reject(&name, node, lead));
            }
        }
        return;
    }
    // A compound `(head tail…)` — recurse into the TAIL (the type-argument positions); the head names the
    // constructor, not a type var.
    if let crate::ast::Struct::List(kids) = db.ast.get(node) {
        let kids = kids.clone();
        // `(Qty T u)` is SPECIAL: its 2nd argument is a UNIT position, not a type position. A bare unbound
        // name THERE — `(Qty Int64 meter)` — is a botched unit, NOT a would-be type variable / unknown
        // type, so the ordinary type-var ("leave the parameter unannotated") / unknown-type ("declare it
        // with `(type …)`") guidance MISLEADS. Give a unit-specific message and do NOT recurse the unit
        // position as a type. (A malformed unit that is a COMPOUND — `(Qty Int64 (bogus))` — still falls
        // through to the normal path; only a bare-name unit is redirected, the common slip.)
        let is_qty = kids.first().is_some_and(|&h| {
            crate::eval::meta_apply_of(db, h) == Some(crate::resolved::Prim::QtyCtor)
        });
        for (i, &child) in kids.iter().enumerate().skip(1) {
            // A bare SYMBOL `#"meter"` in the unit position — `(Qty Float64 #"meter")` — is the twin slip
            // of the bare NAME below: the author wrote the unit's NAME directly (as a symbol) instead of a
            // unit EXPRESSION. It falls through the name check (a symbol is not `as_name`) to the generic
            // "requires a type, but found a non-type", which MISLEADS (the position is a UNIT, not a type)
            // and gives no repair. Name it a unit and show the exact wrap — the symbol text is already in
            // hand, so the `(Unit.base #"<sym>")` fix is spelled precisely. (This mirrors the value-position
            // `Qty.of` unit reject, which already names the unit-expression forms.)
            if is_qty
                && i == 2
                && let Some(sym) = db.ast.as_sym(child).map(str::to_string)
            {
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "`#\"{sym}\"` is not a unit — `Qty`'s second argument is a UNIT expression, \
                             not a bare symbol. Write `(Unit.base #\"{sym}\")` for a base unit, or \
                             `Unit.one` for the dimensionless unit"
                        ),
                    )
                    .at(child)
                    .with_fix(Fix::replace_heuristic(
                        child,
                        format!("(Unit.base #\"{sym}\")"),
                    )),
                );
                continue;
            }
            if is_qty
                && i == 2
                && db.ast.as_name(child).is_some()
                && matches!(resolved_of(db, child), Resolved::Poison(_))
            {
                let unit = db.ast.as_name(child).unwrap().to_string();
                out.push(
                    Reject::coded(
                        Code::Unbound,
                        format!(
                            "`{unit}` is not a unit — `Qty`'s second argument is a UNIT, not a type, so a \
                             bare name does not name one. Write a unit expression, e.g. `(Unit.base \
                             #\"{unit}\")` for a base unit, or `Unit.one` for the dimensionless unit"
                        ),
                    )
                    .at(child)
                    // The name IS the intended base-unit name, so spell the exact `(Unit.base #"<name>")`
                    // wrap — fix-parity with the bare-SYMBOL arm above (same recoverable repair). Heuristic:
                    // the author might instead mean `Unit.one` or a composition, so the base-unit wrap is
                    // the likeliest single repair, not a proven one.
                    .with_fix(Fix::replace_heuristic(
                        child,
                        format!("(Unit.base #\"{unit}\")"),
                    )),
                );
                continue;
            }
            enrich_nested_lowercase_type_vars(db, child, lead, at_parameter, out);
        }
    }
}

/// An ARITY-specific message when a type CONSTRUCTOR is applied to the WRONG number of arguments —
/// `(List Int64 Int64)` (List takes 1), `(Map Int64)` (Map takes 2), `(Set Int64 Bool)` (Set takes 1).
/// Such a form reduces to NO type-value (`reduce_ctor` rejects the arity), so the generic
/// `non_type_annotation_message` calls it "a non-type" — misleading, since `List`/`Map`/`Set` ARE type
/// constructors, just misapplied. This names the constructor + its expected vs supplied arity (rustc's
/// "this type takes N generic arguments but M were supplied"). `None` unless `ty_expr` is a `(Head arg…)`
/// application whose head is a known type constructor (`List`/`Set` = 1, `Map` = 2) AND the argument count
/// differs — every other non-type keeps the generic message. Reads the head's ctor identity via
/// `meta_apply_of` (GENERIC — the prim carries the arity, no hard-coded name match on the source spelling).
///
/// Used at BOTH annotation/prelude positions AND variant-payload / effect-op-type DECLARATION positions:
/// `compile::validate_type_position` calls this (alongside `bare_type_ctor_needs_argument` for the bare
/// no-argument case) so a mis-arity ctor in `(type W (Wrap (Box Int64 Bool)))` / `(op emit (-> (Option Int64
/// Bool) Unit))` is rejected at the declaration with the same message an annotation gives, not waved through
/// to a confusing later construction-site CDZ0201.
pub(crate) fn type_ctor_arity_message(db: &mut Db, ty_expr: StructId) -> Option<String> {
    // Check THIS node's head first; if it is well-formed, RECURSE into its argument positions so a NESTED
    // wrong-arity ctor is caught too — `(List (Box Int64 Bool))`, `(Tuple Int64 (Map Int64))`, a record
    // field's `(Box Int64 Bool)`. The type-argument positions are the list's children after the head (each
    // itself a type expression); a record field pair `(name T)` carries the type in its second child. The
    // FIRST wrong-arity ctor found (outer before inner, left-to-right) is reported — one fault per
    // annotation, naming the deepest-relevant misapplication.
    let this = type_ctor_arity_message_here(db, ty_expr);
    if this.is_some() {
        return this;
    }
    let children = match db.ast.get(ty_expr) {
        crate::ast::Struct::List(cs) => cs.to_vec(),
        _ => return None,
    };
    // Recurse into every child EXCEPT a leading head name (which is the ctor itself, not a type argument —
    // recursing into it would re-examine the same head). A record field pair `(name T)` is a 2-child list
    // whose type is the second child; a bare type application `(Ctor arg…)` has its args after the head.
    // Scanning all non-first children covers both (a field's `name` is an atom → no ctor there).
    for &child in children.iter().skip(1) {
        if let Some(msg) = type_ctor_arity_message(db, child) {
            return Some(msg);
        }
    }
    None
}

/// The wrong-arity message for the type CONSTRUCTOR at THIS node only (not recursing into arguments) —
/// the per-node core of [`type_ctor_arity_message`]. `None` if this node is not a `(Ctor arg…)` list, or
/// the ctor is applied at its correct arity, or it is not a type constructor at all.
fn type_ctor_arity_message_here(db: &mut Db, ty_expr: StructId) -> Option<String> {
    // `cs.len() >= 1` (not `>= 2`): a ZERO-arg constructor application `(Int)` / `(List)` — the head with
    // no arguments — is also a wrong arity, and the messages below name the missing argument. A bare atom
    // (no list) is not an application, so it is excluded.
    let children = match db.ast.get(ty_expr) {
        crate::ast::Struct::List(cs) if !cs.is_empty() => cs.to_vec(),
        _ => return None,
    };
    let head = children[0];
    let supplied = children.len() - 1;
    // A WIDTH-INDEXED integer/float type constructor — `(Int 64)` / `(UInt 8)` / `(Float 32)`. It takes
    // exactly ONE argument, a compile-time WIDTH; `(Int)` / `(Int 32 64)` is a wrong arity. `reduce_ctor`
    // rejects it → the generic `non_type_annotation_message` calls it "a non-type", misleading since `Int`
    // IS a type constructor (just missing/over its width). Name the width requirement + the fix (spell the
    // aliased `Int64` when the width is a plain natural, else `(Int <width>)`).
    if let Some((name, placeholder)) = match crate::eval::meta_apply_of(db, head) {
        Some(crate::resolved::Prim::IntCtor) => Some(("Int", "width")),
        Some(crate::resolved::Prim::UIntCtor) => Some(("UInt", "width")),
        Some(crate::resolved::Prim::FloatCtor) => Some(("Float", "width")),
        _ => None,
    } {
        if supplied == 1 {
            return None; // correct arity — a genuine width fault (non-natural width) surfaces elsewhere
        }
        return Some(format!(
            "`{name}` is a WIDTH-indexed type constructor taking one width, but {supplied} arguments \
             were supplied — write `({name} <{placeholder}>)`, e.g. `{name}64`"
        ));
    }
    // The ARROW (function) type constructor `->`. Unlike the fixed-arity collection ctors, `->` takes ONE
    // OR MORE arguments: `(-> R)` is the nullary `Unit -> R`, `(-> P R)` the ordinary `P -> R`, `(-> A B … R)`
    // the right-curried n-ary arrow (`reduce_ctor`'s `FnCtor` arm) — so ONLY the ZERO-argument `(->)` is a
    // wrong arity. `reduce_ctor` rejects it ("-> takes at least one type argument"), which the generic
    // `non_type_annotation_message` flattens to the misleading "found a non-type" (as if `->` were not a
    // type constructor). Name the arrow shape + its minimum (a result type), the arrow twin of the
    // collection-ctor arity messages. Only the empty case; `(-> …)` with ≥1 arg is well-formed and returns
    // `None` (any non-type argument inside it surfaces as its own fault).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::FnCtor) && supplied == 0
    {
        return Some(
            "an arrow type is `(-> Arg… Result)` — it needs at least a result type, e.g. `(-> Int64)` \
             (a nullary function returning `Int64`) or `(-> Int64 Bool)` (`Int64 -> Bool`); `(->)` names \
             no type"
                .to_string(),
        );
    }
    // A PRELUDE collection/quantity type constructor — its arity is fixed by the prim, and its argument
    // placeholder names read naturally (`List Elem`, `Map Key Value`, `Qty T u`). `Qty` takes 2 — a numeric
    // TYPE + a UNIT (`(Qty Int64 (Unit.base #"meter"))`); a wrong count `(Qty Int64)` / `(Qty)` reads as
    // the generic "not a type", so name the arity here (only on a WRONG count — a correct-arity `(Qty T u)`
    // returns `None` so its unit-position validation stands).
    if let Some((name, expected, placeholder)) = match crate::eval::meta_apply_of(db, head) {
        Some(crate::resolved::Prim::ListCtor) => Some(("List".to_string(), 1usize, "Elem")),
        Some(crate::resolved::Prim::SetCtor) => Some(("Set".to_string(), 1, "Elem")),
        Some(crate::resolved::Prim::MapCtor) => Some(("Map".to_string(), 2, "Key Value")),
        Some(crate::resolved::Prim::QtyCtor) => Some(("Qty".to_string(), 2, "T u")),
        _ => None,
    } {
        if supplied == expected {
            return None;
        }
        let plural = if expected == 1 { "" } else { "s" };
        return Some(format!(
            "`{name}` takes {expected} type argument{plural}, but {supplied} {} supplied — write \
             `({name} {placeholder})`",
            if supplied == 1 { "was" } else { "were" },
        ));
    }
    // A USER GENERIC SUM constructor — `(Box Int64 Bool)` where `(type Box (W a) …)` declares ONE type
    // parameter. Its declared parameter count is the expected arity (read off the sum's decl). Unlike a
    // prelude ctor whose wrong arity fails to reduce (→ the "not a type" path), a generic sum reduces to a
    // `Ty::Sum` with WHATEVER args were given (the extra silently ignored / a missing one left a var), so
    // this check must run even when `typeval_of` SUCCEEDS. Placeholder args echo the sum's own parameter
    // names (`(type Pair (P a b))` → `(Pair a b)`). Fires only when the count DIFFERS and the sum is
    // generic (a monomorphic sum applied to args is the M108 "takes no type parameters" message instead).
    let head_typeval = crate::eval::typeval_of(db, head)?;
    let decl = match &head_typeval {
        Ty::Sum { decl, .. } | Ty::Nominal { decl, .. } => *decl,
        _ => return None,
    };
    let td = db.type_decl_by_occ(decl)?;
    let (name, params) = (td.name.clone(), td.params.clone());
    let expected = params.len();
    if expected == 0 || supplied == expected {
        return None; // monomorphic (M108's message) or a correct arity
    }
    let plural = if expected == 1 { "" } else { "s" };
    Some(format!(
        "`{name}` takes {expected} type argument{plural}, but {supplied} {} supplied — write `({name} \
         {})`",
        if supplied == 1 { "was" } else { "were" },
        params.join(" "),
    ))
}

/// When `ty_expr` is a type-CONSTRUCTOR form (`(List T)`, `(Map K V)`, `(Tuple T…)`, `(-> A… R)`, `(Qty T
/// u)`) with a well-formed NON-TYPE in one of its type-argument positions — a literal or value where a type
/// belongs, `(List 5)` / `(Tuple Int64 5)` / `(-> Int64 5)` — return the offending CHILD node and a message
/// naming that specific position, instead of the flat "requires a type, but found a non-type" that neither
/// says WHICH element is wrong nor anchors at it. The type-argument positions are the form's children after
/// the head (the last child of `->` is its result, the earlier ones its parameters; `Qty`'s second child is
/// a UNIT, not a type, so it is excluded). Only fires for a child that (a) `typeval_of` rejects, (b) is not
/// itself a wrong-arity ctor (that has its own message via `type_ctor_arity_message`), and (c) surfaces no
/// fault of its own from `collect` (an unbound name is already CDZ0101 — this is for a WELL-FORMED value, a
/// literal). The head must be a recognized type constructor (via `meta_apply_of`), so a user application in
/// a type slot is not misread. `None` when no such position exists. Reports the FIRST offending argument
/// (left-to-right), one fault per annotation.
fn non_type_argument_message(db: &mut Db, ty_expr: StructId) -> Option<(StructId, String)> {
    let children = match db.ast.get(ty_expr) {
        crate::ast::Struct::List(cs) if cs.len() >= 2 => cs.to_vec(),
        _ => return None,
    };
    let head = children[0];
    // Recognize the constructor + how to describe its argument positions. `role(i, n)` names the i-th
    // argument (0-based over the args after the head; `n` = arg count) for that constructor.
    let role: fn(usize, usize) -> String = match crate::eval::meta_apply_of(db, head)? {
        crate::resolved::Prim::ListCtor | crate::resolved::Prim::SetCtor => {
            |_, _| "the element type".to_string()
        }
        crate::resolved::Prim::MapCtor => |i, _| {
            if i == 0 {
                "the key type".to_string()
            } else {
                "the value type".to_string()
            }
        },
        crate::resolved::Prim::TupleCtor => |i, _| format!("element {i}'s type"),
        crate::resolved::Prim::FnCtor => |i, n| {
            // `(-> A… R)` — the LAST argument is the result, the earlier ones parameters.
            if i + 1 == n {
                "the result type".to_string()
            } else {
                format!("parameter {i}'s type")
            }
        },
        // `Qty`'s first arg is the inner numeric TYPE; its second is a UNIT (validated separately, not a
        // type), so only position 0 is a type slot here.
        crate::resolved::Prim::QtyCtor => |_, _| "the inner type".to_string(),
        _ => return None,
    };
    let args = &children[1..];
    let n = args.len();
    for (i, &arg) in args.iter().enumerate() {
        // `Qty`'s unit position (index 1) is NOT a type slot — skip it (its own "is not a unit" check stands).
        if matches!(
            crate::eval::meta_apply_of(db, head),
            Some(crate::resolved::Prim::QtyCtor)
        ) && i == 1
        {
            continue;
        }
        if crate::eval::typeval_of(db, arg).is_some() {
            continue; // this position IS a type — fine
        }
        // A wrong-arity nested ctor has its OWN (better) message — leave it to `type_ctor_arity_message`.
        if type_ctor_arity_message(db, arg).is_some() {
            return None;
        }
        // A fault of the argument's own (an unbound name → CDZ0101) is the real report — only a WELL-FORMED
        // non-type (a literal `5`, a compound value) reaches this naming. Probe with a throwaway buffer.
        let mut probe = Vec::new();
        collect(db, arg, &mut probe);
        if !probe.is_empty() {
            return None;
        }
        return Some((
            arg,
            format!(
                "{} must be a type, but this is a value — a type belongs here",
                role(i, n)
            ),
        ));
    }
    None
}

/// A NON-LINEAR parameter list — a name bound more than once (`(fn (x x) …)`, `(f x x)`) — is CDZ0102: a
/// parameter list must be LINEAR, like a pattern (a duplicate binder shadows the first, so a body use
/// reads only one and the other silently binds nothing). Each repeated binder is a separate reject
/// anchored at the repeated occurrence, carrying a rename-to-fresh fix (`x` → `x2`, dodging every other
/// param name). Shared by a top-level DEF's parameter list (`compile::collect_faults`) and an anonymous
/// LAMBDA's (`collect_node`'s `Lambda` arm) so `(fn (x x) …)` is rejected exactly as `(def (f x x) …)` is
/// — the same linearity rule, wherever a parameter list is written.
pub fn param_list_linearity_faults(db: &mut Db, params: &[StructId], out: &mut Vec<Reject>) {
    // All param names — the set the rename fix must avoid so a fresh name collides with neither an earlier
    // NOR a later parameter (renaming `x` in `(f x x)` to `x2` must dodge a real `x2`).
    let all_names: std::collections::HashSet<String> = params
        .iter()
        .filter_map(|&p| {
            db.ast
                .as_name(crate::eval::param_name_occ(db, p))
                .map(str::to_string)
        })
        .collect();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for &p in params {
        let name_occ = crate::eval::param_name_occ(db, p);
        let Some(name) = db.ast.as_name(name_occ).map(|s| s.to_string()) else {
            continue; // a param with no extractable name (a malformed binder) — not a dup check
        };
        if !seen.insert(name.clone()) {
            // RENAME the repeated occurrence to a fresh non-colliding name (`x` → `x2`), making the
            // parameter list linear (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
            // Fix). Heuristic: the rename clears the hard error, but the fresh binder is then unused (a
            // CDZ0306 warning) until the author wires it up — renaming vs. dropping the duplicate (which
            // changes arity) is the author's call. Anchored at the repeated binder.
            let fresh = crate::diag::suggest::fresh_suffixed_name(&name, &all_names);
            out.push(
                Reject::coded(
                    Code::NonLinearBinder,
                    format!(
                        "parameter `{name}` is bound more than once (a parameter list must be linear, \
                         like a pattern)"
                    ),
                )
                .at(name_occ)
                .with_fix(Fix::replace_heuristic(name_occ, fresh)),
            );
        }
    }
}

/// Faults in a DEF PARAMETER's annotation `(: name T)` — the signature-side companion of the value
/// annotation checked in `collect_node`. A parameter's TYPE OPERAND `T` must denote a TYPE; a non-type
/// (an unbound name, a value, a malformed type application `(Int64 Int64)`) is REJECTED, not
/// dropped-and-typed-`Any` (the same drop-instead-of-reject gap the value-annotation form had). `param`
/// is a signature parameter occurrence — a bare name (no annotation, no fault) or a `(: name T)` binder.
/// Reported from `compile::collect_faults` (which walks each def's params); a def's body is checked
/// separately, and a bare param never reaches here with a fault.
pub fn param_annotation_faults(db: &mut Db, param: StructId, out: &mut Vec<Reject>) {
    // Only an ANNOTATED param `(: name T)` has a type operand to validate.
    let Some(tail) = db.ast.as_form(param, ":").map(|t| t.to_vec()) else {
        return;
    };
    if tail.len() != 2 {
        return;
    }
    let ty_expr = tail[1];
    // A RUNTIME WIDTH `(: n (UInt m))` with a runtime `m` is its own CDZ0302 (surfaced where the
    // annotation is used in the body); do not also fault it here as "not a type". An integer type's
    // width must be a COMPILE-TIME value: a width read from runtime data is rejected, never accepted.
    // Descends nested positions too (`(: xs (List (Int n)))`), so a runtime width buried in a compound
    // parameter type is caught, not only a top-level `(Int n)`.
    //= spec/capabilities/numeric-model.md#an-integer-type-is-indexed-by-a-compile-time-width
    //# The bit width of an integer type MUST be resolved from a compile-time value and MUST NOT be determined by runtime data, so that an integer's width is fixed before the program runs rather than dependent on a value computed at runtime.
    if nested_runtime_width_type(db, ty_expr).is_some() {
        return;
    }
    // An OVER-CEILING / zero integer width `(UInt 65)`/`(UInt 0)` is an ILL-FORMED type (no valid
    // representation) wherever it appears — well-formedness is TOTAL, so it must be rejected at the
    // annotation, not only when a literal is fit-checked against it (`literal_width_fault`) or the def is
    // exported. `reduce_ctor` clamps the width to the sentinel 0, so `typeval_of` succeeds with `Int0` and
    // the "not a type" check below never fires; catch it HERE by reading the ORIGINAL width off the
    // annotation, and name that width (not the misleading clamped `UInt0`). CDZ0302, the same code the
    // literal-fit path gives — consistent with the totality the unbound-name rule already has. A width
    // outside the admitted range (1..=64) is rejected at compile time, never accepted or trapped at run.
    //= spec/capabilities/numeric-model.md#an-integer-type-is-indexed-by-a-compile-time-width
    //# A bit width that is outside the range the numeric model admits MUST be rejected at compile time with the machine-readable diagnostic for the unsatisfied width constraint, rather than accepted or trapped at runtime.
    // Checks the top-level type AND every nested type-argument position (`(UInt 65)`, `(Option (Int -8))`,
    // `(List (UInt 0))`), so an ill-formed width buried in a compound annotation is rejected too (it
    // reduces to a clamped-sentinel `Ty` that `typeval_of` would otherwise wave through).
    if let Some((pos, fault)) = nested_ill_formed_int_width(db, ty_expr) {
        trace!(target: "rcdzc::infer", param = param.0, "fault: ill-formed integer width in a parameter annotation (CDZ0302)");
        let mut reject =
            Reject::coded(Code::IntOutOfRange, ill_formed_int_width_message(&fault)).at(pos);
        if let Some(fix) = ill_formed_int_width_fix(&fault, pos) {
            reject = reject.with_fix(fix);
        }
        out.push(reject);
        return;
    }
    // The `(Float W)` companion: an ill-formed float width (outside the admitted IEEE set {32,64}), bare
    // (`(: x (Float 8))`) or nested in a compound (`(: xs (List (Float 8)))`). The parameter path had NO
    // float-width check at all, so a bad float width in a parameter type slipped past `cdz check` entirely
    // — the same totality the integer path already has (a width is ill-formed wherever the annotation
    // appears, reachable or not).
    if let Some(pos) = nested_ill_formed_float_width(db, ty_expr) {
        trace!(target: "rcdzc::infer", param = param.0, "fault: ill-formed float width in a parameter annotation (CDZ0302)");
        let mut reject = Reject::coded(Code::IntOutOfRange, FLOAT_WIDTH_MESSAGE).at(pos);
        if let Some(fix) = ill_formed_float_width_fix(db, pos) {
            reject = reject.with_fix(fix);
        }
        out.push(reject);
        return;
    }
    // An UNBOUND NAME in a width position — `(: a (Int hello))`, or nested `(: xs (List (Int hello)))`. A
    // width is not a type (so the nested-type-var walk skips it) and reads as a non-constant width (so the
    // ill-formed-width check waves it through as if it were a bound width variable), so it slipped past
    // `cdz check` silently. Surface the width-specific CDZ0101 at the offending arg. A BOUND width variable
    // (`(Int a)` with `a` a `Type` param) is valid and does not match.
    if let Some((pos, example)) = nested_unbound_width(db, ty_expr) {
        trace!(target: "rcdzc::infer", param = param.0, "fault: unbound name in a width position (CDZ0101)");
        let name = db.ast.as_name(pos).unwrap_or("?").to_string();
        out.push(Reject::coded(Code::Unbound, unbound_width_message(&name, example)).at(pos));
        return;
    }
    // A TYPE CONSTRUCTOR applied to the WRONG number of arguments — a prelude `(List Int64 Int64)` (fails
    // to reduce → the "not a type" path below) OR a user generic sum `(Box Int64 Bool)` (which REDUCES to
    // a `Ty::Sum`, silently ignoring the extra arg, so `typeval_of` succeeds and the "not a type" branch
    // never fires). Check arity FIRST, independent of whether the operand reduces, so a wrong-arity generic
    // sum is caught. `type_ctor_arity_message` returns `None` for a correct arity / a non-ctor.
    if let Some(msg) = type_ctor_arity_message(db, ty_expr) {
        trace!(target: "rcdzc::infer", param = param.0, "fault: type constructor applied at the wrong arity (CDZ0203)");
        out.push(Reject::coded(Code::TypeMismatch, msg).at(ty_expr));
        return;
    }
    // A BARE type-CONSTRUCTOR name used with NO argument — `(: b Box)` for `(type Box (W a))`, `(: xs
    // List)`. A prelude ctor (`List`/`Set`/`Map`/`Qty`) fails to reduce (caught by the "not a type" branch
    // below, but with the clearer constructor message), and a USER GENERIC sum's bare name REDUCES to a
    // `Ty::Sum` with a fresh var (so `typeval_of` succeeds and the "not a type" branch never fires) —
    // silently accepting an under-applied generic, which then produces a downstream "a Box is not a Box"
    // confusion at each use. Reject it here, consistent with the applied wrong-arity case (`(Box)` /
    // `(Box a b)`), naming the missing argument. `bare_type_ctor_needs_argument` returns `None` for a
    // monomorphic type / a genuine value, so those are unaffected.
    if bare_type_ctor_needs_argument(db, ty_expr).is_some() {
        trace!(target: "rcdzc::infer", param = param.0, "fault: bare type constructor missing its argument (CDZ0203)");
        out.push(
            Reject::coded(
                Code::TypeMismatch,
                non_type_annotation_message(db, ty_expr, "a parameter's annotation"),
            )
            .at(ty_expr),
        );
        return;
    }
    // The operand denotes a type → fine. Otherwise reject. A `(Record (name Type)…)` (or a container
    // bearing one) needs the RECORD-AWARE type-position split: a field's NAME is a LABEL, not a value
    // reference, so `collect`-ing the whole `(Record (x Nonesuch))` as a value mis-resolves the label `x`
    // as an unbound NAME (a misleading "unbound name `x`") on top of the real "unbound name `Nonesuch`".
    // `push_payload_type_positions` splits out each field's TYPE (skipping labels); `validate_type_position`
    // checks each, keeping only a genuinely-unknown type name. This is the same machinery the variant-
    // payload / effect-op type checks use — so a record-type annotation validates its field TYPES exactly
    // as a variant payload's do, without a spurious label fault.
    if crate::eval::typeval_of(db, ty_expr).is_none() {
        trace!(target: "rcdzc::infer", param = param.0, "fault: parameter annotation type is not a type");
        validate_non_type_annotation(db, ty_expr, "a parameter's annotation", true, out);
    }
}

/// The inferred type of an unannotated RECURSIVE-def parameter whose name occurrence is `binder`, or
/// `None` if `binder` is not such a parameter (a non-recursive def's param inlines at its call site and
/// stays `Any`; an annotated param is handled by `param_annot_ty`). Locates `binder`'s def, and — if
/// that def is recursive — runs the connected parameter solve (memoized), returning `binder`'s solved
/// type. This is the ONE place a parameter's type is INFERRED from its uses rather than read off an
/// annotation or a call-site argument (ANF step 2 / A2).
fn solved_param_ty(db: &mut Db, binder: StructId) -> Option<Ty> {
    if let Some(t) = db.param_types.get(&binder) {
        return Some(t.clone());
    }
    let def = def_of_param(db, binder)?;
    // Only a RECURSIVE def needs this: a non-recursive def's call always inlines (β-reduces), so its
    // param's type flows from the concrete argument and never reaches here as `Any`. Solving a
    // non-recursive param would be redundant work and could mask the inlining path — so gate on it.
    let body = db.defs[def].body?;
    if !crate::eval::is_recursive(db, body) {
        return None;
    }
    solve_recursive_params(db, def);
    db.param_types.get(&binder).cloned()
}

/// The type of the `k`-th parameter of def `callee`, for constraining an argument passed there from
/// another def's recursive-param solve. Sources, in order: (a) an explicit ANNOTATION on the param; (b)
/// a param whose type is already solved in `db.param_types` (a recursive callee, or one solved earlier);
/// (c) otherwise, collect the callee's OWN body constraints (the same operator/if/self-call collection
/// `solve_recursive_params` runs) over a fresh env and read the k-th param's solved type — this pins a
/// NON-recursive unannotated helper's param (`byte-at`'s `b` ⇒ `Bytes` via `(Bytes.at b i)`) without
/// requiring the helper to have a standalone scheme. Returns `None` (no constraint) if the param stays
/// undetermined or `callee` has no such param. Guarded against re-entry (a cycle) by `db.solving_params`.
fn callee_param_ty(db: &mut Db, callee: usize, k: usize) -> Option<Ty> {
    let params = db.defs[callee].params.clone();
    let p = *params.get(k)?;
    let binder = match db.ast.as_form(p, ":").and_then(|t| t.first().copied()) {
        Some(name_occ) => name_occ,
        None => p,
    };
    if let Some(t) = param_annot_ty(db, binder) {
        return Some(t);
    }
    if let Some(t) = db.param_types.get(&binder) {
        return Some(t.clone());
    }
    if db.solving_params.contains(&callee) {
        return None;
    }
    let body = db.defs[callee].body?;
    db.solving_params.insert(callee);
    let mut fresh = Fresh::new();
    let mut env: crate::fxhash::FxHashMap<StructId, Ty> = crate::fxhash::FxHashMap::default();
    let mut binders: Vec<(StructId, Ty)> = Vec::new();
    for pp in &params {
        let b = match db.ast.as_form(*pp, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *pp,
        };
        let var = param_annot_ty(db, b).unwrap_or_else(|| Ty::Var(fresh.var()));
        env.insert(b, var.clone());
        binders.push((b, var));
    }
    let mut subst = Subst::new();
    collect_param_constraints(db, body, &env, callee, &mut subst, &mut fresh);
    db.solving_params.remove(&callee);
    let solved = ground_param(subst.apply(&binders.get(k)?.1));
    match solved {
        Ty::Any | Ty::Var(_) => None,
        other => Some(other),
    }
}

/// The INFERRED type of a def-parameter `binder` for a QUERY (hover / `TypeAt` / inlayHint), when the
/// binder carries NO annotation and `type_of(binder)` therefore reads `Any` (a NON-recursive def's param
/// inlines at each call and is never solved standalone in `db.param_types`). Sources in order: (a) an
/// explicit annotation (`param_annot_ty`) or an already-solved `db.param_types` entry — the cases hover's
/// own `type_of` already covers; (b) otherwise, collect the def body's operand constraints and read this
/// param's solved type (the same body-constraint solve `callee_param_ty` runs to pin `byte-at`'s `b ⇒
/// Bytes`) — this is what types `f`'s `x` as `Int64` from `(+ x 1)`. Returns `None` (render "unknown", as
/// today) when the param stays genuinely undetermined (a fully-generic binder like `id`'s `x`) — a query
/// must not invent a monomorphic width for a scheme variable. READ-ONLY: a fresh `Subst`, does not touch
/// `db.param_types` — safe to call from the sidecar query path. Guarded against re-entry by `solving_params`.
pub(crate) fn query_param_ty(db: &mut Db, node: StructId) -> Option<Ty> {
    // Accept the binder-declaration occurrence (signature) OR a USE of the param in the body — a query may
    // land on either. A use resolves as `Resolved::Param { binder }` OR `Resolved::Ref { value: binder }`
    // (a body reference to the param's declaring node); its own parent is the body expression (not the
    // signature), so `def_of_param` below would miss it. Normalize to the binder first so the positional
    // lookup finds the def param. (`node` itself, when it IS the binder, resolves to neither — kept as-is.)
    let binder = match resolved_of(db, node) {
        crate::resolved::Resolved::Param { binder } => binder,
        crate::resolved::Resolved::Ref { value } => value,
        _ => node,
    };
    if let Some(t) = param_annot_ty(db, binder) {
        return Some(t);
    }
    if let Some(t) = db.param_types.get(&binder) {
        return Some(t.clone());
    }
    let def = def_of_param(db, binder)?;
    let params = db.defs[def].params.clone();
    // The binder's positional index in the signature (bare occurrence, or the name of a `(: name T)`).
    let k = params.iter().position(|&p| {
        let b = db
            .ast
            .as_form(p, ":")
            .and_then(|t| t.first().copied())
            .unwrap_or(p);
        b == binder
    })?;
    callee_param_ty(db, def, k)
}

/// Solve the machine type of a LAMBDA parameter `binder` from its uses in the lambda `body` — the
/// lambda analogue of [`solve_recursive_params`], for a lifted closure whose parameter is UNANNOTATED
/// (a bare `(fn (x) …)`). A bare lambda types with a fresh variable at its own occurrence (inference
/// does not thread the use-site arrow back onto it), so `type_of(binder)` is `Any`; but the body's
/// operations DO constrain it — `(* x 2)` pins `x` to an integer. Give the param a fresh variable,
/// walk the body collecting the operand constraints its uses impose (the same `collect_param_constraints`
/// the recursive-def solve uses, with a sentinel def index that no call matches — a lambda body has no
/// self-recursion to a def), then ground the solved variable (a numeric use defaults to Int64, an
/// unconstrained variable stays `Any` → the caller declines rather than invent a width). Returns the
/// grounded type. This is a read-only solve over a fresh `Subst` — it does NOT touch `db.param_types`
/// (a lambda param is not a def param), so it is safe to call at lowering.
pub fn solve_lambda_param_ty(db: &mut Db, binder: StructId, body: StructId) -> Ty {
    let mut fresh = Fresh::new();
    let var = param_annot_ty(db, binder).unwrap_or_else(|| Ty::Var(fresh.var()));
    let mut env: crate::fxhash::FxHashMap<StructId, Ty> = crate::fxhash::FxHashMap::default();
    env.insert(binder, var.clone());
    let mut subst = Subst::new();
    // A sentinel def index no `callee == def` comparison matches (a lambda body has no self-recursive
    // call to a `db.defs` entry — its own applications resolve to the param or to real defs).
    collect_param_constraints(db, body, &env, usize::MAX, &mut subst, &mut fresh);
    ground_param(subst.apply(&var))
}

/// The fully-SOLVED arrow type of the lambda VALUE at `id` — each parameter solved from the lambda body
/// (`solve_lambda_param_ty`, so an unannotated `(fn (x a) (+ a x))` yields `Int64 → Int64 → Int64`, not
/// the `Any → Any → Int64` the bottom-up `type_of` Lambda arm gives), curried onto the body's type.
/// `None` if `id` is not a lambda. Used by `type_specialize`: a bare closure passed to a generic HOF must
/// be re-annotated with its CONCRETE type in the monomorphized copy, or the `Any` param holes encode as
/// `Unit` and mistype the copy (`(-> Unit … R)` → spurious CDZ0203). The per-param body-solve is exactly
/// what `lower_lambda_value` runs, so the recovered arrow agrees with how the closure itself lowers.
pub fn solved_lambda_arrow(db: &mut Db, params: &[StructId], body: StructId) -> Option<Ty> {
    // The RESULT: when the body is ITSELF a function value — a CURRIED / higher-order def that returns a
    // lambda (`(def (adder n) (fn (x) (+ x n)))`) — recurse so the RETURNED function's params are body-
    // solved too, not left `Any` by the bottom-up `type_of` Lambda arm. Without this, `adder :
    // Int64 → Int64 → Int64` and `pick n = (fn (b) (if b n 0)) : Int64 → Bool → Int64` both recover
    // `Int64 → (-> Any Int64)`, so a returned-function's domain is dropped (the same reflection-soundness
    // hole one currying level deeper — `Type.eq` would call them equal). `lambda_params_and_body` follows
    // the body through a `Ref`/`let`/annotation to the lambda, matching how the value actually lowers; a
    // non-function body takes the plain `type_of`. Each recursion strips one lambda layer (finite currying
    // depth); a reduction loop is bounded by `lambda_of`'s own depth guard (returns `None` → `type_of`).
    let result = match crate::eval::lambda_params_and_body(db, body) {
        Some((inner_params, inner_body)) => {
            solved_lambda_arrow(db, &inner_params, inner_body).unwrap_or_else(|| type_of(db, body))
        }
        None => type_of(db, body),
    };
    // Curry right-to-left: each param's solved machine type onto the accumulated result.
    Some(params.iter().rev().fold(result, |acc, &p| {
        let occ = crate::eval::param_name_occ(db, p);
        // An annotated param already types concretely via `type_of`; a bare one bottom-up types `Any`,
        // so solve it from the body (matching `lower_lambda_value`'s fallback). A still-`Any` param
        // (genuinely unconstrained) stays `Any` — the caller declines rather than invent a type.
        let pt = match type_of(db, occ) {
            Ty::Any => solve_lambda_param_ty(db, occ, body),
            t => t,
        };
        Ty::Fn(Box::new(pt), Box::new(acc))
    }))
}

/// The REFLECTED type of the value `id` — what `Type.of id` denotes. Like `type_of`, but a FUNCTION
/// VALUE (bare, or nested inside a compound) reflects its BODY-SOLVED arrow via `solved_lambda_arrow`
/// rather than the bottom-up `type_of` Lambda arm that leaves an unannotated parameter `Any`. The plain
/// `type_of` is what LAYOUT + EMIT consume, so it must stay untouched (its `Any` param holes are erased,
/// not compared); reflection is the ONLY consumer that needs the grounded arrow, so the grounding lives
/// HERE, off the hot path. Recurses the compound VALUE nodes (`tuple`/`list`/`record`/`map`) so a fn
/// stored in an element grounds too — `(Type.of (tuple f 0))` with `f : Int64 → Int64` reflects
/// `(Tuple (-> Int64 Int64) Int64)`, not `(Tuple (-> Any Int64) Int64)`, so `Type.eq` distinguishes it
/// from a `(tuple g 0)` whose `g : Bool → Int64` (else both collapse to `(-> Any Int64)` → wrong `true`,
/// the compound-element facet of the function-domain reflection miscompile). A non-function, non-compound
/// value takes the plain `type_of` (unchanged). Element joins/products mirror `type_of`'s own compound
/// arms exactly, so a value with no fn element reflects byte-identically to before.
/// Whether `reflected_ty` would ground a fn DOMAIN inside `id` that the bottom-up `type_of` leaves `Any`
/// — i.e. `id` is (syntactically, without reducing) a FUNCTION VALUE or a COMPOUND LITERAL that can hold a
/// fn element. A caller uses this to choose `reflected_ty` (the grounded arrow) over `type_of` ONLY when
/// grounding can matter, so a plain `let`/call/scalar keeps its exact `type_of` behaviour — notably the
/// annotation-agreement check, where reflecting a `(let …)`/call expr would trigger a speculative
/// reduction with side effects that suppress a sibling reject (the `?`-error-type soundness pin). Checked
/// on the RESOLVED shape (a `(fn …)` lambda / a def-ref to one, or a compound-value ctor `Apply` / the
/// symbol-headed compound node), never by reducing.
pub fn reflection_may_ground(db: &mut Db, id: StructId) -> bool {
    use crate::resolved::Prim;
    // A function value — a bare/named lambda whose bottom-up type is an arrow (an unannotated param leaks
    // `Any`). `type_of` is memoized, so this is cheap and does not reduce.
    if matches!(type_of(db, id), Ty::Fn(_, _)) {
        return true;
    }
    // A compound-value LITERAL — the name-alias `Apply(TupleNew/ListNew/RecordNew/MapNew)` or the
    // symbol-headed `Resolved::Tuple`/`List`/`Record`/`Map` — or a variant CONSTRUCTOR application (a
    // payload may be a fn). These are the shapes `reflected_ty` recurses; anything else (a `let`, a call,
    // a scalar, a bare variant) it hands straight to `type_of`, so gating on them changes nothing.
    match resolved_of(db, id) {
        Resolved::Tuple { .. }
        | Resolved::List { .. }
        | Resolved::Record { .. }
        | Resolved::Map { .. } => true,
        Resolved::Apply { head, .. } => {
            matches!(
                crate::eval::meta_apply_of(db, head),
                // `MapInsert` is the RUNTIME map builder `(Map.insert m k v)` — `reflected_ty` grounds a fn
                // domain in its key/value (added alongside the reflection fix), so the annotation check must
                // route it through the grounded path too, not just the `(map …)` literal (`MapNew`). Without
                // it, `(: (Map.insert m 1 h) (Map Int64 (-> Bool Int64)))` read the value fn bottom-up as
                // `(-> Any Int64)` and the `Any` domain absorbed the annotated `Bool` — a pure-domain
                // contradiction silently ACCEPTED (the check-side twin of the Map reflection leak; Option/
                // Tuple already rejected the same via their grounded arms).
                Some(
                    Prim::TupleNew
                        | Prim::ListNew
                        | Prim::RecordNew
                        | Prim::MapNew
                        | Prim::MapInsert
                )
            ) || crate::eval::variant_disc_of(db, head).is_some()
        }
        _ => false,
    }
}

pub fn reflected_ty(db: &mut Db, id: StructId) -> Ty {
    use crate::resolved::Prim;
    // A FUNCTION value — bare `(fn …)`, a named def-ref, an annotated/`let`-wrapped lambda — grounds via
    // the body-solve. Checked first: a fn value never resolves to a compound value node below.
    if let Some((params, body)) = crate::eval::lambda_params_and_body(db, id) {
        return solved_lambda_arrow(db, &params, body).unwrap_or_else(|| type_of(db, id));
    }
    match resolved_of(db, id) {
        // A COMPOUND VALUE — `(tuple …)`/`(list …)`/`(record …)`/`(map …)` — resolves to an `Apply` of the
        // compound-value constructor prim (NOT a symbol-headed `Resolved::Tuple` node — the name-alias
        // application is how the ML/s-expr surfaces build it). Recurse `reflected_ty` over its element
        // args so a fn stored in an element grounds, mirroring `compound_ctor_type`'s per-arg `type_of`
        // (which drops the fn domain to `Any`). This is the compound-element facet: `(tuple f 0)` with
        // `f : Int64 → Int64` reflects `(Tuple (-> Int64 Int64) Int64)`, distinguishable from a `(tuple g
        // 0)` whose `g : Bool → Int64`. A malformed record field list has no type (`Any`), as in `type_of`.
        Resolved::Apply { head, args } => match crate::eval::meta_apply_of(db, head) {
            // Each element is an INDEPENDENT type position — freshen its free vars into a disjoint block
            // off a shared counter so two elements that each reflect a colliding var (two bare `None()`,
            // each `Option(?0)`) do NOT cross-contaminate when the tuple unifies against an expected type
            // in one `Subst`. The same disjoint-freshening `compound_ctor_type` applies; this is the
            // reflection twin, reached when the tuple is checked via a synthesized `(: arg paramtype)`.
            Some(Prim::TupleNew) => {
                let mut fresh = crate::unify::Fresh::new();
                Ty::Tuple(
                    args.iter()
                        .map(|&e| crate::unify::freshen_free(&reflected_ty(db, e), &mut fresh))
                        .collect(),
                )
            }
            Some(Prim::ListNew) => {
                let mut elem_ty = Ty::Any;
                for &e in args.iter() {
                    let et = reflected_ty(db, e);
                    elem_ty = elem_ty.join(&et);
                }
                Ty::List(Box::new(elem_ty))
            }
            Some(Prim::RecordNew) => match crate::resolve::read_record_fields(db, &args) {
                Ok(fields) => {
                    // Freshen per field so sibling fields' vars are disjoint (see the TupleNew note): two
                    // bare `None()` fields must NOT share an `Option` element var, or one borrows the
                    // other's element type when the record is checked against the expected param type via
                    // the synthesized annotation. The reflection twin of `compound_ctor_type`'s RecordNew.
                    let mut fresh = crate::unify::Fresh::new();
                    let mut field_tys = std::collections::BTreeMap::new();
                    for (label, &value) in fields.iter() {
                        let ft = crate::unify::freshen_free(&reflected_ty(db, value), &mut fresh);
                        field_tys.insert(label.clone(), ft);
                    }
                    Ty::Record(std::rc::Rc::new(field_tys))
                }
                Err(_) => Ty::Any,
            },
            Some(Prim::MapNew) => {
                let mut key_ty = Ty::Any;
                let mut val_ty = Ty::Any;
                // `(map (k v) …)` — read the `(key, value)` entry nodes via the shared helper (handles
                // both the primitive `Resolved::Map` and this `Apply(MapNew)` name-alias spelling).
                if let Some(entries) = map_entry_nodes(db, id) {
                    for (k, v) in entries {
                        let kt = reflected_ty(db, k);
                        let vt = reflected_ty(db, v);
                        key_ty = key_ty.join(&kt);
                        val_ty = val_ty.join(&vt);
                    }
                }
                Ty::Map(Box::new(key_ty), Box::new(val_ty))
            }
            // A RUNTIME MAP BUILDER `(Map.insert m k v)` — unlike the `(map (k v) …)` LITERAL handled by
            // `MapNew` above, `Map.insert`'s result type comes from `apply_type` (a `Ty::Map(k, v)` from the
            // op scheme), which reads the inserted value/key via bottom-up `type_of` — so a fn VALUE (or a
            // fn KEY) leaks its domain as `Any` (`(Map Int64 (-> Any Int64))`), and two maps with different-
            // domain value-fns reflect the SAME type → `Type.eq` wrong-`true` (the map sibling of the
            // sum-payload leak; also compounds outward through a `(tuple (Map.insert …) …)` wrapper, which
            // recurses `reflected_ty` into this node). Rebuild the map type from the GROUNDED parts: recurse
            // the map operand `m` to its `Ty::Map`, then JOIN this insert's grounded key + value
            // (`reflected_ty` body-solves a fn arg's domain) — composing across a chain of inserts and
            // bottoming at `Map.empty`'s `(Map Any Any)`. A non-`Ty::Map` operand (malformed) falls back to
            // the plain `type_of`.
            Some(Prim::MapInsert) if args.len() == 3 => {
                match reflected_ty(db, args[0]) {
                    Ty::Map(k0, v0) => {
                        let kt = k0.join(&reflected_ty(db, args[1]));
                        let vt = v0.join(&reflected_ty(db, args[2]));
                        Ty::Map(Box::new(kt), Box::new(vt))
                    }
                    // The operand did not reflect as a map (malformed / not yet a map) — keep `type_of`.
                    _ => type_of(db, id),
                }
            }
            // A SUM-VARIANT CONSTRUCTOR applied to a payload — `(Some f)`, `(Box.Wrap f)`. The payload
            // becomes a TYPE ARGUMENT of the resulting `Ty::Sum{args}` via the ctor's scheme (`Some :
            // ∀a. a → Option a`), but `apply_type` unifies `a` with the payload's BOTTOM-UP `type_of`, so a
            // fn payload's unannotated domain leaks in as `Any` (`Option (-> Any Int64)`) — the sum sibling
            // of the tuple/list/record element leak. `(Some f)` and `(Some g)` (different fn domains) then
            // reflect the SAME `Option (-> Any Int64)` and `Type.eq` returns a wrong `true`. Re-run the
            // ctor's scheme unification with each payload's GROUNDED `reflected_ty` (mirroring `apply_type`'s
            // instantiate+unify loop, but body-solving a fn payload's domain) so the sum's type argument is
            // the real payload type. A nullary variant (no payload / no scheme) or any non-fn payload
            // reflects exactly as `type_of` (the grounded type equals the bottom-up one). Guarded to a
            // genuine variant ctor head via `variant_disc_of`; everything else (a call, an operator) keeps
            // the plain `type_of`.
            _ if crate::eval::variant_disc_of(db, head).is_some() => {
                let mut fresh = crate::unify::Fresh::new();
                match crate::eval::scheme_of(db, head, &mut fresh) {
                    Some(scheme) => {
                        let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
                        let mut subst = Subst::new();
                        for &arg in args.iter() {
                            match subst.apply(&cur) {
                                Ty::Fn(param, result) => {
                                    // GROUND the payload (a fn payload's domain body-solves) then freshen
                                    // past the ctor's instantiation counter — the same occurs-check dodge
                                    // `apply_type` applies to a bare-nullary payload sharing from-0 vars.
                                    let arg_ty = reflected_ty(db, arg);
                                    let at = freshen_arg(db, &arg_ty, &mut fresh);
                                    let _ = crate::unify::unify(
                                        &mut subst,
                                        &param,
                                        &at,
                                        &db.name_ctx(),
                                    );
                                    cur = *result;
                                }
                                // Over-applied / non-arrow tail — fall back to the bottom-up type (a fault
                                // is reported elsewhere; reflection stays total).
                                _ => return type_of(db, id),
                            }
                        }
                        subst.apply(&cur)
                    }
                    None => type_of(db, id),
                }
            }
            // A non-compound application (a call, an operator) has no fn element to ground — its result
            // reflects exactly as `type_of` types it.
            _ => type_of(db, id),
        },
        // The SYMBOL-headed compound value nodes (`Resolved::Tuple`/`List`/`Record`/`Map`) — the same
        // shapes reached directly rather than through the name-alias `Apply`. Recurse identically so both
        // spellings ground a fn element.
        Resolved::Tuple { elems } => {
            Ty::Tuple(elems.iter().map(|&e| reflected_ty(db, e)).collect())
        }
        Resolved::List { elems } => {
            let mut elem_ty = Ty::Any;
            for &e in elems.iter() {
                let et = reflected_ty(db, e);
                elem_ty = elem_ty.join(&et);
            }
            Ty::List(Box::new(elem_ty))
        }
        Resolved::Set { elems } => {
            let mut elem_ty = Ty::Any;
            for &e in elems.iter() {
                let et = reflected_ty(db, e);
                elem_ty = elem_ty.join(&et);
            }
            Ty::Set(Box::new(elem_ty))
        }
        Resolved::Record { fields } => {
            if crate::eval::typeval_of(db, id).is_some() {
                Ty::Type
            } else {
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    field_tys.insert(label.clone(), reflected_ty(db, value));
                }
                Ty::Record(std::rc::Rc::new(field_tys))
            }
        }
        Resolved::Map { entries } => {
            let mut key_ty = Ty::Any;
            let mut val_ty = Ty::Any;
            for &(k, v) in entries.iter() {
                let kt = reflected_ty(db, k);
                let vt = reflected_ty(db, v);
                key_ty = key_ty.join(&kt);
                val_ty = val_ty.join(&vt);
            }
            Ty::Map(Box::new(key_ty), Box::new(val_ty))
        }
        // Every other value — a scalar, a sum value, a projection, a call result — reflects exactly as
        // `type_of` types it (no fn domain to ground).
        _ => type_of(db, id),
    }
}

/// The FUNCTION TYPE a lambda VALUE is EXPECTED to have from its immediate CONTEXT — the type its parent
/// construct requires of it — when that context DECLARES an arrow. `type_of` computes a lambda's type
/// bottom-up (from its body + param occurrences), so a bare `(fn (n) …)` whose param the body does not
/// pin stays `(-> Any …)`; but the SITE the lambda occupies often declares the arrow: a variant
/// constructor's PAYLOAD (`(T.Susp (fn (n) …))` where `T.Susp : (-> (-> Int64 C) T)`), the built-in
/// `Some`/`Ok` payload, an annotation. This recovers that expected arrow so `lower_lambda_value` can type
/// an otherwise-`Any` param/result from it — the "thread the use-site arrow back" the bottom-up pass omits
/// (`core-semantics.md` §A Function Is A First-Class Value: a closure stored in a variant payload must
/// type against the payload's declared function type). `None` when the context declares no arrow (a HOF
/// call site — handled by the call's own unification — or a genuinely unconstrained position).
pub(crate) fn expected_arrow_for_lambda(db: &mut Db, lambda: StructId) -> Option<Ty> {
    // RE-ENTRY backstop (see `db::arrow_lambdas_in_progress`). Paths (3)/(3b) below read the storage
    // context head/param's `type_of`, which for an unannotated parameter of a module-qualified/self-applied
    // call walks back to THIS lambda's param → `lambda_param_ty_from_context` → here on the SAME lambda.
    // Absent this guard the cycle recurses to `DESCENT_DEPTH_LIMIT` (~1024) — terminating natively but
    // overflowing the smaller browser/worker compile stack. Break it at re-entry: recover NOTHING, exactly
    // the `None` the `reduce_nodes` budget below already falls back to (the param then types `Any` and the
    // body-solve grounds it). Scoped to THIS lambda so a nested lambda's own recovery is unaffected.
    if !db.arrow_lambdas_in_progress.insert(lambda) {
        return None;
    }
    let out = expected_arrow_for_lambda_inner(db, lambda);
    db.arrow_lambdas_in_progress.remove(&lambda);
    out
}

fn expected_arrow_for_lambda_inner(db: &mut Db, lambda: StructId) -> Option<Ty> {
    // CUMULATIVE-WORK budget — the SAME `reduce_nodes` counter β-reduction charges against
    // (`db::REDUCE_NODE_BUDGET`). This context-recovery recurses through `type_of` (path 3 below reads a
    // parameter's type, which for a SELF-APPLICATION re-enters here on the growing term); like the plain
    // reduction hang (`c2dae9b9`), the term stays within the descent DEPTH limit yet drives an EXPONENTIAL
    // number of these lookups (`(fn v (if (v v) 1 (v v)))` applied to itself), so the depth guard alone
    // does not stop it and inference appears to HANG. Charging each call against the shared work budget
    // caps the TOTAL across every reduction-equivalent path (plain β-reduction AND this type-context
    // recovery); past the budget, recover NOTHING (`None`) — the lambda types without the context hint and
    // the program declines cleanly downstream, rather than looping. A real program makes far fewer of these
    // than the budget (it is 1M; the corpus never approaches it).
    if db.reduce_nodes >= crate::db::REDUCE_NODE_BUDGET {
        return None;
    }
    db.reduce_nodes += 1;
    // (1) An ANNOTATION `(: (fn …) (-> P R))` directly on the lambda — the parent is a `(:` form whose
    //     second child is the type expression.
    let parent = db.parent_of(lambda)?;
    if let Some(ann) = db.ast.as_form(parent, ":")
        && ann.len() == 2
        && ann[0] == lambda
        && let Some(t @ Ty::Fn(_, _)) = crate::eval::typeval_of(db, ann[1])
    {
        return Some(t);
    }
    // (2) A CONSTRUCTOR-PAYLOAD position — the parent is an application `(ctor … lambda …)` whose head is a
    //     variant constructor. The lambda's expected type is the ctor's payload type at that argument
    //     position. Read the ctor's `(-> payload… Sum)` scheme and take the payload at the lambda's index.
    let crate::ast::Struct::List(list) = db.ast.get(parent) else {
        return None;
    };
    let list = list.clone();
    let head = *list.first()?;
    let arg_ix = list.iter().skip(1).position(|&c| c == lambda)?;
    if crate::eval::variant_disc_of(db, head).is_some() {
        let mut fresh = Fresh::new();
        let scheme = crate::eval::scheme_of(db, head, &mut fresh)?;
        let inst = crate::unify::instantiate(&scheme, &mut fresh);
        // Peel to the arg_ix-th arrow parameter — the payload this argument fills.
        let mut cur = inst;
        for _ in 0..arg_ix {
            match cur {
                Ty::Fn(_, r) => cur = *r,
                _ => return None,
            }
        }
        if let Ty::Fn(p, _) = cur
            && matches!(*p, Ty::Fn(_, _))
        {
            return Some(*p);
        }
    }
    // (3) A FUNCTION-ARGUMENT position — the parent is an application `(f … lambda …)` whose head `f` is a
    //     FUNCTION (a lambda / a top-level def) that DECLARES an arrow at the lambda's parameter slot: a
    //     `(def (app (: g (-> Int8 Int8))) …)` applied `(app (fn (n) …))` types the lambda from `app`'s
    //     `g` param. Read `f`'s parameter occurrences and take the type of the one at the lambda's index;
    //     if it is itself an arrow, that is the lambda's expected type. This is the argument-position
    //     analogue of the constructor-payload case above — a declared higher-order parameter is exactly
    //     the "storage context declares an arrow" the recovery serves, so an unannotated closure argument's
    //     narrow param width reaches the body's const-fold (a const arg then overflows at the declared
    //     width, matching an explicit `(fn ((: n Int8)) …)` — else the const-fold runs at the default Int64
    //     and MISSES the narrow overflow). A non-function head, or a param slot that is not an arrow, yields
    //     no expected arrow (the HOF call's own unification, or a genuinely unconstrained position).
    if let Some(params) = crate::eval::lambda_params_of(db, head)
        && let Some(&param_occ) = params.get(arg_ix)
        && let t @ Ty::Fn(_, _) = type_of(db, param_occ)
    {
        return Some(t);
    }
    // (3b) A FUNCTION-VALUED head that is NOT itself a lambda/def — a VARIABLE bound to a function type,
    //      e.g. a higher-order PARAMETER `g : (-> (-> A B) R)` applied `(g lambda)`. Path (3) can't read it
    //      (`lambda_params_of` needs `head` to be a lambda/def), but `head`'s OWN type IS the arrow
    //      `A0 → … → R`; the lambda fills argument slot `arg_ix`, so its expected type is `A_{arg_ix}`. Peel
    //      `type_of(head)` by `arg_ix` arrows and take that domain if it is itself an arrow. This lets an
    //      UNANNOTATED inner closure `(fn (p) …)` passed to a higher-order closure parameter recover its
    //      param type (e.g. `p : (Tuple …)`) from the parameter's declared arrow, instead of solving `Any`
    //      and declining "a closure's parameter type has no machine representation" (a bare `(fn (p) (. p 0))`
    //      in `(g (fn (p) …))`). A non-arrow head type, or a slot that is not an arrow, recovers nothing.
    {
        let mut cur = type_of(db, head);
        let mut peeled_ok = true;
        for _ in 0..arg_ix {
            match cur {
                Ty::Fn(_, r) => cur = *r,
                _ => {
                    peeled_ok = false;
                    break;
                }
            }
        }
        if peeled_ok
            && let Ty::Fn(p, _) = cur
            && matches!(*p, Ty::Fn(_, _))
        {
            return Some(*p);
        }
    }
    None
}

/// The type of an UNANNOTATED lambda PARAMETER recovered from the lambda's storage-context arrow — the
/// per-parameter analogue of [`expected_arrow_for_lambda`], read at the PARAMETER's own `type_of`. A
/// bare `(fn (n) …)` param types `Any` bottom-up (no annotation, no def entry), but when the lambda sits
/// where an arrow is DECLARED — a `(: g (-> Int8 Int8))` higher-order parameter it is passed to, a
/// variant payload, an annotation — that arrow fixes the param's type. Recovering it HERE (not only at
/// `lower_lambda_value`, which runs at LOWERING) is what makes the body's type-check / const-fold see the
/// narrow width: `(+ n 1)` over a context-Int8 `n` then overflows a const arg exactly as an explicit
/// `(fn ((: n Int8)) …)` does, instead of folding at the default Int64 and missing the overflow. `None`
/// unless `binder` is a bare lambda param whose lambda has a recoverable arrow with a concrete type at
/// this param's index (so an annotated param, a non-lambda binder, or an unconstrained position is
/// unaffected — no invented width, no over-reject).
fn lambda_param_ty_from_context(db: &mut Db, binder: StructId) -> Option<Ty> {
    // The binder of a bare lambda param IS the param node, sitting in the lambda's `(fn (p0 p1 …) body)`
    // parameter list. Find that params list (the binder's parent) and the lambda (its grandparent), and
    // the binder's index within the list.
    let params_list = db.parent_of(binder)?;
    let lambda = db.parent_of(params_list)?;
    let crate::resolved::Resolved::Lambda { params, .. } = resolved_of(db, lambda) else {
        return None;
    };
    // The index of THIS binder among the lambda's params (compare against each param's name occurrence,
    // so an annotated sibling `(: m T)` is matched through its `:` form too — though an annotated binder
    // never reaches here, since `param_annot_ty` handled it before this fallback).
    let idx = params
        .iter()
        .position(|&p| crate::eval::param_name_occ(db, p) == binder)?;
    // Peel the lambda's expected arrow to its idx-th parameter type; use it only if concrete (an arrow
    // that runs out of parameters, or an `Any`/unsolved-`Var` at this slot, recovers nothing).
    let mut cur = expected_arrow_for_lambda(db, lambda)?;
    for _ in 0..idx {
        match cur {
            Ty::Fn(_, r) => cur = *r,
            _ => return None,
        }
    }
    match cur {
        // Only a DETERMINED domain wins here. A HOLE at this slot — `Any` OR a free `Var` (the
        // context is a fully-generic HOF param like `f : (-> _ (-> _ _))`, whose domains are unsolved
        // vars, not `Any`) — recovers NOTHING, so `type_of(p)` falls to `Any` and `lower_lambda_value`
        // reaches its body-solve (`solve_lambda_param_ty`). Returning the hole instead would preempt
        // that solve with an unsolvable `Var` and decline "no machine representation" — the exact bug a
        // bare closure passed to a recursive HOF hit (`fold-list (fn (x a) (+ a x)) …`): the closure's
        // OWN body (`(+ a x)` → Int64) pins its params, but only if the context hole doesn't shadow it.
        Ty::Fn(p, _) if !matches!(*p, Ty::Any) && !p.has_free_var() => Some(*p),
        _ => None,
    }
}

/// The `db.defs` index whose signature declares the parameter name-occurrence `binder`, or `None` if
/// `binder` is not a top-level def parameter (e.g. a `fn` lambda parameter). Walks up from the binder to
/// the `(def (NAME param…) body)` it sits in.
fn def_of_param(db: &mut Db, binder: StructId) -> Option<usize> {
    // A param occurrence is either bare (`binder`'s parent is the signature list) or the name of a
    // `(: name T)` binder (parent is the `:` form, whose parent is the signature). Find the signature
    // list, then the def whose `sig_occ` is it.
    let parent = db.parent_of(binder)?;
    let sig = if db.ast.as_form(parent, ":").is_some() {
        db.parent_of(parent)?
    } else {
        parent
    };
    if let Some(i) = db.def_index_by_sig(sig) {
        return Some(i);
    }
    // A MODULE-MEMBER internal def (`Def::internal`, `modules::register_callable`) reuses the member's
    // signature params, but `modules::synthesize` wraps the member body in a synth `(fn params body)`
    // that RE-PARENTS those very param occurrences under the synth `fn`'s params-list — so the parent
    // walk above lands on the synth `fn`, not the member's `(NAME param…)` sig, and no def's `sig_occ`
    // matches. Fall back to a param-MEMBERSHIP scan: the internal def's `params` hold those same
    // occurrences, so match the def whose param list contains `binder`. Scoped to internal defs (the
    // re-parenting only affects a synth-wrapped member); a tiny linear scan (few internal defs).
    db.defs
        .iter()
        .position(|d| d.internal && d.params.contains(&binder))
}

/// Is `id` OUTSIDE the body of every def whose param/scheme solve is currently on the stack? True when
/// `id` sits in NO in-flight def's body subtree — i.e. a CALLER re-entering a producer's solve (typing an
/// argument in `main` whose element is a call to the producer being solved), NOT the producer's own
/// self-recursion. Distinguishes the self-nested-generic-PRODUCER re-entrancy (`(from-list (list (inner)
/// (inner)))` typed in `main` while `from-list`'s param solve is on the stack — external) from a
/// MONOMORPHIC recursive self-call (`(tuple (fold a) (fold b))` typed inside `fold`'s own body while
/// `fold`'s solve is on the stack — internal). Used by the memo guard: only skip caching a nested-`Any`
/// born EXTERNALLY (a re-entrant caller read), leaving an internal self-call's concrete result cached.
fn node_external_to_inflight_solves(db: &mut Db, id: StructId) -> bool {
    if db.solving_params.is_empty() && db.solving_schemes.is_empty() {
        return false;
    }
    // Walk `id`'s ancestor chain; if it passes through the body occurrence of any in-flight def, `id` is
    // INTERNAL to that def (a self-call typing). Reaching the root without hitting one → EXTERNAL.
    let mut cur = Some(id);
    while let Some(n) = cur {
        if let Some(d) = db.def_index_by_body(n)
            && (db.solving_params.contains(&d) || db.solving_schemes.contains(&d))
        {
            return false; // inside an in-flight def's own body
        }
        cur = db.parent_of(n);
    }
    true
}

/// Solve the parameter types of a RECURSIVE def by a single connected, threaded-`Subst` unification
/// over its body — the one place inference is a connected solve rather than a per-node column read
/// (ANF step 2 / A2). Fills `db.param_types` for EVERY parameter of the def at once.
///
/// The method: give each parameter a fresh type variable and bind it in a local env; walk the body
/// collecting constraints on those variables — where a parameter (or an expression over it) is used as
/// an operand of a built-in operation, unify its variable with the operand type the operation's SCHEME
/// requires; where a self-call passes an argument in a parameter position, unify the argument's type
/// with that parameter's variable (the fixpoint — a recursive call constrains the very signature being
/// solved). Then ground each variable: a solved variable becomes its concrete type; an UNCONSTRAINED
/// variable (no use pinned it) grounds to the default integer only if a use marked it numeric, else it
/// is left `Any` (the parameter is genuinely unconstrained — the export/select layer then declines,
/// asking for an annotation, rather than the compiler inventing a type). Order-independent because
/// unification is: the constraints commute, so demand order cannot change the solution.
fn solve_recursive_params(db: &mut Db, def: usize) {
    // Re-entry backstop: if this def's solve is already on the stack, do not recompute (a demand landing
    // mid-solve reads the provisional/absent entry). The local-env walk below does not call `type_of`
    // on a param, so this is defensive.
    if db.solving_params.contains(&def) {
        return;
    }
    db.solving_params.insert(def);

    let sig_params = db.defs[def].params.clone();
    let mut fresh = Fresh::new();
    // Each parameter's name occurrence → its fresh type variable. An ANNOTATED param uses its
    // annotation type as a fixed constraint (not a fresh var), so a mixed signature still solves.
    let mut env: crate::fxhash::FxHashMap<StructId, Ty> = crate::fxhash::FxHashMap::default();
    let mut param_binders: Vec<(StructId, Ty)> = Vec::new();
    for p in &sig_params {
        let binder = match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *p,
        };
        let var = param_annot_ty(db, binder).unwrap_or_else(|| Ty::Var(fresh.var()));
        env.insert(binder, var.clone());
        param_binders.push((binder, var));
    }

    let mut subst = Subst::new();
    if let Some(body) = db.defs[def].body {
        // PRE-PASS: give every FN-TYPED parameter its arrow shape BEFORE the main constraint walk. A
        // parameter applied as a function (`(f h)`) must be an arrow when any enclosing operator reads
        // `(f h)`'s type — otherwise the operator (`(+ (f h) …)`) unifies `f`'s bare var directly with the
        // operand type (Int64), collapsing `f` to a scalar instead of a function. This walk unifies each
        // env-param head with `(-> a0 … aN result)` of fresh vars (arity = the application's argument
        // count), so `f`'s var is `Ty::Fn(…)` by the time its result flows back from `(+ (f h) …)`.
        shape_fn_typed_params(db, body, &env, &mut subst, &mut fresh);
        collect_param_constraints(db, body, &env, def, &mut subst, &mut fresh);
    }

    // CALL-SITE SEEDING: a parameter the BODY alone cannot ground — its type is decided only by HOW a
    // caller invokes the def (`(def (lookup xs i k) (match (List.at xs i) ((Some (tuple key val)) …)))`
    // never pins `xs`'s element from the body; `main`'s `(lookup (list (tuple 1 100) …) 0 2)` does). For
    // each param still an open var after the body walk, find a NON-recursive call site of this def (a
    // caller's application whose head resolves here) and unify its k-th argument's type into the param
    // var. This is monomorphic call-site inference — the caller supplies the concrete type the body
    // leaves generic. Only fires for a still-open param (a body-solved param is untouched), and only a
    // DETERMINED argument type constrains (an `Any`/var arg adds nothing), so a well-solved def is
    // byte-identical. Skips a self/recursive call (its args reference this def's own unsolved params —
    // no new information) and is re-entry-guarded by `solving_params`.
    // The per-position call-site argument types, computed lazily only when a param is still open, then
    // used in the grounding loop to `fill_holes` each open param.
    let mut call_seed_arg_tys: Vec<Option<Ty>> = Vec::new();
    if let Some(body) = db.defs[def].body {
        // A param is "open" if its body-solved type still has a HOLE the body could not pin — a free
        // `Var` (`has_free_var`) OR an `Any` anywhere (`has_any`; the body grounds an UNCONSTRAINED
        // position to `Any`, not a `Var`, so a `(. t 1)` the body never uses lands `(Tuple Int64 Any)`).
        // A call site's concrete argument fills those holes.
        let any_open = param_binders.iter().any(|(_, v)| {
            let t = subst.apply(v);
            t.has_free_var() || t.has_any()
        });
        if any_open {
            call_seed_arg_tys = call_site_arg_types(db, def, body);
        }
    }

    // RECURSIVE-GENERIC MONOMORPHIZATION: a parameter the body only THREADS (its body-solved type is
    // still a free var) AND that callers invoke at TWO OR MORE distinct concrete types is GENERIC — it
    // must NOT be pinned to the first caller's type (which would make a second-type call a spurious
    // CDZ0203). Detect those positions from the body-solved types + the call-site type spread; leave each
    // a canonical `Ty::Var` so a call-site arg of any type unifies with it, and `lower` monomorphizes the
    // call to a per-instantiation copy (`DESIGN-recursive-generic-monomorphization-rcdzc.md`). A param
    // called at ≤1 type stays MONOMORPHIC (seeded below exactly as before — byte-identical).
    let body_solved: Vec<Ty> = param_binders.iter().map(|(_, v)| subst.apply(v)).collect();
    let generic_positions = db.defs[def]
        .body
        .map(|b| generic_param_positions(db, def, b, &body_solved))
        .unwrap_or_default();

    // CROSS-PARAM SEED UNIFY: before grounding each parameter in isolation, unify every NON-generic
    // parameter's call-site argument type into the SHARED `subst`. A type variable the body SHARES across
    // two parameters — `(gmap it f)` with `(match it … ((Iter.Cons h rest) (Iter.Cons (f h) …)))` solves
    // `it : Iter ?e` and `f : (-> ?e ?r)`, the SAME `?e` — must be pinned CONSISTENTLY: the single call
    // `(gmap (Iter.Cons 1 …) (fn (x) x))` fixes `it`'s element `?e = Int64` from its argument, and that
    // must flow to `f`'s DOMAIN too. The per-param `fill_holes` below operates on each param's value in
    // isolation, so it pinned `it`'s `?e` to Int64 while `f`'s `?e` stayed a free var → the emitted scheme
    // was `(-> (Iter Int64) (-> (-> _ _) …))` (domain disconnected from the element) and monomorphization
    // declined CDZ0201 even at a single element type (the recursive-transformer closure-tie gap). Unifying
    // the call-seed into `subst` FIRST pins the shared var once, so both params read it. Only a DETERMINED
    // seed (not `Any`/`Var`) for a NON-generic position unifies; a generic position is left for per-call
    // monomorphization (its var must stay quantified). Unify is order-independent and only ADDS bindings a
    // hole would otherwise take from `fill_holes`, so a program that already solved is unaffected.
    // SOUNDNESS: `unify` mutates `subst` IN PLACE as it binds vars during its recursive descent, so a
    // partway-FAILING unify (some early sub-unifications bound, then a mismatch) would leave `subst`
    // partially updated — later grounding would then read a binding from a unification that ultimately
    // failed, an inconsistent substitution (PR#462 reviewer hazard). Unify against a TRIAL CLONE and COMMIT
    // only on `Ok`, so a failed call-seed unify (the seed didn't constrain this param — a benign non-match)
    // leaves `subst` untouched rather than half-bound. The success path is byte-identical to the old
    // in-place unify; only the failure path is now clean.
    for (i, (_, var)) in param_binders.iter().enumerate() {
        if generic_positions.contains(&i) {
            continue;
        }
        let cur = subst.apply(var);
        if (cur.has_free_var() || cur.has_any())
            && let Some(Some(at)) = call_seed_arg_tys.get(i)
            && !matches!(at, Ty::Any | Ty::Var(_))
        {
            let mut trial = subst.clone();
            if crate::unify::unify(&mut trial, &cur, at, &db.name_ctx()).is_ok() {
                subst = trial;
            }
        }
    }

    // Ground each parameter: apply the substitution, FILL any remaining hole (`Any`/free `Var`) from the
    // call-site argument type — a plain unify cannot repair an `Any` (`Any` absorbs), so `fill_holes`
    // merges the determined call-site type into the open positions while keeping the body-pinned parts —
    // then default a still-unsolved NUMERIC variable to the signed-64 integer (a bare literal's default)
    // and leave anything else `Any` (genuinely unconstrained — no call site fixed it).
    for (i, (binder, var)) in param_binders.into_iter().enumerate() {
        // A GENERIC position stays a canonical `Ty::Var` in its free slots (quantified in the scheme): do
        // not seed it from a call site (which would pin it to ONE type) and do not ground it to `Any`.
        // PRESERVE the body-solved SHAPE when the body gave the param structure — `(match xs ((list h .. t)
        // …))` shapes `xs` to `List ?a` (via `pattern_implied_ty`'s list arm), and keeping that shape (with
        // its free element var) is what lets a body building a generic result FROM the element tie the
        // result's element to the param's. A bare fresh `Var` would DISCARD the `List` shape (a
        // recursive-generic producer's param would lose its container). Only a param the body left a BARE
        // `Var` (structurally unshaped — a plain threaded value) gets a fresh canonical var, byte-identical
        // to before for that case.
        if generic_positions.contains(&i) {
            let solved = subst.apply(&var);
            let generic = if matches!(solved, Ty::Var(_)) {
                Ty::Var(fresh.var())
            } else {
                solved
            };
            trace!(target: "rcdzc::infer", def, binder = binder.0, ty = %generic.render_name(&db.name_ctx()), "A2: recursive param is GENERIC (monomorphized per call site)");
            db.param_types.insert(binder, generic);
            continue;
        }
        let mut solved = subst.apply(&var);
        if (solved.has_free_var() || solved.has_any())
            && let Some(Some(at)) = call_seed_arg_tys.get(i)
            && !matches!(at, Ty::Any | Ty::Var(_))
        {
            solved = solved.fill_holes(at);
        }
        let grounded = ground_param(solved);
        trace!(target: "rcdzc::infer", def, binder = binder.0, ty = %grounded.render_name(&db.name_ctx()), "A2: solved recursive param");
        db.param_types.insert(binder, grounded);
    }

    db.solving_params.remove(&def);
}

/// The per-position ARGUMENT TYPES a NON-RECURSIVE caller of `def` supplies, for call-site inference of a
/// parameter the body alone leaves open. Scans every def body (except `def`'s own — a self-call's args
/// reference the very params being solved, so it adds nothing) for an application whose head resolves to
/// `def`, and returns the k-th argument's `type_of` at the FIRST such call site. `own_body` is `def`'s
/// body, skipped so a self-recursive call is never mistaken for an external seed. Returns a vector indexed
/// by parameter position; an entry is `None` when no call site determines that position. A conservative,
/// read-only scan — it never mutates `db.param_types` and only READS argument types the caller already
/// has (a caller's own params were solved by its own pass, or are concrete literals/constructors).
fn call_site_arg_types(db: &mut Db, def: usize, own_body: StructId) -> Vec<Option<Ty>> {
    // The call sites of `def` — from the CALL-SITE INDEX (built once), not a fresh whole-program scan per
    // query (which was O(defs × program) → O(N²) for N mutually-recursive defs each seeded this way). A
    // call site in `def`'s OWN body carries no external type info (its args reference the very params
    // being solved), so those are excluded when the index is built (keyed to exclude the callee's own body
    // is not possible — a def can call itself — so the index records the CALLER body with each site and we
    // skip `own_body` here).
    ensure_call_site_index(db);
    let call_args: Vec<Vec<StructId>> = db
        .call_sites_by_callee
        .as_ref()
        .and_then(|m| m.get(&def))
        .map(|sites| {
            sites
                .iter()
                .filter(|(caller_body, _)| *caller_body != own_body)
                .map(|(_, args)| args.clone())
                .collect()
        })
        .unwrap_or_default();
    // The widest arg list seen determines the result arity; take the first call site that fixes each
    // position (a determined `type_of`), so multiple call sites together can seed distinct positions.
    let arity = call_args.iter().map(Vec::len).max().unwrap_or(0);
    let mut out: Vec<Option<Ty>> = vec![None; arity];
    for args in &call_args {
        for (i, &arg) in args.iter().enumerate() {
            if out[i].is_none() {
                let mut t = type_of(db, arg);
                if matches!(t, Ty::Any | Ty::Var(_))
                    && let Some((d, j)) = arg_is_other_def_param(db, arg)
                    && !db.seed_transitive.contains(&d)
                {
                    db.seed_transitive.insert(d);
                    if let Some(d_body) = db.defs[d].body {
                        let d_args = call_site_arg_types(db, d, d_body);
                        if let Some(Some(at)) = d_args.get(j)
                            && !matches!(at, Ty::Any | Ty::Var(_))
                        {
                            t = at.clone();
                        }
                    }
                    db.seed_transitive.remove(&d);
                }
                if !matches!(t, Ty::Any | Ty::Var(_)) {
                    out[i] = Some(t);
                }
            }
        }
    }
    out
}

/// The per-position set of DISTINCT concrete argument types a NON-recursive caller supplies to `def` —
/// the raw material for deciding a parameter is GENERIC (recursive-generic monomorphization). Unlike
/// `call_site_arg_types` (which takes the FIRST determined type per position to seed a monomorphic
/// signature), this collects EVERY distinct determined type at each position, so a position invoked at
/// two different types (`(loopn 3 a)` : Int64 and `(loopn 2 "hi")` : String) is detected as generic.
/// `own_body` is `def`'s body, skipped (a self-call's args reference the very params being solved). A
/// read-only scan over the call-site index; only DETERMINED (non-`Any`/non-`Var`) types are recorded.
/// Walk a starting type down a match-pattern access PATH, returning the type at the binder position.
/// A `Payload` step descends into a variant's payload AT THE CURRENT INSTANTIATION
/// (`payload_ty_at_instantiation` unifies the head's `(-> payload Sum)` result against `cur`; a nominal
/// newtype unwraps to its inner); an `Elem(i)` step reads a tuple element / list element type; a
/// `RestFrom` step keeps the list type. `Any` on a malformed path or a non-matching type. SHARED by the
/// `Resolved::SumPayload` type_of arm (which starts from the scrutinee's solved type) and the transitive
/// spread projection in `call_site_distinct_arg_types` (which starts from a caller-param's CONCRETE
/// call-site type, so a binder over a GENERIC caller param inherits the param's per-instantiation
/// sub-type — the L6512 `reduce1`->`go` delegated element).
fn project_path_type(
    db: &mut Db,
    mut cur: Ty,
    steps: &[crate::core::PathStep],
    heads: &[StructId],
) -> Ty {
    let mut heads = heads.iter();
    for step in steps.iter() {
        cur = match step {
            crate::core::PathStep::Payload => {
                let Some(&head) = heads.next() else {
                    return Ty::Any; // malformed path (fewer heads than Payload steps)
                };
                // Over a NOMINAL NEWTYPE the `Payload` step UNWRAPS the tag to its underlying type (a
                // runtime no-op — the value is unchanged). `(Mk n)` over a `Ty::Nominal { inner: Int64 }`
                // binds `n : Int64`.
                if let Ty::Nominal { inner, .. } = &cur {
                    (**inner).clone()
                } else {
                    match payload_ty_at_instantiation(db, head, &cur) {
                        Some(t) => t,
                        None => return Ty::Any,
                    }
                }
            }
            crate::core::PathStep::Elem(i) => match &cur {
                Ty::Tuple(elems) => match elems.get(*i) {
                    Some(t) => t.clone(),
                    None => return Ty::Any,
                },
                // A list-pattern element binder — every element of a `List T` has type `T` (homogeneous),
                // regardless of the index.
                Ty::List(elem) => (**elem).clone(),
                _ => return Ty::Any,
            },
            // A list-pattern REST binder — the tail sublist is still a `List T` (same type as the list
            // scrutinee), independent of where the tail starts.
            crate::core::PathStep::RestFrom(_) => match &cur {
                Ty::List(_) => cur.clone(),
                _ => return Ty::Any,
            },
        };
    }
    cur
}

fn call_site_distinct_arg_types(db: &mut Db, def: usize, own_body: StructId) -> Vec<Vec<Ty>> {
    ensure_call_site_index(db);
    let call_args: Vec<Vec<StructId>> = db
        .call_sites_by_callee
        .as_ref()
        .and_then(|m| m.get(&def))
        .map(|sites| {
            sites
                .iter()
                .filter(|(caller_body, _)| *caller_body != own_body)
                .map(|(_, args)| args.clone())
                .collect()
        })
        .unwrap_or_default();
    let arity = call_args.iter().map(Vec::len).max().unwrap_or(0);
    let mut out: Vec<Vec<Ty>> = vec![Vec::new(); arity];
    for args in &call_args {
        for (i, &arg) in args.iter().enumerate() {
            let t = type_of(db, arg);
            if !matches!(t, Ty::Any | Ty::Var(_)) {
                if !out[i].contains(&t) {
                    out[i].push(t);
                }
                continue;
            }
            // TRANSITIVE genericity: the argument itself typed as a `Var`/`Any` — but if it is ANOTHER
            // def's parameter (`(wrap m y)` calling `(idr 2 y)` — `idr`'s x is fed `wrap`'s `y`), that
            // caller-param's OWN distinct-type spread flows through. So `idr.x` inherits `wrap.y`'s
            // `{Int64, String}` and is detected generic, even though `idr` has only ONE syntactic call
            // site. Guarded by `seed_transitive` against a cycle (a mutually-recursive generic pair).
            if let Some((d, j)) = arg_is_other_def_param(db, arg)
                && !db.seed_transitive.contains(&d)
                && db.defs[d].body.is_some()
            {
                db.seed_transitive.insert(d);
                let d_body = db.defs[d].body.unwrap();
                let d_spread = call_site_distinct_arg_types(db, d, d_body);
                db.seed_transitive.remove(&d);
                if let Some(tys) = d_spread.get(j) {
                    for ty in tys {
                        if !out[i].contains(ty) {
                            out[i].push(ty.clone());
                        }
                    }
                }
            } else if let Resolved::SumPayload {
                scrutinee,
                steps,
                heads,
            } = resolved_of(db, arg)
                && let Some((d, j)) = arg_is_other_def_param(db, scrutinee)
                && !db.seed_transitive.contains(&d)
                && db.defs[d].body.is_some()
            {
                // TRANSITIVE genericity through a MATCH BINDER (not just a direct param). The seed is a
                // pattern binder DESTRUCTURED from a caller PARAM: `reduce1`'s `(go rest h f)` seeds `go`'s
                // acc with `h`, the `Cons` HEAD binder of reduce1's generic iterator param `it`.
                // `arg_is_other_def_param` sees only a DIRECT param, so the binder inherited nothing → `go`'s
                // acc grounded to `Any` → the transitive tie DECLINED (L6512). Recover the spread by
                // projecting the caller param's per-call-site CONCRETE types through the SAME pattern PATH to
                // the binder's sub-position: reduce1's `it` is called at `GIter Int64` + `GIter String`, and
                // the `Cons`-head path projects those to `{Int64, String}` — the two element types `go`'s acc
                // is genuinely generic over. Guarded by `seed_transitive` against a cycle, exactly as the
                // direct-param case above; a binder over a MONOMORPHIC scrutinee projects a single concrete
                // type (or `Any`/`Var`, dropped) and stays non-generic.
                db.seed_transitive.insert(d);
                let d_body = db.defs[d].body.unwrap();
                let d_spread = call_site_distinct_arg_types(db, d, d_body);
                db.seed_transitive.remove(&d);
                if let Some(scrut_tys) = d_spread.get(j).cloned() {
                    for st in scrut_tys {
                        let bt = project_path_type(db, st, &steps, &heads);
                        if !matches!(bt, Ty::Any | Ty::Var(_)) && !out[i].contains(&bt) {
                            out[i].push(bt);
                        }
                    }
                }
            }
        }
    }
    out
}

/// The parameter POSITIONS of recursive def `def` that are GENUINELY GENERIC — a parameter the body
/// only threads (its body-solved type `solved[i]` is still a free `Var`, so NO operator/self-call pinned
/// it to a concrete type) AND that the callers invoke at TWO OR MORE distinct concrete types. Such a
/// parameter must NOT be pinned to the first call site's type (the monomorphic-seeding default): doing so
/// makes a second call at a different type a spurious CDZ0203. Instead it stays a quantified var in the
/// scheme, and `lower` monomorphizes the call by synthesizing a per-instantiation copy. A parameter
/// called at ONE type (or zero — an unexported generic library def) is NOT generic here: it seeds
/// monomorphically exactly as before (byte-identical). `solved` is the body-solved param type vector
/// (post-`subst.apply`), indexed by parameter position.
fn generic_param_positions(db: &mut Db, def: usize, body: StructId, solved: &[Ty]) -> Vec<usize> {
    // A position is a candidate only if the body left it a free var (threaded, never constrained). A
    // position the body already pinned to a concrete type is monomorphic — leave it alone.
    let candidates: Vec<usize> = solved
        .iter()
        .enumerate()
        .filter(|(_, t)| t.has_free_var())
        .map(|(i, _)| i)
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let distinct = call_site_distinct_arg_types(db, def, body);
    candidates
        .into_iter()
        .filter(|&i| distinct.get(i).is_some_and(|tys| tys.len() >= 2))
        .collect()
}

fn arg_is_other_def_param(db: &mut Db, arg: StructId) -> Option<(usize, usize)> {
    let binder = match resolved_of(db, arg) {
        Resolved::Ref { value } => value,
        Resolved::Param { binder } => binder,
        _ => return None,
    };
    let d = def_of_param(db, binder)?;
    let pos = db.defs[d].params.iter().position(|&p| {
        let name_occ = db
            .ast
            .as_form(p, ":")
            .and_then(|t| t.first().copied())
            .unwrap_or(p);
        name_occ == binder
    })?;
    Some((d, pos))
}

/// Build the CALL-SITE INDEX (`db.call_sites_by_callee`) if not already built: `callee def index → the
/// (caller-body, argument-occurrences) of every application whose head resolves to that callee`. ONE
/// whole-program walk over every def body (`callee_def_index_for_infer` per application) replaces the
/// per-query all-bodies scan `call_site_arg_types` did — O(program) once instead of O(defs × program).
/// The caller body is recorded with each site so `call_site_arg_types` can skip the callee's OWN body (a
/// self-call carries no external type info). A pure function of the resolved program.
fn ensure_call_site_index(db: &mut Db) {
    if db.call_sites_by_callee.is_some() {
        return;
    }
    let mut index: crate::db::CallSiteIndex = crate::fxhash::FxHashMap::default();
    let bodies: Vec<StructId> = db.defs.iter().filter_map(|d| d.body).collect();
    for body in bodies {
        collect_calls_into_index(db, body, body, &mut index);
    }
    db.call_sites_by_callee = Some(index);
}

/// The number of call sites whose head resolves to `callee`, across the whole program (the call-site
/// index, built once + cached). The inline COST HEURISTIC (`lower::should_emit_once_by_cost`) uses this to
/// require ≥ N callers before it prefers emit-once — a def called once gains nothing from a shared
/// function. Counts every application occurrence (including a self-call, which the index records); the
/// heuristic only consults this for a NON-recursive callee, so self-calls do not distort the decision.
pub(crate) fn callee_call_site_count(db: &mut Db, callee: usize) -> usize {
    ensure_call_site_index(db);
    db.call_sites_by_callee
        .as_ref()
        .and_then(|idx| idx.get(&callee))
        .map_or(0, |sites| sites.len())
}

/// Walk `node` (within caller body `caller_body`), recording into `index` every application whose head
/// resolves to a user def — keyed by that callee's index, valued by `(caller_body, argument-occurrences)`.
/// Recurses through all structural children so a call nested anywhere is found.
fn collect_calls_into_index(
    db: &mut Db,
    node: StructId,
    caller_body: StructId,
    index: &mut crate::db::CallSiteIndex,
) {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && let Some(callee) = callee_def_index_for_infer(db, head)
    {
        index
            .entry(callee)
            .or_default()
            .push((caller_body, args.to_vec()));
    }
    if let crate::ast::Struct::List(children) = db.ast.get(node) {
        for c in children.clone() {
            collect_calls_into_index(db, c, caller_body, index);
        }
    }
}

/// Walk the resolved body and, for every application whose HEAD is a parameter in `env` (a fn-typed
/// parameter applied as a function, `(f h)`), unify that parameter's variable with a curried arrow of
/// FRESH vars — one arrow level per argument, a fresh result var. Runs BEFORE `collect_param_constraints`
/// so a fn-typed parameter already has its `Ty::Fn` shape when the main walk reads an application of it
/// (otherwise an enclosing operator collapses the bare param var to a scalar). Idempotent per param: a
/// second application at the same arity re-unifies the same shape; a genuine arity mismatch is a fault
/// reported elsewhere. Descends every sub-expression that runs (mirrors `collect_param_constraints`'
/// structural coverage via the generic child walk).
fn shape_fn_typed_params(
    db: &mut Db,
    node: StructId,
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
    subst: &mut Subst,
    fresh: &mut Fresh,
) {
    if let Resolved::Apply { head, args } = resolved_of(db, node)
        && let Some(hvar) = binder_var_of(db, head, env)
        // Only shape a head that is NOT a known callable (a def/op has its own scheme path) — a bare
        // fn-typed parameter. `binder_var_of` already restricts to an env param, so this holds.
        && crate::eval::scheme_of(db, head, fresh).is_none()
        && callee_def_index_for_infer(db, head).is_none()
    {
        let result = Ty::Var(fresh.var());
        let mut arrow = result;
        for _ in 0..args.len() {
            arrow = Ty::Fn(Box::new(Ty::Var(fresh.var())), Box::new(arrow));
        }
        let _ = crate::unify::unify(subst, &hvar, &arrow, &db.name_ctx());
    }
    // Descend into every child (the head and args of an apply, and all structural children of any form)
    // so a fn-typed-param application nested anywhere in the body is shaped.
    if let crate::ast::Struct::List(children) = db.ast.get(node) {
        for c in children.clone() {
            shape_fn_typed_params(db, c, env, subst, fresh);
        }
    }
}

/// The declared default-integer type for the bare literal at `id`, or `None` if it is not written in a
/// `(pragma default-integer <T>)` module. Reads the load-time `default_int_literals` map (keyed by the
/// literal's ORIGINAL node, so it survives β-copy reparenting) and reduces the recorded `<T>` occurrence
/// to a `Ty` via the ordinary evaluator (`typeval_of`, the same path an annotation's type takes). Only an
/// INTEGER type is honored (a non-integer `<T>` is separately the CDZ0303 domain reject); anything that
/// does not reduce to a concrete integer type is `None` (the literal keeps the deferred `Int64` default).
///
/// So a module MAY declare — through the `(pragma default-integer <T>)` directive — the integer type an
/// otherwise-unconstrained literal takes within it; and when a module declares none, `None` here lets the
/// caller fall back to the numeric model's default integer type (`Int64`).
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-integer-literal-type
//# A module MAY declare, through a module directive (modules-and-namespaces.md §"A Module Directive Is Drawn From A Fixed Set"), the integer type that an integer literal with no other constraint takes within that module.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-integer-literal-type
//# When a module declares no default integer literal type, an integer literal with no other constraint MUST take the numeric model's default integer type.
fn module_default_int_ty(db: &mut Db, id: StructId) -> Option<Ty> {
    let ty_expr = *db.default_int_literals.get(&id)?;
    let ty = crate::eval::typeval_of(db, ty_expr)?;
    matches!(ty, Ty::Int(_) | Ty::BigInt).then_some(ty)
}

/// The EXACT-RATIONAL type a bare numeric literal (integer OR decimal) grounds to when it is WRITTEN in a
/// `(pragma default-fraction <T>)` module. Reads the load-time `default_fraction_literals` map (keyed by
/// the literal's ORIGINAL node, β-copy-robust) and reduces the recorded `<T>` occurrence to a `Ty`. Only
/// `Ty::Rational` is honored (a non-rational `<T>` is separately the CDZ0303 domain reject); anything that
/// does not reduce to `Rational` is `None` (the literal keeps its ordinary default). This fixes a TYPE,
/// not a conversion: no-silent-promotion still holds, and an explicit annotation on the literal wins (the
/// `Annot` node fixes its own type, so a literal inside an annotation never reaches this arm of `compute`).
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-fraction-literal-type
//# A module MAY declare, through a module directive (modules-and-namespaces.md §"A Module Directive Is Drawn From A Fixed Set"), that a numeric literal with no other constraint takes an exact fraction type within that module, so that ordinary arithmetic in that module is exact by default.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-fraction-literal-type
//# A declared default fraction literal type MUST apply to both an integer-written literal and a decimal-written literal with no other constraint: an integer literal takes the whole value (a denominator of one) and a decimal literal takes the exact fraction its written digits denote, with no rounding.
// The definition-site, no-conversion, and annotation-precedence rules are the SAME three the default-
// integer twin obeys: the default in force is the one the literal's OWN module declares (the map is
// `default_fraction_literals`, keyed per pragma-module by the original literal node — definition-site,
// not import-site); it fixes a TYPE not a conversion (`matches!(ty, Ty::Rational)`, no-silent-promotion
// still faults a mix); and an explicit annotation WINS (the `(: <lit> T)` guard below).
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-fraction-literal-type
//# The definition-site rule and the fixes-a-type-not-a-conversion rule for a default integer literal type MUST apply equally to a default fraction literal type: the default in force is the one declared by the module in which the literal is written, it introduces no implicit conversion between numeric types, and an explicit annotation or other constraint on the literal takes precedence.
fn module_default_fraction_ty(db: &mut Db, id: StructId) -> Option<Ty> {
    // An EXPLICIT ANNOTATION WINS: a literal that is the expression of a `(: <lit> T)` annotation is
    // governed by `T`, not the module default — so DON'T apply the fraction default to it (else the
    // literal would type `Rational` while its `Annot` types `T`, and `lower` would emit a rational value
    // for a `T`-typed node — a miscompile). The `default-integer` twin needs no such guard: it keeps the
    // literal `Ty::Int`, so an annotated literal has no VALUE-representation conflict. Here the default
    // changes the value form, so the annotated-literal case must be excluded at the source.
    if let Some(parent) = db.parent_of(id)
        && let Some(tail) = db.ast.as_form(parent, ":")
        && tail.first() == Some(&id)
    {
        return None;
    }
    let ty_expr = *db.default_fraction_literals.get(&id)?;
    let ty = crate::eval::typeval_of(db, ty_expr)?;
    matches!(ty, Ty::Rational).then_some(ty)
}

/// The declared default-FLOAT type for the bare DECIMAL literal at `id`, or `None` if it is not written in
/// a `(pragma default-float <T>)` module. Reads the load-time `default_float_literals` map (keyed by the
/// literal's ORIGINAL node, β-copy-robust) and reduces the recorded `<T>` occurrence to a `Ty`. Only a
/// FLOAT type is honored (a non-float `<T>` is separately the CDZ0303 domain reject); anything that does
/// not reduce to a concrete float type is `None` (the literal keeps the deferred `Float64` default). This
/// fixes a TYPE, not a conversion: no-silent-promotion still holds, and an explicit annotation on the
/// literal wins.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-float-literal-type
//# A module MAY declare, through a module directive (modules-and-namespaces.md §"A Module Directive Is Drawn From A Fixed Set"), the floating-point type that a decimal literal with no other constraint takes within that module.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-float-literal-type
//# When a module declares no default float literal type, a decimal literal with no other constraint MUST take the numeric model's default floating-point type.
// This fn is consulted ONLY for a `Resolved::Float(_)` node (a DECIMAL-written literal): a bare
// integer-written literal takes `module_default_int_ty`, so a default-float directive governs how a
// written fraction is represented and never silently makes an integer a float.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-float-literal-type
//# A declared default float literal type MUST apply only to a decimal-written literal with no other constraint, leaving an integer-written literal at its declared or model-default integer type, so that a default float width governs how a written fraction is represented without silently making an integer a float.
// The default in force is the one the literal's OWN module declares (`collect_default_float_literals`
// walks each pragma-module's member subtrees, keyed by the original literal node — definition-site, not
// import-site); it fixes a TYPE not a conversion (no-silent-promotion still faults a mix); and an
// explicit annotation WINS (the `(: <lit> T)` guard below) — the same three rules as default-integer.
//= spec/capabilities/numeric-model.md#a-module-may-declare-its-default-float-literal-type
//# The definition-site rule and the fixes-a-type-not-a-conversion rule for a default integer literal type MUST apply equally to a default float literal type: the default in force is the one declared by the module in which the literal is written, it introduces no implicit conversion between numeric types, and an explicit annotation or other constraint on the literal takes precedence.
fn module_default_float_ty(db: &mut Db, id: StructId) -> Option<Ty> {
    // An EXPLICIT ANNOTATION WINS (numeric-model.md §"An explicit annotation … takes precedence over the
    // module's declared default"): a literal that is the expression of a `(: <lit> T)` annotation is
    // governed by `T`, not the module default — so DON'T apply the default-float to it. Unlike the
    // `default-integer` twin (which relies on the fault walk's integer-literal GROUNDING branch to let a
    // conflicting annotation win), a bare FLOAT literal has no such grounding branch, so a FIXED default
    // width (`Float32`) would UNIFY against a differing annotation (`(: 3.14 Float64)`) and spuriously
    // reject CDZ0203. Excluding the annotated literal here makes it behave exactly as it would with no
    // pragma — the deferred literal grounds through its annotation, the annotation wins. Same guard as
    // `module_default_fraction_ty`.
    if let Some(parent) = db.parent_of(id)
        && let Some(tail) = db.ast.as_form(parent, ":")
        && tail.first() == Some(&id)
    {
        return None;
    }
    let ty_expr = *db.default_float_literals.get(&id)?;
    let ty = crate::eval::typeval_of(db, ty_expr)?;
    matches!(ty, Ty::Float(_)).then_some(ty)
}

/// The join of two `if`-branch types, BIASED toward a RIGID param var when the branches are the same sum
/// whose element vars differ. `Ty::join` for two `Ty::Var`s returns the second operand arbitrarily; when
/// one branch's sum element is a rigid param var (this def's own parameter element, in
/// `db.scheme_rigid_vars`) and the other's is not, we must keep the RIGID one so a recursive-generic
/// transformer's result element stays tied to its parameter (the take-while bare-nullary-stop-leaf case).
/// Detects the shape: both `Ty::Sum` of the same decl, single-arg element, exactly one element a rigid
/// `Ty::Var`; then joins with the rigid side SECOND (so `join`'s `(Var, t) => t` keeps it). Everything
/// else falls back to the plain `then.join(else)` — byte-identical.
fn rigid_biased_join(db: &Db, then_ty: &Ty, else_ty: &Ty) -> Ty {
    let is_rigid = |t: &Ty| matches!(t, Ty::Var(v) if db.scheme_rigid_vars.as_ref().is_some_and(|r| r.contains(v)));
    if let (
        Ty::Sum {
            decl: da, args: aa, ..
        },
        Ty::Sum {
            decl: db_decl,
            args: ab,
            ..
        },
    ) = (then_ty, else_ty)
        && da == db_decl
        && aa.len() == 1
        && ab.len() == 1
    {
        let then_rigid = is_rigid(&aa[0]);
        let else_rigid = is_rigid(&ab[0]);
        // Put the rigid-element branch SECOND so `join`'s `(Var, t) | (t, Var) => t` keeps it (for two
        // vars it returns the second operand). Only reorder when exactly one side is rigid.
        if else_rigid && !then_rigid {
            return then_ty.join(else_ty); // else (rigid) is already second — keep as-is
        }
        if then_rigid && !else_rigid {
            return else_ty.join(then_ty); // swap so the rigid `then` is second
        }
    }
    then_ty.join(else_ty)
}

/// Ground a solved parameter type: a still-unsolved variable that a numeric use constrained becomes the
/// default integer (`Int64`), matching a bare literal's defaulting; a fully-unconstrained variable stays
/// `Any` (the parameter is genuinely undetermined — the boundary layer declines rather than guess).
fn ground_param(ty: Ty) -> Ty {
    match ty {
        // A variable no constraint pinned: leave it `Any` — the caller (layout/select) declines an
        // ambiguous parameter rather than inventing a width.
        Ty::Var(_) => Ty::Any,
        // A deferred-width integer (a numeric use pinned it integer but not the width) grounds to the
        // default width, exactly as a bare literal does.
        Ty::Int(_) => Ty::int64(),
        other => other,
    }
}

/// Walk the resolved body collecting constraints on the parameter variables in `env`, extending
/// `subst`. For each application of a built-in operation, the operation's SCHEME fixes what type each
/// argument must have; unifying the argument's type into the scheme's parameter positions constrains
/// any parameter variable that argument mentions. A self/mutual call to a def in the same recursive
/// group unifies each argument against the CALLEE's parameter variable (read from `env` when the callee
/// is THIS def; a cross-def callee's params are solved by its own pass — its scheme is read there). The
/// walk descends every sub-expression that runs.
fn collect_param_constraints(
    db: &mut Db,
    node: StructId,
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
    def: usize,
    subst: &mut Subst,
    fresh: &mut Fresh,
) {
    match resolved_of(db, node) {
        Resolved::Apply { head, args } => {
            // An operator (a prim with a `(meta t)` scheme): instantiate it and unify each argument's
            // type into the curried parameter positions. This is what pins `n` to an integer in `(= n
            // 0)` / `(+ n …)` and to a Bool in a boolean op.
            if let Some(scheme) = crate::eval::scheme_of(db, head, fresh) {
                let mut cur = crate::unify::instantiate(&scheme, fresh);
                for &arg in args.iter() {
                    let applied = subst.apply(&cur);
                    if let Ty::Fn(param, result) = applied {
                        let at = arg_ty_in_env(db, arg, env, subst, fresh);
                        let _ = crate::unify::unify(subst, &param, &at, &db.name_ctx());
                        cur = *result;
                    } else {
                        break;
                    }
                }
            } else if let Some(callee) = callee_def_index_for_infer(db, head) {
                // A call to a user def. If it is THIS def (self-recursion), unify each argument against
                // this def's own parameter variable — the fixpoint.
                if callee == def {
                    // The parameters in signature order (env is unordered) — arguments match positionally.
                    let ordered = ordered_param_binders(db, def);
                    for (i, &arg) in args.iter().enumerate() {
                        if let Some(&binder) = ordered.get(i)
                            && let Some(pvar) = env.get(&binder)
                        {
                            let at = arg_ty_in_env(db, arg, env, subst, fresh);
                            let _ = crate::unify::unify(subst, pvar, &at, &db.name_ctx());
                        }
                    }
                } else {
                    // A call to ANOTHER def: a parameter passed as the k-th argument is constrained by the
                    // callee's k-th PARAMETER TYPE. Without this, a parameter used ONLY as a call argument
                    // (`(byte-at b i)` — `b` never touched by an operator) stays a free `Var`, grounds to
                    // `Any`, and the recursive-def guard declines a well-typed program. The callee's own
                    // body pins its param type (`byte-at`'s `b` is `Bytes` via `(Bytes.at b i)`), so read
                    // the callee's k-th param type and unify. Only a DETERMINED callee param (not
                    // `Any`/`Var`) constrains — an undetermined one adds nothing, so a genuinely
                    // polymorphic position is never over-constrained.
                    for (i, &arg) in args.iter().enumerate() {
                        let arg_is_param = matches!(
                            resolved_of(db, arg),
                            Resolved::Ref { value } if env.contains_key(&value)
                        ) || matches!(
                            resolved_of(db, arg),
                            Resolved::Param { binder } if env.contains_key(&binder)
                        );
                        // A payload BINDER of a parameter — `h` in `(match it … ((Iter.Cons h rest)
                        // (append h …)))`, a `SumPayload` reading `it`'s element. Passing `h` to a callee
                        // constrains `it`'s ELEMENT the same way passing `it` directly constrains `it`: the
                        // callee's k-th param type unifies with `h`'s type, which `arg_ty_in_env` walks
                        // through the LOCAL subst back to `it`'s element var — so `(append h …)` (append's
                        // domain `Iter _`) pins `it`'s element to `Iter _`, giving `it : Iter(Iter _)` and
                        // (via append's result=domain-element tie) the result `Iter _`. Without this, a
                        // recursive TRANSFORMER whose Cons arm threads its element into a generic callee
                        // (`flatten`'s `append h …`) left `it`'s element and the result as DISCONNECTED vars
                        // — the untied nested-generic tie (`(-> (Iter a) (Iter b))` instead of `(-> (Iter
                        // (Iter a)) (Iter a))`). A payload binder is in NO env (env holds the params), so it
                        // is distinct from `arg_is_param`; `arg_ty_in_env`'s `SumPayload` arm already links
                        // it to the scrutinee param's element var, so the unify propagates into `subst`.
                        let arg_is_param_payload =
                            if let Resolved::SumPayload { scrutinee, .. } = resolved_of(db, arg) {
                                // Resolve the scrutinee ONCE (was two `resolved_of` calls of the same node
                                // in a hot constraint-collection loop, Copilot micro-perf PR #524): the
                                // payload's scrutinee reads a parameter directly as a `Ref` or a `Param`.
                                match resolved_of(db, scrutinee) {
                                    Resolved::Ref { value } => env.contains_key(&value),
                                    Resolved::Param { binder } => env.contains_key(&binder),
                                    _ => false,
                                }
                            } else {
                                false
                            };
                        if !arg_is_param && !arg_is_param_payload {
                            continue;
                        }
                        if let Some(pt) = callee_param_ty(db, callee, i)
                            && !matches!(pt, Ty::Any | Ty::Var(_))
                        {
                            let at = arg_ty_in_env(db, arg, env, subst, fresh);
                            let _ = crate::unify::unify(subst, &at, &pt, &db.name_ctx());
                        }
                    }
                }
            }
            // A PARAMETER APPLIED AS A FUNCTION — `(f h)` where `f` is a fn-typed parameter being solved.
            // Its use as a call head constrains it to a function type: unify `f`'s var with `(-> arg0 ->
            // arg1 -> … -> result)`, where each `argᵢ` is the applied argument's type and `result` is a
            // fresh var. Without this, a recursive HOF's callback param (`(def (map-f f (: l L)) … (f h)
            // …)`) stayed a free `Var` → grounded `Any` → the recursive-def guard declined "annotate its
            // parameters". A fn-typed param used ONLY as a head is the function analogue of the "param used
            // only as a call argument" case handled above. `binder_var_of` finds the param's var when the
            // head resolves (through a `Ref`) to a binder in `env`; a non-param head (a def/op) took a
            // branch above, and an already-scheme'd head is not a bare param, so this only fires for a
            // genuine fn-typed parameter.
            if crate::eval::scheme_of(db, head, fresh).is_none()
                && callee_def_index_for_infer(db, head).is_none()
                && let Some(hvar) = binder_var_of(db, head, env)
            {
                // Build `(-> a0 (-> a1 … (-> aN result)))` from the applied arguments, then unify.
                let result = Ty::Var(fresh.var());
                let mut arrow = result;
                for &arg in args.iter().rev() {
                    let at = arg_ty_in_env(db, arg, env, subst, fresh);
                    arrow = Ty::Fn(Box::new(at), Box::new(arrow));
                }
                let _ = crate::unify::unify(subst, &hvar, &arrow, &db.name_ctx());
            }
            // Descend into the head (a computed head) and every argument for THEIR own constraints.
            if matches!(resolved_of(db, head), Resolved::Apply { .. }) {
                collect_param_constraints(db, head, env, def, subst, fresh);
            }
            for &arg in args.iter() {
                collect_param_constraints(db, arg, env, def, subst, fresh);
            }
        }
        Resolved::If { cond, then_, else_ } => {
            // The condition must be Bool — constrain it (a bare-param condition `(if n …)` pins n Bool).
            let ct = arg_ty_in_env(db, cond, env, subst, fresh);
            let _ = crate::unify::unify(subst, &ct, &Ty::Bool, &db.name_ctx());
            collect_param_constraints(db, cond, env, def, subst, fresh);
            collect_param_constraints(db, then_, env, def, subst, fresh);
            collect_param_constraints(db, else_, env, def, subst, fresh);
        }
        // A boolean connective's operands must be Bool — constrain each (a bare-param operand `(and p …)`
        // pins `p` Bool), then descend.
        Resolved::And { lhs, rhs, .. } => {
            for &op in &[lhs, rhs] {
                let t = arg_ty_in_env(db, op, env, subst, fresh);
                let _ = crate::unify::unify(subst, &t, &Ty::Bool, &db.name_ctx());
                collect_param_constraints(db, op, env, def, subst, fresh);
            }
        }
        Resolved::Not { operand } => {
            let t = arg_ty_in_env(db, operand, env, subst, fresh);
            let _ = crate::unify::unify(subst, &t, &Ty::Bool, &db.name_ctx());
            collect_param_constraints(db, operand, env, def, subst, fresh);
        }
        Resolved::Match { scrutinee, arms } => {
            // Each pattern constrains the scrutinee's type. A LITERAL pattern pins it to the literal's
            // type (`(match n (0 …) (_ …))` → `n : Int64`). When the scrutinee is DIRECTLY A PARAMETER
            // being solved, a STRUCTURAL pattern pins its SHAPE: `(match xs ((Cons (tuple h t)) …))` means
            // `xs` is that sum, so a recursive tree-walker's parameter (`xs : Code`, `node : Core`) is
            // inferred rather than left a free var (which would ground to `Any` and decline). Restricted
            // to a scrutinee that IS a parameter — a scrutinee that is a CALL RESULT (`(List.at xs i)`)
            // carries its own instantiation that this shape-unify would corrupt, so it is left to the
            // ordinary application constraints.
            // Type the scrutinee. When it is a SCHEME CALL (`(List.at xs i)`), type it INTO the outer
            // subst (`type_scheme_apply_into`) so its result's generic var LINKS to the argument param's
            // element (`(Option a)` with `a == xs`'s element); a later constraint on the result — a
            // `(Some x)` arm's `x` pinned by the sibling `(None _) → 0` — then flows back to `xs`'s element,
            // solving the recursive list-consumer's parameter. A non-call scrutinee (a param, a fn-param
            // application) uses `arg_ty_in_env` as before.
            let st = type_scheme_apply_into(db, scrutinee, env, subst, fresh)
                .unwrap_or_else(|| arg_ty_in_env(db, scrutinee, env, subst, fresh));
            let scrut_is_param = matches!(
                resolved_of(db, scrutinee),
                Resolved::Ref { value } if env.contains_key(&value)
            ) || matches!(
                resolved_of(db, scrutinee),
                Resolved::Param { binder } if env.contains_key(&binder)
            );
            // A scrutinee that is an APPLICATION OF A FN-TYPED PARAMETER — `(match (f h) …)` — has type
            // `f`'s RESULT VAR (a fresh, uncommitted var `arg_ty_in_env` peels from `f`'s arrow). The arm
            // patterns pin that result: a `C.A`/`C.B` pattern means `f : (-> _ C)`, so unifying `st` with
            // the pattern-implied sum solves `f`'s result. Safe for this shape (the result is a fresh var,
            // not a determined instantiation), unlike a general call-result scrutinee (a `List.at` whose
            // element instantiation the shape-unify would corrupt) — so gate on the head being an env param.
            let scrut_is_fn_param_app = matches!(
                resolved_of(db, scrutinee),
                Resolved::Apply { head, .. } if binder_var_of(db, head, env).is_some()
            );
            // A SCHEME-CALL scrutinee (`(List.at xs i)`) whose type `type_scheme_apply_into` linked into
            // the OUTER subst above — `st` is that call's real result instantiation (`(Option a)`, `a`
            // tied to `xs`'s element), NOT a disconnected clone. So shape-unifying a pattern against it is
            // SAFE and DESIRABLE: it binds the payload binder (`(Some x)`'s `x`) to the linked `a`, so a
            // sibling arm's determined type pins `a` and thence the argument param's element. (The old
            // "a call-result shape-unify corrupts the instantiation" caveat applied to the clone-typed
            // `st`; with the outer-subst link there is nothing to corrupt — it IS the instantiation.)
            let scrut_is_scheme_call = matches!(
                resolved_of(db, scrutinee),
                Resolved::Apply { head, .. }
                    if crate::eval::scheme_of(db, head, fresh).is_some()
            );
            for (pat, _) in &arms {
                if let Some(pt) = literal_pattern_ty(db, *pat) {
                    let _ = crate::unify::unify(subst, &st, &pt, &db.name_ctx());
                } else if (scrut_is_param || scrut_is_fn_param_app || scrut_is_scheme_call)
                    && let Some(pt) = pattern_implied_ty(db, *pat, fresh)
                {
                    let _ = crate::unify::unify(subst, &st, &pt, &db.name_ctx());
                }
            }
            // A parameter RETURNED DIRECTLY by one arm is constrained by the OTHER arms' result type (the
            // arm bodies agree in type). `(match xs (Nil ys) ((Cons …) (Cons …)))` → the `Nil` arm returns
            // the parameter `ys` and the `Cons` arm returns a `Code`, so `ys : Code` — a pass-through /
            // accumulator parameter otherwise only echoed out is inferred rather than left a free var.
            // NARROW on purpose: only an arm body that IS a bare parameter reference is constrained, and
            // only against a SIBLING arm's DETERMINED (non-var, non-`Any`) result. Unifying arbitrary arm
            // types against each other over-constrains (a sibling's own unsolved var spuriously conflicts);
            // this pins exactly the echoed-parameter case without touching well-typed programs.
            // The VARIABLE an arm body contributes to the arms-agree constraint, when its type is a
            // still-open var this solve owns: a BARE PARAMETER echoed out (`ys`), or an APPLICATION OF A
            // FN-TYPED PARAM (`(f n)`) whose result var is open. Either way the arm's type is an unpinned
            // var that a SIBLING arm's determined type must fix — the pass-through case, extended to a
            // callback result. `None` for a determined/ordinary arm (which SUPPLIES the type, below). A
            // free function (not a closure) so it takes `subst` by shared ref without capturing it.
            fn open_arm_var(
                db: &mut Db,
                body: StructId,
                env: &crate::fxhash::FxHashMap<StructId, Ty>,
                subst: &Subst,
                fresh: &mut Fresh,
            ) -> Option<Ty> {
                // The arm's type must be a STILL-OPEN var for it to be a constrainable pass-through: an
                // echoed BARE param whose var is unsolved, or a fn-param application `(f n)` whose result
                // var is unsolved. An ANNOTATED param (`z : Int64`) is a DETERMINED type — it is a type
                // SOURCE for its siblings, not an open arm — so it must NOT be reported open (else two
                // arms both look open and neither pins the other, and a callback result stays unsolved).
                let t = match resolved_of(db, body) {
                    Resolved::Ref { value } if env.contains_key(&value) => {
                        env.get(&value).cloned()?
                    }
                    Resolved::Param { binder } if env.contains_key(&binder) => {
                        env.get(&binder).cloned()?
                    }
                    // `(f n)` — a fn-typed param applied. Its type is `f`'s result var (peeled by
                    // `arg_ty_in_env`).
                    Resolved::Apply { head, .. } if binder_var_of(db, head, env).is_some() => {
                        arg_ty_in_env(db, body, env, subst, fresh)
                    }
                    // A PAYLOAD BINDER of the scrutinee being solved — `n` in `(match t ((Tree.Leaf n) n)
                    // …)` returned directly by the arm. Its type is `t`'s local instantiation walked down
                    // the payload path (`arg_ty_in_env`'s `SumPayload` case); a still-open var (a generic
                    // sum's payload `a` not yet pinned) makes this arm a pass-through a sibling's
                    // determined type fixes — solving the generic arg. A binder whose type is already
                    // determined (a concrete-payload sum) is NOT open (falls through, supplies its type).
                    // The scrutinee may be a PARAM (`t`) OR a CALL RESULT (`(List.at xs i)`) — `arg_ty_in_env`
                    // types either through the local subst (the call via its scheme, `(Option ?a)`), so the
                    // `(Some x) → x` arm of `(match (List.at xs i) ((Some x) x) ((None _) 0))` is open and a
                    // sibling arm's `0` (Int64) pins `x`, hence `xs`'s element — a recursive list consumer
                    // that indexes with `List.at` and returns the payload directly.
                    Resolved::SumPayload { .. } => arg_ty_in_env(db, body, env, subst, fresh),
                    _ => return None,
                };
                // Open iff it applies to a bare var — a solved/annotated type supplies, not borrows.
                matches!(subst.apply(&t), Ty::Var(_)).then_some(t)
            }
            let arm_list: Vec<StructId> = arms.iter().map(|(_, b)| *b).collect();
            for &body in &arm_list {
                let Some(pvar) = open_arm_var(db, body, env, subst, fresh) else {
                    continue;
                };
                for &other in &arm_list {
                    if other == body || open_arm_var(db, other, env, subst, fresh).is_some() {
                        continue; // self, or another open arm — no determined type to borrow
                    }
                    let ot = arg_ty_in_env(db, other, env, subst, fresh);
                    let applied = subst.apply(&ot);
                    if !matches!(applied, Ty::Any | Ty::Var(_)) {
                        let _ = crate::unify::unify(subst, &pvar, &applied, &db.name_ctx());
                    }
                }
            }
            collect_param_constraints(db, scrutinee, env, def, subst, fresh);
            for (_, body) in &arms {
                collect_param_constraints(db, *body, env, def, subst, fresh);
            }
        }
        Resolved::Let { bindings, body } => {
            for (_, value) in bindings {
                collect_param_constraints(db, value, env, def, subst, fresh);
            }
            collect_param_constraints(db, body, env, def, subst, fresh);
        }
        Resolved::Annot { expr, .. } | Resolved::ConstBlock { expr } => {
            collect_param_constraints(db, expr, env, def, subst, fresh)
        }
        Resolved::Member { operand, .. } => {
            collect_param_constraints(db, operand, env, def, subst, fresh)
        }
        Resolved::Proj { operand, .. } => {
            collect_param_constraints(db, operand, env, def, subst, fresh)
        }
        Resolved::Record { fields } => {
            for value in fields.values() {
                collect_param_constraints(db, *value, env, def, subst, fresh);
            }
        }
        Resolved::Tuple { elems } => {
            for &e in elems.iter() {
                collect_param_constraints(db, e, env, def, subst, fresh);
            }
        }
        // A `(bin …)` CONSTRUCTION: each fixed-width integer segment CONSTRAINS its value to the segment's
        // OWN width type — `(u8 v)` requires `v : UInt8`, `(i16 v)` requires `Int16`, `(bits v k)` requires
        // `v : (UInt k)`. A value that provably fits its type has no out-of-range case, so the segment needs
        // no runtime range-check and cannot trap: an out-of-range value is a COMPILE-TIME type error, and
        // any narrowing (`UInt8.wrap` to truncate, `UInt8.of` checked) is the caller's responsibility. This
        // is the constraint that grounds a parameter used ONLY in a segment to the width type (a `(def (main
        // n) (bin (u8 n)))` infers `n : UInt8`), the segment analogue of an operator pinning its operand.
        Resolved::Bin { segs } => {
            for seg in segs.iter() {
                if let Some(want) = seg_value_ty(&seg.kind) {
                    let at = arg_ty_in_env(db, seg.slot, env, subst, fresh);
                    let _ = crate::unify::unify(subst, &want, &at, &db.name_ctx());
                }
                collect_param_constraints(db, seg.slot, env, def, subst, fresh);
                // A dependent-size occurrence `(bytes b n)` / `(utf8 s n)` is an ordinary integer index.
                match &seg.kind {
                    crate::resolved::SegKind::Bytes { size: Some(n) } => {
                        collect_param_constraints(db, *n, env, def, subst, fresh)
                    }
                    crate::resolved::SegKind::Utf8 { size } => {
                        collect_param_constraints(db, *size, env, def, subst, fresh)
                    }
                    _ => {}
                }
            }
        }
        // Leaves and references contribute no sub-constraints (a bare param ref's constraint comes from
        // the operation that consumes it, handled at the enclosing Apply).
        _ => {}
    }
}

/// The exact integer TYPE a fixed-width `bin` segment requires of its VALUE — the segment's own width and
/// signedness: `(uN v)` → an unsigned N-bit `Ty::Int`, `(iN v)` → a signed N-bit one, `(bits v k)` → an
/// unsigned k-bit one. `None` for a `bytes`/`utf8` segment (whose value is a Bytes/String, not an integer,
/// checked separately). `SegKind::Int.width` is in BYTES, so the machine width is `width * 8` bits; a `bits`
/// field's width is its `k` (already in bits). This is the single source of truth for both the inference
/// CONSTRAINT (`collect_param_constraints`) and the well-formedness CHECK (`collect`), so the type a
/// segment imposes and the type it verifies never drift.
fn seg_value_ty(kind: &crate::resolved::SegKind) -> Option<Ty> {
    match kind {
        crate::resolved::SegKind::Int { width, signed } => Some(Ty::Int(crate::ty::IntTy::fixed(
            *signed,
            (*width as u32) * 8,
        ))),
        crate::resolved::SegKind::Bits { k } => Some(Ty::Int(crate::ty::IntTy::fixed(false, *k))),
        crate::resolved::SegKind::Bytes { .. } | crate::resolved::SegKind::Utf8 { .. } => None,
    }
}

/// The type a LITERAL match pattern implies for the scrutinee, or `None` for a non-literal pattern (the
/// wildcard `_`, or a binder — which constrains nothing). An integer-literal pattern implies a deferred
/// integer (grounds like a bare literal); a boolean-literal pattern implies `Bool`. Used to constrain a
/// scrutinee from its arms (both in the parameter solve and — later — exhaustiveness).
fn literal_pattern_ty(db: &mut Db, pat: StructId) -> Option<Ty> {
    match resolved_of(db, pat) {
        Resolved::Int(_) => Some(Ty::int()),
        Resolved::Bool(_) => Some(Ty::Bool),
        _ => None,
    }
}

/// The type a STRUCTURAL match pattern requires of the value it matches — the pattern's implied
/// scrutinee type, with a fresh variable for every binder/wildcard sub-position. Used by
/// `collect_param_constraints` (only when the scrutinee IS a parameter) to thread a pattern's SHAPE onto
/// that parameter, so a recursive tree-walker's parameter type is inferred rather than left a free var.
/// Returns `None` for a pattern whose shape does not further constrain the scrutinee (a bare binder or a
/// wildcard — matches anything — or a non-constructor head).
///
/// A `(tuple p0 … pn)` implies `Ty::Tuple([implied(p0) …])` (a var where a sub-position is a bare
/// binder). A variant pattern `(V p)` / bare `V` implies the variant's owning SUM at the instantiation
/// its payload sub-pattern implies — instantiate `V`'s ctor scheme `payload… → Sum`, unify the payload
/// against the inner pattern's implied type, and return the partly-solved `Sum` (a nullary variant is the
/// bare `Sum`). A literal / bare-name pattern implies `None`.
///
/// The ctor scheme is instantiated with the CALLER's `fresh` so its variables never collide with the
/// solver's own variables (a collision would bind the wrong variable in the shared `subst`).
fn pattern_implied_ty(db: &mut Db, pat: StructId, fresh: &mut Fresh) -> Option<Ty> {
    if let Some(elems) = db
        .ast
        .compound_form_of(pat, CompoundCtor::Tuple)
        .map(<[StructId]>::to_vec)
    {
        let tys: Vec<Ty> = elems
            .iter()
            .map(|&e| pattern_implied_ty(db, e, fresh).unwrap_or_else(|| Ty::Var(fresh.var())))
            .collect();
        return Some(Ty::Tuple(tys.into()));
    }
    // A LIST pattern — `(list)`, `(list h .. t)`, `(list a b)` — implies `List <elem>`. Without this a
    // recursive list-consumer/PRODUCER whose scrutinee element flows only into a generic construction
    // (`(match xs ((list) …) ((list h .. t) (Iter.Cons h (from-list t))))`) never shaped its parameter:
    // the list head is not a variant-ctor scheme, so `pattern_implied_ty` returned `None`, the match's
    // shape-unify left `xs` a free var, it grounded to `Any`, and the scheme DECLINED (undetermined-param
    // bail). Shaping it `List <elem>` gives the param its list structure; the element is read from the
    // leading POSITIONAL element sub-pattern (a `..` rest marker and its following rest binder bind the
    // tail LIST, not an element, so they are skipped), else a fresh var the surrounding solve pins.
    if let Some(items) = db
        .ast
        .compound_form_of(pat, CompoundCtor::List)
        .map(<[StructId]>::to_vec)
    {
        // `compound_form_of` recognizes the native `#list(…)` ctor-leaf head too (not only the name/string
        // alias) — so a native `#list` match pattern SHAPES its (untyped) scrutinee `List <elem>` exactly like
        // the alias; without this a native-list recursive consumer left its param a free var → grounded Any →
        // the scheme declined (undetermined-param) while the alias compiled (M3 native-recognition parity).
        // Collect the leading positional element sub-patterns (up to the `..` rest marker), dropping the
        // `db.ast` borrow before the recursive `pattern_implied_ty` calls take `db`.
        let positional: Vec<StructId> = items
            .iter()
            .take_while(|&&e| db.ast.as_name(e) != Some(".."))
            .copied()
            .collect();
        let mut elem_ty = None;
        for e in positional {
            if let Some(t) = pattern_implied_ty(db, e, fresh) {
                elem_ty = Some(t);
                break;
            }
        }
        return Some(Ty::List(Box::new(
            elem_ty.unwrap_or_else(|| Ty::Var(fresh.var())),
        )));
    }
    if let Some(g) = db.ast.as_form(pat, "guard").map(<[StructId]>::to_vec)
        && g.len() == 2
    {
        return pattern_implied_ty(db, g[0], fresh);
    }
    let (head, args): (StructId, Vec<StructId>) = match db.ast.get(pat) {
        crate::ast::Struct::List(children) => match children.first().copied() {
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, Vec::new()),
            Some(first) => (first, children[1..].to_vec()),
            None => return None,
        },
        crate::ast::Struct::Atom(_) => (pat, Vec::new()),
    };
    let scheme = crate::eval::scheme_of(db, head, fresh)?;
    let inst = crate::unify::instantiate(&scheme, fresh);
    let mut payloads = Vec::new();
    let mut cur = inst;
    let result = loop {
        match cur {
            Ty::Fn(p, r) => {
                payloads.push(*p);
                cur = *r;
            }
            other => break other,
        }
    };
    // Only a SUM result is a variant constructor; anything else (an ordinary function) is not a pattern.
    if !matches!(result, Ty::Sum { .. }) {
        return None;
    }
    let mut subst = Subst::new();
    if !payloads.is_empty()
        && let Some(&arg) = args.first()
        && let Some(implied) = pattern_implied_ty(db, arg, fresh)
    {
        let payload_ty = if payloads.len() == 1 {
            payloads[0].clone()
        } else {
            Ty::Tuple(payloads.clone().into())
        };
        let _ = crate::unify::unify(&mut subst, &payload_ty, &implied, &db.name_ctx());
    }
    Some(subst.apply(&result))
}

/// The type of an argument occurrence within the parameter-solve env: a reference to a parameter being
/// solved is its variable (applied through `subst`); anything else is its ordinary solved `type_of`
/// (a literal is a deferred int, a nested op is its result type, a cross-def call its scheme result).
fn arg_ty_in_env(
    db: &mut Db,
    arg: StructId,
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
    subst: &Subst,
    fresh: &mut Fresh,
) -> Ty {
    // A reference to a parameter being solved → its variable. A body param reference resolves to
    // `Ref { value: <param binder> }` (or a bare `Param { binder }`).
    match resolved_of(db, arg) {
        Resolved::Ref { value } => {
            if let Some(var) = env.get(&value) {
                return subst.apply(var);
            }
        }
        Resolved::Param { binder } => {
            if let Some(var) = env.get(&binder) {
                return subst.apply(var);
            }
        }
        // A LIST built from parameter references — `(list i)` where `i` is a param being solved. Its
        // element type must be read through the LOCAL `subst` (where `(+ i 1)` pinned `i` to `Int w`),
        // NOT `type_of((list i))` — mid-solve `db.param_types` is empty, so `type_of` reads `i` as `Any`
        // and the whole list types `List Any`. Building the element type via `arg_ty_in_env` recursively
        // links the inner element's var into the subst, so pinning `i` pins the nested element (a runtime
        // `(List (List Int64))` accumulator grounds instead of stranding the inner element at `Any`). The
        // `list` NAME-alias form (`ListNew`) is the same shape reached through `Apply`, handled below.
        Resolved::List { elems } => {
            let mut elem_ty = Ty::Any;
            for &e in elems.iter() {
                elem_ty = elem_ty.join(&arg_ty_in_env(db, e, env, subst, fresh));
            }
            return Ty::List(Box::new(subst.apply(&elem_ty)));
        }
        // A TUPLE built from parameter references — `(tuple i 99)` where `i` is a param being solved. Each
        // COMPONENT's type must be read through the LOCAL `subst` (where `(< i n)`/`(+ i 1)` pinned `i` to
        // `Int w`), NOT `type_of((tuple i 99))` — mid-solve `db.param_types` is empty, so `type_of` reads
        // `i` as `Any` and the tuple types `(Tuple Any Int64)`, stranding the first slot (a runtime
        // `(List (Tuple Int64 Int64))` accumulator threaded through `List.push` then solved `(List (Tuple
        // Any Int64))` → the rust backend declines "no native representation" for the `Any`). Building each
        // component via `arg_ty_in_env` recursively links a param-referencing component's var into the
        // subst, so pinning `i` pins the tuple slot. The `tuple` NAME-alias form (`TupleNew`) is the same
        // shape reached through `Apply`, handled in that arm below (mirrors the `List`/`ListNew` pair).
        Resolved::Tuple { elems } => {
            let tys: Vec<Ty> = elems
                .iter()
                .map(|&e| subst.apply(&arg_ty_in_env(db, e, env, subst, fresh)))
                .collect();
            return Ty::Tuple(tys.into());
        }
        // An APPLICATION OF A FN-TYPED PARAMETER being solved — `(f h)` where `f` is in `env`. Its type is
        // `f`'s var peeled by one arrow per argument, read from the LOCAL `subst` (where the arrow was
        // unified in `collect_param_constraints`), NOT from `type_of` — `db.param_types` is still empty
        // mid-solve, so `type_of((f h))` would be `Any` and the result would never link to `f`'s arrow.
        // This is what lets `(+ (f h) …)` flow Int64 back to `f`'s result var, solving the fn param.
        Resolved::Apply { head, args } => {
            // The `list` NAME alias (`(list i)` via `ListNew`) is a list built from its arguments — the
            // element type read through the LOCAL subst, exactly as the `Resolved::List` arm above.
            // The `tuple` NAME alias (`(tuple i 99)` via `TupleNew`) — each component's type read through
            // the LOCAL subst, exactly as the symbol-headed `Resolved::Tuple` arm above (the two surfaces
            // reach the same shape). Without this the tuple's param-referencing components strand at `Any`.
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleNew) {
                let tys: Vec<Ty> = args
                    .iter()
                    .map(|&e| subst.apply(&arg_ty_in_env(db, e, env, subst, fresh)))
                    .collect();
                return Ty::Tuple(tys.into());
            }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::ListNew) {
                let mut elem_ty = Ty::Any;
                for &e in args.iter() {
                    elem_ty = elem_ty.join(&arg_ty_in_env(db, e, env, subst, fresh));
                }
                return Ty::List(Box::new(subst.apply(&elem_ty)));
            }
            if let Some(var) = binder_var_of(db, head, env) {
                let mut cur = subst.apply(&var);
                for _ in 0..args.len() {
                    match cur {
                        Ty::Fn(_, result) => cur = subst.apply(&result),
                        _ => break,
                    }
                }
                return cur;
            }
            // An OPERATOR/INTRINSIC application (`(List.push out (list i))`) whose result must carry the
            // element type an argument built from a param being solved contributes — but `type_of` reads
            // that arg's param as `Any` mid-solve, stranding the result's element (a runtime `(List (List
            // Int64))` accumulator returned `(List (List Any))`). Instantiate the head's scheme HERE and
            // unify each argument's LOCAL-subst type (`arg_ty_in_env`, so `(list i)`'s element links to
            // `i`'s var) into the curried parameter positions; the applied result then carries the pinned
            // element. A local `Fresh`/`Subst` seeded from the caller's keeps the caller's bindings visible
            // without polluting them. Falls through to `type_of` when the head has no scheme.
            if let Some(scheme) = crate::eval::scheme_of(db, head, fresh) {
                let mut local = subst.clone();
                let mut cur = crate::unify::instantiate(&scheme, fresh);
                for &arg in args.iter() {
                    let applied = local.apply(&cur);
                    if let Ty::Fn(param, result) = applied {
                        let at = arg_ty_in_env(db, arg, env, &local, fresh);
                        let _ = crate::unify::unify(&mut local, &param, &at, &db.name_ctx());
                        cur = *result;
                    } else {
                        break;
                    }
                }
                return local.apply(&cur);
            }
        }
        // A PAYLOAD BINDER of the parameter being solved — `n` in `(match t ((Tree.Leaf n) …))`, which
        // resolves to a `SumPayload` reading `t` down a path. Its type must be walked from `t`'s LOCAL
        // instantiation (`Tree ?a1`, set by the scrutinee shape-unify), NOT from `type_of(SumPayload)`
        // (which reads `t`'s type from the empty-mid-solve `db.param_types` → `Any`, disconnected from
        // `?a1`). Walking the local subst links the binder's type to the sum's generic arg var, so pinning
        // the binder (`n = Int64` via arms-agree) pins `?a1` — solving a GENERIC recursive sum's parameter
        // whose payload binder use / result fixes the instantiation. The analogue of the fn-param arrow
        // peel above, for a data payload.
        Resolved::SumPayload {
            scrutinee,
            steps,
            heads,
        } => {
            let root = match resolved_of(db, scrutinee) {
                Resolved::Ref { value } => env.get(&value).cloned(),
                Resolved::Param { binder } => env.get(&binder).cloned(),
                // The scrutinee is a CALL RESULT — `(match (List.at xs k) ((Some h) …))`, where the
                // `Some` payload binder `h` reads the element out of `List.at`'s `(Option a)` result. Type
                // the call THROUGH the local subst (`arg_ty_in_env`'s scheme-application arm returns
                // `(Option ?a)` with `?a` linked to `xs`'s element var), then walk the payload path from
                // that `Option`. Without this, the binder's type read `Any` and the list's element never
                // got pinned by the arm body (`(+ h …)`) — the recursive list-consumer declined "projecting
                // a tuple element of type ?N needs the value heap". The param-scrutinee cases above cover a
                // binder read directly off a parameter sum; this covers one read off a call's result sum.
                _ => Some(arg_ty_in_env(db, scrutinee, env, subst, fresh)),
            };
            if let Some(mut root) = root {
                root = subst.apply(&root);
                // MUTUAL-RECURSION PARTNER RESULT (SCC freeze fix). When the scrutinee is a call to a MUTUAL
                // PARTNER — `(match (dn b i) ((tuple child nx) …))` where `dn` and the def whose params are
                // being solved (`dac`) are in the SAME recursive SCC (both in `solving_params`) — the call
                // types to `Any` mid-solve: `dn`'s scheme is deferred under the in-flight solve, so its whole
                // result collapses. A payload binder read off it (`child`) then types `Any` and FREEZES into
                // `db.param_types` for the accumulator it is pushed onto (`acc` → `(List Any)`), which the
                // rust backend cannot represent though a clean re-solve grounds it to the real sum. STANDALONE
                // `type_of(dn's body)` DOES resolve the concrete result (its constructors fix the shape). So
                // when the partner-call result is an undetermined `Any`, RE-TYPE it from the callee's BODY,
                // which reads the concrete ctors. Guarded by a visited-set (`scc_result_typing`) so co-solving
                // the SCC does not recurse forever (dn's body calls dac, whose child would re-demand dn's body).
                if matches!(root, Ty::Any)
                    && let Resolved::Apply { head, .. } = resolved_of(db, scrutinee)
                    && let Some(callee) = callee_def_index_for_infer(db, head)
                    && !db.scc_result_typing.contains(&callee)
                    && let Some(callee_body) = db.defs[callee].body
                {
                    db.scc_result_typing.insert(callee);
                    let body_ty = type_of(db, callee_body);
                    db.scc_result_typing.remove(&callee);
                    if !matches!(body_ty, Ty::Any) {
                        root = subst.apply(&body_ty);
                    }
                }
                return walk_payload_ty(db, root, &steps, &heads, subst);
            }
        }
        _ => {}
    }
    // Not a parameter reference — its ordinary type. (Reads the type column; a nested op over a param
    // returns the op's result type, which the enclosing unify relates to the param var separately.)
    type_of(db, arg)
}

/// Type a scheme APPLICATION (`(List.at xs i)`) into the CALLER's `subst` — like `arg_ty_in_env`'s
/// scheme arm, but the argument unifications persist in the passed `subst` instead of a discarded clone.
/// Instantiate the head's `(meta t)` scheme with fresh vars, then unify each argument's local-subst type
/// into the curried parameter positions; the RESULT type shares the same fresh vars, so a param
/// argument's type var LINKS to the result (`(List.at xs i)` → `(Option a)` with `a == xs`'s element).
/// Returns `None` when the head has no scheme or is under-applied. The persisting link is what lets a
/// later constraint on the result (a `(Some x)` arm's `x` pinned by a sibling arm) flow back to the
/// argument's parameter (`xs`'s element), which the clone-based `arg_ty_in_env` cannot do.
fn type_scheme_apply_into(
    db: &mut Db,
    node: StructId,
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
    subst: &mut Subst,
    fresh: &mut Fresh,
) -> Option<Ty> {
    let Resolved::Apply { head, args } = resolved_of(db, node) else {
        return None;
    };
    let scheme = crate::eval::scheme_of(db, head, fresh)?;
    let mut cur = crate::unify::instantiate(&scheme, fresh);
    for &arg in args.iter() {
        let applied = subst.apply(&cur);
        let Ty::Fn(param, result) = applied else {
            return None; // under-applied / not a function chain
        };
        let at = arg_ty_in_env(db, arg, env, subst, fresh);
        let _ = crate::unify::unify(subst, &param, &at, &db.name_ctx());
        cur = *result;
    }
    Some(subst.apply(&cur))
}

/// Walk `root` (a sum's LOCAL-subst instantiation) down a `SumPayload` access path — the same descent
/// `type_of`'s `SumPayload` arm does, but starting from a caller-supplied type and re-applying `subst` at
/// each step so the generic arg vars stay linked. A `Payload` step descends the variant's payload at the
/// current instantiation (`payload_ty_at_instantiation`, or a nominal newtype's inner); an `Elem(i)` step
/// descends a tuple element / a list's element. Used by `arg_ty_in_env` to type a payload binder of the
/// The `Ty::Map` a `MapField`'s access `path` reaches from its `scrutinee` — the scrutinee's type walked
/// down `Elem` steps (a tuple element, a list element) to the nested map. EMPTY path = the scrutinee IS the
/// map (a direct map match). A `Payload` step (a variant-nested map) is not modelled here yet (returns
/// `Ty::Any`, a graceful decline — the value binder then types `Ty::Any` and the read declines at lowering,
/// never a miscompile); the common nesting (a map in a tuple/record/list) uses only `Elem`.
fn map_field_map_ty(db: &mut Db, scrutinee: StructId, path: &[crate::core::PathStep]) -> Ty {
    let mut cur = type_of(db, scrutinee);
    for step in path {
        cur = match (step, &cur) {
            (crate::core::PathStep::Elem(i), Ty::Tuple(elems)) => match elems.get(*i) {
                Some(t) => t.clone(),
                None => return Ty::Any,
            },
            (crate::core::PathStep::Elem(_), Ty::List(elem)) => (**elem).clone(),
            _ => return Ty::Any,
        };
    }
    cur
}

/// The `Ty::Record` a `RecordField`'s access `path` reaches from its `scrutinee` — the scrutinee's type
/// walked down `Elem` steps (a tuple element, a list element) to the NESTED record, then a field looked up
/// in its map by the caller. The record twin of [`map_field_map_ty`]; like it, only `Elem` steps are
/// modelled (a `Payload` step — a variant-nested record — returns `Ty::Any`, a graceful decline: the binder
/// then types `Ty::Any` and never a miscompile). A nominal newtype over a record is stripped so a field
/// read sees through the tag. The common nesting (a record in a tuple/list) uses only `Elem`.
pub(crate) fn record_field_at_path(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
    heads: &[StructId],
) -> Ty {
    let mut cur = type_of(db, scrutinee);
    let mut heads = heads.iter();
    for step in path {
        // A `Payload` step (a record nested UNDER a variant): advance to the entered variant's payload type,
        // reading the variant HEAD at this step — mirrors `walk_payload_ty`. A nominal newtype IS its inner
        // value (erased tag), so unwrap it; otherwise the head's `(-> payload Sum)` at the current
        // instantiation gives the payload sub-value's type.
        if let crate::core::PathStep::Payload = step {
            let Some(&head) = heads.next() else {
                return Ty::Any;
            };
            cur = if let Ty::Nominal { inner, .. } = &cur {
                (**inner).clone()
            } else {
                match payload_ty_at_instantiation(db, head, &cur) {
                    Some(t) => t,
                    None => return Ty::Any,
                }
            };
            continue;
        }
        cur = match (step, cur.strip_nominal()) {
            (crate::core::PathStep::Elem(i), Ty::Tuple(elems)) => match elems.get(*i) {
                Some(t) => t.clone(),
                None => return Ty::Any,
            },
            (crate::core::PathStep::Elem(_), Ty::List(elem)) => (**elem).clone(),
            _ => return Ty::Any,
        };
    }
    cur.strip_nominal().clone()
}

/// The type of a nested-payload binder (a variant pattern's payload, possibly through tuple/list `Elem`
/// steps), walked from the scrutinee's ROOT type `root` down `steps`. `heads` supplies the variant head at
/// each `Payload` step (its `(-> payload Sum)` gives the next sub-value's type at the current
/// instantiation); an `Elem` step reads a tuple/list element. `subst` carries the enclosing generic
/// parameter being solved against its local instantiation. `Ty::Any` on a malformed/unresolvable path.
fn walk_payload_ty(
    db: &mut Db,
    root: Ty,
    steps: &[crate::core::PathStep],
    heads: &[StructId],
    subst: &Subst,
) -> Ty {
    let mut cur = root;
    let mut heads = heads.iter();
    for step in steps {
        cur = subst.apply(&cur);
        cur = match step {
            crate::core::PathStep::Payload => {
                let Some(&head) = heads.next() else {
                    return Ty::Any;
                };
                if let Ty::Nominal { inner, .. } = &cur {
                    (**inner).clone()
                } else {
                    match payload_ty_at_instantiation(db, head, &cur) {
                        Some(t) => t,
                        None => return Ty::Any,
                    }
                }
            }
            crate::core::PathStep::Elem(i) => match &cur {
                Ty::Tuple(elems) => match elems.get(*i) {
                    Some(t) => t.clone(),
                    None => return Ty::Any,
                },
                Ty::List(elem) => (**elem).clone(),
                _ => return Ty::Any,
            },
            // A rest sublist is the same list type as its scrutinee.
            crate::core::PathStep::RestFrom(_) => match &cur {
                Ty::List(_) => cur.clone(),
                _ => return Ty::Any,
            },
        };
    }
    subst.apply(&cur)
}

/// The type VARIABLE of the parameter a call HEAD names, if the head resolves (through a `Ref`) to a
/// binder in `env` — i.e. the head IS a fn-typed parameter being solved (`(f h)`). Returns the var
/// (freshly cloned so the caller can unify a function type into it); `None` for a non-parameter head (a
/// def, an operator, a literal). The head analogue of `arg_ty_in_env`'s param-reference lookup.
fn binder_var_of(
    db: &mut Db,
    head: StructId,
    env: &crate::fxhash::FxHashMap<StructId, Ty>,
) -> Option<Ty> {
    match resolved_of(db, head) {
        Resolved::Ref { value } => env.get(&value).cloned(),
        Resolved::Param { binder } => env.get(&binder).cloned(),
        _ => None,
    }
}

/// The parameter NAME occurrences of def `def` in signature order (the order arguments match). Mirrors
/// the peeling `solve_recursive_params` does, but returns them ordered for positional self-call unify.
fn ordered_param_binders(db: &Db, def: usize) -> Vec<StructId> {
    db.defs[def]
        .params
        .iter()
        .map(
            |p| match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
                Some(name_occ) => name_occ,
                None => *p,
            },
        )
        .collect()
}

/// The generalized SIGNATURE of top-level definition `def` as a [`Scheme`] — its type as a value, so a
/// CALL can be typed by instantiating this scheme rather than by β-reducing the body. Curried: a def
/// `(f a b)` whose params solve to `A`,`B` and whose body solves to `R` has scheme `A -> B -> R` (a
/// nullary def is just `R`). Memoized on `db.def_schemes` keyed by the def index (a pure function of
/// the fixed def structure).
///
/// This computes the scheme for a def
/// whose signature is DETERMINED: an annotated/exported function (its params have definite machine
/// types and the body types reading them — the scheme AGREES with what β-reduction produces at a call,
/// cross-checked in tests), OR an unannotated RECURSIVE def whose parameters the connected solve
/// (`solve_recursive_params`, A2) has pinned. It returns `None` — deferring to β-reduction typing —
/// only when a parameter stays undetermined (`Any`, no use constrained it). This scheme is READ at a
/// runtime call site: `lower` emits a `Core::Call` to a recursive callee whose scheme is `Some`, and
/// `infer` types a recursive call by it.
pub fn def_scheme(db: &mut Db, def: usize) -> Option<Scheme> {
    if let Some(cached) = db.def_schemes.get(&def) {
        return cached.clone();
    }
    // If this def's PARAMETERS are still being solved (`solve_recursive_params` is on the stack — the A2
    // connected solve is demanded BEFORE `def_scheme` for a module-member internal def, whose external
    // call and self-call both route through the param-solve first), computing the scheme now would read
    // an as-yet-`Any` parameter type and cache a spurious `None` that POISONS every later call. Return
    // `None` WITHOUT caching, so once the solve completes and stores the parameter types, a subsequent
    // `def_scheme` recomputes the real scheme. (A top-level recursive def reaches `def_scheme` first, so
    // its own `type_of`→solve completes inline and this guard never fires — byte-identical there.)
    if db.solving_params.contains(&def) {
        return None;
    }
    // RE-ENTRY GUARD (self- AND mutual recursion). Computing a def's scheme demands `type_of` of its
    // body, whose recursive call — to THIS def or a mutually-recursive sibling — demands a scheme not yet
    // computed. Track in-progress solves in `solving_schemes`: a demand for a def already on the stack
    // returns `None` (the call types as `Any`, absorbed by the base case — the same behavior as the
    // β-reduction recursion guard) rather than looping forever. Kept in a SEPARATE set from the
    // `def_schemes` memo so an in-progress solve is not indistinguishable from a determined-`None`
    // scheme (the old `None`-sentinel-in-`def_schemes` conflated the two and poisoned mutual dispatchers,
    // below).
    if db.solving_schemes.contains(&def) {
        return None;
    }
    let reentrant_solve = !db.solving_schemes.is_empty();
    db.solving_schemes.insert(def);
    let scheme = compute_def_scheme(db, def);
    db.solving_schemes.remove(&def);
    // CACHE, EXCEPT a spurious mutual-recursion `None`. A `None` computed while ANOTHER scheme solve was
    // still on the stack may have read that sibling's in-progress (as-yet-`None`) signature as `Any` — as
    // a mutually-recursive PURE DISPATCHER does, whose body is ENTIRELY the sibling call (e.g.
    // `(def (od (: n Int64)) (ev (- n 1)))` where the sibling `ev` performs an effect and is the entry
    // demanded first). Caching that `None` would poison the dispatcher permanently, even once `ev` is
    // determined. Leave it uncached so the next demand — once the sibling's real scheme is memoized —
    // recomputes the true signature. A `Some` scheme, or a `None` reached at the TOP of the stack (a
    // genuinely undetermined signature), caches exactly as before (the common non-mutual case is
    // byte-identical: a top-level demand has an empty stack, so `reentrant_solve` is false).
    if scheme.is_some() || !reentrant_solve {
        db.def_schemes.insert(def, scheme.clone());
    }
    scheme
}

/// Whether EVERY parameter of `def` has a DETERMINED (non-`Any`) type — the param half of what
/// `compute_def_scheme` checks before it also demands the body/result. When `def_scheme` returns `None`
/// but this returns `true`, the undetermined thing is the def's RESULT, not a parameter: the body never
/// yields a concrete type (a recursive function with NO base case — every path recurses — so its result
/// is unconstrained). The `def_scheme`-declined call site uses this to tell those two failures apart, so
/// it doesn't tell an author whose parameter is ALREADY annotated `(: n Int64)` to "add an explicit
/// annotation" — the parameter is fine; the missing base case is the fault. Mirrors `compute_def_scheme`'s
/// param loop exactly (same binder extraction + `type_of` + `Ty::Any` test) so the two agree.
pub(crate) fn recursive_def_params_all_determined(db: &mut Db, def: usize) -> bool {
    let sig_params = db.defs[def].params.clone();
    for p in &sig_params {
        let binder = match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *p,
        };
        if matches!(type_of(db, binder), Ty::Any) {
            return false;
        }
    }
    true
}

fn compute_def_scheme(db: &mut Db, def: usize) -> Option<Scheme> {
    let body = db.defs[def].body?;
    let sig_params = db.defs[def].params.clone();
    // Each parameter's type — read `type_of` on its NAME occurrence (the annotation type for an
    // annotated param, `Any` for an unannotated one). An `Any` parameter is UNDETERMINED here: it
    // needs the connected def-body solve A2 adds, so decline the scheme and let the caller β-reduce.
    let mut param_tys: Vec<Ty> = Vec::new();
    for p in &sig_params {
        let binder = match db.ast.as_form(*p, ":").and_then(|t| t.first().copied()) {
            Some(name_occ) => name_occ,
            None => *p,
        };
        let ty = type_of(db, binder);
        if matches!(ty, Ty::Any) {
            trace!(target: "rcdzc::infer", def, "def_scheme: an undetermined param → defer to β-reduction (A2)");
            return None;
        }
        param_tys.push(ty);
    }
    // RIGID PARAM VARS for the body solve: the whole-type variables of this def's OWN parameter types are
    // the tie a recursive-generic producer must keep (`xs : List ?a` → the result must stay `Iter ?a`, not
    // a freshened `Iter ?b`). Mark them rigid so `apply_type`'s recursive-call arg-freshen (`freshen_arg`)
    // preserves them, while a genuinely-fresh local placeholder (`(None) : Option ?0`, `Map.empty : Map ?k
    // ?v` — NOT a param var) still freshens. This is the var-PROVENANCE distinction a shape-based check
    // cannot make. Collected from `param_tys` (only whole-type `Ty::Var`s — a numeric width/sign var grounds
    // to a default and is not a generic-element tie). Cleared after the solve; never nested.
    let mut rigid: crate::fxhash::FxHashSet<u32> = crate::fxhash::FxHashSet::default();
    for pt in &param_tys {
        let mut vs = Vec::new();
        pt.collect_free_vars(&mut vs);
        rigid.extend(vs);
    }
    let prev_rigid = db.scheme_rigid_vars.take();
    if !rigid.is_empty() {
        db.scheme_rigid_vars = Some(rigid);
    }
    // The result type is the body's solved type. `Any` here means the body could not be typed without
    // reducing a self-call (recursion) — defer to the fixpoint A2 adds.
    let result = type_of(db, body);
    db.scheme_rigid_vars = prev_rigid;
    if matches!(result, Ty::Any) {
        trace!(target: "rcdzc::infer", def, "def_scheme: undetermined result (recursive?) → defer (A2)");
        return None;
    }
    // DEFER A RESULT WITH A DATA-POSITION `Any` BORN FROM AN IN-FLIGHT SIBLING (mutual-recursion SCC). A
    // member whose result is determined ONLY by a recursive sibling — `parse-if` returns `(id, q, t4)`
    // where `q` is the next-index field of a `parse-any` result, and `parse-if` has NO non-recursive base
    // that fixes that field — types its result `(Tuple Int64 Any Tree)` while the sibling's scheme is
    // still on the stack (the re-entry guard below returns `None` → the call types `Any` → the projected
    // field collapses to `Any`). Unlike a bare-`Any` result (caught above), this `Any` hides inside a
    // Tuple, so `matches!(result, Ty::Any)` misses it and the scheme was finalized with the hole —
    // FREEZING the field `Any`, which the backend then boxes so the mutual recursion never terminates
    // (`((1))` hangs) and `--target rust` declines "no native representation". DEFER (return `None`,
    // uncached under `reentrant_solve`) exactly like the bare-`Any` case: a later CLEAN demand — after the
    // SCC's schemes settle — recomputes the result with the sibling concrete, grounding the field to its
    // real `Int64`. SCOPED to a REENTRANT solve (`!db.solving_schemes.is_empty()`, minus this def itself):
    // a data-`Any` result at the TOP of the solve stack is genuinely undetermined (no in-flight sibling to
    // re-ground it), so it keeps its scheme as before — deferring it would loop forever with no retry that
    // makes progress. `has_any_in_data_element` excludes an arrow's not-yet-solved closure hole (kept, per
    // `across_def_flavors`).
    let sibling_in_flight = db.solving_schemes.iter().any(|&d| d != def);
    if sibling_in_flight && result.has_any_in_data_element() {
        trace!(target: "rcdzc::infer", def, result = %result.render_name(&db.name_ctx()), "def_scheme: result has a data-Any from an in-flight sibling → defer (re-grounds after the SCC settles)");
        return None;
    }
    // Curry: `p_0 -> p_1 -> … -> result`. A nullary def is just `result`.
    let mut ty = result;
    for pt in param_tys.into_iter().rev() {
        ty = Ty::Fn(Box::new(pt), Box::new(ty));
    }
    // GENERALIZE: quantify over every free type variable the signature still carries — a recursive-generic
    // parameter the body only threads is left a `Ty::Var` by the A2 solve (see `generic_param_positions`),
    // so the scheme is `∀a. … a … a` and each call site `instantiate`s it fresh (recursive-generic
    // monomorphization; `unify::instantiate` freshens the bound vars). A signature with NO free var
    // generalizes to `Scheme::mono` exactly as before (`ty_vars` empty) — the monomorphic recursive case
    // is byte-identical. Only whole-type vars are quantified; a numeric width/sign grounds to a default.
    let mut ty_vars = Vec::new();
    ty.collect_free_vars(&mut ty_vars);
    if ty_vars.is_empty() {
        trace!(target: "rcdzc::infer", def, scheme = %ty.render_name(&db.name_ctx()), "def_scheme: determined monomorphic signature");
        return Some(Scheme::mono(ty));
    }
    trace!(target: "rcdzc::infer", def, scheme = %ty.render_name(&db.name_ctx()), quantified = ty_vars.len(), "def_scheme: generalized polymorphic signature (recursive-generic)");
    Some(Scheme {
        ty_vars,
        width_vars: Vec::new(),
        sign_vars: Vec::new(),
        ty,
    })
}

/// The result type of applying `head` to `args` — the ONE generic application rule. Read the head's
/// type as a [`Scheme`] (its `(meta t)`, a type-lambda reduced by the evaluator), instantiate it with
/// fresh variables, and unify each argument's type into the curried parameter positions; the result
/// is the instantiated return type after substitution. This types an operator (`+ : ∀a. (Int a) →
/// (Int a) → (Int a)`) and a user function alike, with no per-operator logic. On any failure — a
/// head with no type, a non-function head, an arity/unify mismatch — return `Any` so the value column
/// stays total; the actual FAULT is reported by `type_errors`.
/// The type of a compound-VALUE constructor (`tuple`/`record`/`list` alias) applied to `args` — a
/// compound VARIADIC in the arguments, so it cannot be a fixed `(meta t)` scheme (unlike a sum variant's
/// arrow). A tuple is the product of the arg types; a record maps each `(key value)` pair's key to its
/// value type; a list is `List <elem>` where `<elem>` is the JOIN of the argument types (homogeneous,
/// `Any` for the empty list). Typed the same way the symbol-headed `Resolved::Tuple`/`Record`/`List`
/// forms are, so the name-alias application and the symbol primitive agree.
fn compound_ctor_type(db: &mut Db, prim: crate::resolved::Prim, args: &[StructId]) -> Ty {
    use crate::resolved::Prim;
    match prim {
        // Each element is an INDEPENDENT type position, so its type's free vars must be DISJOINT from its
        // siblings' — otherwise two elements that each `type_of` to a colliding var (a bare `None()` types
        // `Option(?0)` from its own `Fresh::new()`, so two of them share `?0`) would cross-contaminate when
        // the whole tuple/record type unifies against an expected type in ONE `Subst` (unifying `?0 := A`
        // for one element then `?0 := B` for the sibling — a spurious mismatch). Freshen each element's free
        // vars into a fresh disjoint block off a SHARED counter (the same `freshen_free` the arg-check uses).
        Prim::TupleNew => {
            let mut fresh = crate::unify::Fresh::new();
            Ty::Tuple(
                args.iter()
                    .map(|&e| crate::unify::freshen_free(&type_of(db, e), &mut fresh))
                    .collect(),
            )
        }
        Prim::RecordNew => match crate::resolve::read_record_fields(db, args) {
            Ok(fields) => {
                let mut fresh = crate::unify::Fresh::new();
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    // Freshen per field so sibling fields' vars are independent (see the TupleNew note): two
                    // bare `None()` fields must NOT share an `Option` element var, or one field borrows the
                    // other's element type when the record unifies against its expected type.
                    let ft = crate::unify::freshen_free(&type_of(db, value), &mut fresh);
                    field_tys.insert(label.clone(), ft);
                }
                Ty::Record(std::rc::Rc::new(field_tys))
            }
            // A malformed field list has no well-formed record type — `Any`; the fault is reported by
            // `type_errors`.
            Err(_) => Ty::Any,
        },
        Prim::ListNew => {
            let mut elem_ty = Ty::Any;
            for &e in args {
                elem_ty = elem_ty.join(&type_of(db, e));
            }
            Ty::List(Box::new(elem_ty))
        }
        // `ast-splice-lift : (List Int64) → (List Ast)` — the quasiquote-splice lift (compiler-internal).
        // Its result is a list of `Ast` nodes; a bad operand shape is caught at the fold (declines), so
        // typing is unconditional here.
        Prim::AstSpliceLift => match ast_sum_ty(db) {
            Some(ast_ty) => Ty::List(Box::new(ast_ty)),
            None => Ty::Any,
        },
        // `ast-lift : ∀a. a → Ast` — the runtime active-unquote lift (compiler-internal). Whatever the
        // operand's type, the RESULT is an `Ast` node (identity when the operand is already `Ast`, else a
        // wrapped `Ast.Int`/`Bool`/`Str`). The operand's type is checked at the fold (`lower_ast_lift`).
        Prim::AstLift => ast_sum_ty(db).unwrap_or(Ty::Any),
        _ => Ty::Any,
    }
}

/// The `Ty::Sum` of the built-in `Ast` prelude sum (a monomorphic sum — no args), or `None` if the
/// declaration is somehow absent. Used to type `ast-splice-lift`'s `(List Ast)` result.
fn ast_sum_ty(db: &Db) -> Option<Ty> {
    let occ = db.type_decls.iter().find(|t| t.name == "Ast")?.occ;
    Some(db.normalize_sum(occ, Vec::new()))
}

/// `Option T` for a concrete element `T` — the prelude `Option` sum instantiated at one arg. `None` when
/// the `Option` declaration is somehow absent (a prelude-less compile). Used to spell the `correlation`
/// and `payload` fields of a world-effect request record.
fn option_ty(db: &Db, elem: Ty) -> Option<Ty> {
    let occ = db.type_decls.iter().find(|t| t.name == "Option")?.occ;
    Some(db.normalize_sum(occ, vec![elem]))
}

/// The TYPE a REIFIED async world-effect perform carries in the reducer's returned effect-list — the
/// effect-request record `{ correlation: Option Bytes, kind: String, payload: Option Bytes [, target: Bytes] }`
/// (schema-hash phase-1a, the shape v-rust-backend's `reify_effect_to_tuple` emits). Fields are name-sorted
/// (`correlation < kind < payload < target`) so the `Ty::Record` `BTreeMap` order matches reify's
/// value-encode sorted-slot column order. `has_target` (v-rb ruling A, 2026-08-14): a target-having effect
/// (one whose op carries an `@resource` marker — e.g. `Emit.send(@resource dest, …)`) reifies the dest as a
/// RUNTIME VALUE riding its own `target: Bytes` field (SEC-F1 authorizes the dest value), so the type gains
/// that field; a target-FREE effect (model/now/timer/tool) stays 3-field. Phase-1a is the single-`Bytes`-arg
/// (or zero-arg) payload case: `payload` is `Option Bytes`; a STRUCTURED/multi-arg payload rides R2 (the
/// in-fold value-encode primitive). `None` when the prelude `Option` is absent.
fn world_effect_request_ty(db: &Db, has_target: bool, has_descriptor: bool) -> Option<Ty> {
    let correlation = option_ty(db, Ty::Bytes)?;
    let payload = option_ty(db, Ty::Bytes)?;
    let mut fields = std::collections::BTreeMap::new();
    fields.insert(crate::resolved::Symbol::plain("correlation"), correlation);
    fields.insert(crate::resolved::Symbol::plain("kind"), Ty::String);
    fields.insert(crate::resolved::Symbol::plain("payload"), payload);
    if has_target {
        fields.insert(crate::resolved::Symbol::plain("target"), Ty::Bytes);
    }
    // `schema_descriptor: Bytes` (phase-3 producer-bake, v-rb): present iff the reify EMITS the field — i.e.
    // iff `effect_has_schema_descriptor` (the reify's descriptor builds). The typed shape MUST match the emit
    // or the emitted `schema_descriptor` field is DROPPED when the record types against this shape (the bug
    // v-pc case-32 hit: emit 4-field, type 3-field → field lost → schema_hash None). `has_descriptor` is
    // computed at the callsite via the SAME `lower::effect_has_schema_descriptor` the emit gates on, so they
    // cannot drift. Name-sorted, so it slots after correlation/kind/payload/target.
    if has_descriptor {
        fields.insert(
            crate::resolved::Symbol::plain("schema_descriptor"),
            Ty::Bytes,
        );
    }
    Some(Ty::Record(std::rc::Rc::new(fields)))
}

/// The result type of `(Qty.pow q n)`: q's inner numeric type carried over, with q's unit raised to the
/// `n`th power (`Unit::pow`). `None` when arg0 is not a quantity or arg1 is not a compile-time `Int`
/// literal (the caller then falls through). `#[inline(never)]` so it does NOT enlarge `apply_type`'s
/// stack frame — that function is on the deep `type_of`↔`apply_type` recursion.
///
/// This is a dimension-DERIVING operation: `Qty.pow` produces the dimension its rule defines (the unit
/// raised to `n`, e.g. `meter` → `meter²`) carried on the result `Ty::Qty`, rather than discarding the
/// dimensional information to a bare numeric.
//= spec/capabilities/units-of-measure.md#dimensional-mismatch-is-an-error
//# An operation that derives a dimension MUST produce the dimension the operation's rule defines rather than discard dimensional information.
#[inline(never)]
fn qty_pow_type(db: &mut Db, args: &[StructId]) -> Option<Ty> {
    if let Ty::Qty { inner, unit } = type_of(db, args[0])
        && let Resolved::Int(v) = resolved_of(db, args[1])
        && let Some(n) = v.to_i64()
    {
        return Some(Ty::Qty {
            inner,
            unit: unit.pow(n),
        });
    }
    None
}

/// The type of a tuple built from `elems`, EXCEPT the empty tuple IS the unit value (`Ty::Unit`) — the
/// empty-tuple-is-unit convention (`core-semantics.md` §The Empty Tuple Is The Unit Value). Used by the
/// tuple row ops' result-type arms (a `split-at 0` prefix / an all-consumed suffix is `Unit`, not a
/// zero-arity tuple). There is no zero-arity `Ty::Tuple`: `()` and `unit` are the same value.
//= spec/capabilities/core-semantics.md#a-tuple-is-a-fixed-size-positional-product
//# The empty tuple MUST be the unit value, so that unit and `()` are the same value.
fn tuple_or_unit(elems: &[Ty]) -> Ty {
    if elems.is_empty() {
        Ty::Unit
    } else {
        Ty::Tuple(elems.iter().cloned().collect())
    }
}

/// The tuple type built from `id`'s element occurrences when `id` is a TUPLE CONSTRUCTOR (the
/// symbol-headed `Resolved::Tuple` or the `tuple` NAME-alias application) — `(Tuple <type_of(e0)>
/// <type_of(e1)> …)`. Typing each element on its OWN resolves a RECURSIVE-call element via its cached
/// `def_scheme`, where the aggregate `type_of((tuple …))` reads it as `Any` during the enclosing def's
/// own solve. `None` for a non-tuple `id`, so the caller falls back to the ordinary `type_of`.
fn tuple_constructor_ty(db: &mut Db, id: StructId) -> Option<Ty> {
    let elems: Vec<StructId> = match resolved_of(db, id) {
        Resolved::Tuple { elems } => elems.to_vec(),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleNew) =>
        {
            args.to_vec()
        }
        _ => return None,
    };
    Some(Ty::Tuple(elems.iter().map(|&e| type_of(db, e)).collect()))
}

/// Freshen an application argument's type past the head's instantiation counter (the occurs-check
/// dodge — see the call sites), BUT preserve the def's own parameter type vars while a
/// `compute_def_scheme` body solve is active (`db.scheme_rigid_vars`). Preserving those vars keeps a
/// recursive-generic producer's param↔result element TIE connected across the recursive-call arg-freshen
/// (`List a -> Iter a` stays tied, not a disjoint `∀a b`); a genuinely-fresh local placeholder (`(None)`,
/// `Map.empty`) is NOT a param var, so it still freshens — the var-provenance distinction. Outside a
/// scheme solve (`None` — every ordinary application) this is byte-identical to a plain `freshen_free`.
fn freshen_arg(db: &Db, at: &Ty, fresh: &mut crate::unify::Fresh) -> Ty {
    match &db.scheme_rigid_vars {
        Some(rigid) => crate::unify::freshen_free_except(at, fresh, rigid),
        None => crate::unify::freshen_free(at, fresh),
    }
}

fn apply_type(db: &mut Db, head: StructId, args: &[StructId]) -> Ty {
    // CASE-OF-CASE (matches `lower`): a head that reduces to a runtime `if` — `((if c a b) args…)` —
    // types as the `if` of the two branch applications. Each branch's lambda applies (β-reduces) to a
    // concrete result type, so the application's type is that (`Int64`), NOT `Ty::Fn` (the naive type
    // of the `if`, which would then have no machine representation at the boundary). Type each branch
    // applied to the same args and JOIN — an `if`'s two branches must agree, so either branch's type is
    // the result; take the then-branch's (the else must unify with it, checked at the `if`'s own node).
    if let Some((_cond, then_head, else_head)) = crate::eval::reduce_to_if(db, head) {
        let then_ty = apply_type(db, then_head, args);
        if !matches!(then_ty, Ty::Any) {
            return then_ty;
        }
        // The then-branch didn't determine a type (recursive/undetermined); fall back to the else.
        return apply_type(db, else_head, args);
    }
    // A LAMBDA head β-reduces; the application's type is the reduced body's type. The reduction runs
    // under the recursion guard (keyed by the lambda body), so a recursive call declines to `Any`
    // rather than diverging — matching lowering. (For C's corpus every function call folds this way;
    // a scheme-based typing of a lambda head arrives with a def's inferred scheme.)
    if crate::eval::lambda_body(db, head).is_some() {
        trace!(target: "rcdzc::infer", head = head.0, args = args.len(), "apply: β-reduce lambda head for its type");
        let reduced_ty = match db.enter_reduction() {
            Some(mut guard) => {
                let g = guard.db();
                match crate::eval::apply_lambda(g, head, args) {
                    Ok(Some(reduced)) => Some(type_of(g, reduced)),
                    _ => None, // recursive / partial — β-reduction can't type it; try the scheme below.
                }
            }
            None => None, // depth limit (recursive) — try the scheme below.
        };
        if let Some(t) = reduced_ty {
            return t;
        }
        // β-reduction couldn't type it (a RECURSIVE callee). Type the call by the callee's DEF SCHEME
        // instead — an annotated recursive def has a determined signature (`def_scheme`), so applying
        // it to the args yields the real result type (Int64 for `(sum-to 3)`) rather than `Any`. This
        // is what lets a recursive call's RESULT flow to a machine type (a wasm return valtype). A
        // callee with no determined scheme (unannotated, needs the connected solve) stays `Any`.
        if let Some(callee) = callee_def_index_for_infer(db, head)
            && let Some(scheme) = def_scheme(db, callee)
        {
            trace!(target: "rcdzc::infer", head = head.0, callee, "apply: recursive call typed by def_scheme");
            return apply_scheme_to_args(db, &scheme, args);
        }
        return Ty::Any; // recursive with an undetermined signature — fault reported elsewhere.
    }
    // `(Int64.of b)` / `(UInt N).of b` where `b : BigInt` — the CHECKED NARROWING from the unbounded
    // integer back to a fixed width (`options/numeric-model/explicit-checked.md`: `Int64.of` converts a
    // `BigInt` back, trapping when out of range). `CheckedOf`'s prelude scheme source is `(Int a)`, which
    // does NOT accept a `BigInt` — so a dedicated arm handles a `BigInt` source: the result is the
    // conversion op's TARGET type (this application's own solved type, an `Ty::Int`), exactly as the
    // fixed-width→fixed-width `of` result is. The lower fold already range-checks the constant (`fits_width`
    // on the unbounded `IntValue`) and rejects an out-of-range one CDZ0302, source-type-agnostically.
    // Only when the source really is a `BigInt`; a fixed-width source stays on the scheme path below.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::CheckedOf)
        && args.len() == 1
        && matches!(type_of(db, args[0]), Ty::BigInt)
    {
        // The result is the conversion op's TARGET width — the RESULT of its `∀a. (Int a) → TARGET`
        // scheme (TARGET is baked into this module's `of` field). Instantiate the head's scheme and peel
        // the arrow's result; the source `(Int a)` is ignored (a `BigInt` source, not unified here). A
        // head without a scheme (malformed) falls through to `Any`.
        let mut fresh = Fresh::new();
        if let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh)
            && let Ty::Fn(_, result) = crate::unify::instantiate(&scheme, &mut fresh)
        {
            return *result;
        }
        return Ty::Any;
    }
    // `Qty.of x u` — attach a compile-time unit to a numeric value. Its result type is `(Qty T u)` where
    // `T` is the VALUE argument's type and `u` is the VALUE of the second argument (a compile-time unit,
    // read by `eval::unit_of` — NOT an HM-unified variable, exactly as `Prim::Wrap` reads its target
    // width off the solved type rather than the scheme). Checked before the scheme-peeling loop because
    // the unit is not expressible as a static `(meta t)` arrow. If the unit does not reduce (a malformed
    // unit expression), fall through to the generic path (which yields `Any`, faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyOf) && args.len() == 2
    {
        let inner = type_of(db, args[0]);
        if let Some(unit) = crate::eval::unit_of(db, args[1]) {
            return Ty::Qty {
                inner: Box::new(inner),
                unit,
            };
        }
    }
    // Each `Record.*` row operation below computes a NEW closed `Ty::Record` from the OPERANDS' record
    // shapes alone (project keeps the named subset, without drops them, merge unions, extend/with insert)
    // — it never mutates an operand type and never leaves a row variable open in the result, so the
    // reshaped record is a fresh value with a statically-fixed field set and the emitted component carries
    // no runtime field-set computation. A shape change happens ONLY through such an explicit `Record.*`
    // operation — inference never widens or narrows a record's field set, so a reshape is always written.
    //= spec/capabilities/type-system.md#a-record-row-is-reshaped-only-through-an-explicit-operation-yielding-a-new-value
    //# A program MUST be able to derive a new record from existing records by an explicit row operation — restricting to named fields, dropping named fields, or combining two records — rather than by an implicit widening or narrowing that inference introduces, so that a shape change is always something the program wrote.
    //= spec/capabilities/type-system.md#a-record-row-is-reshaped-only-through-an-explicit-operation-yielding-a-new-value
    //# A record row operation MUST yield a new record value and MUST NOT alter the operand records, consistent with the immutable value heap, so that reshaping a record is the derivation of a new value with a new shape and not a mutation of an existing one.
    //= spec/capabilities/type-system.md#a-record-row-is-reshaped-only-through-an-explicit-operation-yielding-a-new-value
    //# The shape of a record row operation's result MUST be determined statically from the operands' shapes, so that the emitted component carries a concrete closed record shape and the operation introduces no runtime field set.
    //
    // `Record.project r (a c)` — narrow `r` to the named fields. The result is a NEW closed record type
    // whose fields are EXACTLY the named ones, each carrying the type it had in `r` (`type-system.md` §A
    // Record Is Restricted To A Named Set Of Its Fields). The second operand is a LITERAL field-name list
    // `(a c)` (labels via `record_op_labels`, not an evaluated value — not HM-unified, like `Qty.of`'s
    // unit / `Wrap`'s width). A named field ABSENT from `r`'s record type has no type to carry — it is
    // the CDZ0212 rejection (reported by `check_application`); here it is simply dropped from the result
    // shape so the value column stays a sane record. A non-record operand → `Any` (faulted elsewhere).
    //= spec/capabilities/type-system.md#a-record-is-restricted-to-a-named-set-of-its-fields
    //# A program MUST be able to project a record onto a stated set of field names, yielding a record whose fields are exactly those names bound to the values the operand holds for them, so that narrowing a record to a sub-shape is an explicit operation rather than an overloaded equality.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordProject)
        && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(labels) = crate::resolve::record_op_labels(db, args[1])
    {
        let mut kept = std::collections::BTreeMap::new();
        for label in &labels {
            if let Some(ty) = fields.get(label) {
                kept.insert(label.clone(), ty.clone());
            }
        }
        return Ty::Record(std::rc::Rc::new(kept));
    }
    // `Record.without r (b)` — `r` MINUS the named fields (the complement of `project`). The result is a
    // NEW record type keeping every field of `r` whose label is NOT named. Same literal field-name list;
    // an absent named field is CDZ0212 (`check_application`), not reflected in the shape.
    //= spec/capabilities/type-system.md#a-record-is-reduced-by-dropping-a-named-set-of-its-fields
    //# A program MUST be able to derive a record that drops a stated set of field names from an operand record, yielding a record whose fields are exactly the operand's remaining fields, so that removing a field is the complement of projecting the fields kept.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordWithout)
        && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(labels) = crate::resolve::record_op_labels(db, args[1])
    {
        let drop: std::collections::BTreeSet<_> = labels.into_iter().collect();
        let kept: std::collections::BTreeMap<_, _> = fields
            .iter()
            .filter(|(k, _)| !drop.contains(*k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        return Ty::Record(std::rc::Rc::new(kept));
    }
    // `Record.merge a b` — the UNION of two records' fields. The result is a NEW record type with every
    // field of BOTH operands. The field sets MUST be disjoint (a shared name is CDZ0211, reported by
    // `check_application`); the type here is the union regardless (a shared field's fault fires there, and
    // last-writer here keeps the shape sane). A non-record operand → the generic path (Any).
    //= spec/capabilities/type-system.md#two-records-are-combined-only-when-their-field-sets-are-disjoint
    //# A program MUST be able to combine two records into one whose field set is the union of the operands' field sets, each field bound to the value its source record holds, so that merging records is the row analogue of forming a record from two groups of fields.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordMerge)
        && args.len() == 2
        && let (Ty::Record(a), Ty::Record(b)) = (type_of(db, args[0]), type_of(db, args[1]))
    {
        let mut union: std::collections::BTreeMap<_, _> =
            a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in b.iter() {
            union.insert(k.clone(), v.clone());
        }
        return Ty::Record(std::rc::Rc::new(union));
    }
    // `Record.extend r (z v)` / `Record.with r (z v)` — ADD (extend) or REPLACE (with) field `z` with the
    // VALUE `v`'s type. Both yield a NEW record type = `r`'s fields with `z ↦ typeof(v)` inserted (an
    // insert covers both: for `extend` z is new, for `with` it overwrites the old entry with the possibly-
    // DIFFERENT new type — §A Field Is Added To Or Replaced …, 'a new value of a possibly different type').
    // The presence/absence fault (extend→CDZ0211 if present, with→CDZ0212 if absent) is `check_application`'s;
    // the shape here is the same insert for both. The second operand is a `(name value)` pair read by
    // `record_op_pair`; `v` IS an evaluated value (its type is `typeof(v)`), unlike a label list.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that adds a field absent from an operand record, and a combination that adds a field the operand already contains MUST be rejected at compile time with the machine-readable code for a field that is already present, so that adding a field never silently overwrites an existing one.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that replaces a field present in an operand record with a new value of a possibly different type, so that updating a field is an explicit operation distinct from adding one and the replacement's type is whatever the new value holds.
    // THREE-operand form (DESIGN-record-update-syntax.md): `(Record.with r #z v)` /
    // `(Record.extend r #z v)` — a record, a `#symbol` field LABEL (`read_key` reads the label statically,
    // NOT as a `Ty::Symbol` value — the row-op field name stays compile-time), and the VALUE `v` (an
    // ordinary expression, its type is `typeof(v)`). Result = the record with field `z` inserted/replaced.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that adds a field absent from an operand record, and a combination that adds a field the operand already contains MUST be rejected at compile time with the machine-readable code for a field that is already present, so that adding a field never silently overwrites an existing one.
    //= spec/capabilities/type-system.md#a-field-is-added-to-or-replaced-in-a-record-by-a-derived-operation
    //# A program MUST be able to derive a record that replaces a field present in an operand record with a new value of a possibly different type, so that updating a field is an explicit operation distinct from adding one and the replacement's type is whatever the new value holds.
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::RecordExtend | crate::resolved::Prim::RecordWith)
    ) && args.len() == 3
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(label) = crate::resolve::read_label(db, args[1])
    {
        let mut out: std::collections::BTreeMap<_, _> =
            fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        out.insert(label, type_of(db, args[2]));
        return Ty::Record(std::rc::Rc::new(out));
    }
    // `Record.pop r z` — yields `(tuple (. r z) (r without z))`: the field's value paired with the record
    // of the remaining fields (`type-system.md` §A Record Is Reduced By Dropping A Named Set Of Its
    // Fields). Result type `(Tuple <typeof field z> (Record <r minus z>))`. The absent-field CDZ0212 is
    // `check_application`'s; here an absent field would leave the tuple's first element `Any` (faulted
    // there). The second operand is a BARE field NAME (a label via `read_key`).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordPop)
        && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some(label) = crate::resolve::read_label(db, args[1])
    {
        let field_ty = fields.get(&label).cloned().unwrap_or(Ty::Any);
        let rest: std::collections::BTreeMap<_, _> = fields
            .iter()
            .filter(|(k, _)| **k != label)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let rest_ty = Ty::Record(std::rc::Rc::new(rest));
        return Ty::Tuple(std::rc::Rc::from([field_ty, rest_ty]));
    }
    // Each `Tuple.*` positional row op below derives a NEW `Ty::Tuple` (or `Ty::Unit` for the empty
    // prefix) from the OPERANDS' element types — cat sums the arities, split-at/pop partition — so the
    // result arity is fixed statically from the operand arities, an operand tuple is never mutated, and
    // the emitted component carries a concrete tuple shape rather than a runtime-length tuple.
    //= spec/capabilities/type-system.md#a-tuple-is-reshaped-positionally-by-an-explicit-operation-yielding-a-new-value
    //# A program MUST be able to derive a new tuple from existing tuples by an explicit positional operation — concatenating two tuples or splitting one at a stated position — rather than by an implicit change of arity, consistent with a tuple being a fixed-size positional value whose length is part of its type.
    //= spec/capabilities/type-system.md#a-tuple-is-reshaped-positionally-by-an-explicit-operation-yielding-a-new-value
    //# A tuple positional operation MUST yield a new tuple value and MUST NOT alter the operand tuples, consistent with the immutable value heap, so that reshaping a tuple is the derivation of a new value and not a mutation.
    //= spec/capabilities/type-system.md#a-tuple-is-reshaped-positionally-by-an-explicit-operation-yielding-a-new-value
    //# The arity of a tuple positional operation's result MUST be determined statically from the operands' arities, so that the emitted component carries a concrete tuple shape and the operation introduces no runtime-length tuple.
    //
    // `Tuple.concat a b` — the concatenation of two tuples' element types (`type-system.md` §Two Tuples Are
    // Concatenated Into One Of Their Combined Length). Result arity = the sum; each element keeps its
    // source position's type. A non-tuple operand → the generic path (Any).
    //= spec/capabilities/type-system.md#two-tuples-are-concatenated-into-one-of-their-combined-length
    //# A program MUST be able to concatenate two tuples into one whose elements are the first tuple's elements in order followed by the second tuple's elements in order, so that its arity is the sum of the operands' arities and each element keeps the type of its source position.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleCat)
        && args.len() == 2
        && let (Ty::Tuple(a), Ty::Tuple(b)) = (type_of(db, args[0]), type_of(db, args[1]))
    {
        let cat: Vec<Ty> = a.iter().chain(b.iter()).cloned().collect();
        return Ty::Tuple(cat.into());
    }
    // `Tuple.split-at t k` — split at compile-time position `k` into a PAIR `(prefix suffix)`
    // (`type-system.md` §A Tuple Is Split At A Position Into A Prefix And A Suffix). The result type is
    // `(Tuple <prefix-tuple> <suffix-tuple>)`; `k=0` makes the empty prefix the UNIT value (the empty
    // tuple IS unit), so the prefix type is `Ty::Unit`, not a zero-arity tuple. `k` out of `0..=arity` is
    // CDZ0201 (`check_application`); here an out-of-range k falls through to the generic path (Any).
    //= spec/capabilities/type-system.md#a-tuple-is-split-at-a-position-into-a-prefix-and-a-suffix
    //# A program MUST be able to split a tuple at a stated position into a pair of tuples — a prefix holding the elements before the position and a suffix holding the elements from the position onward — so that partitioning a tuple positionally is an explicit operation yielding both parts.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TupleSplitAt)
        && args.len() == 2
        && let Ty::Tuple(elems) = type_of(db, args[0])
        && let Resolved::Int(k) = resolved_of(db, args[1])
        && let Some(k) = k.to_i64()
        && (0..=elems.len() as i64).contains(&k)
    {
        let k = k as usize;
        let prefix = tuple_or_unit(&elems[..k]);
        let suffix = tuple_or_unit(&elems[k..]);
        return Ty::Tuple(std::rc::Rc::from([prefix, suffix]));
    }
    // `Tuple.remove t` — element 0 off: `(tuple (. t 0) <rest>)`. Result `(Tuple <e0> (Tuple <rest…>))`. A
    // non-tuple or empty-tuple operand falls through (Any); the arity-≥1 requirement is `check_application`'s.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TuplePop)
        && args.len() == 1
        && let Ty::Tuple(elems) = type_of(db, args[0])
        && !elems.is_empty()
    {
        let rest = tuple_or_unit(&elems[1..]);
        return Ty::Tuple(std::rc::Rc::from([elems[0].clone(), rest]));
    }
    // `Type.of e` — compile-time type reflection. The application itself has type `Type` (it IS a
    // type-value, like `(Qty T u)` / `(-> A B)` in type position); the type-value it REDUCES to (via
    // `eval::typeval_of` → the `reduce_ctor` arm) is `e`'s inferred type, consumed only in a type
    // position (an annotation, further type-level computation). A `Type` value is erased before the
    // boundary, so exporting one is rejected downstream ("a type value has no runtime form").
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TypeOf)
        && args.len() == 1
    {
        return Ty::Type;
    }
    // `Type.eq a b` — compile-time type equality. Its result is an ordinary `Bool` (the type COMPARISON
    // is compile-time — `lower` folds it to a constant — but the produced value is a runtime `Bool`, so
    // it flows into an `if`/`and`/… and branches on types). The arguments are type-values, read (not
    // HM-unified) by `lower`; a non-type argument is faulted where it fails to reduce there.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::TypeEq)
        && args.len() == 2
    {
        return Ty::Bool;
    }
    // `Qty.value q` — recover the underlying numeric value, DISCARDING the unit. Its result is the
    // quantity's INNER type; a non-quantity argument yields `Any` (faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyValue)
        && args.len() == 1
    {
        return match type_of(db, args[0]) {
            Ty::Qty { inner, .. } => *inner,
            _ => Ty::Any,
        };
    }
    // `Qty.pow q n` — the result unit is q's unit raised to the `n`th power (`Unit::pow`, composing
    // exponents + scale exactly as `Unit.^`); the inner numeric type is unchanged. `n` is a compile-time
    // Int literal read off arg1 (not an HM variable, like `Unit.^`'s power / `Qty.of`'s unit). A
    // negative `n` is left to `check_application`/lower to reject; here we still shape the type (the unit
    // map handles a negative power fine) so downstream sees a sane `Ty::Qty`. A non-quantity arg0 or a
    // non-literal exponent falls through to the generic path (→ Any, faulted elsewhere). The body is a
    // separate (never-inlined) helper so it keeps its `Ty::Qty` destructure (a `Box` + an inline `Unit`
    // map) out of `apply_type`'s frame, which is on the deep `type_of`↔`apply_type` recursion.
    if args.len() == 2
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyPow)
        && let Some(ty) = qty_pow_type(db, args)
    {
        return ty;
    }
    // `Unit.in target q` — explicit conversion that UNWRAPS to a bare dimensionless number
    // (DESIGN-quantity-reference-normalized-unwrap.md §1b): it converts q's magnitude into the target
    // unit's scale AND strips the quantity wrapper, so `(Unit.in meter (Qty.of 3.0 km))` : `Float64`
    // (the *number* of meters, 3000.0), NOT `(Qty Float64 meter)`. `as`/`in` is the deliberate EXIT
    // from the units world — the result is an ordinary number, no longer dimension-checked. The target
    // is still required to share q's DIMENSION (`check_application`, CDZ0501); here we fill the
    // value-column type with q's INNER numeric type. If the target unit doesn't reduce, or q isn't a
    // quantity, fall through (→ Any, faulted elsewhere).
    //= spec/capabilities/units-of-measure.md#an-explicit-conversion-unwraps-to-a-bare-number
    //# An explicit conversion of a quantity into a chosen unit — the `as`/`in` operation — MUST yield the dimensionless number counting how many of the chosen unit the quantity is, with the quantity wrapper removed, so that a conversion is the deliberate exit from the dimensional layer rather than a re-expression that stays dimensioned.
    //= spec/capabilities/units-of-measure.md#an-explicit-conversion-unwraps-to-a-bare-number
    //# The result of an explicit conversion MUST be an ordinary number of the quantity's underlying numeric type, subject to ordinary numeric rules and no longer dimension-checked, so that once a program has asked "how many of this unit is it?" it holds the answer as a plain number and may combine it freely.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
        && args.len() == 2
        && let (Some(_target), Ty::Qty { inner, .. }) =
            (crate::eval::unit_of(db, args[0]), type_of(db, args[1]))
    {
        return *inner;
    }
    // UNARY NEGATION `(- e)` — the arity-1 subtraction (the ML surface's prefix `-<expr>`, canonicalized
    // to `(- e)`; `lower` rewrites it to `0 - e` at the operand's numeric type). Its result type is the
    // OPERAND's type: negation is closed over every numeric type (`Int N`, `Float N`, `Rational`,
    // `BigInt`, and a `Qty` — negating a quantity keeps its unit), unlike the fixed-width `∀a. (Int a)`
    // scheme (which would ground a Float/Rational/BigInt/Qty operand to Int64 and phantom-fault). A
    // NON-numeric operand falls through to `Any` — `check_application` reports the honest "negation is not
    // defined on <type>" (the unary twin of the binary arithmetic-on-non-number reject). Checked before
    // the arity-2 numeric block below; the two are disjoint by operand count.
    if args.len() == 1 && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Sub) {
        let t = type_of(db, args[0]);
        if matches!(
            t,
            Ty::Int(_) | Ty::Float(_) | Ty::Rational | Ty::BigInt | Ty::Qty { .. }
        ) {
            return t;
        }
    }
    // A binary OPERATOR applied to QUANTITIES — the dimensional result type (units-of-measure.md §How
    // Arithmetic Composes Dimensions). Only engages when an operand is a `Ty::Qty` (two bare numbers take
    // the ordinary scheme path). `+`/`-` keep the shared unit (result `(Qty T u)`); `*`/`/` COMPOSE units
    // by the group product/quotient (result `(Qty T (u_a·u_b))` / `(Qty T (u_a/u_b))` — a `Unit.one`
    // result renders dimensionless); comparisons yield `Bool`. The unit COMPOSITION is why this is not an
    // HM scheme — a unit multiplies, it does not unify. A dimensional/​numeric FAULT is reported by
    // `check_application`; here we only fill the value-column TYPE. (When the units disagree for `+`/`-`,
    // the fault fires there and this type is unused; we still return the lhs quantity so the shape stays
    // sane.)
    if args.len() == 2
        && let Some(prim) = crate::eval::meta_apply_of(db, head)
    {
        {
            let a = type_of(db, args[0]);
            let b = type_of(db, args[1]);
            // PROVISIONAL-OPERAND DEFER: an arith operand that is `Any` WHILE A SCHEME SOLVE IS IN FLIGHT is
            // a not-yet-resolved SELF-CALL result — the recursion guard returns `Any` for a self-call typed
            // inside its own def's body during that def's scheme solve. Committing the arith result NOW
            // (the numeric arms below all fail to match `Any`, so it falls to the generic `∀a.(Int a)`
            // scheme → a DEFERRED `Int`) FREEZES the wrong type: once the def's result grounds (e.g. a
            // sibling `(Leaf n)` arm fixes it `BigInt`), a clean re-solve types the self-calls `BigInt` and
            // `+` over two BigInts is `BigInt` — but the frozen deferred-`Int` already conflicts with the
            // `BigInt` arm → a spurious CDZ0203 "arms differ: BigInt vs Int64" (`(+ (s a) (s b))` with two
            // self-calls and no anchoring literal). Return `Any` here so the node is NOT cached (the
            // `type_of` memo skips `Any`) and RE-solves cleanly once the operands' real types settle. Only
            // under `solving_schemes` (a re-grounding fixpoint); outside a solve an `Any` operand is a real
            // fault the generic path reports. EITHER-side provisional defers (the guard is `a || b` Any):
            // this block PRECEDES the numeric arms, so even `(+ 1N (f …))` — one anchoring literal + one
            // provisional self-call — defers here rather than committing to the literal's width, which is
            // correct: the `Any` operand's real type is not yet known (it may ground to `BigInt`/`Float`,
            // and `+` over a `BigInt` sibling is `BigInt`, not the literal's `Int`), so no width can be
            // soundly committed until the self-call resolves. A clean re-solve then routes it through the
            // numeric arms below with both operands' real types. (Deferring on either-Any, not both-Any, is
            // the safe choice — never commit an arith width while an operand is an unresolved self-call.)
            if !db.solving_schemes.is_empty()
                && (matches!(a, Ty::Any) || matches!(b, Ty::Any))
                && matches!(
                    prim,
                    crate::resolved::Prim::Add
                        | crate::resolved::Prim::Sub
                        | crate::resolved::Prim::Mul
                        | crate::resolved::Prim::Div
                        | crate::resolved::Prim::Rem
                )
            {
                return Ty::Any;
            }
            // A QUANTITY operand takes the dimensional arm below, NOT the bare-numeric arms here — even
            // when its SIBLING is a bare `Float`/`BigInt`/`Rational`. `(* (Qty Float64 meter) 3.0)` must
            // stay `(Qty Float64 meter)` (a bare number scales, contributing `Unit.one`), not collapse to
            // the bare `Float64` the Float arm would return by matching the bare sibling. So gate the three
            // bare-numeric arms on `!any_qty`, mirroring `check_application`'s `is_multiplicative && any_qty`
            // ordering (the quantity check runs first there too). Without this the unit was silently dropped
            // and `Qty.value` of the result declined ("no machine representation").
            let any_qty = matches!(a, Ty::Qty { .. }) || matches!(b, Ty::Qty { .. });
            // A `+`/`-`/`*`/`/` over BIGINT operands is `BigInt` — the unbounded arithmetic, NOT the
            // fixed-width int scheme (whose `∀a. (Int a) → …` would reject a `BigInt` operand). `lower`
            // routes it to the runtime `bigint-*` op (or folds a constant). A `BigInt`/fixed mix is
            // rejected in `check_application` (CDZ0301), so if one operand is `BigInt` the well-typed
            // case has both — return `BigInt`. (Comparison over BigInt is `Bool`, via the generic path.)
            if matches!(
                prim,
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
                    | crate::resolved::Prim::Rem
            ) && !any_qty
                && (matches!(a, Ty::BigInt) || matches!(b, Ty::BigInt))
            {
                return Ty::BigInt;
            }
            // A `+`/`-`/`*`/`/` over RATIONAL operands is `Rational` — the exact arithmetic, NOT the
            // fixed-width int scheme (whose `∀a. (Int a) → …` rejects a `Rational` operand). `lower` folds
            // a constant pair (normalized over `IntValue` bignum) or (later) emits the runtime op. A
            // `Rational`/integer mix is rejected in `check_application` (CDZ0301), so if one operand is
            // `Rational` the well-typed case has both — return `Rational`. (`%` is NOT a rational op —
            // exact division is total, there is no remainder; a `%` over rationals falls through and its
            // scheme-unify rejects it. Comparison over Rational is `Bool`, via the generic path.)
            if matches!(
                prim,
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            ) && !any_qty
                && (matches!(a, Ty::Rational) || matches!(b, Ty::Rational))
            {
                return Ty::Rational;
            }
            // A `+`/`-`/`*`/`/` over FLOAT operands is that float type — the SAME arithmetic operator as
            // the integer case, dispatched here on a `Ty::Float` operand (there is no distinct `+.`). Its
            // `∀a. (Int a) → …` scheme does NOT accept a `Float`, so the generic scheme-unify would reject
            // it; type it directly instead, and `lower` remaps to `Prim::FAdd`… + `lower_float_arith`. A
            // `Float`/integer mix is rejected in `check_application` (CDZ0301), so if one operand is
            // `Float` the well-typed case has both — return the float type (the concrete-width operand, so
            // a `Float32` op stays `Float32`; a deferred literal grounds to `Float64`). Comparison over
            // floats is `Bool` via the generic path; `%`/bitwise/shift have no float form and fall through
            // to the scheme (which rejects a float operand — those stay integer-only).
            if matches!(
                prim,
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            ) && !any_qty
                && (matches!(a, Ty::Float(_)) || matches!(b, Ty::Float(_)))
            {
                // Prefer whichever operand fixed the width (a concrete `Float32`/`Float64` over a deferred
                // literal), mirroring the `join` width preference — so `(+ x 1.0)` with `x : Float32`
                // stays `Float32` rather than the literal's deferred width.
                return match (&a, &b) {
                    (Ty::Float(fa), _) if fa.width_is_fixed() => a.clone(),
                    (_, Ty::Float(fb)) if fb.width_is_fixed() => b.clone(),
                    (Ty::Float(_), _) => a.clone(),
                    (_, Ty::Float(_)) => b.clone(),
                    _ => Ty::float(),
                };
            }
            let a_qty = matches!(a, Ty::Qty { .. });
            let b_qty = matches!(b, Ty::Qty { .. });
            if a_qty || b_qty {
                match prim {
                    crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Rem => {
                        // `+`/`-`/`%` on same-dimension quantities KEEP the shared unit (a remainder is
                        // same-in/same-out like addition: `7m % 3m = 1m`), unlike `*`/`/` which compose
                        // units. The result unit: when both operands are at the SAME unit (scale), keep it (the
                        // common Layer-1 case — no conversion); when they share a dimension but DIFFER in
                        // scale (`meter` + `kilometer`), the result is the dimension's REFERENCE unit (the
                        // deterministic common unit each operand converts to — units-of-measure.md
                        // §Combining Units Of One Dimension Is Well-Formed). The inner numeric type is the
                        // lhs quantity's (both share it; the fault check enforces agreement). The choice is
                        // a pure function of the two operand units (equal → that unit; differ → the
                        // reference), independent of evaluation order, so the result is reproducible.
                        //= spec/capabilities/units-of-measure.md#combining-units-of-one-dimension-is-well-formed
                        //# The result unit of a combination of same-dimension quantities MUST be a deterministic function of the operands' units, so that the result is reproducible rather than dependent on evaluation order.
                        if let (
                            Ty::Qty {
                                inner: ia,
                                unit: ua,
                            },
                            Ty::Qty { unit: ub, .. },
                        ) = (&a, &b)
                        {
                            let unit = if ua == ub {
                                ua.clone()
                            } else {
                                ua.at_reference()
                            };
                            return Ty::Qty {
                                inner: Box::new((**ia).clone()),
                                unit,
                            };
                        }
                        if let Ty::Qty { inner, unit } = &a {
                            return Ty::Qty {
                                inner: Box::new((**inner).clone()),
                                unit: unit.clone(),
                            };
                        }
                        return b;
                    }
                    crate::resolved::Prim::Mul | crate::resolved::Prim::Div => {
                        // Compose the units. A bare-number operand contributes `Unit.one` (scaling keeps
                        // the other's dimension — `(* (Qty 2 meter) 3)` stays meter).
                        let ua = match &a {
                            Ty::Qty { unit, .. } => unit.clone(),
                            _ => crate::ty::Unit::one(),
                        };
                        let ub = match &b {
                            Ty::Qty { unit, .. } => unit.clone(),
                            _ => crate::ty::Unit::one(),
                        };
                        let unit = if matches!(prim, crate::resolved::Prim::Mul) {
                            ua.mul(&ub)
                        } else {
                            ua.div(&ub)
                        };
                        // The inner numeric type is a quantity operand's inner (both share it).
                        let inner = match (&a, &b) {
                            (Ty::Qty { inner, .. }, _) | (_, Ty::Qty { inner, .. }) => {
                                (**inner).clone()
                            }
                            _ => Ty::Any,
                        };
                        return Ty::Qty {
                            inner: Box::new(inner),
                            unit,
                        };
                    }
                    crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq => return Ty::Bool,
                    _ => {}
                }
            }
        }
    }
    // A compound-VALUE constructor (the `tuple`/`record`/`list` alias) applied — its type is the compound
    // of the argument types, even at ZERO arguments (an empty `(list)` / `(tuple)` is a valid empty
    // compound, NOT the ctor record). This is checked BEFORE the zero-arg identity short-circuit below so
    // `(list)` types as `List Any` rather than as the `list` ctor record's type. (The TupleNew/RecordNew/
    // ListNew arms are the ones that can be nullary; a scheme-typed head falls through.)
    if let Some(
        prim @ (crate::resolved::Prim::TupleNew
        | crate::resolved::Prim::RecordNew
        | crate::resolved::Prim::ListNew),
    ) = crate::eval::meta_apply_of(db, head)
    {
        return compound_ctor_type(db, prim, args);
    }
    // The COMPILER-INTERNAL `ast-splice-lift` intrinsic (`(intrinsic "ast-splice-lift") args`) — its head
    // resolves DIRECTLY to a prim (no `(meta apply)` module record), so read it via `prim_of`. Result is
    // `(List Ast)` (`compound_ctor_type`'s `AstSpliceLift` arm). Only the quasiquote-splice desugar emits
    // it, never user surface.
    if crate::eval::prim_of(db, head) == Some(crate::resolved::Prim::AstSpliceLift) {
        return compound_ctor_type(db, crate::resolved::Prim::AstSpliceLift, args);
    }
    // The COMPILER-INTERNAL `ast-lift` intrinsic (`(intrinsic "ast-lift") e`) — same direct-prim head as
    // `ast-splice-lift`. Result is `Ast` (`compound_ctor_type`'s `AstLift` arm) whatever the operand's
    // type. Only the quasiquote desugar emits it around a runtime active-unquote operand.
    if crate::eval::prim_of(db, head) == Some(crate::resolved::Prim::AstLift) {
        return compound_ctor_type(db, crate::resolved::Prim::AstLift, args);
    }
    // The `map` VALUE-constructor alias applied — `(map (k v) …)` written as a bare NAME head. Its `args`
    // are the ENTRY-PAIR nodes (each a two-element `(key value)` list), NOT curried arguments — so type
    // it as `Map <join keys> <join values>` DIRECTLY (like the `list` alias types `List <join elems>`),
    // never peeling the pairs as a curried application (which would wrongly type `(a 1)` as applying `a`).
    // An empty `(map)` is `Map Any Any`. Homogeneity is `type_errors`' job; this fills the value column.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapNew) {
        let mut key_ty = Ty::Any;
        let mut val_ty = Ty::Any;
        for &entry in args {
            // Read the entry's RAW `(key value)` children — a map entry is structure, not an application
            // (`(a 1)` is the pair a↦1, not `a` applied to `1`), so read the AST list directly.
            if let crate::ast::Struct::List(items) = db.ast.get(entry)
                && items.len() == 2
            {
                let (k, v) = (items[0], items[1]);
                key_ty = key_ty.join(&type_of(db, k));
                val_ty = val_ty.join(&type_of(db, v));
            }
            // A malformed entry (not a 2-element list) — the fault is reported elsewhere; the map's
            // key/value stay whatever the well-formed entries determined.
        }
        return Ty::Map(Box::new(key_ty), Box::new(val_ty));
    }
    // A NULLARY PERFORM `(E.op)` — an effect operation applied to no argument. Its `(meta t)` scheme is
    // `(-> Unit result)` (a `Unit`-domain op whose unit argument is elided in the corpus surface), so the
    // performance's type is `result`, NOT the op record's type (which the zero-arg identity short-circuit
    // below would wrongly return). Checked before that short-circuit, only for an effect operation, so an
    // ordinary nullary def `(g)` is unaffected. (A non-nullary op reaches the scheme-peeling loop below.)
    if crate::eval::effect_op_of(db, head).is_some() {
        // A REIFIED ASYNC WORLD-EFFECT perform in a reducer-world compile types as the effect-request
        // RECORD (schema-hash phase-1a), NOT the op-sig result. The discriminator (agreed with
        // v-rust-backend, matching its reify fork in `lower`): in a reducer-world compile
        // (`db.wit_world.is_some()`), a HOST-DELEGATED perform (`perform_host_target` = Some — its
        // entrypoint `(host …)` block is its CDZ0401 home) whose effect is NOT a declared world IMPORT
        // (`!is_world_import_op`) is an ASYNC world-effect returned in the effect-list → it reifies. A
        // world-IMPORT op (`is_world_import_op` TRUE — e.g. `kv.get`/`kv.delete`, the sole `kv` import) is
        // SYNCHRONOUS: its result is consumed inline, so it keeps its op-sig result type (the fall-through
        // arms below) — reifying it would break the host-fused kv reducers. Polarity is REIFY-WHEN-FALSE:
        // the world declares only `kv` as an import, so `Model.request`/`Emit.send`/`Tool.run` are
        // `is_world_import_op` FALSE and reify, while `kv.*` is TRUE and stays synchronous. Generic-
        // compiler-clean: reads the declared world imports, zero hard-coded capability vocabulary. A
        // NON-reducer compile (`wit_world` None) or a homeless perform (`perform_host_target` None → the
        // CDZ0401 no-home decline) never reaches the record type.
        if db.wit_world.is_some()
            && let Some((effect, op, _result)) = crate::effects::perform_host_target(db, head, head)
            && !crate::wit_world::is_world_import_op(db.wit_world.as_deref(), &effect, &op)
        {
            // has_target mirrors the reify fork in `lower`: a target-having effect (its op carries an
            // `@resource` marker) reifies the dest to a `target` field, so the record type gains it (ruling A).
            let effect_decl = crate::eval::effect_op_of(db, head).map(|(decl, _)| decl);
            let has_target = effect_decl
                .and_then(|decl| {
                    let op_idx = crate::eval::effect_op_of(db, head).map(|(_, i)| i)?;
                    db.effect_decl_by_occ(decl)
                        .and_then(|e| e.ops.get(op_idx as usize))
                        .and_then(|o| o.resource)
                })
                .is_some();
            // has_descriptor mirrors the reify EMIT: the record type gains `schema_descriptor: Bytes` iff the
            // reify emits it — i.e. iff the effect's descriptor builds. Computed via the SAME
            // `lower::effect_has_schema_descriptor` the emit gates on, so the typed shape can't drift from the
            // emitted shape (the phase-3 bug: emit 4-field, type 3-field → dropped field → schema_hash None).
            let has_descriptor = effect_decl
                .is_some_and(|decl| crate::lower::effect_has_schema_descriptor(db, decl));
            if let Some(record) = world_effect_request_ty(db, has_target, has_descriptor) {
                return record;
            }
        }
        let mut fresh = Fresh::new();
        if let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh) {
            let cur = crate::unify::instantiate(&scheme, &mut fresh);
            if let Ty::Fn(param, result) = &cur
                && (args.is_empty() && matches!(**param, Ty::Unit))
            {
                return (**result).clone();
            }
        }
    }
    // A ZERO-ARGUMENT application `(g)` with a non-lambda head is the head value — applying to no
    // arguments is the identity (a nullary def `(def (g) 7)` called). Its type is the head's type.
    // Mirrors the same short-circuit in `lower`, so `(g)` types and lowers as its body value. A
    // RECURSIVE nullary call (`(def (f) (f))`) has no normal form — type it `Any` (the fault is
    // reported by the collection side) rather than recursing into its own body without end.
    if args.is_empty() {
        if let Some(body) = crate::eval::lambda_body_of_nullary(db, head)
            && crate::eval::is_recursive(db, body)
        {
            return Ty::Any;
        }
        return type_of(db, head);
    }
    // A NULLARY variant CONSTRUCTOR applied to the unit value — `(None unit)` / `(Nil ())` — constructs
    // the sum (core-semantics.md §Construction MUST Be Via Application). Its ctor `(meta t)` is the bare
    // sum (no arrow — `variant_payload_type` is `None`), so the "peel a curried parameter per arg" loop
    // below would find a non-function and yield `Any`. The application's type IS the sum (the ctor's own
    // `(meta t)`); the unit argument is the payload, not a curried parameter. This is what lets a
    // `(match (None unit) …)` scrutinee type as the sum and route to the sum matcher.
    if crate::eval::variant_disc_of(db, head).is_some()
        && crate::eval::variant_payload_type(db, head).is_none()
    {
        return type_of(db, head);
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No `(meta t)` scheme. A RUNTIME FUNCTION VALUE head whose type is directly a `Ty::Fn` — a
        // closure bound out of a compound by a match binder (`(match t ((T.Mk f) (f 5)))`, where `f`
        // reads the `T.Mk` payload arrow), or a function-typed parameter — has no prelude scheme (a
        // `SumPayload`/`Proj`/`Param` binder is not a prelude entry), but its OWN type carries the arrow.
        // Peel one arrow per argument to get the result: `f : (-> Int64 Int64)` applied to one arg is
        // `Int64`. Without this the application typed `Any`, leaving the enclosing function's return type
        // non-machine ("function return type has no machine representation"). A prelude sum like `Some`
        // resolved WITHOUT this because its ctor scheme threads the payload type; a USER sum's
        // payload-bound closure relies on this arm. (Applied to more args than the arrow takes stops at
        // the non-function tail — a fault reported elsewhere.)
        None => {
            let head_ty = type_of(db, head);
            if matches!(head_ty, Ty::Fn(_, _)) {
                let mut cur = head_ty;
                for _ in args {
                    match cur {
                        Ty::Fn(_, result) => cur = *result,
                        _ => break,
                    }
                }
                return cur;
            }
            trace!(target: "rcdzc::infer", head = head.0, "apply: head has no (meta t) scheme → Any");
            return Ty::Any;
        }
    };
    trace!(target: "rcdzc::infer", head = head.0, scheme = %scheme.ty.render_name(&db.name_ctx()), args = args.len(), "apply: instantiate head scheme");
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    for &arg in args {
        // Peel one curried parameter: `cur` must be a function type; unify the arg into its parameter.
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                // FRESHEN the argument's free variables past the head's instantiation counter before
                // unifying: `type_of` types the arg with its OWN private `Fresh` (from 0), so an
                // under-constrained arg (a bare nullary variant `(None) : Option ?0`) shares variable
                // numbers with the head's instantiation (`Some : (-> ?0 (Option ?0))`). Without freshening
                // the two `?0`s alias and `?0 = Option ?0` trips the occurs-check, spuriously rejecting a
                // well-typed `(Some (None))`. Freshening makes them disjoint (`?0 = Option ?1`).
                let arg_ty = type_of(db, arg);
                let at = freshen_arg(db, &arg_ty, &mut fresh);
                // A unify failure here is a real type fault; ignore it for the VALUE (reported by
                // `type_errors`) and continue with the declared result so the shape stays sane.
                let _ = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx());
                cur = *result;
            }
            // Applied to more args than it takes — not a function; the fault is reported elsewhere.
            _ => return Ty::Any,
        }
    }
    // A NULLARY PERFORM: an effect operation declared `(-> Unit T)` performed as `(E.op)` (no argument —
    // the `unit` domain is elided, the corpus surface). After the given args are peeled, if the head is
    // an effect operation and `cur` is still `(-> Unit result)`, the elided unit is the implicit argument
    // — so the performance's type is `result`, not the un-applied arrow. Without this a nullary perform
    // types as its arrow (a `Type`), and using it as a value (`(+ (E.op) 1)`) faults "unify Int64 with
    // Type". (Only for an effect op — an ordinary partial application keeps its arrow type.)
    let applied = subst.apply(&cur);
    if crate::eval::effect_op_of(db, head).is_some()
        && let Ty::Fn(param, result) = &applied
        && matches!(**param, Ty::Unit)
    {
        return subst.apply(result);
    }
    // `Value.decode : ∀a. Bytes → (Option a)` — its result `a` is UNCONSTRAINED by the argument (a
    // `Bytes`), so per-node bottom-up `type_of` leaves the application `(Option ?a)` with `?a` free. The
    // target `a` is fixed by the CALL-SITE EXPECTED TYPE — an enclosing `(: (Value.decode …) (Option T))`
    // annotation. But an annotation is the node's PARENT: its `type_of` arm unifies `annot_ty` with this
    // node's type in a LOCAL subst and returns the grounded type for the ANNOTATION node — it does NOT
    // thread the `a := T` binding back into THIS node's memoized `type_of`. `lower_value_decode` reads
    // `type_of(decode-node)` to build the shape descriptor, so an ungrounded `?a` there DECLINES
    // ("target type is unsolved") even though the annotation names it. So when this application is
    // `Value.decode` and its result is still `(Option <free>)`, CLIMB to the enclosing annotation (the same
    // parent-context grounding `literal_binop_context_ty` does for a deferred integer width) and unify the
    // annotation's type into the result. Keyed on the PRIM, not a name — generic-compiler-clean; no other
    // op needs this (an arg-grounded result is already concrete). An UNANNOTATED decode stays `(Option ?a)`
    // and declines at lower (the honest needs-annotation contract).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::ValueDecode)
        && matches!(&applied, Ty::Sum { args, .. } if args.first().is_some_and(has_free_var))
        && let Some(expected) = annotation_context_ty(db, head)
    {
        let mut gsubst = Subst::new();
        if crate::unify::unify(&mut gsubst, &applied, &expected, &db.name_ctx()).is_ok() {
            return gsubst.apply(&applied);
        }
    }
    applied
}

/// Whether `ty` contains an unsolved type VARIABLE anywhere (a free `Ty::Var`). Used to detect a
/// `Value.decode` result `(Option ?a)` whose target is not yet grounded, so `type_of` climbs to the
/// annotation to fix it before `lower_value_decode` reads the node's type for its descriptor.
fn has_free_var(ty: &Ty) -> bool {
    match ty {
        Ty::Var(_) => true,
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => args.iter().any(has_free_var),
        Ty::List(e) => has_free_var(e),
        Ty::Tuple(es) => es.iter().any(has_free_var),
        Ty::Map(k, v) => has_free_var(k) || has_free_var(v),
        Ty::Record(fs) => fs.values().any(has_free_var),
        _ => false,
    }
}

/// The type an enclosing `(: <this> <Type>)` annotation grounds `id` to — climb PARENTS to the nearest
/// `Resolved::Annot` whose annotated `expr` is on the path from `id`, and reduce its `ty_expr` to a type
/// value. `None` if no annotation encloses `id` (then a `Value.decode` there stays ungrounded and declines
/// at lower — the honest needs-annotation contract). Mirrors `literal_binop_context_ty`'s parent-climb for
/// a deferred integer width — the shared "consult the context because per-node `type_of` can't thread it
/// back" path. Starts at `head` (the decode op node), whose first parent is the application node, whose
/// parent is the annotation; a non-annotation parent chain simply finds no annotation and returns `None`.
fn annotation_context_ty(db: &mut Db, id: StructId) -> Option<crate::ty::Ty> {
    let mut child = id;
    loop {
        let parent = db.parent_of(child)?;
        if let Resolved::Annot { expr, ty_expr } = resolved_of(db, parent) {
            // Only the annotation whose annotated expression is (an ancestor-or-self on the path from) THIS
            // node grounds it — `(: (Value.decode bs) (Option T))` has `expr` == the decode application node.
            if expr == child {
                return crate::eval::typeval_of(db, ty_expr);
            }
            return None;
        }
        // An annotated let-binder `((: <pat> T) <this>)` grounds its initializer exactly as a direct
        // `(: <this> T)` annotation does — the author named the type either way, so the idiomatic
        // `(let (((: p (Option T)) (Value.decode bs))) …)` must ground the decode identically to
        // `(: (Value.decode bs) (Option T))`. When `parent` is the binding PAIR whose SECOND element is
        // this node, the declared type on its `(: <pat> T)` LHS is the grounding type.
        if let Some(ty) = let_binder_annotation_ty(db, parent, child) {
            return Some(ty);
        }
        child = parent;
    }
}

/// The declared type of an annotated let-binder whose INITIALIZER is `init` — i.e. `pair` is the binding
/// pair `((: <pat> T) init)` (a two-element list whose second element is `init`) sitting in a `let`'s
/// bindings-list, with an `(: <pat> T)` LHS that reduces to a type value `T`. `None` for a bare-name
/// binding, a non-annotated LHS, or a `pair` that isn't a let binding at all. Shares the shape guard with
/// `annotated_let_binder_ty` (pair-in-bindings-list, `:`-form LHS), but grounds UNCONDITIONALLY — there is
/// no "initializer inferred type" to contradict here (the initializer is a `Value.decode` whose target is
/// still a free var, which is exactly what needs grounding), so the disagreement check that helper makes
/// (to suppress a contradictory body-use cascade) does not apply.
fn let_binder_annotation_ty(db: &mut Db, pair: StructId, init: StructId) -> Option<crate::ty::Ty> {
    let kv = match db.ast.get(pair) {
        crate::ast::Struct::List(kv) if kv.len() == 2 && kv[1] == init => kv.clone(),
        _ => return None,
    };
    let bindings_occ = db.parent_of(pair)?;
    crate::resolve::let_of_bindings_list(db, bindings_occ)?;
    let ann = db.ast.as_form(kv[0], ":").map(<[_]>::to_vec)?;
    if ann.len() != 2 {
        return None;
    }
    crate::eval::typeval_of(db, ann[1])
}

/// Apply a def SCHEME to `args`, returning the result type — instantiate it with fresh variables, then
/// peel one curried parameter per argument (unifying the arg's type into the parameter). The result is
/// the instantiated return type after substitution. Used to type a RECURSIVE call by its callee's
/// signature (which β-reduction can't type). Mirrors the operator-scheme application in `apply_type`.
fn apply_scheme_to_args(db: &mut Db, scheme: &Scheme, args: &[StructId]) -> Ty {
    let generic = !scheme.ty_vars.is_empty();
    // Type each argument up front so a GENERIC scheme can seed its instantiation counter PAST every
    // variable the args carry (below). A MONOMORPHIC scheme (no `ty_vars`) is unaffected by this.
    let arg_tys: Vec<Ty> = args.iter().map(|&a| type_of(db, a)).collect();

    // For a GENERIC scheme (recursive-generic monomorphization), the connection between the callee's
    // result variable and its parameter variable MUST survive so a caller that THREADS its own generic
    // param through this call (`(def (wrap m y) … (idr 2 y))` — `wrap`'s result is `idr`'s result is
    // `y`'s type) gets a result type equal to that param var, not a disconnected fresh one. The old path
    // `freshen_free`s each arg to dodge an occurs-check collision between the arg's from-0 vars and the
    // scheme's from-0 instantiation — but freshening a bare param-var arg is exactly what SEVERS the
    // connection (a three-level generic chain then decoupled result from param → "looped function result
    // has no machine rep"). Instead, seed the scheme's instantiation counter ABOVE every ty-var the args
    // mention, so the scheme's fresh vars cannot collide with an arg's var and NO freshening is needed —
    // the arg's canonical param var flows through the unify untouched. A generic def scheme has EMPTY
    // width/sign var lists (`compute_def_scheme`), so only ty-vars can collide, and `collect_free_vars`
    // finds them. A MONOMORPHIC scheme keeps the exact old freshen path (byte-identical).
    let mut fresh = Fresh::new();
    if generic {
        let mut arg_vars = Vec::new();
        for t in &arg_tys {
            t.collect_free_vars(&mut arg_vars);
        }
        if let Some(&max) = arg_vars.iter().max() {
            fresh.reserve(max + 1); // instantiate above every arg var → no collision without freshening
        }
    }
    let mut cur = crate::unify::instantiate(scheme, &mut fresh);
    let mut subst = Subst::new();
    for (i, at_raw) in arg_tys.into_iter().enumerate() {
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                // A MONOMORPHIC scheme freshens the arg (dodges the from-0 occurs-check collision, see
                // `apply_type`); a GENERIC scheme skips it (the seeded counter already avoids collision)
                // so a threaded param var stays connected to the callee's result var.
                let at = if generic {
                    at_raw
                } else {
                    crate::unify::freshen_free(&at_raw, &mut fresh)
                };
                // A CLOSURE argument — `(fn (s) s)` identity, `(fn (x) (tuple x x))` aggregate — types
                // bottom-up with an `Any` at every unannotated-param position (`(-> Any Any)`, `(-> Any
                // (Tuple Any Any))`). The DOMAIN is pinned by a sibling arg (`gmap`'s `it` element), and
                // once it is concrete we RE-SOLVE the closure body UNDER that domain
                // (`solved_lambda_arrow_under`) to recover the closure's fully-CONCRETE arrow — tying the
                // callee's result var (`gmap`'s `b`, hence its `Iter b` result) to the closure's real
                // result. Unify THAT recovered arrow, NOT the bottom-up `at`:
                //
                // Unifying `at` first POISONS `b` for an AGGREGATE result. A SCALAR-`Any` result
                // (`(-> Any Any)`) leaves `b` free — `unify(?b, Any)` hits the `Any`-poison arm
                // (`unify.rs`), no bind — so the recovery below could refine it. But a COMPOUND result
                // (`(Tuple Any Any)`) is NOT bare `Any`: `unify(?b, (Tuple Any Any))` binds `?b :=
                // (Tuple Any Any)`, and then the recovery's re-unify against `(Tuple Int64 Int64)` cannot
                // refine the inner `Any`s (Any-absorbs). So the OUTER call node types `(GIter (Tuple Any
                // Any))`, the tuple elements ground to `Unit`, and a consumer specialized off THIS node
                // type (`count(gmap …)`) takes `GIter<((),())>` while `gmap` itself specializes
                // `GIter<(i64,i64)>` → rust E0308 (wasm erases the element, so it is a rust-visible
                // miscompile). Unifying the recovered CONCRETE arrow instead binds `b` to `(Tuple Int64
                // Int64)` directly. The recovered arrow's domain equals the pinned domain `at` would
                // supply, so skipping `at` here loses nothing. Falls back to `at` when recovery can't fire
                // (domain not yet concrete, non-lambda arg, or an unrecoverable body) — byte-identical to
                // the prior path for every non-recovered shape.
                let mut tied_via_recovery = false;
                if at.has_any()
                    && let Some(node) = args.get(i).copied()
                    && let (Some(lam_params), Some(lam_body)) = (
                        crate::eval::lambda_params_of(db, node),
                        crate::eval::lambda_body(db, node),
                    )
                {
                    let pinned = subst.apply(&param);
                    if let Ty::Fn(dom, _) = &pinned
                        && !matches!(**dom, Ty::Any)
                        && !dom.has_free_var()
                        && let Some(solved) =
                            solved_lambda_arrow_under(db, &lam_params, lam_body, &pinned)
                        && !solved.has_any()
                    {
                        let _ = crate::unify::unify(&mut subst, &param, &solved, &db.name_ctx());
                        tied_via_recovery = true;
                    }
                }
                if !tied_via_recovery {
                    let _ = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx());
                }
                cur = *result;
            }
            _ => return Ty::Any,
        }
    }
    subst.apply(&cur)
}

/// The instantiated PARAMETER-ARROW type at position `pos` when `callee` is applied to `args` — the arrow
/// the callee's scheme requires of the argument at `pos`, with the OTHER (determined) sibling arguments
/// unified in so a scheme type variable shared across parameters is pinned. Used by `type_specialize` to
/// recover a bare CLOSURE argument's expected arrow at a call site: a recursive-generic transformer
/// `gmap : (Iter a) → (a → b) → (Iter b)` applied to a concrete `it : Iter String` pins `f`'s expected
/// arrow to `(-> String ?b)`, so an IDENTITY closure `(fn (s) s)` — whose body cannot type `s` bottom-up —
/// solves `s : String` from that domain and its identity body then determines `b = String`. The closure's
/// OWN argument (at `pos`) is NOT unified in (its `(-> Any …)` type would re-introduce the hole the
/// recovery exists to fill); every other DETERMINED argument is. Returns `None` when `callee` has no
/// scheme, the arrow runs out before `pos`, or the recovered slot is not itself an arrow (nothing to
/// recover). The domain MAY be concrete while the result stays a free var (the closure body solves it).
pub(crate) fn instantiated_param_arrow(
    db: &mut Db,
    callee: usize,
    args: &[StructId],
    pos: usize,
) -> Option<Ty> {
    let scheme = def_scheme(db, callee)?;
    let arg_tys: Vec<Ty> = args.iter().map(|&a| type_of(db, a)).collect();
    let generic = !scheme.ty_vars.is_empty();
    let mut fresh = Fresh::new();
    if generic {
        let mut arg_vars = Vec::new();
        for t in &arg_tys {
            t.collect_free_vars(&mut arg_vars);
        }
        if let Some(&max) = arg_vars.iter().max() {
            fresh.reserve(max + 1);
        }
    }
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    let mut recovered: Option<Ty> = None;
    for (i, at_raw) in arg_tys.into_iter().enumerate() {
        let applied = subst.apply(&cur);
        let Ty::Fn(param, result) = applied else {
            break;
        };
        if i == pos {
            // The closure's own slot — capture the expected arrow AFTER the sibling unifications, but do
            // NOT unify the closure's bare `(-> Any …)` type in (it would re-introduce the hole).
            recovered = Some(*param);
        } else {
            let at = if generic {
                at_raw
            } else {
                crate::unify::freshen_free(&at_raw, &mut fresh)
            };
            // Only a DETERMINED sibling pins the scheme's shared vars; an undetermined one adds nothing.
            if !matches!(at, Ty::Any) && !at.has_free_var() {
                let _ = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx());
            }
        }
        cur = *result;
    }
    let arrow = subst.apply(&recovered?);
    matches!(arrow, Ty::Fn(_, _)).then_some(arrow)
}

/// The fully-solved arrow of a bare lambda `(params, body)` when its parameters' EXPECTED types are known
/// from the call context (the `expected` arrow, e.g. `(-> String ?b)` recovered by
/// [`instantiated_param_arrow`]). Binds each param to the corresponding expected DOMAIN before typing the
/// body, so a pass-through body (`(fn (s) s)` — identity) whose result the bottom-up solve cannot pin
/// takes its result FROM the domain: `s : String` ⇒ body `s : String` ⇒ arrow `(-> String String)`. For a
/// param the expected arrow does not determine (a hole at that slot), fall back to the body-only solve
/// (`solve_lambda_param_ty`), so a genuinely-unconstrained param still declines rather than inventing a
/// type. `None` if the counts do not line up as an arrow. This is the closure-body result←domain flow the
/// plain `solved_lambda_arrow` omits (it solves each param from the body ALONE).
pub fn solved_lambda_arrow_under(
    db: &mut Db,
    params: &[StructId],
    body: StructId,
    expected: &Ty,
) -> Option<Ty> {
    // Peel the expected arrow into per-parameter expected domains (as many as it provides).
    let mut expected_doms: Vec<Ty> = Vec::new();
    let mut cur = expected.clone();
    for _ in 0..params.len() {
        match cur {
            Ty::Fn(p, r) => {
                expected_doms.push(*p);
                cur = *r;
            }
            _ => break,
        }
    }
    // Bind each param to its expected domain (when concrete) in a local subst, so typing the body reads
    // the param at that type. A param whose expected domain is a hole is left to the body-only solve.
    let mut subst = Subst::new();
    // ALSO seed each param's concrete expected domain into `db.param_types` (save + restore). The var-subst
    // above only rewrites `Ty::Var`s, but a BARE lambda param types as `Ty::Any` (not a var — `type_of`'s
    // Lambda arm), which `subst.apply` cannot touch. So an AGGREGATE body over the param — `(fn (x) (tuple
    // x x))`, `(fn (x) (Cons x Nil))` — types its element positions at `Any` and stays `(Tuple Any Any)`
    // even after the subst, so the recovered arrow keeps an `Any` hole and `type_specialize` DECLINES (the
    // gmap `int→tuple` + `string→string` CDZ0201 — `cdz check` green, `cdz test` red). Seeding
    // `db.param_types[x] = Int64` makes `type_of(x)` return the domain DIRECTLY, so the tuple types `(Tuple
    // Int64 Int64)`. Restored below so the transient seeding never leaks into another def's solve.
    let mut seeded: Vec<(StructId, Option<Ty>)> = Vec::new();
    for (i, &p) in params.iter().enumerate() {
        let occ = crate::eval::param_name_occ(db, p);
        if let Some(dom) = expected_doms.get(i)
            && !matches!(dom, Ty::Any)
            && !dom.has_free_var()
        {
            let pv = type_of(db, occ);
            // Unify the param's own (var/Any) type with the expected domain, so a body reference to it
            // reads the concrete domain through the subst.
            let _ = crate::unify::unify(&mut subst, &pv, dom, &db.name_ctx());
            // Seed the concrete domain so an aggregate body reads the param's type directly (not `Any`).
            // Only when the param is currently a HOLE (`Any`/var) — an already-concrete param is untouched.
            if matches!(pv, Ty::Any) || pv.has_free_var() {
                seeded.push((occ, db.param_types.insert(occ, dom.clone())));
            }
        }
    }
    // Type the body, then read the result + each param through the subst (a pass-through body's result is
    // now the bound domain). Curry right-to-left, exactly like `solved_lambda_arrow`.
    let result = subst.apply(&type_of(db, body));
    for (occ, prev) in seeded {
        match prev {
            Some(t) => db.param_types.insert(occ, t),
            None => db.param_types.remove(&occ),
        };
    }
    Some(
        params
            .iter()
            .enumerate()
            .rev()
            .fold(result, |acc, (i, &p)| {
                let occ = crate::eval::param_name_occ(db, p);
                let pt = match expected_doms.get(i) {
                    Some(dom) if !matches!(dom, Ty::Any) && !dom.has_free_var() => dom.clone(),
                    _ => match type_of(db, occ) {
                        Ty::Any => solve_lambda_param_ty(db, occ, body),
                        t => t,
                    },
                };
                Ty::Fn(Box::new(pt), Box::new(acc))
            }),
    )
}

/// Which built-in fallible sum a type is, for the `?`/`try` operator (`DESIGN-try-operator-rcdzc.md`).
/// `Option` short-circuits with `None`; `Result` short-circuits with `Err`. Identified STRUCTURALLY by
/// the declaration's variant NAMES (`Some`/`None`, `Ok`/`Err`) — never by the type's spelled name — so
/// the built-in prelude sums are recognized without a hard-coded name key (the no-keys-outside-the-prelude
/// rule; the same variant-name scan `option_discs`/`result_discs` do in `lower`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FallibleKind {
    /// A `(type Option (Some a) None)` — `?` unwraps `Some a`, short-circuits on `None`.
    Option,
    /// A `(type Result (Ok a) (Err b))` — `?` unwraps `Ok a`, short-circuits carrying `Err b`.
    Result,
}

/// Classify a solved type as a built-in fallible sum (`Option`/`Result`) for the `?` operator, returning
/// its kind together with its SUCCESS PAYLOAD type (the `a` a `?` yields: `Some`'s / `Ok`'s payload) — for
/// `Result` also the error type (`Err`'s payload). `None` when `ty` is not one of the two built-in
/// fallible sums (a non-sum, a user sum, or an `Option`/`Result` whose args are not yet solved to the
/// arity its variant set implies). Recognizes the sum by its declaration's variant names, then reads the
/// payload types off the positional `Ty::Sum::args` (params are in first-appearance order, so `Option`'s
/// `a` and `Result`'s `Ok`/`Err` payloads are `args[0]`/`args[1]`).
pub(crate) fn fallible_shape(db: &Db, ty: &Ty) -> Option<(FallibleKind, Ty, Option<Ty>)> {
    let Ty::Sum { decl, args, .. } = ty else {
        return None;
    };
    let decl_ref = db.type_decl_by_occ(*decl)?;
    let names: Vec<&str> = decl_ref.variants.iter().map(|v| v.name.as_str()).collect();
    let has = |n: &str| names.contains(&n);
    if names.len() == 2 && has("Some") && has("None") {
        let payload = args.first().cloned().unwrap_or(Ty::Any);
        Some((FallibleKind::Option, payload, None))
    } else if names.len() == 2 && has("Ok") && has("Err") {
        let ok = args.first().cloned().unwrap_or(Ty::Any);
        let err = args.get(1).cloned();
        Some((FallibleKind::Result, ok, err))
    } else {
        None
    }
}

/// The FALLIBLE BOUNDARY a `?`/`try` at `try_node` short-circuits to (`DESIGN-try-operator-rcdzc.md` §4
/// v1: the enclosing FUNCTION). Walks up the ancestor chain from `try_node` to the nearest enclosing
/// function BODY — a top-level/module/do-local def body (`def_index_by_body`) or a `(fn params body)`
/// lambda body (the ancestor that is child position 1 of a `fn` form) — and returns THAT BODY's type,
/// which is the boundary type `T_B` (Cadenza declares a return type by ASCRIBING the body `(: body T)`,
/// so the body's type IS the function's result type). `None` if the `?` is not inside any function body.
///
/// No circularity: `type_of(Try)` reads only its OPERAND's type (never the boundary), so demanding the
/// body's type from inside a `?` under it does not re-enter this `?`'s own type. The walk stops at the
/// FIRST enclosing function so a `?` in a helper targets the helper (Rust-identical), not an outer caller.
pub(crate) fn enclosing_boundary_ty(db: &mut Db, try_node: StructId) -> Option<Ty> {
    let mut cur = try_node;
    while let Some(parent) = db.parent_of(cur) {
        // `cur` is a def body (top-level, module-member, or do-local) — its type is the boundary.
        if db.def_index_by_body(cur).is_some() {
            return Some(type_of(db, cur));
        }
        // `cur` is a `(fn params body)` lambda's BODY — position 1 of a `fn` form. Its type is the
        // lambda's result, the boundary for a `?` in the lambda body.
        if db.ast.head_name(parent) == Some("fn")
            && let crate::ast::Struct::List(kids) = db.ast.get(parent)
            && kids.get(1) == Some(&cur)
        {
            return Some(type_of(db, cur));
        }
        cur = parent;
    }
    None
}

/// The PAYLOAD type of a variant `variant_head` at the scrutinee's instantiation `scrut_ty` — the type
/// a match-arm payload binder takes. The ctor's scheme is `∀params. payload… → Sum params`;
/// instantiate it, unify its RESULT (`Sum ?params`) against `scrut_ty` (a `Ty::Sum{args}` — the solved
/// scrutinee type) to solve the fresh params from the scrutinee's concrete args, then return the
/// (substituted) FIRST payload. So `(match (s : Option Int64) ((Some x) …))` reads `Some`'s payload as
/// `Int64`, not a free var. For a MONOMORPHIC ctor the scheme has no vars and the result unify is a
/// no-op, so the payload is the declared type directly. `None` if the ctor has no payload (a nullary
/// variant — no binding) or its scheme is not a single-payload arrow.
pub(crate) fn payload_ty_at_instantiation(
    db: &mut Db,
    variant_head: StructId,
    scrut_ty: &Ty,
) -> Option<Ty> {
    let mut fresh = Fresh::new();
    // SEED the instantiation counter PAST every free variable the SCRUTINEE type carries, so the ctor
    // scheme's fresh vars cannot COLLIDE with them. `scrut_ty` is not always ground: a recursive call's
    // result is typed by `apply_scheme_to_args`, which instantiates the callee's scheme from 0 — so a
    // `Result A ?0` scrutinee (an error slot no branch of the recursive callee fixes) carries `?0`. The
    // ctor scheme also instantiates from 0 (`Err : ∀a b. b → Result a b` → `?1 → Result ?0 ?1`), so its
    // `?0`/`?1` alias the scrutinee's `?0`: unifying `Result ?0 ?1` against `Result A ?0` first solves
    // `?0 = A`, then `?1 = ?0 = A` — binding `et` to `A` instead of the scrutinee's error var, which then
    // read the whole match as `Result A A` (a spurious CDZ0203 "if branches differ", and downstream a
    // backend "no local slot" decline). Reserving above the scrutinee's vars keeps the two var domains
    // DISJOINT, so `Err`'s payload solves to the scrutinee's own error var. A GROUND scrutinee has no free
    // var and reserves nothing (byte-identical to the old from-0 path). Mirrors `apply_scheme_to_args`,
    // which seeds the same way to keep a recursive-generic result var connected.
    {
        let mut scrut_vars = Vec::new();
        scrut_ty.collect_free_vars(&mut scrut_vars);
        if let Some(&max) = scrut_vars.iter().max() {
            fresh.reserve(max + 1);
        }
    }
    let scheme = crate::eval::scheme_of(db, variant_head, &mut fresh)?;
    let inst = crate::unify::instantiate(&scheme, &mut fresh);
    // The ctor type is a curried arrow `payload… → result`. Peel EVERY arrow: a single-payload variant
    // has one (`p → Sum`); a MULTI-payload variant curries (`p0 → p1 → Sum`) and its payloads are boxed
    // as ONE tuple handle (the same representation `(Cons (tuple p0 p1))` builds), so its bound payload
    // TYPE is the `Ty::Tuple(p0, p1, …)` that the `[Payload, Elem(i)]` access path indexes into.
    let mut payloads = Vec::new();
    let mut cur = inst;
    let result = loop {
        match cur {
            Ty::Fn(p, r) => {
                payloads.push(*p);
                cur = *r;
            }
            other => break other,
        }
    };
    if payloads.is_empty() {
        return None; // nullary (bare sum) — no payload to bind
    }
    // Solve the scheme's params from the scrutinee's concrete instantiation: unify the ctor's RESULT
    // (`Sum ?a`) against the scrutinee type (`Sum Int64`).
    let mut subst = Subst::new();
    let _ = crate::unify::unify(&mut subst, &result, scrut_ty, &db.name_ctx());
    let payload = if payloads.len() == 1 {
        payloads.pop().unwrap()
    } else {
        Ty::Tuple(payloads.into())
    };
    Some(subst.apply(&payload))
}

/// The integer TEXT a FLOAT LITERAL denotes when it is integer-valued AND fits the `expected` integer
/// type — the replacement for the mirror of the `of-int` coercion: an `Int`-expected position given a
/// float literal (`(+ 2 2.0)`). A reader-normalized `Decimal` is integer-valued exactly when its base-10
/// `exponent` is ≥ 0 (its significand carries no trailing zeros — `2.0` → `{[2], exp:0}`, `300.0` →
/// `{[3], exp:2}`, but `2.5` → `{[25], exp:-1}`). We reconstruct the integer as `significand · 10^exp`,
/// then REJECT it unless the value fits `expected`'s `(signed, width)` — so the suggested `2.0` → `2`
/// type-checks in ONE shot (the D7/D9 one-shot rule: never suggest a spelling that just cascades to the
/// next mismatch, here a CDZ0302 out-of-range). No prelude float→int conversion exists, so a NON-integer
/// float (`2.5`) or an out-of-range one gets `None` — an honest "no clean fix" rather than a wrong one
/// (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Returns the minus-signed
/// decimal text (`-0.0` → `0`, since the integer zero has no sign).
fn integer_text_of_float_literal(
    d: &crate::ast::Decimal,
    expected: crate::ty::IntTy,
) -> Option<String> {
    // Only a non-negative exponent is a whole number (a normalized `Decimal` has no trailing-zero
    // significand, so a fractional value keeps a negative exponent).
    if d.exponent < 0 {
        return None;
    }
    // Base-256 significand → base-10 digit string (Horner, little-endian digits printed reversed), the
    // same conversion `Decimal::to_f64_bits` performs. An empty significand is zero.
    let mut digits: Vec<u8> = vec![0];
    for &byte in &d.significand {
        let mut carry = byte as u32;
        for dig in digits.iter_mut() {
            let v = (*dig as u32) * 256 + carry;
            *dig = (v % 10) as u8;
            carry = v / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    let mut s: String = digits.iter().rev().map(|d| (b'0' + d) as char).collect();
    let trimmed = s.trim_start_matches('0');
    s = if trimmed.is_empty() {
        "0".to_string()
    } else {
        trimmed.to_string()
    };
    // Multiply by 10^exponent = append that many zeros (unless the value is zero).
    if s != "0" {
        for _ in 0..d.exponent {
            s.push('0');
        }
    }
    // Build the value to range-check it against the expected width. Parse the decimal digit string into a
    // big-endian magnitude (Horner: acc = acc*10 + digit), then apply the sign — the inverse of the digit
    // build above, reused so the width check sees the exact integer.
    let mut mag: Vec<u8> = Vec::new(); // little-endian base-256 during build
    for ch in s.bytes() {
        let mut carry = (ch - b'0') as u32;
        for byte in mag.iter_mut() {
            let v = (*byte as u32) * 10 + carry;
            *byte = (v & 0xff) as u8;
            carry = v >> 8;
        }
        while carry > 0 {
            mag.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    while mag.last() == Some(&0) {
        mag.pop();
    }
    mag.reverse();
    let negative = d.negative && !mag.is_empty(); // zero is never negative
    let value = crate::ast::IntValue {
        negative,
        magnitude: mag,
    };
    if !value.fits_width(expected.ground_signed(), expected.ground_width()) {
        return None;
    }
    Some(if negative { format!("-{s}") } else { s })
}

/// A homogeneity mismatch between an INTEGER LITERAL element and a FLOAT type has the SAME one-shot
/// repair the annotation site's `(: 3 Float64)` does: rewrite the integer literal `n` as a FLOAT
/// literal `n.0`, so a `(list 1 2.0)` / `(list 1.0 2)` unifies at the float type rather than staying a
/// no-silent-promotion reject. `elem`'s type is `elem_ty`; `other_ty` is the type it clashed with. The
/// fix applies exactly when the clashing pair is {integer literal, float}: `elem` is a bare integer
/// LITERAL (an atom Int leaf — a computed integer expression cannot be retyped in one edit) whose type
/// is `Ty::Int`, and `other_ty` is `Ty::Float`. Returns a `ReplaceNode` fix editing the literal token
/// (`1` → `1.0`), or `None` when the pair is not this shape. Mirrors the annotation-site literal-retype
/// (`Some((format!("{n}.0"), …))`) so the two sites suggest the identical rewrite.
fn float_literal_retype_fix(
    db: &mut Db,
    elem: StructId,
    elem_ty: &Ty,
    other_ty: &Ty,
) -> Option<crate::diag::Fix> {
    if !(matches!(elem_ty, Ty::Int(_)) && matches!(other_ty, Ty::Float(_))) {
        return None;
    }
    let crate::ast::Struct::Atom(lid) = db.ast.get(elem) else {
        return None;
    };
    let crate::ast::Leaf::Int { value, .. } = db.ast.leaf(*lid).clone() else {
        return None;
    };
    let n = value.to_i128()?;
    Some(crate::diag::Fix::replace_heuristic(elem, format!("{n}.0")))
}

/// The sole variant NAME of an ERASABLE SINGLE-VARIANT NEWTYPE `ty` (a `Ty::Nominal` whose declaration
/// has exactly one variant) — the constructor to UNWRAP it by, `(match v ((<Name> n) n))`. The INVERSE of
/// `wrap_variant_for`: where that names the ctor to WRAP a bare value into a newtype, this names the ctor
/// to UNWRAP a newtype back to its underlying value (the repair for a nominal-boundary comparison
/// `(= u 5)` — unwrap `u` to compare the `Int64` inside). `None` unless `ty` is a nominal whose decl has
/// EXACTLY ONE variant (a multi-variant sum has no single unwrap; a non-nominal has nothing to unwrap).
fn newtype_unwrap_variant(db: &mut Db, ty: &Ty) -> Option<String> {
    let decl = match ty {
        Ty::Nominal { decl, .. } => *decl,
        _ => return None,
    };
    let t = db.type_decl_by_occ(decl)?;
    match t.variants.as_slice() {
        [only] => Some(only.name.clone()),
        _ => None,
    }
}

/// If `expected` is a SUM type with a SINGLE-payload variant whose payload type (at `expected`'s
/// instantiation) agrees with `actual`, the variant's constructor NAME — the "try wrapping the
/// expression in `Some`" suggestion (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To
/// A Fix). Derives the name GENERICALLY from the sum's declaration (which variant's payload fits),
/// never a hard-coded `Some`/`Ok` (the no-keys-outside-the-prelude rule). `None` when `expected` is not
/// a sum, or no variant's single payload matches — so a wrap is offered only when it would actually
/// resolve the mismatch. When TWO variants could each wrap the value — `(Result Int64 Int64)` given an
/// `Int64`, where both `Ok` and `Err` fit — the choice is AMBIGUOUS and `None` is returned rather than
/// guess one; a forced (unique) match is required. Variants are scanned in declaration order → deterministic.
fn wrap_variant_for(db: &mut Db, expected: &Ty, actual: &Ty) -> Option<String> {
    // Both a boxed `Ty::Sum` and an erased single-variant `Ty::Nominal` newtype offer the wrap: `(: n
    // Box)` for `(type Box (Wrap Int64))` suggests `(Wrap n)` exactly as `(: n Option)` suggests
    // `(Some n)` — the ctor is derived from the declaration either way.
    let decl = nominal_or_sum_decl(expected)?;
    // Snapshot (name, ctor) pairs first — `payload_ty_at_instantiation` borrows `db` mutably, so we
    // cannot hold a `&TypeDecl` across the calls.
    let variants: Vec<(String, StructId)> = db
        .type_decl_by_occ(decl)?
        .variants
        .iter()
        .filter_map(|v| v.ctor.map(|c| (v.name.clone(), c)))
        .collect();
    let mut hit: Option<String> = None;
    for (name, ctor) in variants {
        // A single-payload variant whose payload agrees with `actual` → wrapping the value in it yields
        // the expected sum. (A multi-payload variant's payload is a tuple, which a bare value never
        // matches, so it is naturally excluded.)
        if let Some(payload) = payload_ty_at_instantiation(db, ctor, expected)
            && payload.agrees_with(actual)
        {
            if hit.is_some() {
                return None; // two variants could each wrap it — ambiguous, don't guess
            }
            hit = Some(name);
        }
    }
    hit
}

/// An actionable message TAIL (no fix) when the mismatch is the common "used an optional value directly"
/// shape: `actual` is `(Option T)` and `expected` is exactly its payload `T` — a fallible read (`List.at`,
/// `String.at`, `from-bytes`) whose optional result was used where the bare payload was wanted. An
/// `(Option T)` has NO total unwrap (it is eliminated only by matching its `None` case, which is the
/// author's decision), so there is no mechanical fix — but the diagnostic can still say HOW to fix it
/// rather than only naming two types. `None` unless `actual` is `(Option <expected>)` (so a genuine
/// unrelated mismatch — `Int64` vs `String` — is untouched). Detects Option by the built-in sum's `name`
/// plus a single type argument (the `(Option a)` prelude shape), payload compared by `agrees_with` so a
/// deferred/`Any` payload still matches. An honest "match it" route where no one-shot spelling exists —
/// the diagnostic-carries-a-route-to-a-fix rule of `spec/capabilities/diagnostics.md`.
/// The `(module, member)` names of an application head that is a `(. Module member)` MEMBER ACCESS with
/// BOTH parts bare names — `(. List push)` → `("List", "push")`. Used to name a prelude operation in a
/// wrong-argument-type diagnostic ("`List.push` expects an argument of type …"), instead of the generic
/// unify mismatch. `None` when the head is not that shape (a bare operator atom `+`, a user-fn name, a
/// computed head) — those take their own diagnostic paths, so this fires only for a member-op call.
fn member_op_head_name(db: &Db, head: StructId) -> Option<(String, String)> {
    let tail = db.ast.as_form(head, ".")?;
    let module = db.ast.as_name(*tail.first()?)?.to_string();
    let member = db.ast.as_name(*tail.get(1)?)?.to_string();
    Some((module, member))
}

/// The source NAME of an application head that is a bare VARIANT CONSTRUCTOR — `(Mk 1 2)` → `"Mk"`. Used
/// to name the constructor in an OVER-APPLICATION diagnostic ("`Mk` takes 2 arguments, but 3 were given")
/// so a bare ctor reads as well as the member-access spelling `(. P Mk)` does (which `member_op_head_name`
/// already covers). `None` when the head is not a bare-name variant constructor — an ordinary function, an
/// operator, or a member-access head (that path names itself). Reads the head's source name first, then
/// confirms it constructs a variant via `variant_disc_of` (the same predicate the wrong-type-payload
/// branch uses) — GENERIC, no hard-coded ctor list (`no-keys-outside-the-prelude`).
fn variant_ctor_head_name(db: &mut Db, head: StructId) -> Option<String> {
    let name = db.ast.as_name(head)?.to_string();
    crate::eval::variant_disc_of(db, head).map(|_| name)
}

/// The callee's source NAME for a lambda-application head, when it is a plain bare-name function
/// reference — `(h true)` → `"h"`. Used to name the function in a wrong-typed-argument diagnostic
/// ("argument to `h` …"). `None` for an anonymous lambda applied directly (`((fn (x) …) 5)`), a
/// member-access head, or any non-name head — those callers fall back to the un-named phrasing. Reads the
/// head's source spelling (`db.ast.as_name`), so it works for a top-level def and a scoped local alike.
fn callee_head_name(db: &Db, head: StructId) -> Option<String> {
    db.ast.as_name(head).map(str::to_string)
}

/// Whether the application head names a NAMED definition — a top-level `(def (f …) …)`, a lambda-valued
/// `(def f (fn …))`, or a module member reached by name — as opposed to an INLINE / anonymous `(fn …)`
/// literal applied in place. A named def's body is INDEPENDENTLY collected by `compile::collect_faults`
/// (so a call site must NOT re-report its body faults — the baseline-subtraction de-duplicates them); an
/// inline lambda's body is checked ONLY at its β-reduction call site, so its faults must NOT be
/// subtracted. Follows a `Ref`/`Member` chain to the def the way `callee_def_index_for_infer` does, but
/// answers "is this a named def?" via the head resolving to a `Lambda` reached THROUGH a name — robust for
/// a lambda-valued def whose registered body node differs from the head's resolved inner body (which is
/// exactly what makes `callee_def_index_for_infer`'s `def_index_by_body` miss it).
fn named_callee_head(db: &mut Db, head: StructId) -> bool {
    // A direct name application `(helper x)` — the head IS a name occurrence.
    if db.ast.as_name(head).is_some() {
        return true;
    }
    // A `Ref`/`Member` chain to a named def (`(m.f x)`, or a name that resolved to a Ref). An inline
    // `(fn …)` head resolves straight to `Lambda` with no intervening name → false.
    match crate::resolve::resolved_of(db, head) {
        crate::resolved::Resolved::Ref { value } if value != head => named_callee_head(db, value),
        crate::resolved::Resolved::Member { .. } => true,
        _ => false,
    }
}

/// The wrong-typed-CALL-ARGUMENT message: the argument's type does not satisfy the parameter's declared
/// type at a call site. Framed as an ARGUMENT ("this argument is a Bool, but …"), NOT an annotation — the
/// author wrote no annotation — the same phrasing the synthesized-parameter-annotation path (M106) uses,
/// so a referenced-param arg (reported by that path) and an UNREFERENCED-param / recursive-callee arg
/// (reported here at the call-site unify, step 1) read identically instead of the raw "type mismatch: X
/// and Y must be the same type here" the unify produced. When the callee + parameter NAMES are known
/// (a bare-name function with a named parameter) it names them — "argument to `h`'s parameter `a`" —
/// which the synthesized-annotation path cannot (it has only the annotation node). `expected` is the
/// parameter's declared type, `actual` the argument's type; `tail` carries an optional structural hint.
fn call_argument_mismatch_message(
    callee: Option<&str>,
    param: Option<&str>,
    expected: &Ty,
    actual: &Ty,
    tail: &str,
    ncx: &NameCtx,
) -> String {
    let subject = match (callee, param) {
        (Some(f), Some(p)) => format!("the argument for `{f}`'s parameter `{p}`"),
        (Some(f), None) => format!("the argument to `{f}`"),
        _ => "this argument".to_string(),
    };
    format!(
        "{subject} is {}, but a value of type {} is expected here{tail}",
        actual.render_with_article(ncx),
        expected.render_name(ncx),
    )
}

fn option_payload_mismatch_hint(ncx: &NameCtx, expected: &Ty, actual: &Ty) -> Option<String> {
    if let Ty::Sum { decl, args } = actual
        && ncx.name_of(*decl) == Some("Option")
        && let [payload] = &args[..]
        && payload.agrees_with(expected)
    {
        return Some(
            " — the value is optional; match it to handle the absent (`None`) case, \
             e.g. `(match v ((Some x) …) ((None) …))`"
                .to_string(),
        );
    }
    None
}

/// Drill through two SAME-SHAPE nested compounds (records with the same field set, tuples of the same
/// arity) to the DEEPEST single leaf where the types actually differ, returning the relative access PATH
/// to that leaf and the leaf's expected-vs-actual types. `(Record (a (Record (b Int64))))` vs `(… (b
/// Bool))` drills to `("a.b", Int64, Bool)` so a caller can say "field `a.b` should be Int64, but this one
/// is Bool" instead of re-rendering the whole differing sub-record. Segments are dotted — a record field
/// contributes its name, a tuple position its 0-based index (`pt.1` = field `pt`, element 1) — matching
/// the member-access spelling. `None` when the two types are NOT further drillable at this level: a scalar
/// (or other) leaf, a field-SET / arity difference, or a cross-kind clash — in those cases the caller
/// keeps its own single-level phrasing (naming the immediate member + rendering the two sub-types, whose
/// difference the render then shows). Terminates because each step descends into a strictly smaller
/// structural sub-type (a `Ty::Nominal`/collection is a non-drillable leaf, so a recursive nominal stops).
fn deep_leaf_delta<'a>(want: &'a Ty, got: &'a Ty) -> Option<(String, &'a Ty, &'a Ty)> {
    match (want, got) {
        (Ty::Record(w), Ty::Record(g)) => {
            // Only drill a SAME field-set record; a field-set difference is not a single-leaf type diff
            // (the caller renders the sub-record, whose missing/extra field the render shows).
            if w.len() != g.len() || !w.keys().all(|k| g.contains_key(k)) {
                return None;
            }
            let (k, wt) = w
                .iter()
                .find(|(k, wt)| g.get(k).is_some_and(|gt| !wt.agrees_with(gt)))?;
            let gt = &g[k];
            Some(match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{}.{sub}", k.name), lw, lg),
                None => (k.name.to_string(), wt, gt),
            })
        }
        (Ty::Tuple(w), Ty::Tuple(g)) => {
            if w.len() != g.len() {
                return None; // an arity difference is reported at this level, not drilled
            }
            let (i, (wt, gt)) = w
                .iter()
                .zip(g.iter())
                .enumerate()
                .find(|(_, (wt, gt))| !wt.agrees_with(gt))?;
            Some(match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{i}.{sub}"), lw, lg),
                None => (i.to_string(), wt, gt),
            })
        }
        _ => None,
    }
}

/// A coercion FIX for a numeric leaf buried inside a directly-written COMPOUND value whose annotated type
/// differs only at that leaf — `(record (x 5))` annotated `(Record (x Float64))` retypes the inner `5` →
/// `5.0`, `(tuple 1 2)` vs `(Tuple Int64 Float64)` retypes the `2`, `(list 5)` vs `(List Float64)` retypes
/// the `5`. The structural-delta hint NAMES the leaf (`field \`x\` should be Float64 …`); this gives it the
/// same one-shot repair a bare `(: 5 Float64)` gets, anchored at the INNER value node. Drills the VALUE
/// expression (`expr`) in lockstep with the type delta (record field / tuple position / list element),
/// mirroring `deep_leaf_delta`'s type walk, to reach the leaf value node, then defers to
/// `numeric_text_coercion_fix` (the same helper the bare annotation uses). `None` unless the value is a
/// directly-written compound literal whose single differing leaf has a numeric coercion — a value bound to
/// a name / returned from a call has no inner literal to edit, and a non-numeric leaf (Bool vs Int) has no
/// coercion (its structural-delta message stands alone).
fn compound_inner_coercion_fix(
    db: &mut Db,
    expr: StructId,
    expected: &Ty,
    actual: &Ty,
) -> Option<Fix> {
    match (expected, actual) {
        (Ty::Record(w), Ty::Record(g))
            if w.len() == g.len() && w.keys().all(|k| g.contains_key(k)) =>
        {
            // Find the first field (sorted key order) whose type differs, then the matching value node in
            // the written record literal.
            let (key, wt) = w
                .iter()
                .find(|(k, wt)| g.get(k).is_some_and(|gt| !wt.agrees_with(gt)))?;
            let (gt, wt) = (g[key].clone(), (*wt).clone());
            let value_node = *record_value_nodes(db, expr)?.get(key)?;
            compound_inner_coercion_fix(db, value_node, &wt, &gt)
                .or_else(|| numeric_text_coercion_fix(db, &wt, &gt, value_node))
        }
        (Ty::Tuple(w), Ty::Tuple(g)) if w.len() == g.len() => {
            let (i, (wt, gt)) = w
                .iter()
                .zip(g.iter())
                .enumerate()
                .find(|(_, (wt, gt))| !wt.agrees_with(gt))?;
            let (gt, wt) = (gt.clone(), wt.clone());
            let value_node =
                *positional_value_nodes(db, expr, crate::resolved::Prim::TupleNew)?.get(i)?;
            compound_inner_coercion_fix(db, value_node, &wt, &gt)
                .or_else(|| numeric_text_coercion_fix(db, &wt, &gt, value_node))
        }
        (Ty::List(we), Ty::List(ge)) if !we.agrees_with(ge) => {
            // A list is homogeneous — retype the FIRST element whose inner numeric a coercion bridges (the
            // fix an agent applies element-by-element; the message names the element axis).
            let (we, ge) = ((**we).clone(), (**ge).clone());
            positional_value_nodes(db, expr, crate::resolved::Prim::ListNew)?
                .iter()
                .find_map(|&e| {
                    compound_inner_coercion_fix(db, e, &we, &ge)
                        .or_else(|| numeric_text_coercion_fix(db, &we, &ge, e))
                })
        }
        (Ty::Sum { decl: wd, .. }, Ty::Sum { decl: gd, .. }) if wd == gd => {
            // A variant-constructed value whose payload's inner numeric differs from the annotated sum's
            // payload — `(Some 5)` vs `(Option Float64)`, `(Ok 5)` vs `(Result Float64 String)`. The value
            // is a ctor application `(Some 5)` = `Apply{head=Some, args=[5]}`; drill its payload
            // argument(s) against the EXPECTED payload type at the annotated sum's instantiation
            // (`payload_ty_at_instantiation`). Same `decl` (same sum) so the variant + payload align; the
            // per-arg coercion is the sum-payload twin of the collection-element fix. (A DECLARED sum
            // applied directly — `(Mk 5)` — already coerces via the variant-ctor-payload branch; this
            // covers the ANNOTATION/arg-mismatch site, incl. the prelude `Option`/`Result`.)
            let Resolved::Apply { head, args } = resolved_of(db, expr) else {
                return None;
            };
            // A SINGLE-payload variant construction — `(Some 5)`, `(Ok 5)`, `(Mk 5)` — whose one argument
            // IS the payload. `payload_ty_at_instantiation` gives that payload's type at the EXPECTED sum
            // (`Float64` for `(Option Float64)`); coerce the argument against it. A multi-payload variant
            // (its payload is a tuple) is left alone — the single-arg shape is the common numeric case and
            // keeps the type-arg↔argument alignment unambiguous.
            if crate::eval::variant_disc_of(db, head).is_none() || args.len() != 1 {
                return None;
            }
            let want = payload_ty_at_instantiation(db, head, expected)?;
            let arg = args[0];
            let got = type_of(db, arg);
            compound_inner_coercion_fix(db, arg, &want, &got)
                .or_else(|| numeric_text_coercion_fix(db, &want, &got, arg))
        }
        (Ty::Map(wk, wv), Ty::Map(gk, gv)) => {
            // A map is homogeneous on each axis. Retype the FIRST entry's KEY (if the key axis differs) or
            // VALUE (if the value axis differs) whose inner numeric a coercion bridges — mirroring the
            // collection-axis hint (`collection_element_mismatch_hint` reports KEY before VALUE). The
            // entries are `(key-occ, value-occ)` pairs of the written `(map (k v) …)` literal.
            let key_diff = !wk.agrees_with(gk);
            let (wt, gt) = if key_diff {
                ((**wk).clone(), (**gk).clone())
            } else if !wv.agrees_with(gv) {
                ((**wv).clone(), (**gv).clone())
            } else {
                return None;
            };
            map_entry_nodes(db, expr)?.iter().find_map(|&(k, v)| {
                let node = if key_diff { k } else { v };
                compound_inner_coercion_fix(db, node, &wt, &gt)
                    .or_else(|| numeric_text_coercion_fix(db, &wt, &gt, node))
            })
        }
        _ => None,
    }
}

/// The ordered `(key-node, value-node)` entries of a directly-written MAP literal `expr` — both the
/// `Resolved::Map` primitive form and the `map` NAME-alias application (`Apply` of `Prim::MapNew`, whose
/// args are the `(k v)` entry-pair lists). `None` when `expr` is not a written map literal. Lets
/// `compound_inner_coercion_fix` reach a map key/value leaf regardless of which map spelling was used.
fn map_entry_nodes(db: &mut Db, expr: StructId) -> Option<Vec<(StructId, StructId)>> {
    match resolved_of(db, expr) {
        Resolved::Map { entries } => Some(entries.to_vec()),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapNew) =>
        {
            // Each arg is a map ENTRY: the native `(= k v)` FieldPair leaf (M2, what `#map`/`(map (= k v))`
            // emit), the transitional name-head `(= k v)`, or the legacy 2-element `(k v)` pair — mirror
            // `resolve_map`. Before this, a native FieldPair entry (3-element) failed the 2-element read →
            // the whole `.collect()` returned `None` → `map_entry_nodes` yielded no entries → the map's
            // key/value TYPE stayed `Any`, so the value/key-type HOMOGENEITY + duplicate-key checks never
            // ran → a two-different-value-types / duplicate-key map silently type-checked (miscompile).
            args.iter()
                .map(|&pair| {
                    db.ast
                        .field_pair_parts(pair)
                        .or_else(|| db.ast.field_pair(pair))
                        .or_else(|| match db.ast.get(pair) {
                            crate::ast::Struct::List(kids) => match kids.as_slice() {
                                [k, v] => Some((*k, *v)),
                                _ => None,
                            },
                            _ => None,
                        })
                })
                .collect()
        }
        _ => None,
    }
}

/// The `label → value-node` map of a directly-written RECORD literal `expr` — both the `{}`/bare-string
/// primitive form (`Resolved::Record`) and the `record` NAME-alias application (`Resolved::Apply` whose
/// `(meta apply)` is `Prim::RecordNew`, read via the shared `read_record_fields`). `None` when `expr` is
/// not a written record literal (a value bound to a name / returned from a call has no field-value nodes
/// to edit). Lets `compound_inner_coercion_fix` reach the inner leaf regardless of which record spelling
/// the author used.
fn record_value_nodes(
    db: &mut Db,
    expr: StructId,
) -> Option<std::collections::BTreeMap<crate::resolved::Symbol, StructId>> {
    match resolved_of(db, expr) {
        Resolved::Record { fields } => Some((*fields).clone()),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::RecordNew) =>
        {
            crate::resolve::read_record_fields(db, &args).ok()
        }
        _ => None,
    }
}

/// A one-shot RENAME fix for a MISSPELLED record-literal field: when the `actual` record value carries a
/// field whose name is a plausible typo of a field the `expected` record type requires but the value is
/// MISSING, rewrite that field's KEY to the expected name (`(record … (yy 2))` where `(y Int64)` was
/// wanted → replace the key `yy` with `y`). The record-literal twin of the member-access
/// `no_field_reject` typo fix — a wrong field NAME in a CONSTRUCTED record is exactly the same slip as a
/// wrong field name in a projection, so it earns the same mechanical repair. `None` unless both are
/// records, the value literal is written inline (its key nodes are editable), and there is a confident
/// single typo pairing (a missing expected field that is `suggest::nearest` to an extra supplied one).
fn record_field_typo_fix(db: &mut Db, expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    // The fields the value SUPPLIES that the type has no place for (candidate typos) and the fields the
    // type REQUIRES that the value is missing (candidate intended names).
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    let missing: Vec<&str> = want
        .keys()
        .filter(|k| !got.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    // A CONFIDENT single pairing at THIS level: exactly one extra field that is the nearest typo of some
    // missing one. (More than one extra/missing is not a single mechanical rename — the field-set-diff
    // message still guides; we just don't auto-fix an ambiguous multi-field slip.)
    if let [typo] = extra.as_slice()
        && let Some(intended) = crate::diag::suggest::nearest(typo, missing.iter().copied())
        // Find the KEY occurrence of the typo'd field in the WRITTEN record literal — the `k` in a `(k v)`
        // entry — so the fix rewrites exactly that token. `None` if the value is not an inline literal (a
        // name-bound record has no editable key node), matching the honest-no-fix rule.
        && let Some(key_occ) = record_field_key_occ(db, arg, typo)
    {
        return Some(Fix::replace_heuristic(key_occ, intended));
    }
    // No typo at THIS level. If the field SETS agree, a typo may live inside a shared field whose want/got
    // are BOTH records that differ — DRILL into that field's value literal and recurse (the field-typo twin
    // of `compound_inner_coercion_fix`'s nested-leaf drill, so `(record (inner (record (fooo 1))))` against
    // `(Record (inner (Record (foo Int64))))` renames the deep `inner.fooo`→`foo`). Only when the field sets
    // are identical (no top-level extra/missing) — otherwise the top-level set diff is the real fault.
    if extra.is_empty() && missing.is_empty() {
        for (k, wt) in want.iter() {
            let gt = got.get(k)?;
            if let (Ty::Record(_), Ty::Record(_)) = (wt, gt)
                && !wt.agrees_with(gt)
                && let Some(sub_arg) = record_field_value_occ(db, arg, &k.name)
                && let Some(fix) = record_field_typo_fix(db, wt, gt, sub_arg)
            {
                return Some(fix);
            }
        }
    }
    None
}

/// The (key-node, value-node) of a record-literal field ENTRY, handling both the canonical
/// `(= name value)` ascription triple (RV2, Phase B — key = child 1, value = child 2) and the legacy
/// `(name value)` pair. `None` if `entry` is neither shape. A record VALUE field carries the `=` head;
/// a record PATTERN field stays a bare pair (patterns were not migrated), and both are read here.
fn record_field_kv(db: &Db, entry: StructId) -> Option<(StructId, StructId)> {
    match db.ast.get(entry) {
        crate::ast::Struct::List(kv) if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") => {
            Some((kv[1], kv[2]))
        }
        crate::ast::Struct::List(kv) if kv.len() == 2 => Some((kv[0], kv[1])),
        _ => None,
    }
}

/// The ENTRY node (the `(= k v)` / `(k v)` field) named `field` in a WRITTEN record literal `expr` — the
/// whole field, so a delete fix can remove it as a unit. `None` if `expr` is not an inline record literal
/// (in the RAW AST) or has no such field. Same raw-AST discipline as [`record_field_key_occ`].
fn record_field_entry_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    let entries = db.ast.compound_form_of(expr, CompoundCtor::Record)?;
    for &entry in entries {
        if let Some((key_id, _)) = record_field_kv(db, entry)
            && let Some(sym) = crate::resolve::read_key(db, key_id)
            && &*sym.name == field
        {
            return Some(entry);
        }
    }
    None
}

/// The WRITTEN record-literal FORM node of `expr` (the `(record …)` list itself), if `expr` is an inline
/// record literal in the RAW AST — the node an `InsertArms` fix appends new `(field value)` entries to.
/// `None` for a name-bound / call-result record (no source form to edit).
fn record_literal_form(db: &Db, expr: StructId) -> Option<StructId> {
    // The native `#record(…)` ctor-leaf head, the reserved-symbol head `("record" …)`, and the bare `record`
    // name-alias all spell the same record form; `compound_form_of` recognizes all three, so the add-missing-
    // fields fix appends to a native record literal too (M3 native-recognition parity).
    if db
        .ast
        .compound_form_of(expr, CompoundCtor::Record)
        .is_some()
    {
        Some(expr)
    } else {
        None
    }
}

/// A one-shot ADD-MISSING-FIELDS fix for a record literal that is MISSING fields its expected type
/// requires: append a `(field (trap "TODO"))` entry per missing field to the written `(record …)` form
/// (`(record (x 1))` against `(Record (x Int64) (y Int64))` → `(record (x 1) (y (trap "TODO")))`). The
/// construction analogue of the "add the missing match arms" fix: the SHAPE is right (the field set now
/// matches, clearing CDZ0203) and `trap : ∀a. String → a` inhabits ANY field type so the placeholder never
/// introduces a fresh type error — but the VALUES are placeholders the author fills, so it is heuristic
/// (`--verify-fixes` upgrades it, since applying it clears the fault). Requires BOTH records, the value
/// written inline (its form is editable), and a NON-EMPTY missing set with NO extra field (a pure omission
/// — a value that also carries an extra field is a rename candidate `record_field_typo_fix` handles first,
/// or a genuinely different shape, not a clean "you forgot these" add).
fn record_field_add_fix(db: &mut Db, expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let missing: Vec<&crate::resolved::Symbol> =
        want.keys().filter(|k| !got.contains_key(*k)).collect();
    let extra = got.keys().any(|k| !want.contains_key(k));
    // A pure omission — some required field absent, and no supplied field the type lacks (that would be a
    // rename or a genuinely different shape, not a clean "add the fields you forgot").
    if missing.is_empty() || extra {
        return None;
    }
    let form = record_literal_form(db, arg)?;
    // Synthesize each missing field as the canonical `(= name value)` ascription triple (RV2, Phase B),
    // so the inserted entry matches the value-record field shape the printer/resolver expect.
    let arms: Vec<String> = missing
        .iter()
        .map(|k| format!("(= {} (trap \"TODO\"))", k.name))
        .collect();
    Some(Fix::insert_arms_heuristic(form, arms))
}

/// A one-shot DELETE-EXTRA-FIELD fix for a record literal carrying a field its expected type has no place
/// for: remove that `(field value)` entry from the written `(record …)` form (`(record (x 1) (y 2))`
/// against `(Record (x Int64))` → `(record (x 1))`). The construction analogue of the "remove the unused
/// element" delete fix. Requires BOTH records, the value written inline, and EXACTLY ONE extra field with
/// NO missing field (a single clean surplus — an extra alongside a missing one is a rename candidate
/// `record_field_typo_fix` handles first; multiple extras are not one mechanical delete).
fn record_field_delete_fix(db: &mut Db, expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    let missing = want.keys().any(|k| !got.contains_key(k));
    if let [surplus] = extra.as_slice()
        && !missing
        && let Some(entry) = record_field_entry_occ(db, arg, surplus)
    {
        return Some(Fix::delete_heuristic(
            entry,
            format!("remove the field `{surplus}` — the expected record has no such field"),
        ));
    }
    None
}

/// The WRITTEN tuple-literal FORM node of `expr` (the `(tuple …)` list itself), if `expr` is an inline
/// tuple literal in the RAW AST — the node an `InsertArms` fix appends new element forms to. `None` for a
/// name-bound / call-result tuple (no source form to edit). Mirrors [`record_literal_form`]; a tuple is
/// spelled by either the `tuple` NAME alias or the `"tuple"` string-literal primitive head.
fn tuple_literal_form(db: &Db, expr: StructId) -> Option<StructId> {
    if db.ast.compound_form_of(expr, CompoundCtor::Tuple).is_some() {
        Some(expr)
    } else {
        None
    }
}

/// A one-shot ADD-MISSING-ELEMENTS fix for a tuple literal with too FEW elements: append a `(trap "TODO")`
/// placeholder per missing trailing position to the written `(tuple …)` form (`(tuple 1 2)` against
/// `(Tuple Int64 Int64 Int64)` → `(tuple 1 2 (trap "TODO"))`). The tuple analogue of `record_field_add_fix`:
/// `trap : ∀a. String → a` inhabits any element type, so the placeholder clears the arity fault in one shot
/// (`--verify-fixes` upgrades it). Requires BOTH tuples, the value written inline, and the value having
/// STRICTLY FEWER elements — a value with MORE is the delete case; equal arity is a per-position type
/// mismatch (its own message), not an arity fix.
fn tuple_element_add_fix(db: &mut Db, expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let (Ty::Tuple(want), Ty::Tuple(got)) = (expected, actual) else {
        return None;
    };
    if got.len() >= want.len() {
        return None;
    }
    let form = tuple_literal_form(db, arg)?;
    let arms: Vec<String> = (got.len()..want.len())
        .map(|_| "(trap \"TODO\")".to_string())
        .collect();
    Some(Fix::insert_arms_heuristic(form, arms))
}

/// A one-shot DELETE-SURPLUS-ELEMENT fix for a tuple literal with EXACTLY ONE too many elements: remove the
/// trailing surplus element from the written `(tuple …)` form (`(tuple 1 2 3)` against `(Tuple Int64
/// Int64)` → `(tuple 1 2)`). The tuple analogue of `record_field_delete_fix`. Gated to a SINGLE surplus
/// (exactly one over) — one clean mechanical delete; two-or-more surplus is not one edit (the message still
/// names the arity), and a value with FEWER elements is the add case.
fn tuple_element_delete_fix(db: &mut Db, expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let (Ty::Tuple(want), Ty::Tuple(got)) = (expected, actual) else {
        return None;
    };
    if got.len() != want.len() + 1 {
        return None;
    }
    let elems = positional_value_nodes(db, arg, crate::resolved::Prim::TupleNew)?;
    // The surplus is the trailing element (position `want.len()`), the one past the expected arity.
    let surplus = *elems.get(want.len())?;
    Some(Fix::delete_heuristic(
        surplus,
        "remove the extra tuple element — the expected tuple has fewer positions",
    ))
}

/// The VALUE occurrence (`v` in a `(k v)` entry) of the field named `field` in a WRITTEN record literal
/// `expr` — the companion of [`record_field_key_occ`] that returns the field's value node, so a nested
/// typo fix can recurse into a sub-record literal. `None` if `expr` is not an inline record literal or has
/// no such field.
fn record_field_value_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    let entries = db.ast.compound_form_of(expr, CompoundCtor::Record)?;
    for &entry in entries {
        if let Some((key_id, val_id)) = record_field_kv(db, entry)
            && let Some(sym) = crate::resolve::read_key(db, key_id)
            && &*sym.name == field
        {
            return Some(val_id);
        }
    }
    None
}

/// The KEY occurrence (`k` in a `(k v)` entry) of the field named `field` in a WRITTEN record literal
/// `expr` — `(record (a 1) (b 2))`. `None` if `expr` is not an inline record literal (in the RAW AST) or
/// has no such field. Reads the RAW `db.ast` structure of `expr` (NOT `resolved_of`, whose `RecordNew`
/// args are RE-NODED entries that do not map back to the source key tokens the fix must edit) — the same
/// raw-AST discipline `no_field_reject` uses to target the exact `(. operand key)` key token.
fn record_field_key_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    // The `(key value)` entry list is the tail of a `(record …)` form (both the reserved-symbol head and a
    // bare `record` name-alias spell the same shape here).
    let entries = db.ast.compound_form_of(expr, CompoundCtor::Record)?;
    for &entry in entries {
        if let Some((key_id, _)) = record_field_kv(db, entry)
            && let Some(sym) = crate::resolve::read_key(db, key_id)
            && &*sym.name == field
        {
            return Some(key_id);
        }
    }
    None
}

/// The ordered element value-nodes of a directly-written TUPLE (`prim = TupleNew`) or LIST (`ListNew`)
/// literal `expr` — both the `Resolved::Tuple`/`List` primitive form and the `tuple`/`list` NAME-alias
/// application (`Resolved::Apply` of the matching `(meta apply)` prim, whose `args` ARE the elements).
/// `None` when `expr` is not the requested written literal kind.
pub(crate) fn positional_value_nodes(
    db: &mut Db,
    expr: StructId,
    prim: crate::resolved::Prim,
) -> Option<Vec<StructId>> {
    match resolved_of(db, expr) {
        Resolved::Tuple { elems } if prim == crate::resolved::Prim::TupleNew => {
            Some(elems.to_vec())
        }
        Resolved::List { elems } if prim == crate::resolved::Prim::ListNew => Some(elems.to_vec()),
        Resolved::Apply { head, args } if crate::eval::meta_apply_of(db, head) == Some(prim) => {
            Some(args.to_vec())
        }
        _ => None,
    }
}

/// An actionable message TAIL when `expected` and `actual` are BOTH records that differ. Two shapes,
/// each pointing at the SPECIFIC difference instead of leaving the reader to diff two full record renders:
///  • a FIELD-SET difference — the value is MISSING a field the type requires, and/or carries an EXTRA one
///    the type has no place for (rustc's "missing field `y`" / "no field `z`"); OR
///  • a same-field-set PER-FIELD TYPE difference — the field names all match but some field's TYPE differs
///    (`(x Int64)` vs `(x Bool)`), named as "field `x` should be Int64, but this one is Bool" (rustc's
///    "expected `Int64`, found `Bool`" anchored on the field). Buried in a 3-field render otherwise — the
///    reader must diff `(Record (x Int64) (y Int64) (z Int64))` against `(… (y Bool) …)` to spot `y`.
/// `None` unless both are records AND some difference is found. Field names are compared as SETS (the
/// `BTreeMap` keys) and the differing-type fields are visited in sorted key order, so the hint is
/// deterministic and order-independent. A field-set difference takes precedence (a record with both a
/// wrong field-set AND a type mismatch on a shared field is first told which fields to add/remove).
fn record_field_diff_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let missing: Vec<&str> = want
        .keys()
        .filter(|k| !got.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| &*k.name)
        .collect();
    if missing.is_empty() && extra.is_empty() {
        // Same field-name set — look for the FIRST field (sorted key order) whose type differs and name
        // it. `agrees_with` is the same relation `unify` uses, so we only flag a genuine clash (a deferred
        // `Var`/`Any` field agrees and is skipped). Naming one field is enough to point the fix; the full
        // render still carries the complete picture for a multi-field clash. When the differing field is
        // itself a same-shape nested compound, DRILL to the deepest scalar leaf so the hint reads "field
        // `a.b.c` should be Int64, but this one is Bool" instead of re-rendering the whole sub-record.
        let culprit = want
            .iter()
            .find(|(k, wt)| got.get(k).is_some_and(|gt| !wt.agrees_with(gt)));
        return culprit.map(|(k, wt)| {
            let gt = &got[k];
            let (path, lw, lg) = match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{}.{sub}", k.name), lw, lg),
                None => (k.name.to_string(), wt, gt),
            };
            format!(
                " — field `{path}` should be {}, but this one is {}",
                lw.render_name(ncx),
                lg.render_name(ncx)
            )
        });
    }
    let quote = |names: &[&str]| {
        names
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut parts = Vec::new();
    if !missing.is_empty() {
        let plural = if missing.len() == 1 { "" } else { "s" };
        parts.push(format!("missing field{plural} {}", quote(&missing)));
    }
    if !extra.is_empty() {
        let plural = if extra.len() == 1 { "" } else { "s" };
        parts.push(format!(
            "no such field{plural} {} on the expected record",
            quote(&extra)
        ));
    }
    Some(format!(" — {}", parts.join("; ")))
}

/// An actionable message TAIL when `expected` and `actual` are BOTH tuples that differ. Two shapes,
/// mirroring the record hint, each pointing at the SPECIFIC difference:
///  • DIFFERENT ARITY — the value has more or fewer elements than the type ("expected a tuple with 3
///    elements, but this one has 2", rustc's arity message); OR
///  • same-arity PER-POSITION TYPE difference — a `(Tuple Int64 Bool)` where `(Tuple Int64 Int64)` is
///    wanted, named as "element 1 should be Int64, but this one is Bool" (0-indexed, matching the tuple
///    projection `(. t 1)` and the out-of-range "tuple index N" message). Buried in the full render
///    otherwise for a wide tuple.
/// `None` unless both are tuples AND some difference is found. Positions are visited left-to-right, so the
/// hint is deterministic (the first differing position). `agrees_with` (the relation `unify` uses) gates
/// the per-position check, so a deferred `Var`/`Any` position is skipped.
fn tuple_arity_mismatch_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    let (Ty::Tuple(want), Ty::Tuple(got)) = (expected, actual) else {
        return None;
    };
    if want.len() != got.len() {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        return Some(format!(
            " — expected a tuple with {} element{}, but this one has {}",
            want.len(),
            plural(want.len()),
            got.len(),
        ));
    }
    // Same arity — name the FIRST position whose type differs (0-indexed). When that position is itself a
    // same-shape nested compound, DRILL to the deepest scalar leaf ("element 0.x should be …") rather than
    // re-rendering the whole sub-compound.
    want.iter()
        .zip(got.iter())
        .enumerate()
        .find(|(_, (wt, gt))| !wt.agrees_with(gt))
        .map(|(i, (wt, gt))| {
            let (path, lw, lg) = match deep_leaf_delta(wt, gt) {
                Some((sub, lw, lg)) => (format!("{i}.{sub}"), lw, lg),
                None => (i.to_string(), wt, gt),
            };
            format!(
                " — element {path} should be {}, but this one is {}",
                lw.render_name(ncx),
                lg.render_name(ncx)
            )
        })
}

/// An actionable message TAIL when `expected` and `actual` are the SAME collection KIND (both `List`, both
/// `Map`, or both `Set`) but an ELEMENT / KEY / VALUE type differs — the collection analogue of the
/// record/tuple per-member hint. Naming both full types (`(Map String Int64)` vs `(Map Int64 Int64)`)
/// makes the reader diff two renders to see it is the KEY axis that differs; this names the axis and its
/// expected-vs-actual type ("its keys should be String, but these are Int64"). For a `Map` the KEY is
/// checked before the VALUE (report the leftmost differing axis, deterministic). `agrees_with` (the
/// relation `unify` uses) gates each axis, so a deferred `Var`/`Any` element is skipped. `None` unless
/// both are the same collection kind AND some axis genuinely differs. Tail only — the repair is retyping
/// the offending element(s), which the author must supply, so no mechanical fix (like the record/tuple
/// per-member hints).
fn collection_element_mismatch_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    let axis = |role: &str, want: &Ty, got: &Ty, plural: &str| {
        (!want.agrees_with(got)).then(|| {
            format!(
                " — its {role} should be {}, but {plural} {}",
                want.render_name(ncx),
                got.render_name(ncx)
            )
        })
    };
    match (expected, actual) {
        (Ty::List(want), Ty::List(got)) => axis("elements", want, got, "these are"),
        (Ty::Set(want), Ty::Set(got)) => axis("elements", want, got, "these are"),
        (Ty::Map(wk, wv), Ty::Map(gk, gv)) => {
            // KEY axis first, then VALUE — report the leftmost differing axis.
            axis("keys", wk, gk, "these are").or_else(|| axis("values", wv, gv, "these are"))
        }
        _ => None,
    }
}

/// An actionable message TAIL when `expected` and `actual` are the SAME sum type (same `decl` + variant
/// set) whose TYPE PARAMETER differs — `(Option Float64)` vs `(Option Int64)`, `(Result Float64 String)`
/// vs `(Result Int64 String)`. Names the differing payload axis (" — its payload should be Float64, but
/// this one is Int64") instead of leaving the reader to diff two full `(Option …)` renders, the sum twin
/// of the collection-axis hint. `None` unless both are the same sum with the same arg count and a genuine
/// arg difference (a DIFFERENT sum — `Option` vs `Result` — is unrelated, the generic path). `agrees_with`
/// gates each arg. Reports the first (leftmost) differing type argument.
fn sum_payload_mismatch_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    let (
        Ty::Sum {
            decl: wd, args: wa, ..
        },
        Ty::Sum {
            decl: gd, args: ga, ..
        },
    ) = (expected, actual)
    else {
        return None;
    };
    if wd != gd || wa.len() != ga.len() {
        return None;
    }
    wa.iter().zip(ga.iter()).find_map(|(w, g)| {
        (!w.agrees_with(g)).then(|| {
            format!(
                " — its payload should be {}, but this one is {}",
                w.render_name(ncx),
                g.render_name(ncx)
            )
        })
    })
}

/// An actionable message TAIL when `expected` and `actual` are BOTH FUNCTION types (`Ty::Fn`) that differ
/// — the function analogue of the record/tuple/sum per-member hints. A `Ty::Fn` is CURRIED (`p0 → p1 →
/// result`), so naming two full arrow renders (`(-> Int64 (-> Int64 Int64))` vs `(-> Int64 Int64)`) makes
/// the reader unwind the curry to see what actually differs. Peel both arrow chains and name the SPECIFIC
/// difference, in the order a caller most cares about:
///  • DIFFERENT ARITY — the two take a different number of arguments ("expected a function taking 2
///    arguments, but this one takes 1", rustc's arrow-arity message); OR
///  • same-arity RESULT-type difference — the parameter types agree but the RESULT differs (`(-> Int64
///    Bool)` vs `(-> Int64 Int64)`), named "its result should be Bool, but this one returns Int64".
/// A same-arity PARAMETER-type difference is deliberately NOT named here: the generic arg-unify already
/// descends into the first differing parameter position and reports it directly (`(-> Bool …)` vs `(-> Int64
/// …)` surfaces as an ordinary "Int64 vs Bool" at the parameter), so a param-axis hint here would duplicate
/// it. `None` unless both are functions AND they differ in arity or result. `agrees_with` (the relation
/// `unify` uses) gates the result check. Tail only — the repair (add/drop a parameter, or change the return
/// expression) is the author's, so no mechanical fix, like the sibling per-member hints.
fn fn_signature_delta_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    if !matches!(expected, Ty::Fn(..)) || !matches!(actual, Ty::Fn(..)) {
        return None;
    }
    // Peel the curried arrows: collect the parameter count and the ultimate result of each.
    let peel = |mut t: &Ty| {
        let mut arity = 0usize;
        while let Ty::Fn(_, r) = t {
            arity += 1;
            t = r;
        }
        (arity, t.clone())
    };
    let ((want_arity, want_result), (got_arity, got_result)) = (peel(expected), peel(actual));
    if want_arity != got_arity {
        let plural = |n: usize| if n == 1 { "" } else { "s" };
        return Some(format!(
            " — expected a function taking {} argument{}, but this one takes {}",
            want_arity,
            plural(want_arity),
            got_arity,
        ));
    }
    (!want_result.agrees_with(&got_result)).then(|| {
        format!(
            " — its result should be {}, but this one returns {}",
            want_result.render_name(ncx),
            got_result.render_name(ncx)
        )
    })
}

/// The combined per-member structural-delta TAIL for two SAME-KIND compound types that differ inside — a
/// record field-set / per-field type, a tuple arity / per-position type, or a collection element/key/value
/// type. This bundles the three per-member hints (`record_field_diff_hint`, `tuple_arity_mismatch_hint`,
/// `collection_element_mismatch_hint`) behind one call so the JOIN sites — a list literal, an `if`'s
/// branches, a `match`'s arms — can point at the SPECIFIC differing sub-part exactly as the annotation-
/// mismatch site does, instead of rendering two whole compound types the reader must diff. The `first`
/// argument is the type the join is unifying against (the first element / then-branch / first arm); the
/// hints phrase it as "should be <first>, but this one is <other>", which reads correctly for a join (the
/// first occurrence sets the type the rest must match). `None` when the two types are not the same
/// structured kind or agree on every member. No mechanical fix (the repair is retyping a value the author
/// supplies), matching the per-member hints it composes.
///
/// This is how a compound-type rejection names the MINIMAL conflict rather than an arbitrary casualty: it
/// points at the one differing field / position / element axis (the constraint that actually failed),
/// leaving the agreeing members out of the blame, instead of rendering two whole compound types that share
/// most of their structure.
//= spec/capabilities/type-system.md#a-type-rejection-reports-the-minimal-conflict-at-both-sites
//# A rejection for a failed unification MUST report the minimal unsatisfiable set of constraints rather than the first constraint that failed, so that the diagnostic names the actual conflict and not an arbitrary casualty of it.
pub(crate) fn structural_delta_hint(first: &Ty, other: &Ty, ncx: &NameCtx) -> Option<String> {
    record_field_diff_hint(first, other, ncx)
        .or_else(|| tuple_arity_mismatch_hint(first, other, ncx))
        .or_else(|| collection_element_mismatch_hint(first, other, ncx))
        .or_else(|| sum_payload_mismatch_hint(first, other, ncx))
        .or_else(|| fn_signature_delta_hint(first, other, ncx))
        .or_else(|| qty_scale_mismatch_hint(first, other))
        .or_else(|| same_name_distinct_type_hint(first, other, ncx))
}

/// A message TAIL for two `(Qty T u)` types that are the SAME DIMENSION but at DIFFERENT SCALES — the
/// units-specific reason two quantities fail to unify at a join (`if`/`match`/`list`). `(Qty Int64 km)`
/// and `(Qty Int64 m)` are distinct types (their `Ty::Qty.unit` carries the scale — km is 1000/1, m is
/// 1/1), so a join rejects them, but BOTH `render_name()` to `(Qty Int64 (Unit.base #"meter"))` (the
/// render shows the reference-unit NAME and drops the scale). Without this the generic
/// `same_name_distinct_type_hint` fires and blames a "declaration shadows a built-in" — flat wrong for a
/// scale clash. Name the REAL cause and the repair: the branches carry the same dimension at different
/// units, so convert one to the other's unit (`in`/`as`) to make the join well-typed — a quantity join
/// does NOT auto-normalize (the no-silent-promotion rule; a conversion is explicit). Fires ONLY for two
/// same-dimension different-scale quantities; a genuine cross-DIMENSION clash (meter vs second) renders
/// distinguishably and needs no tail, and a same-scale pair unifies (no mismatch to explain).
fn qty_scale_mismatch_hint(first: &Ty, other: &Ty) -> Option<String> {
    let (Ty::Qty { unit: ua, .. }, Ty::Qty { unit: ub, .. }) = (first, other) else {
        return None;
    };
    // Same dimension, different scale — the case the generic same-name hint would misdiagnose. A different
    // dimension renders distinguishably (the units differ by name), so leave it to the plain mismatch.
    if !ua.same_dimension(ub) || ua.scale() == ub.scale() {
        return None;
    }
    Some(
        " — these are the SAME dimension at DIFFERENT units (their scales to the reference differ); a \
         quantity join does not auto-convert, so convert one branch to the other's unit with `in`/`as` \
         to make them the same type"
            .to_string(),
    )
}

/// A message TAIL for two DISTINCT types that RENDER to the SAME name — "an Int64, but a value of type
/// Int64 is expected here" reads as a contradiction. This happens when a user type SHADOWS a prelude type
/// name: `(type Int64 (A))` binds `Int64` to a user sum, so `(f 5)` to a `(: x Int64)` parameter passes the
/// prelude `Int64` literal where the user's `Int64` sum is expected — two genuinely different types printed
/// identically. Name that the names collide because a declaration shadows a built-in, so the reader sees the
/// mismatch is real (not a compiler confusion) and knows to rename the shadowing type. Fires ONLY when the
/// rendered names are equal AND the types are NOT equal (a real, same-name clash); a normal mismatch (two
/// different names) renders unambiguously and needs no tail. No mechanical fix — the repair (rename the
/// shadowing declaration, or the reference) is the author's choice. Guards against a Var/Any side (which can
/// still unify — not a settled clash) so this only speaks for two CONCRETE same-named-but-distinct types.
fn same_name_distinct_type_hint(first: &Ty, other: &Ty, ncx: &NameCtx) -> Option<String> {
    if matches!(first, Ty::Var(_) | Ty::Any) || matches!(other, Ty::Var(_) | Ty::Any) {
        return None;
    }
    if first == other || first.render_name(ncx) != other.render_name(ncx) {
        return None;
    }
    Some(
        " — these are two DIFFERENT types printed with the same name (a declaration shadows a \
         built-in of that name); rename the shadowing type so the two are distinguishable"
            .to_string(),
    )
}

/// The message TAIL for a PEER-JOIN type clash — two `if` branches, two `match` arm bodies, two `list`
/// elements — that must share one type but do not. A peer clash is SYMMETRIC (neither side is the fixed
/// "expected" type, unlike an annotation/argument mismatch), so this first tries the structural-delta
/// hints (record field-set / tuple arity / collection axis / sum payload — themselves order-tolerant),
/// then the two DIRECTIONAL hints the annotation site carried but the join sites did not:
/// `option_payload_mismatch_hint` (one side is `(Option T)`, the other its payload `T` — "match it to
/// handle `None`") and `fn_not_applied_hint` (one side is an unapplied `(-> … T)` whose full application
/// yields the other side — "apply it to N more arguments") — each tried in BOTH orderings, so whichever
/// branch/arm/element is the Option wrapper or the unfinished call gets named. This brings the join sites
/// to fix-parity with the annotation site's hint chain. Tail only, no mechanical `Fix`: a peer clash's
/// repair (match the Option, finish the call, or retype a branch) is the author's choice, and the join
/// sites keep their own one-shot INT-LITERAL→FLOAT retype fix for the numeric case.
fn peer_type_delta_hint(first: &Ty, other: &Ty, ncx: &NameCtx) -> Option<String> {
    structural_delta_hint(first, other, ncx)
        // Two same-dimension DIFFERENT-scale quantities (`(list (Qty … km) (Qty … m))`) render to the SAME
        // name (`render_name` drops the scale), so the generic "must share one type: (Qty … meter) and
        // (Qty … meter)" reads as a contradiction. Route the peer join through the scale-mismatch hint (as
        // the `if`/`match` join sites already do at their own chain) so the list-element / peer clash names
        // the real cause (same dimension, different units — convert with `in`/`as`) rather than two
        // identical-looking types.
        .or_else(|| qty_scale_mismatch_hint(first, other))
        .or_else(|| option_payload_mismatch_hint(ncx, first, other))
        .or_else(|| option_payload_mismatch_hint(ncx, other, first))
        .or_else(|| fn_not_applied_hint(first, other, ncx))
        .or_else(|| fn_not_applied_hint(other, first, ncx))
}

/// An actionable message TAIL when `actual` is a FUNCTION value used where a NON-function `expected` is
/// required — the "forgot to call it" slip. A partial application `(h 1)` (h takes 2) or a bare function
/// name `h` used as a value has type `(-> … …)`; the generic "type mismatch: Int64 and (-> Int64 Int64)"
/// reads like an internal clash and never says the value is simply an UNAPPLIED function. This names the
/// slip and how to fix it — SUPPLY the missing arguments (rustc's "you might have forgotten to call this
/// function"). Fires ONLY when applying the function to its remaining N argument(s) would yield the
/// expected type (the fully-applied result `agrees_with` expected) — then "just apply it" is the true
/// repair. If the fully-applied result would STILL differ (a deeper mismatch), there is no "call it"
/// story, so no hint (honest-no-fix). No mechanical `Fix` — we cannot know WHICH argument values the
/// author meant — so this is a tail only, like `option_payload_mismatch_hint`. `expected` must be a
/// DEFINITE non-function (not a `Var`/`Any` that could still unify with the arrow, and not a `Fn` — a
/// fn-vs-fn signature mismatch is a real clash reported on its own terms, not a missing application).
fn fn_not_applied_hint(expected: &Ty, actual: &Ty, ncx: &NameCtx) -> Option<String> {
    if !matches!(actual, Ty::Fn(..)) || matches!(expected, Ty::Fn(..) | Ty::Var(_) | Ty::Any) {
        return None;
    }
    // Peel the curried arrows: how many arguments remain, and what the fully-applied result would be.
    let mut result = actual;
    let mut remaining = 0usize;
    while let Ty::Fn(_, r) = result {
        remaining += 1;
        result = r;
    }
    if !result.agrees_with(expected) {
        return None; // supplying the args would not produce the expected type — no "just call it" fix
    }
    let plural = if remaining == 1 { "" } else { "s" };
    Some(format!(
        " — the value is a function that hasn't been fully applied; apply it to {remaining} more \
         argument{plural} to get {}",
        expected.render_with_article(ncx)
    ))
}

/// The `(prefix, suffix, verb)` of a prelude CONVERSION that turns a value of type `actual` into the
/// `expected` type in ONE shot — the coercion-wrap repair for a mismatch the numeric model / text model
/// has a total conversion for. Today: `String` where `Bytes` is wanted → `(String.to-bytes …)` (the total
/// UTF-8 encode, `collections-and-text.md`). NOT the reverse (`Bytes → String` is `from-bytes : Bytes →
/// (Option String)`, FALLIBLE — no one-shot wrap that type-checks, so it stays unfixed rather than suggest
/// a spelling that just cascades to an Option mismatch). The wrap text is a member-access spelling the
/// resolver handles generically — the same shape as the `(Float64.of-int …)` / `(Int64.of …)` coercions,
/// NOT a hard-coded name lookup (`no-keys-outside-the-prelude`). `None` when no total conversion applies.
fn total_conversion_wrap(expected: &Ty, actual: &Ty) -> Option<(String, String, String)> {
    match (expected, actual) {
        (Ty::Bytes, Ty::String) => Some((
            "(String.to-bytes ".to_string(),
            ")".to_string(),
            "encode the string to its UTF-8 bytes with `String.to-bytes`".to_string(),
        )),
        // A fixed-width integer where a BigInt is expected — `(+ (BigInt.of 5) 3)`. `BigInt.of : ∀a.
        // (Int a) → BigInt` is TOTAL (every fixed int fits the unbounded BigInt), so wrapping the int
        // operand repairs the numeric mix in one shot — the BigInt twin of the int-width `.of` coercion.
        (Ty::BigInt, Ty::Int(_)) => Some((
            "(BigInt.of ".to_string(),
            ")".to_string(),
            "convert the integer to a BigInt with `BigInt.of`".to_string(),
        )),
        // A fixed-width integer where a Rational is expected — `(+ r 1)`, `r : Rational`. `Rational.of-int
        // : ∀a. (Int a) → Rational` is TOTAL (the whole rational `n/1`), so wrapping the int operand
        // repairs the mix in one shot — the Rational twin.
        (Ty::Rational, Ty::Int(_)) => Some((
            "(Rational.of-int ".to_string(),
            ")".to_string(),
            "convert the integer to a Rational with `Rational.of-int`".to_string(),
        )),
        // A Char where Int64 is expected — `(+ #\a 1)`. `Char.to-int : Char → Int64` is TOTAL (a char's
        // scalar value), so wrap it. Only when the expected width is the Int64 `Char.to-int` yields (a
        // narrower target would still mismatch after the wrap — leave that to the author).
        (Ty::Int(exp), Ty::Char) if exp.ground_signed() && exp.ground_width() == 64 => Some((
            "(Char.to-int ".to_string(),
            ")".to_string(),
            "convert the char to its Int64 scalar value with `Char.to-int`".to_string(),
        )),
        // A Char where a FLOAT is expected — `(+ #\a 1.0)`, `(< #\a 1.0)`. `Char.to-int` yields Int64, and
        // Cadenza never implicitly promotes Int64 → Float, so a bare `Char.to-int` re-fails; the TWO-STEP
        // `(Float{W}.of-int (Char.to-int …))` is the working repair (W = the expected float's width, so the
        // result matches the sibling's exact type). The float twin of the Char→Int64 wrap above.
        (Ty::Float(exp), Ty::Char) => {
            let module = format!("Float{}", exp.ground_width());
            Some((
                format!("({module}.of-int (Char.to-int "),
                "))".to_string(),
                format!(
                    "convert the char to its Int64 scalar value then to a float with `{module}.of-int`"
                ),
            ))
        }
        // A Char where a BIGINT is expected — `(+ #\a (BigInt.of 5))`. `Char.to-int` yields Int64 and
        // `BigInt.of : ∀a. (Int a) → BigInt` lifts it; a bare `Char.to-int` re-fails (Int64 vs BigInt, no
        // implicit promotion), so the working repair is the TWO-STEP `(BigInt.of (Char.to-int …))` — the
        // BigInt twin of the Char→Float wrap.
        (Ty::BigInt, Ty::Char) => Some((
            "(BigInt.of (Char.to-int ".to_string(),
            "))".to_string(),
            "convert the char to its Int64 scalar value then to a BigInt with `BigInt.of`"
                .to_string(),
        )),
        // A Char where a RATIONAL is expected — `(+ #\a (Rational.of-int 5))`. `Rational.of-int : ∀a. (Int
        // a) → Rational` lifts the `Char.to-int` scalar; a bare `Char.to-int` re-fails (Int64 vs Rational),
        // so the working repair is the TWO-STEP `(Rational.of-int (Char.to-int …))` — the Rational twin.
        (Ty::Rational, Ty::Char) => Some((
            "(Rational.of-int (Char.to-int ".to_string(),
            "))".to_string(),
            "convert the char to its Int64 scalar value then to a Rational with `Rational.of-int`"
                .to_string(),
        )),
        // A Symbol where a String is expected — `Symbol.to-string : Symbol → String` is TOTAL (recovers
        // the symbol's underlying content), so wrap it. The text-model twin of the String→Bytes case.
        (Ty::String, Ty::Symbol) => Some((
            "(Symbol.to-string ".to_string(),
            ")".to_string(),
            "recover the symbol's content string with `Symbol.to-string`".to_string(),
        )),
        _ => None,
    }
}

/// The `(prefix, suffix, verb)` of the conversion `(<Expected>.of …)` when a NUMERIC value of one
/// width/precision is supplied where a DIFFERENT one of the SAME numeric kind is `expected` — the
/// numeric-model coercion wrap (`06-numeric-model.sexp` — `(+ 2 (Int64.of 1))`). Covers BOTH an INTEGER
/// width mismatch (`Int8`/`Int64`) and a FLOAT precision mismatch (`Float32`/`Float64`); shared by every
/// site the same mismatch surfaces — an operator/ctor argument, a value/param `(: … T)` annotation, and an
/// annotated let-binder. GATED to an ALIASED expected width/precision (int {8,16,32,64}, float {32,64}) —
/// only those render to a BOUND type name, so `(<Expected>.of …)` resolves; a non-aliased `(Int 48)` would
/// suggest an unbound `Int48.of`, worse than no fix. The wrap text is a member-access spelling the resolver
/// handles generically, not a hard-coded name. Heuristic: which value to convert is the author's intent —
/// and the int `.of` is CHECKED (traps out of range) while the float `.of` is TOTAL (widen exact, narrow
/// rounds), reflected in the verb. `None` unless both types are the same numeric kind at an aliased width.
/// The SPELLING of an integer width type's MODULE — the thing a `.wrap`/`.of` conversion is a member of —
/// that actually RESOLVES for `it`'s width. An ALIASED width ({8,16,32,64}) has a pre-installed BOUND
/// module NAME (`UInt8`, `Int64`, `prelude::install`), so a bare `UInt8` resolves; a NON-aliased width
/// (`(UInt 4)` — a bit-field's own type) has NO bound name (`render_name` produces `UInt4`, an UNBOUND
/// identifier), so the only spelling that resolves is the type-CONSTRUCTOR applied form `(UInt 4)`. Returns
/// the resolvable spelling either way — a bound name for an aliased width, the `(UInt N)` / `(Int N)` form
/// otherwise — so a diagnostic that suggests `<module>.wrap` never names an identifier that is not in scope
/// (PR #377 review: the raw `render_name()` suggested `UInt4.wrap`, an unbound name). `signed`/`width` are
/// read via the grounding accessors (a fixed segment width type is always concrete here).
pub(crate) fn width_module_spelling(it: &crate::ty::IntTy) -> String {
    let signed = it.ground_signed();
    let width = it.ground_width();
    let stem = if signed { "Int" } else { "UInt" };
    if crate::ty::ALIASED_INT_WIDTHS.contains(&width) {
        // An aliased width has a bound module name — `Int8`, `UInt64`.
        format!("{stem}{width}")
    } else {
        // A non-aliased width has no bound name; the type-constructor form is the resolvable spelling.
        format!("({stem} {width})")
    }
}

/// A `.wrap`/`.of` conversion spelled off a width MODULE spelling. A BOUND module name takes the ordinary
/// dotted member `UInt8.wrap`; the type-CONSTRUCTOR form `(UInt 4)` cannot take a trailing `.op` token
/// (`(UInt 4).wrap` does not lex as one member access), so it uses the explicit member form `(. (UInt 4)
/// wrap)` — both of which the resolver accepts (verified: `((. (UInt 4) wrap) n)` type-checks). Keyed off
/// the leading `(` of the constructor form.
fn width_conversion_spelling(module: &str, op: &str) -> String {
    if module.starts_with('(') {
        format!("(. {module} {op})")
    } else {
        format!("{module}.{op}")
    }
}

fn int_coercion_wrap(
    expected: &Ty,
    actual: &Ty,
    ncx: &NameCtx,
) -> Option<(String, String, String)> {
    let (kind_matches, aliased, checked) = match (expected, actual) {
        (Ty::Int(exp), Ty::Int(_)) => (
            true,
            crate::ty::ALIASED_INT_WIDTHS.contains(&exp.ground_width()),
            true,
        ),
        (Ty::Float(exp), Ty::Float(_)) => (
            true,
            crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&exp.ground_width()),
            false,
        ),
        _ => (false, false, false),
    };
    if kind_matches && aliased {
        let n = expected.render_name(ncx);
        let verb = if checked {
            format!("convert to {n} with `{n}.of` (checked)")
        } else {
            format!("convert to {n} with `{n}.of`")
        };
        return Some((format!("({n}.of "), ")".to_string(), verb));
    }
    None
}

/// The coercion [`Fix`] that repairs a NUMERIC/TEXT mismatch between an `expected` type and the value
/// `arg` (of type `actual`), or `None` when no total one-shot conversion applies. Consolidates the
/// coercions the argument-unify chain offers — int-LITERAL→float retype (`3`→`3.0`, preferred over the
/// wrap), int→float (`of-int`, width-aware), int-width (`.of`), int-valued-float-literal drop
/// (`3.0`→`3`), and String→Bytes (`to-bytes`) — so EVERY site the same mismatch surfaces (an operator/ctor
/// argument, a VARIANT-CONSTRUCTOR PAYLOAD, an annotated LET-BINDER) offers the identical repair. Does NOT
/// include the sum-wrap ("wrap in `Some`", `wrap_variant_for`) — that is a distinct structural repair a
/// caller adds separately. Every fix is heuristic (`.of` is checked, retype/drop-`.0`/to-bytes are intent
/// guesses); the wrap text is a resolver-generic member-access spelling, not a hard-coded name.
fn numeric_text_coercion_fix(
    db: &mut Db,
    expected: &Ty,
    actual: &Ty,
    arg: StructId,
) -> Option<Fix> {
    // integer LITERAL where a float is expected → RETYPE it to a float literal (`3` → `3.0`), the same
    // one-shot repair the value-annotation site gives `(: 3 Float64)`. Preferred over the `of-int` WRAP
    // below: a literal has no reason to round-trip through a runtime conversion — just write it as a float.
    // (A non-literal int expression has no float spelling, so it falls through to the `of-int` wrap.)
    if let Ty::Float(_) = expected
        && let crate::ast::Struct::Atom(lid) = db.ast.get(arg)
        && let crate::ast::Leaf::Int { value, .. } = db.ast.leaf(*lid).clone()
        && let Some(n) = value.to_i128()
    {
        return Some(Fix::replace_heuristic(arg, format!("{n}.0")));
    }
    // int → float: `(<Float>.of-int …)`, widening a narrower int to Int64 first (`of-int : Int64 → Float`).
    if let (Ty::Float(_), Ty::Int(actual_int)) = (expected, actual) {
        let float_name = expected.render_name(&db.name_ctx());
        let (prefix, suffix, verb) =
            if actual_int.ground_width() == 64 && actual_int.ground_signed() {
                (
                    format!("({float_name}.of-int "),
                    ")".to_string(),
                    format!("convert the integer to {float_name} with `{float_name}.of-int`"),
                )
            } else {
                (
                    format!("({float_name}.of-int (Int64.of "),
                    "))".to_string(),
                    format!("convert to {float_name} with `{float_name}.of-int (Int64.of …)`"),
                )
            };
        return Some(Fix::wrap_heuristic(arg, prefix, suffix, verb));
    }
    // two integers / two floats of different width → `(<Expected>.of …)`.
    if let Some((prefix, suffix, verb)) = int_coercion_wrap(expected, actual, &db.name_ctx()) {
        return Some(Fix::wrap_heuristic(arg, prefix, suffix, verb));
    }
    // integer-valued float LITERAL where an int is expected → drop the `.0`.
    if let Ty::Int(exp) = expected
        && let crate::ast::Struct::Atom(lid) = db.ast.get(arg)
        && let crate::ast::Leaf::Float(dec) = db.ast.leaf(*lid).clone()
        && let Some(int_text) = integer_text_of_float_literal(&dec, *exp)
    {
        return Some(Fix::replace_heuristic(arg, int_text));
    }
    // a total prelude conversion (`String` → `Bytes` via `String.to-bytes`).
    if let Some((prefix, suffix, verb)) = total_conversion_wrap(expected, actual) {
        return Some(Fix::wrap_heuristic(arg, prefix, suffix, verb));
    }
    None
}

/// The coercion [`Fix`] that repairs a BARE NUMBER passed where a QUANTITY is expected — a plain `Int`/
/// `Float` `arg` against a `(Qty inner unit)` `expected` — by wrapping it in `(Qty.of <n> <unit>)` with the
/// unit read from the EXPECTED quantity type (`Unit::render` is the re-parseable `(Unit.base #"…")`
/// surface). This is the ARGUMENT-position twin of the dimensional-mismatch site's `Qty.of` wrap (which
/// offers the same repair for `(+ q 5)`): the same "give the bare number the required unit" fix wherever a
/// bare number meets a quantity. The bare number's INNER numeric type must match the quantity's inner type
/// (`Qty.of` grounds the value but does not convert its numeric type — an `Int64` bare into an `(Qty Int64
/// …)` param; a numeric mix would still fault after the wrap, so decline it and let the numeric-coercion
/// path speak). HEURISTIC — the author may have meant the magnitude of an existing quantity, but supplying
/// the required unit is the direct resolution. `None` unless `expected` is a `Qty` and `actual` a bare
/// matching-inner scalar.
fn qty_coercion_fix(expected: &Ty, actual: &Ty, arg: StructId) -> Option<Fix> {
    let Ty::Qty { inner, unit } = expected else {
        return None;
    };
    // The bare value must already be the quantity's inner numeric type — `Qty.of` grounds it to the unit,
    // it does not also convert Int↔Float / widen (that would still mismatch after the wrap).
    if !actual.agrees_with(inner) || matches!(actual, Ty::Qty { .. }) {
        return None;
    }
    let unit_src = unit.render();
    Some(Fix::wrap_heuristic(
        arg,
        "(Qty.of ",
        format!(" {unit_src})"),
        format!("give the number the required unit: `(Qty.of … {unit_src})`"),
    ))
}

/// The UNDERLYING structural type of the nominal NEWTYPE declared at `decl`, or `None` if `decl` is not
/// an erasable newtype (so it stays an ordinary boxed `Ty::Sum`). A newtype is a SINGLE-variant sum
/// whose runtime box is erased — the realization of `type-system.md §Nominal Is An Orthogonal Modifier`
/// (the tag "adds nothing to the value's runtime representation", §156). The returned type is the
/// nominal's `inner` (a caller wraps it as `Ty::Nominal { decl, name, inner }`); the underlying shape:
///  - 0 payloads (`(type Marker (The))`) → `Ty::Unit`,
///  - 1 payload  (`(type UserId (Mk Int64))`) → that payload's type,
///  - n payloads (`(type Point (Mk Int64 Int64))`) → `Ty::Tuple([payload tys…])` — the same shape a
///    multi-payload variant already boxes, so the erased value IS that tuple handle.
///
/// A declaration is an erasable newtype iff ALL of:
///  1. it has EXACTLY ONE variant (a multi-variant sum needs the discriminant → stays boxed),
///  2. it is MONOMORPHIC (no type parameters — a generic single-variant sum's erasure is a follow-up;
///     until then it stays boxed, which is still correct), AND
///  3. it is NON-RECURSIVE — the inner type does not transitively reach `decl` itself (via
///     `reaches_decl`, which follows every nested sum/nominal into its variants' payloads, catching a
///     direct self-reference `(type Stream (More (Tuple Int64 Stream)))` AND mutual recursion `(type A
///     (Mk B)) (type B (Mk A))`, load-order-independent). A recursive newtype's erased type is INFINITE
///     (`μX. …`), which `Ty::Nominal{inner}` cannot represent, so it stays boxed. A newtype whose inner is
///     a sum that does NOT cycle back (`(type Cached (Mk (Option Value)))`) DOES erase: its `Option` box
///     is genuine, but the outer `Mk` box (disc always 0) is pure overhead — removing it avoids the
///     double-box. (This replaced a blunt "contains ANY sum" guard that conservatively boxed every
///     newtype-over-a-sum, recursive or not.)
///
/// **Why this decodes payloads DIRECTLY (not via `scheme_of`).** It is precomputed once at load into
/// `Db::newtype_inner`, which `decode_ty` then consults to normalize a `Sum` into a `Nominal`. Using the
/// CACHED `scheme_of` here would cache the ctor's result type as `Sum` (the pre-normalization value),
/// and that stale scheme would then make `(Mk 42)` type as `Sum` even after `decode_ty` starts returning
/// `Nominal` — a representation split. Decoding the payload occurrences directly (via `typeval_of`,
/// uncached for types) avoids seeding any scheme with a soon-to-be-stale result, and the sum-free
/// restriction makes the result independent of which other decls are already cached.
pub(crate) fn newtype_underlying(db: &mut Db, decl: StructId) -> Option<Ty> {
    let (payload_count, payloads, params) = {
        let td = db.type_decl_by_occ(decl)?;
        // An OPEN sum (`(type T (Wrap Int64) .. r)`) is NEVER a newtype, even with a single NAMED variant:
        // the row variable stands for variants not named, so a value's discriminant is NOT statically the
        // sole variant — the sole constructor pattern does NOT cover it, and a match needs an open-tail
        // `_` arm (`type-system.md §206`). Erasing it to a `Ty::Nominal` would make the sole-ctor pattern
        // irrefutable (no discriminant to test), silently skipping the exhaustiveness check that requires
        // the `_`. Keep it a boxed `Ty::Sum` so `lower::build_tree`'s open-sum arm runs. (A CLOSED
        // single-variant sum erases as before — the default.)
        if td.open_tail.is_some() {
            return None;
        }
        // (1) exactly one variant. GENERICS are now IN scope — a generic newtype's template carries its
        // params as `Ty::Var(i)` (positional), substituted per-instantiation at `decode_ty`.
        if td.variants.len() != 1 {
            return None;
        }
        let v = &td.variants[0];
        (v.payloads.len(), v.payloads.clone(), td.params.clone())
    };
    // A NULLARY single variant is a nominal Unit (`(type Marker (The))`).
    if payload_count == 0 {
        return Some(Ty::Unit);
    }
    // Decode each payload TYPE occurrence directly to a template `Ty` (no `scheme_of` — see the doc
    // comment), mapping a declaration PARAM name to `Ty::Var(its index)`. A single payload's type IS the
    // underlying template; multiple payloads box as one tuple (the shape a multi-payload variant already
    // builds), so the underlying type is their `Ty::Tuple`.
    let mut tys = Vec::with_capacity(payload_count);
    for p in payloads {
        tys.push(decode_payload_template(db, p, &params)?);
    }
    let inner = if tys.len() == 1 {
        tys.pop().unwrap()
    } else {
        Ty::Tuple(tys.into())
    };
    // A RECURSIVE newtype ERASES TOO (Phase 2). Its inner's self-reference decodes to a finite
    // `Ty::Sum { decl }` LEAF (the μ-binder — `Ty::Sum` holds only the decl, not inline variants), so the
    // inner `(Option (Tuple Int64 Ty::Sum{Lst}))` is finite, NOT infinite. The old "infinite type" fear
    // was wrong. The equirecursive equality problem (a recursive nominal's inner diverging by derivation
    // path) is dissolved by Phase 1: `Ty::Nominal` is compared by `decl + args`, never `inner`. So there
    // is NO recursion guard — every single-variant sum erases; a recursive one's inner is a finite
    // machine-rep hint whose `Ty::Sum` back-edge terminates every reader.
    Some(inner)
}

/// Decode a newtype payload TYPE occurrence to a template `Ty`, mapping a declaration PARAMETER name
/// (`params[i]`) to `Ty::Var(i)` — the positional slot `decode_ty` later substitutes the instantiation's
/// arg into. A NON-param type occurrence (a concrete `Int64`, `(List Int64)`, a self-reference `Box`,
/// another sum `(Option …)`) decodes via the ordinary `typeval_of` (a self-reference / sum yields a
/// `Ty::Sum`, which the sum-free guard then rejects). Descends the structural type forms so a param
/// NESTED in the payload — `(List a)`, `(Tuple a Int64)` — becomes a `Var` at its position while the rest
/// decodes concretely. This is the load-time, `scheme_of`-free dual of the substitution `decode_ty` does.
fn decode_payload_template(db: &mut Db, occ: StructId, params: &[String]) -> Option<Ty> {
    // A bare name that IS a param → its positional `Ty::Var`.
    if let Some(name) = db.ast.as_name(occ)
        && let Some(i) = params.iter().position(|p| p == name)
    {
        return Some(Ty::Var(i as u32));
    }
    // A compound type form whose ARGUMENTS may mention params — descend the shapes that carry element
    // types, mapping each child through this template decoder. A head we don't special-case (a concrete
    // scalar, a sum/self-ref) falls through to `typeval_of` below.
    if let crate::ast::Struct::List(children) = db.ast.get(occ) {
        let children = children.clone();
        match children.first().and_then(|&h| db.ast.as_name(h)) {
            Some("Tuple") => {
                let mut elems = Vec::with_capacity(children.len() - 1);
                for &c in &children[1..] {
                    elems.push(decode_payload_template(db, c, params)?);
                }
                return Some(Ty::Tuple(elems.into()));
            }
            Some("List") if children.len() == 2 => {
                return Some(Ty::List(Box::new(decode_payload_template(
                    db,
                    children[1],
                    params,
                )?)));
            }
            _ => {}
        }
    }
    // A concrete type occurrence with no free param — decode normally. (A self-reference or another sum
    // yields a `Ty::Sum` here, which `newtype_underlying`'s sum-free guard rejects — staying boxed.)
    crate::eval::typeval_of(db, occ)
}

/// The `db.defs` index of the top-level def an application head names, if any — for typing a recursive
/// call by its scheme. Follows a `Ref` to a `Lambda` whose body matches a def's body occurrence. (The
/// infer-side sibling of `lower::callee_def_index`; kept here so infer does not depend on lower.)
fn callee_def_index_for_infer(db: &mut Db, head: StructId) -> Option<usize> {
    // The head resolves to a `Lambda { body }` for a named function (top-level, or a module member via
    // Case R) or a `Ref` chain to one; match its BODY back to the def index. A `Member` projection `(. m
    // f)` reduces to the field lambda's body — reached WITHOUT the general `lambda_of` β-reduction
    // machinery (which would inline a deep non-recursive call chain, an exponential cost on the hot infer
    // path): read the field VALUE via `member_value` and recurse on it. So a recursive MODULE MEMBER
    // called through the projection chain is typed by its registered internal def's scheme
    // (`modules::register_callable`), matching `lower::callee_def_index`, at no cost to ordinary calls.
    match crate::resolve::resolved_of(db, head) {
        crate::resolved::Resolved::Lambda { body, .. } => db.def_index_by_body(body),
        crate::resolved::Resolved::Ref { value } => callee_def_index_for_infer(db, value),
        crate::resolved::Resolved::Member { operand, key } => {
            match crate::eval::member_value(db, operand, &key) {
                crate::eval::Member::Field(v) => callee_def_index_for_infer(db, v),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The CDZ0405 non-exhaustive-HANDLER rejection, enriched with an "add the missing arm" fix (the effect
/// analogue of `non_exhaustive_sum_reject`'s missing-match-arm fix — `spec/capabilities/diagnostics.md`
/// §A Diagnostic Carries A Route To A Fix, realizing `capabilities-and-effects.md` §A Handler Discharges
/// Its Effect's "SHOULD identify the omitted operations"). Names the omitted operations and appends a
/// TEMPLATE arm per omission to the handler's arms LIST — each `(op (_p0 …) s (resume (trap …) s))` in
/// the canonical bare-op surface, with the op's arm arity of `_`-prefixed parameter binders (so it does
/// not itself warn unused), the state binder `s`, and a `(resume (trap "TODO: op") s)` body the author
/// fills in (the trap resume value type-checks whatever the op's result type is). Heuristic: the arms are
/// shaped right but their bodies are the author's to write. `handle_id` is
/// the `(handle seed (arms…) body)` form (internal shape after desugar); the fix anchors on its arms
/// LIST (child index 2), whose source span the in-place desugar preserved. Falls back to the plain
/// reject (no fix) if that arms node is absent.
fn non_exhaustive_handler_reject(
    db: &Db,
    handle_id: StructId,
    missing: &[crate::effects::MissingOp],
) -> Reject {
    // One template arm per missing op: `(op (_p0 …) s (resume (trap "TODO: op") s))`. A nullary op → empty
    // params; an N-ary op → N `_`-prefixed binders (so an unfilled placeholder does not itself warn
    // unused). The state binder is `s`; the RESUME VALUE is `(trap "TODO: op")` — a DIVERGING placeholder
    // the author replaces. `trap : ∀a. String → a`, so it type-checks as the resume value whatever the
    // operation's declared RESULT type is; a bare `unit` resume value cascaded to a CDZ0201 "a handler
    // resumes with a value of type Unit but the operation's result type is <T>" the moment the op returned
    // non-Unit (`(op get (-> Unit Int64))`), trading one fault for another — a fix must resolve in ONE
    // shot (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix, the same lesson as
    // the match add-arm fix's trap body). The `resume … s` scaffold is kept so the author sees the
    // canonical tail-resumptive shape to fill, and `s` stays used (no unused-binder warning).
    let arms: Vec<String> = missing
        .iter()
        .map(|m| {
            let binders: Vec<String> = (0..m.arity).map(|i| format!("_p{i}")).collect();
            format!(
                "({} ({}) s (resume (trap \"TODO: {}\") s))",
                m.name,
                binders.join(" "),
                m.name
            )
        })
        .collect();
    // The message NAMES the omitted operations AND spells the arm(s) to add inline — the guidance is
    // legible even without applying the structural fix (rustc "patterns `X` and `Y` not covered" style).
    let names: Vec<String> = missing.iter().map(|m| format!("`{}`", m.name)).collect();
    let message = format!(
        "this handler does not discharge every operation its effect declares: operation{} {} not \
         handled — a handle must discharge its effect's whole operation set; add {}",
        if missing.len() == 1 { "" } else { "s" },
        join_and_names(&names),
        arms.join(" "),
    );
    // The arms LIST is the handle form's 3rd child (internal shape `[handle, seed, arms, body]`). The
    // in-place desugar preserved its source span, so a structural `InsertArms` fix can splice the arms in
    // (unlike the fresh synthesized arms-list a rewrite-returning desugar would leave span-less).
    let arms_node = match db.ast.get(handle_id) {
        crate::ast::Struct::List(items) if items.len() == 4 => Some(items[2]),
        _ => None,
    };
    match arms_node {
        Some(arms_list) => Reject::coded(Code::HandlerNotExhaustive, message)
            .with_fix(Fix::insert_arms_heuristic(arms_list, arms)),
        None => Reject::coded(Code::HandlerNotExhaustive, message),
    }
}

/// Join names as `a`, `a and b`, or `a, b, and c` — the English list the "not handled" message reads
/// naturally with (matching the sibling `join_and` in `lower.rs`).
fn join_and_names(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// Check a handler arm's resume VALUE against the operation's declared RESULT type — the resume-value
/// companion of the perform-argument check. The value in `(resume value state)` is returned to the
/// perform site, so it must have the op's result type (`capabilities-and-effects.md` §Performing An
/// Operation Is Typed). A mismatch is CDZ0201. Only checks a TAIL resume in the arm body (the shipping
/// surface); an arm with no resume, or a non-tail resume, is out of scope (E4/E5) and not checked here.
fn check_resume_result_type(db: &mut Db, arm: &crate::resolved::HandleArm, out: &mut Vec<Reject>) {
    // The op's result type: instantiate the op value's `(meta t)` scheme (`(fn () (-> P… Result))`) and
    // peel to the final arrow result. `None` (a malformed op, or no scheme) → skip (its own fault surfaces).
    let mut fresh = Fresh::new();
    let Some(scheme) = crate::eval::scheme_of(db, arm.op, &mut fresh) else {
        return;
    };
    let mut result = crate::unify::instantiate(&scheme, &mut fresh);
    while let Ty::Fn(_, r) = result {
        result = *r;
    }
    // The resume value at the arm body's TAIL. The arm binds its params + state, but the RESUME VALUE's
    // type does not depend on those bindings here — its declared-type check is against a concrete op
    // result type, and a value like `true`/`42`/`(tuple …)` types independently of the (unsubstituted)
    // binders. So read the tail resume off the UN-substituted arm body and type its value.
    // EVERY tail resume value — a bare `(resume v s)`, OR each branch of an `if`/`match`-JOIN whose arms
    // tail-resume (`(if c (resume 1.5 s) (resume 1 s))`). A single mismatched resume is CDZ0201, but a
    // per-arm mismatch inside an if/match-join ESCAPED this check when it read only a top-level `Resume`
    // node — the join is an `If`/`Match`, not a `Resume`, so the check was SKIPPED and the ill-typed
    // resume (e.g. a Float value into an Int64 op result) reached emit as an INVALID module (v-cdz-smith
    // fuzzer bucket, routed by v-inference — the effect-resume check was not applied per-arm). Collect all
    // tail resume values through the join shapes and check EACH against the op result, so a mismatched arm
    // faults CDZ0201 per-arm rather than escaping to InvalidWasm.
    let mut values = Vec::new();
    collect_tail_resume_values(db, arm.body, &mut values);
    // No tail resume (abortive / non-tail) — out of scope for this check.
    for value in values {
        let value_ty = type_of(db, value);
        // WIDTH FIT-CHECK (breaker nw-class, nw8 result face): a DEFERRED integer LITERAL resume value
        // `(resume 999 s)` AGREES with any int width, so the `agrees_with` clash-check below does NOT fire
        // — yet `999` does not FIT a `UInt8` op RESULT. The perform site returns this value into a
        // `UInt8`-typed position (and, for a HOST op, across the component boundary), so it must pass the
        // same CDZ0302 range-check the argument direction + every annotated narrow position enforces. Run
        // it against the op's declared result type BEFORE the agreement check (a genuine kind-clash still
        // falls to CDZ0201 below). `width_fault_against_ty` handles narrow-int / Float32 / compound nesting.
        if let Some(reject) = width_fault_against_ty(db, value, &result) {
            trace!(target: "rcdzc::infer", value = value.0, "fault: resume value literal does not fit the operation's declared narrow result width (CDZ0302)");
            out.push(reject);
            continue;
        }
        if !value_ty.agrees_with(&result) {
            trace!(target: "rcdzc::infer", value = value.0, "fault: resume value's type does not match the operation's result type (CDZ0201)");
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a handler resumes with a value of type {} but the operation's result type is {}",
                    value_ty.render_name(&db.name_ctx()),
                    result.render_name(&db.name_ctx())
                ),
            )
            .at(value);
            // A resume value that mismatches the op result by a NUMERIC/TEXT coercion — `(resume x s)` with
            // `x:Int8` where the op returns Int64 → `(Int64.of x)` — has the same one-shot repair every other
            // expected-vs-actual site does (arg/annotation/let-binder/ctor-payload). Offer it here too.
            if let Some(fix) = numeric_text_coercion_fix(db, &result, &value_ty, value) {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
        }
    }
}

/// Collect EVERY tail resume value in an arm body `node` — a bare `(resume value …)`, and each branch of
/// an `if`/`match`-JOIN (or through `do`/`let` tails) whose tail is a resume. The per-arm companion of
/// [`tail_resume_value`], so the resume-value/result-type check ([`check_resume_result_type`]) applies to
/// each branch of a join rather than skipping a whole `If`/`Match` (which is not a `Resume` node). A
/// non-tail / abortive branch contributes nothing. Reads the ORIGINAL (un-substituted) body.
fn collect_tail_resume_values(db: &mut Db, node: StructId, out: &mut Vec<StructId>) {
    match resolved_of(db, node) {
        Resolved::Resume { value, .. } => out.push(value),
        Resolved::If { then_, else_, .. } => {
            collect_tail_resume_values(db, then_, out);
            collect_tail_resume_values(db, else_, out);
        }
        Resolved::Match { arms, .. } => {
            for (_, body) in arms {
                collect_tail_resume_values(db, body, out);
            }
        }
        Resolved::Let { body, .. } => collect_tail_resume_values(db, body, out),
        _ => {
            // A `do` sequence: its tail (last item) carries the resume.
            if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec)
                && let Some(&tail) = items.last()
            {
                collect_tail_resume_values(db, tail, out);
            }
        }
    }
}

/// Check a handler arm's tail NEXT-STATE against the handler's SEED type — the state-side companion of
/// [`check_resume_result_type`]. A handler threads a STATE fixed by its `init` seed; the next state in
/// `(resume value next-state)` continues that fold, so it MUST have the seed's type. A mismatch —
/// `(resume 5 "x")` under an Int64 seed — would change the state's type mid-fold (a type-confusion
/// miscompile if accepted). CDZ0201, anchored at the next-state, with the same numeric/text coercion fix
/// every expected-vs-actual site offers. GUARDED with `agrees_with` so an undetermined seed/state
/// (`Any`/`Var` — a recursive handler whose state type inference has not fixed, or an unconstrained seed)
/// is never falsely flagged; only a DEFINITE clash faults. Only a TAIL resume is checked (the shipping
/// surface), matching `check_resume_result_type`.
fn check_resume_next_state_type(
    db: &mut Db,
    init: StructId,
    arm: &crate::resolved::HandleArm,
    out: &mut Vec<Reject>,
) {
    let seed_ty = type_of(db, init);
    // An undetermined seed type carries no constraint — skip (never a false reject). The `agrees_with`
    // below would also pass, but bailing early keeps the common recursive-handler case cheap.
    if matches!(seed_ty, Ty::Any | Ty::Var(_)) {
        return;
    }
    // EVERY tail next-state — a bare resume AND each branch of an `if`/`match`-JOIN — for the same reason
    // as the resume-VALUE check: a per-arm next-state mismatch inside a join (`(if c (resume 1 "x")
    // (resume 1 s))` under an Int64 seed) escaped when only a top-level `Resume` was read, threading a
    // wrong-typed state mid-fold (a type-confusion miscompile). Collect through the join shapes + check each.
    let mut next_states = Vec::new();
    collect_tail_resume_next_states(db, arm.body, &mut next_states);
    for next_state in next_states {
        let next_ty = type_of(db, next_state);
        if !next_ty.agrees_with(&seed_ty) {
            trace!(target: "rcdzc::infer", next_state = next_state.0, "fault: resume next-state type does not match the handler's seed/state type (CDZ0201)");
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a handler resumes with a next-state of type {} but the handler's state type is {} \
                     (the seed fixes the state type; each resume threads a state of that type)",
                    next_ty.render_name(&db.name_ctx()),
                    seed_ty.render_name(&db.name_ctx())
                ),
            )
            .at(next_state);
            if let Some(fix) = numeric_text_coercion_fix(db, &seed_ty, &next_ty, next_state) {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
        }
    }
}

/// Collect EVERY tail next-state in an arm body — the next-state twin of [`collect_tail_resume_values`],
/// recursing through `if`/`match`-joins + `do`/`let` tails so a per-arm next-state mismatch in a join is
/// checked rather than skipped.
fn collect_tail_resume_next_states(db: &mut Db, node: StructId, out: &mut Vec<StructId>) {
    match resolved_of(db, node) {
        Resolved::Resume { next_state, .. } => out.push(next_state),
        Resolved::If { then_, else_, .. } => {
            collect_tail_resume_next_states(db, then_, out);
            collect_tail_resume_next_states(db, else_, out);
        }
        Resolved::Match { arms, .. } => {
            for (_, body) in arms {
                collect_tail_resume_next_states(db, body, out);
            }
        }
        Resolved::Let { body, .. } => collect_tail_resume_next_states(db, body, out),
        _ => {
            if let Some(items) = db.ast.as_form(node, "do").map(<[_]>::to_vec)
                && let Some(&tail) = items.last()
            {
                collect_tail_resume_next_states(db, tail, out);
            }
        }
    }
}

/// Whether a handler-arm match SCRUTINEE `scrutinee_binder` ESCAPES via a resume — i.e. it is referenced in
/// any arm's tail resume VALUE or NEXT-STATE (recursing if/match-joins + do/let tails, per the #4966
/// collectors). This is the PRE-REDUCTION signal a shell-reclaim fence (v-core-opt's FIND3 MatchTuple
/// scrutinee-dead-after-destructure fence) needs but CANNOT compute at select.rs: by the backend the resume
/// is threaded away, so a scrutinee re-referenced inside the (reduced) resume-continuation — `(match st …
/// (resume -1 st))` — is invisible there and the reclaim would deep-drop a shell the continuation still
/// holds (a UAF). Computed here at infer (where the resume is intact) over the ORIGINAL arm bodies, it lets
/// the reclaim fire only when `!scrutinee_resume_escapes`. `arm_bodies` are the handler arm bodies whose
/// resume threads that match's scrutinee; `scrutinee_binder` is the binding being reclaimed. (The rarer
/// CAPTURED-continuation subcase — the scrutinee captured into a reified resume-thunk — is caught by
/// `capture_escapes_via_body` on the closure side; this covers the direct resume value/next-state ref.)
// STAGED for v-core-opt's FIND3 (B) scrutinee-dead-after-destructure fence (v-memory-safety-directed): dead
// until the reclaim gate consults it (mirrors how `capture_escapes_via_body` was staged for the hcz gate).
// The `allow` retires when v-core-opt threads it (compute at infer → db map → consult at select.rs).
#[allow(dead_code)]
pub(crate) fn scrutinee_resume_escapes(
    db: &mut Db,
    scrutinee_binder: StructId,
    arm_bodies: &[StructId],
) -> bool {
    for &body in arm_bodies {
        let mut escaping = Vec::new();
        collect_tail_resume_values(db, body, &mut escaping);
        collect_tail_resume_next_states(db, body, &mut escaping);
        if escaping
            .into_iter()
            .any(|n| crate::effects::subtree_references_binder(db, n, scrutinee_binder))
        {
            return true;
        }
    }
    false
}

/// Whether the binding `binder` (a match SCRUTINEE's Param/LocalRef binder, extracted by the caller) ESCAPES
/// via ANY resume in the program — i.e. some `(resume <value> <next-state>)` carries `binder` (directly or
/// through a Ref/alias chain, per [`crate::effects::subtree_references_binder`]) out to the perform site or
/// forward as threaded state. This is the GLOBAL, PRE-REDUCTION resume-escape signal the FIND3 (B) shell-
/// reclaim fence (`sum_shell_reclaim_payload_ok`, select.rs) needs to WIDEN its conservative `Core::Call`-only
/// proxy: by the backend the resume is threaded away, so a scrutinee re-referenced inside the reduced
/// continuation — `(match st … (resume -1 st))` — is invisible to the Core-level dead-after-destructure walk,
/// and reclaiming its shell would deep-drop a value the continuation still holds (a UAF). The fence reclaims
/// an all-scalar-product OWNED dead-after scrutinee IFF it is a NON-binder (a fresh `Call`/materialize result
/// — cannot be resume-referenced) OR a binder that does NOT resume-escape (`!binder_resume_escapes`).
///
/// COMPLETENESS (v-memory-safety's crux — a false-negative is a UAF, a false-positive only a leak-safe missed
/// reclaim): collects TAIL and NON-TAIL resumes alike (a non-tail resume escapes its operands too — the tail-
/// only `scrutinee_resume_escapes` collectors would miss it), and `subtree_references_binder` follows the
/// Ref/alias chain (the chr1 family), so the four escape vectors (direct / alias / captured-continuation via
/// the closure-side `capture_escapes_via_body` / call-boundary via the dead-after + opaque-call backstop) are
/// covered by composition. The resume-operand list is cached in `db.resume_escape_operands` (built once on
/// first call; empty for a resume-free program → an O(1) miss on pure code).
// STAGED for v-core-opt's FIND3 (B) consult (v-memory-safety-directed): dead until the reclaim gate calls it
// (mirrors the staging of `scrutinee_resume_escapes` above + `capture_escapes_via_body` for the hcz gate).
#[allow(dead_code)]
pub(crate) fn binder_resume_escapes(db: &mut Db, binder: StructId) -> bool {
    if db.resume_escape_operands.is_none() {
        let mut operands = Vec::new();
        for ix in 0..db.ast.structure.len() {
            let id = <StructId as crate::arena::Index>::from_ix(ix);
            if let Resolved::Resume { value, next_state } = resolved_of(db, id) {
                operands.push(value);
                operands.push(next_state);
            }
        }
        db.resume_escape_operands = Some(operands);
    }
    // Take the cache out to iterate without holding a borrow across the `&mut db` calls below, then restore.
    let operands = db.resume_escape_operands.take().unwrap_or_default();
    let escapes = operands
        .iter()
        .any(|&n| crate::effects::subtree_references_binder(db, n, binder));
    db.resume_escape_operands = Some(operands);
    escapes
}

/// Check an application for type faults — the ONE rule's fault side. Instantiate the head's scheme and
/// unify each argument into its curried parameter; a unify failure is the conflicting-use type error.
/// A head with no `(meta t)` scheme (a type constructor, or a not-yet-typed value) is not checked here.
/// Whether `ty` is a DEFINITE non-function type — a ground value type that can never be applied (a
/// scalar Int/Bool/Float/String/Bytes/Unit, or a structural Record/Tuple/Sum/List/Map/Set value). Used
/// to turn "applying a non-function" into a coded reject: only a type KNOWN not to be a function faults,
/// so an UNDETERMINED head (`Ty::Any` — a not-yet-modeled construct or an unresolved variable) is NOT
/// flagged (it falls through to a clean decline, never a spurious reject). `Ty::Fn` is applyable;
/// `Ty::Type` (a type-value) is a constructor-like value handled on its own paths, so it is excluded too.
fn is_definite_non_function(ty: &Ty) -> bool {
    match ty {
        // Applyable or undetermined — not a "definitely can't apply this" case.
        Ty::Fn(_, _) | Ty::Any | Ty::Var(_) | Ty::Type => false,
        // Every other ground/structural value type is a non-function.
        _ => true,
    }
}

/// Whether `ty` is a DEFINITE non-sum type a variant pattern could never match — a scalar (`Int`, `Bool`,
/// `Float`, `Unit`), a `String`/`Bytes`, a `List`/`Map`/`Set`, a tuple, or a record. Excludes `Ty::Sum`
/// and `Ty::Nominal` (a variant pattern's legitimate targets) AND the UNDETERMINED types `Any`/`Var`/
/// `Type`/`Fn` — an unsolved scrutinee must still DECLINE, never be rejected here (a not-yet-inferred
/// parameter grounds to `Any`; rejecting it would fault a program a later solve types fine). So a variant
/// pattern over such a scrutinee is a genuine confusion, whereas over an undetermined one it is unknown.
fn definite_non_sum_scalar(ty: &Ty) -> bool {
    !matches!(
        ty,
        Ty::Sum { .. } | Ty::Nominal { .. } | Ty::Any | Ty::Var(_) | Ty::Type | Ty::Fn(_, _)
    )
}

/// Whether `ty` is DEFINITELY not a record — a record row operation (`Record.project`/`without`/`merge`/
/// `extend`/`with`) over it is a kind error, the same way member access on a non-record is. A NOMINAL
/// newtype over a record erases to that record, so strip the tag first (a member access sees through it).
/// An unconstrained `Any`/`Var` (a bare, not-yet-inlined parameter) is NOT definite — its real type flows
/// in at the call site — so it is not flagged here (mirrors the member-access `Ty::Any => {}` arm).
fn definite_non_record(ty: &Ty) -> bool {
    !matches!(ty.strip_nominal(), Ty::Record(_) | Ty::Any | Ty::Var(_))
}

/// The surface NAME of a RECORD row-operation prim (`project`/`without`/`merge`/`extend`/`with`), used to
/// render the "`Record.<op>` requires a record" message; `None` for any other prim. The set of record
/// row ops whose record operand must be a `Ty::Record`.
fn record_row_op_name(prim: Option<crate::resolved::Prim>) -> Option<&'static str> {
    match prim {
        Some(crate::resolved::Prim::RecordProject) => Some("project"),
        Some(crate::resolved::Prim::RecordWithout) => Some("without"),
        Some(crate::resolved::Prim::RecordMerge) => Some("merge"),
        Some(crate::resolved::Prim::RecordExtend) => Some("extend"),
        Some(crate::resolved::Prim::RecordWith) => Some("with"),
        _ => None,
    }
}

/// Whether `ty` is DEFINITELY not a tuple — the tuple twin of [`definite_non_record`]. A tuple row op
/// (`cat`/`split-at`/`pop`) over it is a kind error. `Any`/`Var` (an unconstrained param) is not definite.
/// (A tuple is structural, never nominal, so no `strip_nominal` is needed here.)
fn definite_non_tuple(ty: &Ty) -> bool {
    !matches!(ty, Ty::Tuple(_) | Ty::Any | Ty::Var(_))
}

/// Whether `ty` is DEFINITELY not an integer — a `bin` int/bits segment value must be an integer. An
/// `Any`/`Var` (a binder, an unreduced param, a bin-pattern binder that types as the decoded int) is
/// NOT definite, so it is never flagged.
fn definite_non_int(ty: &Ty) -> bool {
    !matches!(ty, Ty::Int(_) | Ty::Any | Ty::Var(_))
}

/// Whether `ty` DEFINITELY conflicts with `want` — a concrete type that is neither `want` nor an
/// unconstrained `Any`/`Var`. Used to check a `bin` utf8/bytes segment's value against its required kind.
fn definite_conflicts_with(ty: &Ty, want: &Ty) -> bool {
    !matches!(ty, Ty::Any | Ty::Var(_)) && ty != want
}

/// The surface NAME of a bin segment kind (`int`/`bits`/`bytes`/`utf8`) for a diagnostic message.
fn seg_kind_name(kind: &crate::resolved::SegKind) -> &'static str {
    match kind {
        crate::resolved::SegKind::Int { .. } => "integer",
        crate::resolved::SegKind::Bits { .. } => "bit-field",
        crate::resolved::SegKind::Bytes { .. } => "bytes",
        crate::resolved::SegKind::Utf8 { .. } => "utf8",
    }
}

/// The surface NAME of a TUPLE row-operation prim (`concat`/`split-at`/`remove`) whose operand(s) must be
/// a `Ty::Tuple`; `None` otherwise. `concat` takes two tuple operands, `split-at`/`remove` one. These are
/// the SURFACE spellings a program writes (post the consistent-naming cutover — the intrinsic Prims stay
/// `TupleCat`/`TuplePop`), so a diagnostic naming `Tuple.<op>` matches what the author typed.
fn tuple_row_op_name(prim: Option<crate::resolved::Prim>) -> Option<&'static str> {
    match prim {
        Some(crate::resolved::Prim::TupleCat) => Some("concat"),
        Some(crate::resolved::Prim::TupleSplitAt) => Some("split-at"),
        Some(crate::resolved::Prim::TuplePop) => Some("remove"),
        _ => None,
    }
}

/// Whether the match PATTERN at `pat` is headed by a VARIANT CONSTRUCTOR — `(C.Red)`, `(Some x)`, a bare
/// nullary variant name `None`, or such a pattern under a `(guard <pat> <cond>)` wrapper. Peels the guard,
/// then reads the pattern's constructor head the way the binding-pattern classifier does (a bare atom /
/// `(. Sum V)` member used whole, or a `(head arg…)` application's head) and asks `variant_owner_decl`
/// whether it names a sum's variant. `false` for a literal / bare binder / wildcard / tuple pattern — none
/// of which is variant-specific, so none conflicts with a scalar scrutinee.
fn pattern_is_variant_ctor(db: &mut Db, pat: StructId) -> bool {
    // Peel a `(guard <inner-pat> <cond>)` wrapper — the variant-ness is the inner pattern's.
    let inner = match db.ast.as_form(pat, "guard") {
        Some(g) if g.len() == 2 => g[0],
        _ => pat,
    };
    let head = match db.ast.get(inner) {
        crate::ast::Struct::Atom(_) => inner,
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor is the pattern itself.
            Some(first) if db.ast.as_name(first) == Some(".") => inner,
            Some(first) => first,
            None => return false,
        },
    };
    crate::eval::variant_owner_decl(db, head).is_some()
}

/// If `value` is the INITIALIZER occurrence of an ANNOTATED let-binding `((: <pat> T) value)` whose
/// declared type `T` DISAGREES with the initializer's inferred type, the declared type `T`; otherwise
/// `None`. This is the body-use side of the annotation-wins rule: a `let` binder reference (resolve's
/// `binder_in`, Case 1) resolves to a `Resolved::Ref { value: kv[1] }` — the initializer occurrence — so a
/// body use follows `value`'s inferred type. When the annotation and the initializer contradict, that
/// initializer type is the WRONG one to expose (the annotation is what the author declared and what the
/// binder-mismatch diagnostic told them to keep), so we hand back the annotation instead — suppressing the
/// contradictory downstream cascade.
///
/// Deliberately narrow, so a well-typed program is byte-identical. `value` must be the SECOND element
/// (`kv[1]`) of a two-element binding pair, whose pair sits in a `let`'s bindings-list
/// (`let_of_bindings_list`), whose LHS is an annotation `(: <pat> T)` with a resolvable type `T`, AND `T`
/// must disagree with the initializer's inferred type (`!agrees_with`). An agreeing annotation, a bare-name
/// binding, a destructuring pattern, or a non-let use all return `None` — the caller then follows the
/// initializer's type exactly as before.
fn annotated_let_binder_ty(db: &mut Db, value: StructId) -> Option<Ty> {
    // `value` is a binding pair's second element: pair = [lhs, value], and the pair's parent is a let's
    // bindings-list.
    let pair = db.parent_of(value)?;
    let kv = match db.ast.get(pair) {
        crate::ast::Struct::List(kv) if kv.len() == 2 && kv[1] == value => kv.clone(),
        _ => return None,
    };
    let bindings_occ = db.parent_of(pair)?;
    crate::resolve::let_of_bindings_list(db, bindings_occ)?;
    // The LHS must be an annotation `(: <pat> T)` with a resolvable type value `T`.
    let ann = db.ast.as_form(kv[0], ":")?;
    if ann.len() != 2 {
        return None;
    }
    let ty_expr = ann[1];
    let annot_ty = crate::eval::typeval_of(db, ty_expr)?;
    // Only override when the annotation and the initializer genuinely CONTRADICT — an agreeing (or
    // deferred/`Any`) annotation leaves the initializer type in force (byte-identical to before).
    let value_ty = type_of(db, value);
    if annot_ty.agrees_with(&value_ty) {
        return None;
    }
    Some(annot_ty)
}

/// The SET of parameter binders the lambda `head`'s body REFERENCES — the binder identity every body
/// reference resolves to, collected in ONE structural walk. A parameter present here is USED (its argument
/// appears substituted in the reduced body, so that argument's faults are already collected there and it
/// need not be re-descended); one ABSENT is DEAD (the body ignores it, so its argument must be descended
/// for its own faults). Replaces a per-parameter `references_binder` scan — asking this per argument was a
/// full-body walk each, so a WIDE application `(f a0 … aN)` was O(args × body) = O(N²); one walk + O(1)
/// membership per argument is O(body + args).
///
/// A body reference resolves (via resolve's `binder_in`) to a `Resolved::Ref` whose transitive chain ends
/// at the parameter's binder, or to a `Resolved::Param { binder }`. The set collects EVERY identity a
/// reference matches — each link of a `Ref` chain plus a terminal `Param`'s binder — so `p ∈ set` is
/// byte-identical to the old `references_binder(body, p)` (which returned true if `p` equalled ANY chain
/// link or the terminal binder). `Ref`/`Param` nodes are leaf references (not recursed); every other node
/// recurses its raw AST children, visiting every value position (no reduction, no lowering).
fn referenced_binders(db: &mut Db, body: StructId) -> std::collections::HashSet<StructId> {
    fn walk(db: &mut Db, node: StructId, out: &mut std::collections::HashSet<StructId>) {
        match resolved_of(db, node) {
            Resolved::Param { binder } => {
                out.insert(binder);
            }
            Resolved::Ref { mut value } => {
                // Follow the ref chain, recording every link — a chain end at a `Param` records its binder.
                loop {
                    out.insert(value);
                    match resolved_of(db, value) {
                        Resolved::Ref { value: next } => value = next,
                        Resolved::Param { binder } => {
                            out.insert(binder);
                            break;
                        }
                        _ => break,
                    }
                }
            }
            _ => {
                if let crate::ast::Struct::List(children) = db.ast.get(node) {
                    let children = children.clone();
                    for c in children {
                        walk(db, c, out);
                    }
                }
            }
        }
    }
    let mut out = std::collections::HashSet::new();
    walk(db, body, &mut out);
    out
}

/// The diagnostic code for a COLLECTION HOMOGENEITY violation between two element types that do not
/// unify. A HETEROGENEOUS COLLECTION is CDZ0201 (a MALFORMED collection), UNIFORMLY — a list/map/set
/// whose elements/keys/values do not share one type is ill-formed AS A COLLECTION, regardless of HOW
/// the element types differ (a cross-kind scalar clash `(list 1 true)`, a no-silent-promotion numeric
/// mix `(list 1 2.5)`, or two same-kind-different-shape compounds). This is the collection-homogeneity
/// taxonomy rule (collections-and-text.md §A Collection's Homogeneity Violation Is A Malformed
/// Collection): the map/set homogeneity checks already code it CDZ0201, and the `List.push`/`update`/
/// `concat` checks do too, so every collection homogeneity fault agrees. CDZ0203 (`TypeMismatch`) is
/// reserved for a genuine two-types-must-AGREE UNIFICATION conflict — an `if`'s branches, a value
/// annotation `(: e T)`, a cross-shape comparison — NOT a collection's internal heterogeneity. Takes
/// the peer types for signature stability with the call sites, but the code is now uniform.
///
/// Returning `Code::Malformed` (`CDZ0201`) is the reject code every collection-homogeneity fault
/// carries, and returning ONE code regardless of the peer types `_a`/`_b` is what makes it uniform:
//= spec/capabilities/collections-and-text.md#a-collection-s-homogeneity-violation-is-a-malformed-collection
//# A construction whose elements, keys, or values do not share one type MUST be rejected as a malformed collection with the diagnostic code `CDZ0201`, so that a heterogeneous collection is treated as the collection being unbuildable rather than as a value of some other type.
//= spec/capabilities/collections-and-text.md#a-collection-s-homogeneity-violation-is-a-malformed-collection
//# The malformed-collection code a heterogeneous construction takes MUST be the same code independent of the collection kind — list, map, or set — so that the diagnostic names one category rather than one per collection kind.
//= spec/capabilities/collections-and-text.md#a-collection-s-homogeneity-violation-is-a-malformed-collection
//# The malformed-collection code a heterogeneous construction takes MUST be the same code independent of how the construction is written, whether a literal or a functional-construction operation such as append, replace-at-index, concatenate, or insert, so that the code does not vary with the construction form.
//= spec/capabilities/collections-and-text.md#a-collection-s-homogeneity-violation-is-a-malformed-collection
//# The malformed-collection code a heterogeneous construction takes MUST be the same code independent of how the element types differ, whether a cross-kind clash, a numeric mix that does not silently promote, or two same-kind values of different shape, so that a consumer branching on the code sees one category for "this collection is not homogeneous" rather than a code that varies with the incidental shape of the disagreement (*diagnostics.md §Every Diagnostic Has A Stable Code*).
fn list_homogeneity_code(_a: &Ty, _b: &Ty) -> Code {
    Code::Malformed
}

/// Check every `(Unit.define #"name" base num den)` declaration for a CONFLICT — a name bound to two
/// different conversions (`units-of-measure.md` §A Named Unit's Conversion Is Unique) — and push CDZ0502
/// for each. A name conflicts when its reduced unit (`base` scaled by `num/den`) differs from the
/// BUILT-IN family unit of that name, or from an EARLIER `Unit.define` of it; an agreeing redeclaration
/// is admissible (a `Unit` compares by dimension AND scale, so "agrees" is exact). Runs once over
/// `db.unit_defines` (these are top-level declarations, not def bodies, so `type_errors` never reaches
/// them). `pub(crate)` — called from `compile::collect_faults`.
pub(crate) fn check_unit_defines(db: &mut Db, out: &mut Vec<Reject>) {
    // The declaration rows, in source order (name, base occurrence, scale).
    let defines: Vec<(String, StructId, i128, i128)> = db.unit_defines.clone();
    // The reduced unit for each name seen so far (built-ins seeded lazily on first mention).
    let mut seen: std::collections::HashMap<String, crate::ty::Unit> =
        std::collections::HashMap::new();
    for (name, base_occ, num, den) in &defines {
        // Reduce this declaration to the unit it denotes.
        let Some(base) = crate::eval::unit_of(db, *base_occ) else {
            continue; // a malformed base — its fault surfaces via the ordinary descent
        };
        let Some(this_unit) = base.scaled(*num, *den) else {
            continue;
        };
        // Compare against the BUILT-IN family unit of this name (reduce its registry entry once).
        if let Some((dim_pairs, bnum, bden)) = db.unit_families.get(name).cloned() {
            let mut u = crate::ty::Unit::one();
            for (b, e) in &dim_pairs {
                u = u.mul(&crate::ty::Unit::base(b.clone()).pow(*e));
            }
            if let Some(builtin) = u.scaled(bnum, bden)
                && builtin != this_unit
            {
                // Anchor at the declaration's base-unit occurrence (`base_occ` = `(Unit.of #"…")`), the
                // one node of this `(Unit.define …)` the scan kept — so the conflict carries a
                // `file:line:col` at the offending declaration instead of an unanchored `cdz:`/`file:`
                // prefix.
                out.push(
                    Reject::coded(
                        Code::UnitConflict,
                        format!(
                            "unit `{name}` is already a built-in unit with a different conversion"
                        ),
                    )
                    .at(*base_occ),
                );
                continue;
            }
        }
        // Compare against an EARLIER declaration of the same name.
        match seen.get(name) {
            Some(prior) if *prior != this_unit => {
                out.push(
                    Reject::coded(
                        Code::UnitConflict,
                        format!("unit `{name}` is declared twice with different conversions"),
                    )
                    .at(*base_occ),
                );
            }
            _ => {
                seen.insert(name.clone(), this_unit);
            }
        }
    }
}

/// Reject a MALFORMED `(Unit.define #"name" base num den)` top-level declaration — one whose shape does
/// not match what `db::scan_unit_defines` accepts (exactly 4 args, a SYMBOL name, INTEGER num + den). The
/// scan SILENTLY DROPS a malformed one (its guard is one big `&&` chain), so a `Unit.define` written with
/// the wrong arity / a string name / a fractional scale registers NO family unit — and a later use of
/// that unit surfaces only as "unknown unit `furlong`" (naming REAL units, never hinting the author's own
/// `Unit.define` was malformed), the real defect lost. Reject it here CDZ0201 at the `Unit.define` form so
/// the fault names the actual shape — the `Unit.define` analogue of the malformed-extern / -effect
/// scan-and-drop checks. Only fires on a form whose head IS `(. Unit define)` (so a WELL-FORMED define, or
/// any non-`Unit.define` call, is untouched). Walked over every arena node.
pub(crate) fn check_malformed_unit_defines(db: &mut Db, out: &mut Vec<Reject>) {
    let node_count = db.ast.structure.len();
    for id in (0..node_count as u32).map(crate::ast::StructId) {
        // The form must be a call `((. Unit define) arg…)` — read its head + args off the raw list.
        let crate::ast::Struct::List(items) = db.ast.get(id) else {
            continue;
        };
        let Some((&head, args)) = items.split_first() else {
            continue;
        };
        // The head must be `(. Unit define)`.
        let is_unit_define = db.ast.as_form(head, ".").is_some_and(|dot| {
            dot.len() == 2
                && db.ast.as_name(dot[0]) == Some("Unit")
                && db.ast.as_name(dot[1]) == Some("define")
        });
        if !is_unit_define {
            continue;
        }
        let args = args.to_vec();
        // The shape `scan_unit_defines` requires: exactly 4 args, a SYMBOL name (`#"furlong"`), and INTEGER
        // num + den. Any deviation is what the scan silently dropped.
        let well_formed = args.len() == 4
            && db.ast.as_sym(args[0]).is_some()
            && db.ast.as_int(args[2]).is_some()
            && db.ast.as_int(args[3]).is_some();
        if !well_formed {
            out.push(
                Reject::coded(
                    Code::Malformed,
                    "a `Unit.define` is `(Unit.define #\"name\" <base-unit> <num> <den>)` — a symbol \
                     name, a base-unit expression, and two integer scale factors",
                )
                .at(id),
            );
        }
    }
}

/// Reject a QUANTITY LITERAL / `Unit.of` naming a unit that is neither a built-in family
/// (`unit_families`) nor a user `Unit.define` — `5zorks` / `5gram` (a plausible-but-undefined unit). The
/// unit fails to reduce (`eval::unit_of` → `None`), so `Qty.of`'s type falls through to a non-`Qty` and
/// the value later declines "no machine representation" — a GENERIC message that never mentions the unit.
/// Name it here (CDZ0201, a malformed quantity literal, the code a malformed numeric literal gets). A
/// CONFIDENT typo of a known unit (`metre`→`meter`) gets a "did you mean?"; an unrecognized name that is
/// NOT a near-miss (an abbreviation like `mph`, whose edit-distance neighbours are unrelated data-rate
/// units) instead gets ACTIONABLE guidance — how to COMPOSE a compound unit and how to DECLARE a new one
/// with `Unit.define` — rather than a misleading "closest matches" list.
/// Well-formedness independent of reachability — checked over every `(Unit.of #"…")` occurrence.
pub(crate) fn check_unknown_units(db: &mut Db, out: &mut Vec<Reject>) {
    // The known unit vocabulary (built-in families + every user `Unit.define` name) is built LAZILY — only
    // when a first candidate `(Unit.of …)` is actually found — so a program with NO unit applications (the
    // common case) never allocates it.
    let mut known: Option<Vec<String>> = None;
    // Scan only USER nodes, not the full structure (which appends the O(prelude) built-in bindings + every
    // evaluator-synthesized β-copy). A genuine unknown-unit `(Unit.of #"zorks")` fault is reported `.at(id)`,
    // and a fault must anchor at a USER node to carry a source span — a prelude / synthesized anchor has none
    // and is nulled (or relocated to its user origin) by `compile::sanitize_origin`. The prelude never applies
    // `Unit.of` to an UNKNOWN unit (it DEFINES the known families), and a β-copy of a user `(Unit.of …)` still
    // has its ORIGINAL user occurrence in-range (which this scan covers); `dedup_faults` collapses any copy.
    // So bounding to `user_node_count` never drops a reportable fault, and skips the built-in-node bulk that a
    // unit-free program (the common case — e.g. the whole ML compiler) would otherwise walk for nothing
    // (~6% of a large real compile: this pass had inclusive-time dominance on `emit-db.cdz`, which uses no units).
    //
    // ADVERSARIAL WITNESS (PR#1101 review, corpus-bugfix-confirmed): could an `eval` reconstruction GRAFT a
    // fresh synth `(Unit.of #"zorks")` node (id ≥ `user_node_count`, so skipped) with NO in-range user origin?
    // No — `(eval (quote (Qty.of 5 (Unit.of #"zorks"))))` declines CDZ0101 up front, because `eval` refuses to
    // reconstruct ANY quote carrying a `#"…"` SYMBOL literal (strings/ints/floats reconstruct; a bare symbol
    // does not — verified `(eval (quote #"hi"))` → CDZ0101 vs `(eval (quote "hi"))`/`(quote 42)` clean). A
    // `Unit.of` REQUIRES a symbol arg, so a quoted `Unit.of` trips that decline BEFORE any runnable synth
    // `Unit.of` node is built — the "escaping synth node" never materializes. And the quoted `Unit.of`'s own
    // `#"zorks"` still gets CDZ0201'd as an in-range user literal. Pinned by the eval-quoted assertion in
    // `check_unknown_units_scans_only_user_nodes_not_the_prelude` + a corpus tripwire (the CDZ0101 decline)
    // that flips the day `eval` learns to reconstruct symbol literals — at which point re-examine this bound.
    let node_count = db.user_node_count();
    #[cfg(test)]
    crate::db::CHECK_UNKNOWN_UNITS_SCAN_NODES.with(|c| c.set(c.get() + node_count as u64));
    for id in (0..node_count).map(StructId) {
        // A `(Unit.of #"name")` application whose name is a symbol/string literal. Dispatch through
        // `resolved_ref` (a BORROW, not a `resolved_of` clone): this scans EVERY node of every program, and
        // the vast majority are not `Apply`, so cloning the whole `Resolved` per node just to test the
        // variant was pure churn (on a large unit-FREE program this whole pass was ~30% of compile). The
        // `head` is Copy; `args.first()` is read through the borrow (no `args` Rc clone needed here).
        let (head, name_occ) = match crate::resolve::resolved_ref(db, id) {
            crate::resolved::Resolved::Apply { head, args } => match args.first() {
                Some(&name_occ) => (*head, name_occ),
                None => continue,
            },
            _ => continue,
        };
        if crate::eval::meta_apply_of(db, head) != Some(crate::resolved::Prim::UnitOf) {
            continue;
        }
        let name = match resolved_of(db, name_occ) {
            crate::resolved::Resolved::SymbolConst(s) | crate::resolved::Resolved::Str(s) => s,
            _ => continue, // a non-literal unit argument — not a static unknown-unit case
        };
        let known = known.get_or_insert_with(|| {
            let mut v: Vec<String> = db.unit_families.keys().cloned().collect();
            v.extend(db.unit_defines.iter().map(|(n, _, _, _)| n.clone()));
            v
        });
        if known.contains(&name) {
            continue; // a known family / user-defined unit
        }
        // Two-tiered hint. A CONFIDENT typo of a real unit (`metre`→`meter`, `secnd`→`second`) gets a
        // "did you mean?" AND a heuristic Replace fix on the NAME occurrence, so the suggestion is
        // machine-applyable (the unit-literal analogue of a member-access / import-name did-you-mean).
        // Otherwise DON'T fall back to `did_you_mean`'s raw "closest matches" list: for an ABBREVIATION
        // like `mph`/`kmh`/`rpm` the nearest units by edit distance are semantically unrelated noise
        // (`mph` → `bps`, `mbps`, `bit`) — misleading, and it never says what to DO. Give ACTIONABLE
        // guidance instead — a compound unit is COMPOSED from known units, and a genuinely new named unit
        // is introduced with `Unit.define` (the example carries the actual unknown name, so it is a
        // copy-paste starting point); that far-miss guidance is not one mechanical edit, so no fix.
        let mut fix = None;
        let hint = match crate::diag::suggest::nearest(&name, known.iter()) {
            Some(near) => {
                // WARNING: The unit name is a LITERAL whose delimiter must be preserved so the applied edit
                // re-renders a valid `Unit.of` argument: a symbol `#"metre"` (`Leaf::Sym`) → `#"meter"`,
                // a plain string `"metre"` (`Leaf::Str`) → `"meter"`. Detect which from the AST node —
                // a bare `meter` would re-read as a NAME and break the `(Unit.of …)` form.
                let replacement = if db.ast.as_sym(name_occ).is_some() {
                    format!("#\"{near}\"")
                } else {
                    format!("\"{near}\"")
                };
                fix = Some(crate::diag::Fix::replace_heuristic(name_occ, replacement));
                format!(" — did you mean `{near}`?")
            }
            None => format!(
                " — compose a compound unit from known units \
                 (e.g. miles per hour is `(Unit./ (Unit.of #\"mile\") (Unit.of #\"hour\"))`), \
                 or declare it with `(Unit.define #\"{name}\" <base-unit> <num> <den>)`"
            ),
        };
        let mut reject = Reject::coded(
            Code::Malformed,
            format!(
                "unknown unit `{name}` — it is neither a built-in unit nor declared by a \
                 `Unit.define`{hint}"
            ),
        )
        .at(id);
        if let Some(fix) = fix {
            reject = reject.with_fix(fix);
        }
        out.push(reject);
    }
}

/// Whether `id` is a UNIT-BUILDER form — its head (or itself, for the nullary `Unit.one`) is one of the
/// `Unit.*` prims (`one`/`base`/`of`/`*`/`/`/`^`/`define`). Used to tell a genuine unit expression that
/// merely fails to REDUCE (a `(Unit.of #"zorks")` naming an unknown unit — handled by
/// `check_unknown_units` with a did-you-mean) from a value that is NOT a unit form at all (a literal, a
/// tuple), so `Qty.of`'s not-a-unit reject fires only on the latter and never shadows the richer
/// unknown-unit message.
fn is_unit_builder_form(db: &mut Db, id: crate::ast::StructId) -> bool {
    use crate::resolved::Prim;
    let head = match db.ast.get(id) {
        crate::ast::Struct::List(kids) => kids.first().copied().unwrap_or(id),
        _ => id,
    };
    matches!(
        crate::eval::meta_apply_of(db, head),
        Some(
            Prim::UnitOne
                | Prim::UnitBase
                | Prim::UnitOf
                | Prim::UnitMul
                | Prim::UnitDiv
                | Prim::UnitPow
                | Prim::UnitDefine
        )
    )
}

/// A `Unit.*`/`Unit./`/`Unit.^` composition whose `eval::unit_of` returned `None` has a MALFORMED OPERAND —
/// a non-unit factor (`(Unit.* (Unit.base #"m") 5)`) or a non-integer exponent (`(Unit.^ u 2.5)`). Walk the
/// composition to NAME the offending operand (CDZ0201). Without this, `Qty.of`'s not-a-unit check SKIPS a
/// unit-builder-headed arg (deferring to the builder's own validation — see `is_unit_builder_form`), but the
/// builder had NONE: the composition silently reduced to `Any`, `cdz check` passed, and `cdz compile` leaked
/// "function return type has no machine representation" — a check-miss + poor-compile-message gap. Handles the
/// EXPLICIT builder members (`UnitMul`/`UnitDiv`/`UnitPow`); recurses into nested compositions. Returns true
/// if it pushed a fault (the first bad operand only — one actionable message, not a cascade).
fn check_unit_composition(db: &mut Db, id: crate::ast::StructId, out: &mut Vec<Reject>) -> bool {
    use crate::resolved::Prim;
    let Resolved::Apply { head, args } = resolved_of(db, id) else {
        return false;
    };
    let Some(prim) = crate::eval::meta_apply_of(db, head) else {
        return false;
    };
    match prim {
        // A PRODUCT / QUOTIENT composes two UNITS — each operand must reduce to a unit.
        Prim::UnitMul | Prim::UnitDiv if args.len() == 2 => {
            let op = if prim == Prim::UnitDiv {
                "Unit./"
            } else {
                "Unit.*"
            };
            for &operand in args.iter() {
                if crate::eval::unit_of(db, operand).is_some() {
                    continue; // a valid unit factor
                }
                // Not a unit. If it is itself a composition, recurse to the DEEPER bad operand; otherwise
                // THIS operand is the fault (a literal, a plain value where a unit was expected).
                if !check_unit_composition(db, operand, out) {
                    let t = type_of(db, operand);
                    out.push(
                        Reject::coded(
                            Code::Malformed,
                            format!(
                                "`{op}` composes two UNITS, but this operand is not a unit — write a \
                                 unit expression (e.g. `(Unit.base #\"meter\")`), not a {} value",
                                t.render_name(&db.name_ctx())
                            ),
                        )
                        .at(operand),
                    );
                }
                return true; // report the first bad operand only
            }
            false
        }
        // A POWER raises a UNIT to a compile-time INTEGER — the base must be a unit, the exponent an integer.
        Prim::UnitPow if args.len() == 2 => {
            let base = args[0];
            if crate::eval::unit_of(db, base).is_none() && !check_unit_composition(db, base, out) {
                let t = type_of(db, base);
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "`Unit.^` raises a UNIT to a power, but this base is not a unit — write a \
                             unit expression (e.g. `(Unit.base #\"meter\")`), not a {} value",
                            t.render_name(&db.name_ctx())
                        ),
                    )
                    .at(base),
                );
                return true;
            }
            if !matches!(resolved_of(db, args[1]), Resolved::Int(_)) {
                let t = type_of(db, args[1]);
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "`Unit.^`'s exponent must be a compile-time integer (e.g. `2` for a square), \
                             but this is a {} value",
                            t.render_name(&db.name_ctx())
                        ),
                    )
                    .at(args[1]),
                );
                return true;
            }
            false
        }
        // A `Unit.*`/`Unit./`/`Unit.^` at the WRONG ARITY — `(Unit.^ u)` (one arg), `(Unit.* u)` — falls
        // through the `args.len() == 2` arms above to here. `unit_of` declined it (an under/over-applied
        // builder is not a unit), and the M227 partial-builtin-arity check does NOT fire because the form is
        // CONSUMED (its parent is the `Qty.of`/composition that fed it), so it leaked past `cdz check` →
        // opaque "no machine representation" at compile. Name the arity (PR#506 Copilot finding).
        Prim::UnitMul | Prim::UnitDiv | Prim::UnitPow => {
            let (op, n) = match prim {
                Prim::UnitDiv => ("Unit./", 2),
                Prim::UnitPow => ("Unit.^", 2),
                _ => ("Unit.*", 2),
            };
            let shape = if op == "Unit.^" {
                format!("`({op} <unit> <integer>)`")
            } else {
                format!("`({op} <unit> <unit>)`")
            };
            out.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "`{op}` takes {n} operands, but {} were given — write {shape}",
                        args.len(),
                    ),
                )
                .at(id),
            );
            true
        }
        // A `Unit.of`/`Unit.base` whose unit-NAME argument is not a SYMBOL — `(Unit.of 42)`, `(Unit.base
        // (tuple 1 2))`. A unit builder names its unit with a `#"…"` symbol (`unit_of` declined, so the arg
        // is not a symbol); a bare-NAME arg is caught with a `#"name"` fix in the unbound-name handler, but
        // a non-name non-symbol (an integer/string/compound) reached NEITHER that nor `check_unit_composition`
        // and leaked. Name it. (Wrong arity — `(Unit.of)` / `(Unit.of a b)` — also lands here since the arg
        // read below finds no single symbol; the message names the symbol requirement, the actionable fix.)
        Prim::UnitOf | Prim::UnitBase
            // A single bare-NAME arg (`(Unit.of foot)`) is DELIBERATELY left to the unbound-name handler,
            // which names it with a `#"foot"` replace fix — richer than this generic message. A single
            // valid SYMBOL arg (`(Unit.of #"furlong")`) is a well-formed unit NAME — the arg IS a symbol,
            // so this "not a SYMBOL" reject is WRONG; an UNKNOWN such unit is `check_unknown_units`'s job
            // (CDZ0201 with a did-you-mean), and a KNOWN one reduces fine. Without excluding it, `5 furlong`
            // (an unknown unit) got a SPURIOUS "names its unit with a SYMBOL, but this is a Symbol value"
            // ALONGSIDE the correct "unknown unit `furlong`" — two errors, the first self-contradictory
            // (it names the very `#"…"` form the arg already is). Only a non-name non-symbol arg (an
            // integer/string/compound), or a wrong arity, reaches here.
            if !(args.len() == 1
                && (db.ast.as_name(args[0]).is_some() || db.ast.as_sym(args[0]).is_some())) =>
        {
            let op = if prim == Prim::UnitBase {
                "Unit.base"
            } else {
                "Unit.of"
            };
            let arg_desc = match args.first() {
                Some(&a) => type_of(db, a).render_name(&db.name_ctx()),
                None => "nothing".to_string(),
            };
            out.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "`{op}` names its unit with a SYMBOL, but this is a {arg_desc} value — write a \
                         `#\"…\"` symbol literal, e.g. `({op} #\"meter\")`"
                    ),
                )
                .at(args.first().copied().unwrap_or(id)),
            );
            true
        }
        _ => false,
    }
}

/// The SURFACE spelling of a simple leaf `Atom` — a name or an integer literal — for splicing into a
/// fix replacement (`(: <value> <Type>)`). Returns `None` for a compound node or any other leaf
/// (a float, whose faithful re-spelling needs `Decimal` reconstruction; a string/char, which needs
/// quoting/escaping), so a fix's replacement is only ever emitted when its exact text is trivially
/// reconstructible; the caller then carries the message alone, no fix.
fn atom_surface(db: &Db, id: StructId) -> Option<String> {
    if let Some(name) = db.ast.as_name(id) {
        return Some(name.to_string());
    }
    if let Some(int) = db.ast.as_int(id) {
        return Some(int.to_decimal_string());
    }
    None
}

/// Whether the application `app` (head `head`, this level's `args`) is a BUILT-IN OPERATION applied at
/// FEWER args than it takes AND left UNCONSUMED — the partial-application both-miss hole. `head` is already
/// known to be a prim (`meta_apply_of` is `Some`) at the call site. Returns `true` only when ALL hold:
///  (a) SPINE-TOP: `app`'s parent (peeled through ref/annot wrappers) is not an `Apply` feeding `app` as
///      its HEAD — so no ENCLOSING application saturates it. This O(1) parent read is the completion test
///      (an inner partial whose outer Apply completes it has that Apply as parent → not spine-top → false).
///  (b) NOT A COMPLETED/ETA-LIFTABLE CONSTRUCTOR: a ctor-headed spine that reaches its payload arity
///      (`ctor_spine` + `variant_payload_arity`) builds; a bare partial ctor that `eta_ctor_closure` lifts
///      is a legitimate first-class value. Only a non-ctor OPERATION (or a ctor spine that neither
///      completes nor eta-lifts) is the unbuilt partial.
///  (c) UNDER-APPLIED: the gathered spine arg count is < the head's value-param arity. Arity is the
///      head's scheme INSTANTIATED then arrow-peeled — instantiation renames the `∀` quantifiers to fresh
///      vars so only genuine `Ty::Fn` value params are counted (peeling the raw scheme would miscount).
/// A `Var`/`Any`/unknown arity (no scheme) is conservatively NOT flagged (never a false reject).
fn is_builtin_partial_application(
    db: &mut Db,
    app: StructId,
    _head: StructId,
    _args: &[StructId],
) -> bool {
    // (a) SPINE-TOP. If `app` sits in the HEAD position of an enclosing APPLICATION form, an outer level
    // supplies more args — `app` is an inner node of a larger spine, not its top; skip (the top is checked
    // on its own visit). Test this STRUCTURALLY on the AST: `app` is the FIRST child (index 0) of its
    // parent list, and that parent is an application (not a grammar/ctor-string head). The resolver
    // FLATTENS a curried spine `((f a) b)` into ONE `Apply { head: f, args: [a, b] }`, so the parent's
    // RESOLVED head is `f` (the bottom), NOT this inner node — a resolved-form `head == app` test misses
    // it. The raw-AST head-position test is exact for both the nested-parens and any flatten. (`peel_ref_
    // annot` on `app` handles a `let`-bound spine whose parent points at the binding.)
    if let Some(parent) = db.parent_of(app)
        && let crate::ast::Struct::List(kids) = db.ast.get(parent)
        && kids.first() == Some(&app)
        && db.ast.head_ctor(parent).is_none()
        && db
            .ast
            .head_name(parent)
            .is_none_or(|h| !crate::resolve::is_grammar_head(h))
    {
        return false;
    }
    // FLATTEN THE SPINE to its BOTTOM head + full gathered args. The flat `(String.slice s 0)` and the
    // nested-parens `((String.slice s) 0)` are the SAME application (`(f a b)` desugars to `((f a) b)`), so
    // both must be treated identically. Driving the checks off the IMMEDIATE head skipped the nested form:
    // its head is the inner `Apply` `(String.slice s)`, whose `meta_apply_of`/`scheme_of` is `None`. Peel to
    // the bottom head (a builtin op / ctor / anything) and gather every arg across the spine, then gate +
    // exclude + arity all off THAT — so the two surfaces reject identically (Copilot PR#491 / v-inference).
    let (head, args) = crate::lower::apply_spine(db, app);
    // The check applies only to a BUILT-IN OPERATION head (a prim). A user fn / module member has no
    // `(meta apply)` prim — its partial application is legitimate currying, not flagged.
    if crate::eval::meta_apply_of(db, head).is_none() {
        return false;
    }
    // UNARY NEGATION `(- e)` — `Sub` applied to ONE arg. `Sub`'s scheme is BINARY (`∀a.(Int a)→(Int a)→
    // (Int a)`), so the arity check below would read it as under-applied; but arity-1 `Sub` is the LEGIT
    // prefix-negation `lower` builds as `0 - e` (its own `check_application` arm handles it), NOT an unbuilt
    // partial. Exclude it — the concrete instance of "a prim `lower` CAN build at this arg count is not a
    // partial". (Binary `Sub` at 2 args is full-arity anyway; only the 1-arg negation needs this guard.)
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Sub) && args.len() == 1 {
        return false;
    }
    // A binary OPERATOR applied to ONE of its two operands CURRIES (operator ruling: "operators should
    // curry") — `(+ 1)`, `(< 3)`, `(* 2)` — to a first-class function `(fn (b) (op supplied b))` that
    // `lower` synthesizes (`partial_binop_eta`). So a 1-of-2 partial of a curryable binop (arith /
    // comparison / float-arith; `Sub`-as-negation already excluded above) is NOT the unbuilt-partial hole —
    // exclude it, exactly as a curryable/eta-lifting CONSTRUCTOR partial is excluded below. A well-formed
    // curry lowers to a closure; an ill-formed one (an unfixed-type operand) still declines at lower, and
    // zero-arg `(+)` / over-application are separate faults untouched by this arity-1 gate.
    if args.len() == 1
        && let Some(p) = crate::eval::meta_apply_of(db, head)
        && (p.is_arith() || p.is_comparison() || p.is_float_arith())
    {
        return false;
    }
    // (b) A CONSTRUCTOR that COMPLETES its payload arity via the curried spine is a real construction, not a
    // partial — exclude. (`ctor_spine` gathers the whole spine's payloads; equal to the variant's arity ⇒
    // built.) A bare partial ctor that ETA-LIFTS is a legitimate first-class value — exclude.
    if let Some((ctor, all_args)) = crate::lower::ctor_spine(db, app)
        && crate::eval::variant_payload_arity(db, ctor) == Some(all_args.len())
    {
        return false;
    }
    if crate::eval::variant_disc_of(db, head).is_some() {
        // A ctor bottom head: partial iff it neither completed above nor eta-lifts. If it eta-lifts, it is a
        // first-class closure value — not the unbuilt-partial hole.
        if crate::lower::eta_ctor_closure(db, head).is_some() {
            return false;
        }
    }
    // (c) UNDER-APPLIED against the bottom head's VALUE-param arity. `args` is the flattened spine's full
    // arg count. Arity from the INSTANTIATED scheme's arrow chain — instantiate first so `∀` quantifiers
    // become fresh vars, not counted as params.
    let mut fresh = crate::unify::Fresh::new();
    let Some(scheme) = crate::eval::scheme_of(db, head, &mut fresh) else {
        return false; // no scheme (malformed / not typed) — never a false reject
    };
    let mut ty = crate::unify::instantiate(&scheme, &mut fresh);
    let mut arity = 0usize;
    while let Ty::Fn(_, r) = ty {
        arity += 1;
        ty = *r;
    }
    // Under-applied is the partial; exactly-applied builds; over-applied is the coded CDZ0203 elsewhere.
    args.len() < arity
}

/// The DISPLAY name of the constructor an `(Ctor arg…)` application applies — read from the SOURCE
/// spelling (`app`'s first child), so it works whether the head was written bare (`(None x)`) or qualified
/// (`(Option.None x)` → the member key `None`). Reads the surface spelling rather than the resolved head
/// (which may be a synthesized cached-ctor record, not a name atom). `"this variant"` when the spelling is
/// unreadable — a safe fallback for a message subject. The construction-site twin of `ctor_pattern_name`
/// (`lower.rs`), which does the same for a `(Ctor …)` MATCH pattern.
fn ctor_app_name(db: &Db, app: StructId) -> String {
    let first = match db.ast.get(app) {
        crate::ast::Struct::List(cs) => cs.first().copied(),
        _ => None,
    };
    first
        .and_then(|h| {
            db.ast
                .as_form(h, ".")
                .and_then(|t| t.get(1).copied())
                .or(Some(h))
        })
        .and_then(|k| db.ast.as_name(k))
        .unwrap_or("this variant")
        .to_string()
}

/// Whether two sum DECLARATIONS have the same STRUCTURAL SHAPE — the same variant NAMES in the same
/// order, each with the same payload arity. Used to tell a NOMINAL-boundary comparison (`CDZ0202`, two
/// same-shape sums `A`/`B` differing only in tag) from an ordinary type mismatch (`CDZ0203`, two sums of
/// DIFFERENT shape — `Option` vs `Result` — which are unrelated types). This is the structural relation
/// nominal identity is an orthogonal tag OVER (`type-system.md` §Nominal Is An Orthogonal Modifier Over
/// Any Structural Type): the tag distinguishes two types that would otherwise be the same shape. Compares
/// the declared variant names + payload counts (the shape the corpus's disjoint-vs-same-shape cases draw
/// the line on); it does not descend payload TYPES (a same-names-different-payload edge is out of scope).
/// The declaration occurrence of a NOMINAL type — a boxed `Ty::Sum` OR an erased `Ty::Nominal` newtype —
/// so the nominal-boundary comparison (`CDZ0202`) fires for BOTH: a single-variant sum that erased to a
/// `Ty::Nominal` (`(type A (Mk Int64))`) has the same "distinct declarations of the same shape are not
/// comparable" property its boxed multi-variant sibling does. `None` for a non-nominal type.
/// The nearest field NAME of `fields` to `name` under the shared did-you-mean cutoff — the closed-set
/// suggestion the row ops (`without`/`project`/`with`/`pop`) offer for a mistyped field, the same
/// `suggest::nearest` a member access uses. `None` when no field is a plausible typo. (`db` unused today
/// but kept for signature symmetry with the other field helpers, in case a future record shape needs it.)
fn nearest_record_field(
    _db: &Db,
    fields: &std::collections::BTreeMap<crate::resolved::Symbol, Ty>,
    name: &str,
) -> Option<String> {
    crate::diag::suggest::nearest(name, fields.keys().map(|k| &*k.name))
}

fn nominal_or_sum_decl(ty: &Ty) -> Option<StructId> {
    match ty {
        Ty::Sum { decl, .. } | Ty::Nominal { decl, .. } => Some(*decl),
        _ => None,
    }
}

/// Whether `ty` — used as a Map/Set KEY at site `at` — CONTAINS an abstract type ANYWHERE in its
/// structure (the key type ITSELF, or nested inside a tuple element / list element / map key-or-value /
/// set element / Qty inner / sum-or-nominal type-argument). CHAMP key equality/hashing walks the WHOLE
/// compound key structurally, so an abstract type nested in a compound key is observed by its private
/// representation exactly as a bare abstract key is — the same `type-system.md` boundary violation, one
/// structural level down (PR#890, Copilot; the compound-key generalization of the bare-key CDZ0202).
/// Returns the FIRST abstract type found (for the message). Uses `nominal_or_sum_decl` +
/// `is_abstract_type_at(at, …)` — the SAME predicate the top-level check used — at every structural node,
/// so a concrete/prelude/own constituent never flags (only a genuinely handle-only imported one). A
/// function-typed constituent (`Ty::Fn`/`Cont`) is NOT walked: a function value is not a comparable key
/// spine (a Map/Set keyed by a function is its own separate decline), so an abstract type reachable only
/// through an arrow is not observed by key comparison.
fn key_ty_contains_abstract_at(db: &Db, at: StructId, ty: &Ty) -> Option<Ty> {
    // The node ITSELF, if it is an abstract nominal/sum here.
    if nominal_or_sum_decl(ty).is_some_and(|decl| db.is_abstract_type_at(at, decl)) {
        return Some(ty.clone());
    }
    // Otherwise recurse into the comparable-key structure. A `Ty::Sum`/`Nominal` that was NOT abstract
    // here still may carry an abstract TYPE-ARGUMENT (`(Box Temp)` — the box is concrete/prelude but its
    // element is abstract), so walk `args` too.
    match ty {
        Ty::Tuple(elems) => elems
            .iter()
            .find_map(|e| key_ty_contains_abstract_at(db, at, e)),
        Ty::List(e) | Ty::Set(e) => key_ty_contains_abstract_at(db, at, e),
        Ty::Map(k, v) => key_ty_contains_abstract_at(db, at, k)
            .or_else(|| key_ty_contains_abstract_at(db, at, v)),
        Ty::Record(fields) => fields
            .values()
            .find_map(|f| key_ty_contains_abstract_at(db, at, f)),
        Ty::Qty { inner, .. } => key_ty_contains_abstract_at(db, at, inner),
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => args
            .iter()
            .find_map(|a| key_ty_contains_abstract_at(db, at, a)),
        _ => None,
    }
}

/// The FUNCTION-typed sibling of [`key_ty_contains_abstract_at`]: does `ty` contain a `Ty::Fn`/`Cont` at a
/// comparable-key/equality spine position? A function/closure has NO canonical identity — it is neither
/// equatable nor orderable AT ALL (unlike an abstract type, which is comparable-but-opaque; that is
/// CDZ0202). So a Map/Set keyed by a function (or a direct `(=)`/order over one), including a function
/// NESTED in a compound key (`(Tuple Fn Int64)` — built-in comparison walks the whole spine to the arrow),
/// cannot be compared → CDZ0216. Walks the SAME comparable structure as the abstract check (tuple/list/set/
/// map/record/Qty/sum-or-nominal args); an arrow found at any of those positions flags. Returns the first
/// function type found (for the message). Unlike the abstract check this is site-INDEPENDENT (a function is
/// never comparable, regardless of `at`), so it takes no `db`/`at`. `Ty::Cont` (a continuation) is likewise
/// an un-comparable arrow-like value.
fn key_ty_contains_fn(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::Fn(_, _) | Ty::Cont { .. } => Some(ty.clone()),
        Ty::Tuple(elems) => elems.iter().find_map(key_ty_contains_fn),
        Ty::List(e) | Ty::Set(e) => key_ty_contains_fn(e),
        Ty::Map(k, v) => key_ty_contains_fn(k).or_else(|| key_ty_contains_fn(v)),
        Ty::Record(fields) => fields.values().find_map(key_ty_contains_fn),
        Ty::Qty { inner, .. } => key_ty_contains_fn(inner),
        Ty::Sum { args, .. } | Ty::Nominal { args, .. } => args.iter().find_map(key_ty_contains_fn),
        _ => None,
    }
}

/// Push the map/set KEY-HASHABILITY fault for `key_ty`, anchored at `at`, if any: an ABSTRACT key type
/// (CDZ0202 — its representation is not observable across its boundary) or a FUNCTION-typed key (CDZ0216 —
/// a closure has no canonical identity, so it is neither equatable nor orderable). Returns whether a fault
/// was pushed. SHARED by `check_application` (the `Set.of`/`Map.new`/lookup/algebra PRIM apps) AND the
/// native `#set`/`#map` LITERAL arms in `collect`: both store keys the runtime later compares/hashes
/// structurally (`champ_eq`), so both must enforce the same constraint — a native literal must NOT be a
/// silent bypass of the prim-app check (the M2 native-literal soundness hole: pre-M2 there was no native
/// `#set`/`#map` literal, so the prim-app-only gate was complete; the literal reopened it).
fn push_unhashable_key_fault(db: &Db, at: StructId, key_ty: &Ty, out: &mut Vec<Reject>) -> bool {
    if let Some(abstract_ty) = key_ty_contains_abstract_at(db, at, key_ty) {
        trace!(target: "rcdzc::infer", at = at.0, "fault: abstract-typed map/set key (CDZ0202)");
        out.push(
            Reject::coded(
                Code::NominalMismatch,
                format!(
                    "`{}` is an abstract type here (its constructors are not exported to this \
                     file), so it cannot be a map/set key — key insertion, lookup, and membership \
                     observe its representation through a built-in comparison; compare it through \
                     a function exported by the module that declares it",
                    abstract_ty.render_name(&db.name_ctx())
                ),
            )
            .at(at),
        );
        return true;
    }
    if let Some(fn_ty) = key_ty_contains_fn(key_ty) {
        trace!(target: "rcdzc::infer", at = at.0, "fault: function-typed map/set key (CDZ0216)");
        out.push(
            Reject::coded(
                Code::NotEquatable,
                format!(
                    "a value of function type `{}` cannot be a map/set key — a function has no \
                     canonical identity, so it is neither equatable nor orderable; key the \
                     collection by a value type (or by a field the closure captures)",
                    fn_ty.render_name(&db.name_ctx())
                ),
            )
            .at(at),
        );
        return true;
    }
    false
}

fn same_sum_shape(db: &Db, a: StructId, b: StructId) -> bool {
    let (Some(da), Some(dbecl)) = (db.type_decl_by_occ(a), db.type_decl_by_occ(b)) else {
        return false;
    };
    da.variants.len() == dbecl.variants.len()
        && da
            .variants
            .iter()
            .zip(dbecl.variants.iter())
            .all(|(va, vb)| va.name == vb.name && va.payloads.len() == vb.payloads.len())
}

/// The type-agreement faults reachable from `id` — a query READ over the demand-filled type column,
/// separate from the value it holds. An `if` whose condition is not `Bool`, or whose branches do not
/// agree, is a coded mismatch. Descends into children for their own faults.
pub fn type_errors(db: &mut Db, id: StructId) -> Vec<Reject> {
    let mut out = Vec::new();
    collect(db, id, &mut out);
    trace!(target: "rcdzc::infer", node = id.0, faults = out.len(), "type check complete");
    out
}

/// The CATEGORY subject + member-noun for an absent-member access `(. operand key)`, so the rejection
/// names what the operand ACTUALLY is instead of always calling it a "record". `(. E emt)` on an effect
/// reads "effect `E` has no operation `emt`", `(. List nonesuch)` on a prelude module "the `List` module
/// has no member `nonesuch`", `(. Option Nonesuch)` on a sum type "the sum type `Option` has no variant
/// `Nonesuch`" — a plain user record keeps "record has no field". The operand's SOURCE NAME classifies it
/// (an effect via `effect_decl_by_name`, a prelude module via `db.prelude`, a sum/nominal type via
/// `type_decl_by_name`); anything else (a runtime record value, a `record` literal) is the record default.
/// Returns `(subject, member_word)` where the message is `<subject> has no <member_word> \`key\``. The
/// `has no <word> \`` shape is the shared DEDUP invariant (`compile::dedup_faults`' `no_field_key` splits on
/// it), so every category still collapses its infer/emit twin.
pub(crate) fn member_category(db: &mut Db, operand: StructId, key: &str) -> (String, &'static str) {
    let _ = key;
    if let Some(name) = db.ast.as_name(operand) {
        if db.effect_decl_by_name(name).is_some() {
            return (format!("effect `{name}`"), "operation");
        }
        // A prelude MODULE name (`List`/`Map`/`Set`/`String`/`Bytes`/`Int64`/…) — a closed record of ops.
        if db.prelude.contains_key(name) {
            return (format!("the `{name}` module"), "member");
        }
        // A user (or built-in) SUM / nominal TYPE name — its members are its variant constructors.
        if db.type_decl_by_name(name).is_some() {
            return (format!("the type `{name}`"), "variant");
        }
    }
    // A USER `(module m …)` value — a bare `m` OR a nested projection `(. outer inner)` — reduces to the
    // module's SYNTHESIZED record. Name it "the `m` module has no member `k`" (matching the prelude-module
    // arm) rather than leaking the internal "record has no field `k`" — a user module is a module, not a
    // bare record, so an absent-member miss should read as a module miss. Recognized by ORIGIN (the reduced
    // record IS a module's synth record, `module_name_by_synth_record`), not by name — a nested projection
    // has no operand name for the `as_name` arms above to catch. Falls through to the record arm below for a
    // genuine record value (whose reduced record is not a module synth).
    if let Some(record) = crate::eval::reduce_to_record_id(db, operand)
        && let Some(mname) = db.module_name_by_synth_record(record)
    {
        return (format!("the `{mname}` module"), "member");
    }
    ("record".to_string(), "field")
}

/// The canonical replacement for a RETIRED prelude collection-op name, or `None` if `(module, old)` is
/// not one of the three renames from the consistent-naming cutover (2026-07-15). This table is a
/// DIAGNOSTIC-ONLY hint (drives CDZ0603's message + fix); it does NOT participate in name resolution —
/// the retired name still fails to resolve, honoring `no-keys-outside-the-prelude` (the prelude in
/// `prelude.rs` is the ONE place a member name resolves). Kept tight to the exact retired set so a
/// genuine typo (a name that was never a member) still gets the ordinary unknown-member did-you-mean.
pub(crate) fn retired_collection_op(module: &str, old: &str) -> Option<&'static str> {
    match (module, old) {
        ("Map", "size") => Some("len"),
        ("Tuple", "cat") => Some("concat"),
        ("Tuple", "pop") => Some("remove"),
        _ => None,
    }
}

/// The `record has no field \`key\`` rejection for a member access `member` (`(. operand key)`),
/// enriched two-tier (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix — the
/// record analogue of the unbound-name suggestion). `operand`'s field names are the candidate set (a
/// CLOSED, small set — a record's fields or a module's operations, unlike the unbounded in-scope names an
/// unbound variable draws from, so LISTING them is signal, not noise). TIER 1 — a field is a plausible
/// typo of `key`: name it (`` — did you mean `field`? ``) AND carry a heuristic ReplaceNode fix on the
/// KEY occurrence (so an editor rewrites exactly the field token, `(. r fild)` → `(. r field)`). TIER 2 —
/// no confident typo: LIST the closest few fields (`` — closest matches: `a`, `b`, `c` ``) so an agent
/// sees what the record/module actually offers (`((. List get) …)` → "closest matches: `at`, `of`" —
/// rustc's "available fields are: …") instead of reading the prelude to discover them; no fix (a list of
/// options is not one mechanical edit). Both tiers via the shared `did_you_mean` helper (same as the
/// unknown-top-form head), so the phrasing + determinism match every other did-you-mean site. The
/// `did_you_mean` suffix always begins ` — `, so the dedup's `no_field_key` (which splits on ` — `) still
/// keys both tiers by the invariant `` record has no field `k` `` core — collapse stays intact.
fn no_field_reject(
    db: &mut Db,
    member: StructId,
    operand: StructId,
    key: &crate::resolved::Symbol,
) -> Reject {
    // The key occurrence is the second child of the `(. operand key)` form — the node any fix rewrites.
    let key_occ = db.ast.as_form(member, ".").and_then(|t| t.get(1).copied());
    // RENAMED-OP hint (CDZ0603): if the operand is a prelude collection MODULE and `key` is one of the
    // three retired names from the consistent-naming cutover, give a targeted message + VERIFIED fix
    // pointing at the canonical spelling. This is a diagnostic-only hint — the retired name still failed
    // to resolve (no alias participates in resolution; `no-keys-outside-the-prelude`), it just gets a
    // better message than the generic "no member". A name that is NOT in this fixed retired set falls
    // through to the ordinary unknown-member did-you-mean below.
    if let Some(module) = db.ast.as_name(operand)
        && let Some(new_name) = retired_collection_op(module, &key.name)
    {
        let reject = Reject::coded(
            Code::RenamedOp,
            format!(
                "`{module}.{}` was renamed to `{module}.{new_name}`; write `(. {module} {new_name})`",
                key.name
            ),
        );
        return match key_occ {
            Some(occ) => reject.with_fix(Fix::replace_verified(
                occ,
                new_name,
                format!("rename to `{new_name}`"),
            )),
            None => reject,
        };
    }
    // The CONFIDENT single (tier 1) drives the FIX; the two-tier message adds the closest-matches list when
    // there is none. `nearest` and `did_you_mean`'s tier-1 branch use the same cutoff, so a `Some` winner
    // ⟺ the message says "did you mean `field`?" — the fix targets the very field the message names.
    //
    // MEMOIZE per `(reduced-record occ, key)`: building the record's O(fields) name list + edit-distance-
    // scanning it (twice — `nearest` then `did_you_mean`) per access made a WIDE record with a renamed field
    // accessed from N sites O(N²). N sites over one record share its reduced occurrence, so the (winner,
    // hint) pair caches. A type-only operand (no concrete record occ) falls through to a fresh compute — the
    // rare non-repeating case. (The record-field twin of `variant_suggest_winner`, fix-26/45.)
    let cache_key =
        crate::eval::reduce_to_record_id(db, operand).map(|rec| (rec, key.name.clone()));
    let (suggestion, hint) = if let Some(k) = &cache_key
        && let Some(hit) = db.no_field_suggestion.get(k)
    {
        hit.clone()
    } else {
        #[cfg(test)]
        crate::db::NO_FIELD_SUGGESTION_MISSES.with(|c| c.set(c.get() + 1));
        let fields = crate::eval::record_field_names(db, operand);
        let suggestion = crate::diag::suggest::nearest(&key.name, &fields);
        let hint = crate::diag::suggest::did_you_mean(&key.name, &fields, 3);
        if let Some(k) = &cache_key {
            db.no_field_suggestion
                .insert(k.clone(), (suggestion.clone(), hint.clone()));
        }
        (suggestion, hint)
    };
    // NAME the operand's real category (effect / module / type / record) instead of always "record" —
    // `(. E emt)` reads "effect `E` has no operation `emt`", `(. List nonesuch)` "the `List` module has no
    // member `nonesuch`". The `has no <word> \`key\`` shape stays the shared dedup invariant.
    let (subject, member_word) = member_category(db, operand, &key.name);
    // SHADOWED-OP HINT: when the operand is an effect NAME and the missing operation `key` is NOT a typo of
    // the resolved effect's ops but IS declared on a DIFFERENT, LATER same-named `(effect …)`, the ordinary
    // "no operation `key` — closest matches: …" is baffling (the author sees `key`'s declaration in plain
    // sight). Two same-named effects are DISTINCT (an effect's identity is its declaration, not its name —
    // pinned by `14-effects:3129`), so a bare `E` resolves the FIRST and `key` on a later `E` is genuinely
    // out of reach. Explain THAT (the diagnostic-quality half of the works-as-specified duplicate-effect
    // finding) instead of/atop the typo hint. Only when the operand is a bare effect name whose resolved
    // declaration lacks `key` while a later same-named one declares it — a narrow, non-typo case.
    let shadow_hint = if let Some(name) = db.ast.as_name(operand)
        && let Some(first_occ) = db.effect_decl_by_name(name)
        && crate::effects::op_on_other_same_named_effect(db, name, first_occ, &key.name)
    {
        format!(
            " — operation `{}` is declared on a LATER `(effect {name} …)`; a bare `{name}` resolves the \
             FIRST declaration (an effect's identity is its declaration, not its name), so that operation \
             is out of reach here — merge the operations into one `(effect {name} …)` or handle the \
             intended declaration",
            key.name
        )
    } else {
        String::new()
    };
    // The shadow hint SUPERSEDES the generic did-you-mean suffix (a "closest matches: a" list is noise when
    // the real cause is a shadowed later declaration); otherwise keep the ordinary two-tier `hint`.
    let is_shadow = !shadow_hint.is_empty();
    let suffix = if is_shadow { shadow_hint } else { hint };
    // A genuine RECORD-value field absence is CDZ0212 (AbsentField) — the code `type-system.md` §A Record
    // Is Restricted To A Named Set Of Its Fields + the corpus pin `15-rows:235` assign to "projecting a
    // record onto an absent field", and `Record.project` already emits it. A `.`-access of an absent field
    // is the SAME user error via a different surface, so it gets the SAME code (was CDZ0201). ONLY the
    // record case flips: `member_word == "field"` now reliably means a GENUINE record value — a module
    // MEMBER, effect OPERATION, and sum-type VARIANT each get their own category word (`member`/`operation`/
    // `variant`) from `member_category`, and a user-module export record is routed to `"member"` too (via
    // `module_name_by_synth_record`), so none of them flips. This is the narrow half of the CDZ0201/CDZ0212
    // consistency (`Record.project` twin) that needs no module-privacy design ruling. The EMIT-side copy
    // (`lower.rs`) flips identically off the same `member_word`, so the two copies keep equal codes and
    // `dedup_faults` still collapses them to one diagnostic.
    let code = if member_word == "field" {
        Code::AbsentField
    } else {
        Code::Malformed
    };
    let reject = Reject::coded(
        code,
        format!("{subject} has no {member_word} `{}`{suffix}", key.name),
    );
    // A shadow case has no single mechanical replace fix (the op is real, just in another declaration), so
    // suppress the typo-replace fix there; otherwise the confident-typo fix stands.
    match (suggestion, key_occ) {
        (Some(field), Some(occ)) if !is_shadow => {
            reject.with_fix(Fix::replace_heuristic(occ, field))
        }
        _ => reject,
    }
}

/// The canonical, HASHABLE identity of a DIRECT-LITERAL map key — a scalar written in the map literal.
/// Two keys are the duplicate the spec forbids exactly when their tokens are EQUAL, and this token is
/// built to reproduce `const_compound_eq`'s scalar equality precisely: an integer by its VALUE (leading
/// zero bytes trimmed and a zero sign-normalized, so `1`/`0x1` and `0`/`-0` collide exactly as
/// `IntValue::eq_value` decides), a float by its canonical `Float64` bits (so `-0.0` and `0.0` are
/// DISTINCT keys), a string/bool by value, unit a singleton. Only the five direct-literal scalar kinds
/// (int/string/bool/float/unit) produce a token; every other key (crucially a NAME reference, even one
/// that folds to a literal value) yields `None` so it is never compared — a runtime overwrite, not a
/// compile-time duplicate.
#[derive(PartialEq, Eq, Hash)]
enum LitKey {
    Int { negative: bool, magnitude: Vec<u8> },
    Str(String),
    Bool(bool),
    FloatBits(u64),
    Unit,
}

/// The [`LitKey`] token for a DIRECT-LITERAL key at `id`, or `None` for any non-literal key (a name
/// reference, a compound). Reads `resolved_of` ONCE per key — the O(1)-per-key basis of the linear
/// duplicate scan. A NAME key resolves to `Resolved::Ref`
/// (not one of these arms) → `None`, preserving the "two distinct names bound to the same value are a
/// runtime overwrite, not a reject" rule. For a direct literal the resolved value equals its lowered
/// `core_of` constant, so a token match is exactly `const_compound_eq == Some(true)` on that pair.
fn literal_key_token(db: &mut Db, id: StructId) -> Option<LitKey> {
    match resolved_of(db, id) {
        // Trim leading zero bytes and normalize a zero's sign to non-negative — the SAME canonicalization
        // `IntValue::eq_value` applies, so equal tokens ⟺ `eq_value` is true (magnitude representation and
        // a signed zero do not create spurious distinctions).
        Resolved::Int(v) => {
            let start = v.magnitude.iter().take_while(|&&b| b == 0).count();
            let magnitude = v.magnitude[start..].to_vec();
            let negative = !magnitude.is_empty() && v.negative;
            Some(LitKey::Int {
                negative,
                magnitude,
            })
        }
        Resolved::Str(s) => Some(LitKey::Str(s)),
        Resolved::Bool(b) => Some(LitKey::Bool(b)),
        // A written float literal is always finite (`Decimal` holds no NaN), and its canonical `Float64`
        // bits are what `const_compound_eq` compares — so `-0.0` ≠ `0.0` and two spellings of one value
        // (`2.0`/`2.00`) collide, matching the scalar `=` fold.
        Resolved::Float(d) => Some(LitKey::FloatBits(d.to_f64_bits())),
        Resolved::Unit => Some(LitKey::Unit),
        _ => None,
    }
}

/// A map literal has a DUPLICATE WRITTEN-LITERAL key → CDZ0201 (the association is ambiguous — which
/// value does the key hold? collections-and-text.md §A Map Associates Keys With Values: each key at most
/// once). Only DIRECT LITERAL keys are checked (see `literal_key_token`): two
/// literal keys that compare structurally equal are a duplicate. A NAME key — even two distinct names
/// bound to the same value — is a runtime overwrite (size 1), never a reject. `None` if no duplicate.
///
/// LINEAR in the entry count: each direct-literal key is canonicalized to a hashable [`LitKey`] ONCE and
/// inserted into a set; a collision is the duplicate. This replaced an O(entries²) pairwise
/// `const_compound_eq` scan — which additionally re-derived and deep-cloned each key's `Core` on every
/// one of the ~N²/2 comparisons (a `(map (0 0) (1 1) …)` literal of N distinct integer keys was
/// quadratic: N=1600 spent ~72% of the whole compile in `const_compound_eq`). The verdict is IDENTICAL —
/// a duplicate exists iff two direct-literal keys share a token, iff two of them are `const_compound_eq`-
/// equal — and the reject is anchored to the map node with no pair-specific data, so reporting the FIRST
/// collision (insertion order) rather than the first pair (scan order) yields byte-identical output.
fn map_duplicate_const_key(db: &mut Db, entries: &[(StructId, StructId)]) -> Option<Reject> {
    let mut seen: crate::fxhash::FxHashSet<LitKey> = crate::fxhash::FxHashSet::default();
    for &(key, _) in entries {
        if let Some(token) = literal_key_token(db, key)
            && !seen.insert(token)
        {
            // The mechanical repair: DELETE the redundant `(key value)` entry — the entry is `key`'s
            // enclosing list (`parent_of(key)`). An earlier entry already binds this key; a map holds each
            // key once. Anchor at the entry so `cdz fix` edits it. Heuristic: WHICH duplicate to drop (and
            // whether the author meant a different key) is a guess, but removing THIS one resolves the
            // ambiguity. (Falls back to an unanchored reject if the entry structure is unexpected.)
            let mut reject = Reject::coded(
                Code::Malformed,
                "a map contains each key at most once (a duplicate literal key)",
            );
            if let Some(entry) = db.parent_of(key)
                && matches!(db.ast.get(entry), crate::ast::Struct::List(_))
            {
                reject = reject
                    .at(entry)
                    .with_fix(crate::diag::Fix::delete_heuristic(
                        entry,
                        "remove the duplicate map entry",
                    ));
            }
            return Some(reject);
        }
    }
    None
}

/// Walk the subtree at `node` for the FIRST leaf kind that quote's reifier cannot turn into an `Ast`
/// value — a `#"…"` symbol (`Sym`) or a `#\c` char (`Char`) — since the `Ast` sum has no `Ast.Symbol`
/// / `Ast.Char` variant for them. (A `b"…"` bytes literal IS reifiable: `Bytes` → `Ast.Bytes`, operator
/// seq 113 — it is NOT flagged here.) `quote`'s reify bails (`_ => None`,
/// `quote.rs`) on any such leaf, so the WHOLE `(quote …)` declines and an enclosing `(eval …)` then falls
/// through as an unbound `eval` name. Returns a human phrase naming the offending literal kind (for the
/// `eval` diagnostic below), or `None` if every leaf is reifiable (the ordinary "nothing to reconstruct"
/// runtime/non-constant case). A pre-order walk: the first non-reifiable leaf found is the one to name.
fn first_non_reifiable_leaf(db: &Db, node: crate::ast::StructId) -> Option<&'static str> {
    match db.ast.get(node) {
        crate::ast::Struct::Atom(l) => match db.ast.leaf(*l) {
            // Sym/Char have no `Ast` variant yet, so `quote` bails on them (reject-don't-miscompile).
            // NOT `Bytes` — it reifies to `Ast.Bytes` (operator seq 113), so a `b"…"` is reifiable and
            // must not be flagged here.
            crate::ast::Leaf::Sym(_) => Some("a `#\"…\"` symbol literal"),
            crate::ast::Leaf::Char(_) => Some("a `#\\…` char literal"),
            _ => None,
        },
        crate::ast::Struct::List(kids) => {
            let kids = kids.clone();
            kids.iter().find_map(|&k| first_non_reifiable_leaf(db, k))
        }
    }
}

/// Enrich a bare UNBOUND-name reject (`resolve_name` emits `unbound name \`x\``, no suggestion) with the
/// rustc-gold "did you mean?" — the nearest in-scope name + a heuristic replace fix — at the ONE site
/// that surfaces an unbound name as a user fault. The nearest-name search is an O(names-in-scope)
/// candidate scan (with a Levenshtein per candidate); doing it HERE (once per surfaced fault) instead of
/// in `resolve_name` (once per resolve, including the many pattern-binder / shape-test resolves whose
/// unbound Poison is never surfaced) is what keeps a match over an N-variant sum LINEAR rather than O(N²).
/// The resulting message + heuristic fix are byte-identical to the old eager form.
fn enrich_unbound(db: &mut Db, id: crate::ast::StructId, r: Reject) -> Reject {
    // Only a genuinely-unstamped bare unbound reject is enriched; read the name off the faulting node.
    let Some(name) = db.ast.as_name(id).map(str::to_string) else {
        return r;
    };
    // `eval` is a RECOGNIZED metaprogramming form (`desugar_eval` rewrites `(eval AST)` to the source the
    // AST denotes) — but ONLY when its argument is a COMPILE-TIME-VISIBLE `Ast` construction (a `(quote
    // …)` / literal `Ast.*`). A `(eval 5)` (non-Ast arg), `(eval a)` (a runtime Ast), or `(eval)` (no arg)
    // does not desugar, so the `eval` head falls through to `resolve` as an unbound NAME — a MISLEADING
    // "unbound name `eval`" (and worse, a did-you-mean to a near name like `even`), as if `eval` were a
    // typo, when the real situation is a recognized form whose argument this compiler does not execute. Name
    // that — the metaprogramming analogue of the top-level `import`/`pragma` recognized-but-not-modeled
    // messages (`compile::collect_faults`). Fires only when `id` heads a `(eval …)` form (so a bare `eval`
    // reference elsewhere still gets the ordinary unbound path).
    if let Some(eval_args) = db
        .parent_of(id)
        .filter(|&p| {
            matches!(db.ast.get(p), crate::ast::Struct::List(kids) if kids.first() == Some(&id))
        })
        .and_then(|p| db.ast.as_form(p, "eval").map(<[_]>::to_vec))
        .filter(|_| name == "eval")
    {
        // A `(quote …)` argument IS compile-time-visible, so "nothing to reconstruct" is the WRONG
        // reason when it declined because it carries a leaf kind the `Ast` sum has no variant for — a
        // `#"…"` symbol or a `#\c` char (`quote`'s reify bails on those, so the whole quote — and the
        // enclosing `eval` — declines). Name the actual offending literal instead of the misleading
        // runtime/non-constant phrasing. (These reify once the corresponding `Ast` variant lands — e.g.
        // `Ast.Symbol` with the symbols vertical; `Ast.Bytes` already landed, so `b"…"` is NOT flagged.)
        if let Some(kind) = eval_args
            .iter()
            .find_map(|&a| first_non_reifiable_leaf(db, a))
        {
            return Reject::coded(
                Code::Unbound,
                format!(
                    "`eval` reconstructs a compile-time-visible AST to source, but this one contains \
                     {kind}, which has no `Ast` leaf variant to reconstruct (the `Ast` sum covers \
                     integers, floats, booleans, strings, names, byte sequences, and lists). `quote` \
                     cannot reify such a literal, so the whole `(quote …)` — and this `eval` — declines. \
                     Use a reifiable literal, or compute the value outside `quote`."
                ),
            )
            .at(id);
        }
        return Reject::coded(
            Code::Unbound,
            "`eval` executes only a COMPILE-TIME-VISIBLE AST construction (a `(quote …)` or literal \
             `Ast.*`): it reconstructs the source that AST denotes and compiles it. A runtime / \
             non-constant AST argument is not executed (the compiler builds and analyzes AST but does not \
             run a dynamically-built one), so this `eval` has nothing to reconstruct."
                .to_string(),
        )
        .at(id);
    }
    // A bare NAME in the SYMBOL-NAME argument of a unit builder — `(Unit.base foot)`, `(Unit.of foot)`,
    // `(Unit.define furlong …)` — is the value-expression twin of the bare-name-in-a-`(Qty T u)`-position
    // slip: the author wrote the unit's NAME as an identifier where a `#"…"` SYMBOL belongs (a unit name is
    // a symbol, `enrich_nested_lowercase_type_vars`'s Qty sibling). It resolves as a MISLEADING "unbound
    // name `foot`" (with a did-you-mean to some near value), as if `foot` were a mistyped binding, when the
    // real fix is to quote it as a symbol. Name that + carry the `#"foot"` replace fix (the name text is in
    // hand). Fires only when `id` is the FIRST argument (the name slot) of a `Unit.base`/`Unit.of`/
    // `Unit.define` form — the symbol-consuming builders; `Unit.*`/`Unit./`/`Unit.^`/`Unit.prefix` take
    // unit/int operands, not a name, so they are excluded.
    if let Some(parent) = db.parent_of(id)
        && let crate::ast::Struct::List(kids) = db.ast.get(parent)
        && kids.get(1) == Some(&id)
        && let Some(&head) = kids.first()
        && matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::UnitBase
                    | crate::resolved::Prim::UnitOf
                    | crate::resolved::Prim::UnitDefine
            )
        )
    {
        // The SYMBOL name to suggest: strip a single leading `#`. A user who writes `#meter` typed the
        // symbol SIGIL but forgot the quotes — the reader read `#meter` as an identifier, so `name` carries
        // the `#`. Suggesting `#"#meter"` (keeping the stray `#`) is wrong: it fails again as unknown unit
        // `#meter`. Strip it so the suggestion + fix are `#"meter"` — the unit the author meant. A bare
        // `meter` (no sigil) is unaffected.
        let sym_name = name.strip_prefix('#').unwrap_or(&name);
        return Reject::coded(
            Code::Malformed,
            format!(
                "`{name}` is not a unit name here — a unit builder names its unit with a SYMBOL, not a \
                 bare identifier. Write `#\"{sym_name}\"` (a `#\"…\"` symbol literal)"
            ),
        )
        .at(id)
        .with_fix(Fix::replace_heuristic(id, format!("#\"{sym_name}\"")));
    }
    match crate::resolve::nearest_unbound_suggestion(db, id, &name) {
        Some(candidate) => Reject::coded(
            Code::Unbound,
            format!("unbound name `{name}` — did you mean `{candidate}`?"),
        )
        .at(id)
        .with_fix(Fix::replace_heuristic(id, candidate)),
        None => r,
    }
}

/// Collect faults at and under `id`, stamping each with its origin node. The recursive `collect_node`
/// pushes this node's own faults and recurses into children (each child stamped by its OWN frame
/// first); afterwards we stamp every fault THIS frame added that is still unanchored with `id` — so a
/// fault ends up anchored to the innermost node whose frame produced it, with no per-`push` threading.
fn collect(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    // Recursive-descent DEPTH GUARD (shared with `type_of` and `core_of`). `collect_node` recurses into
    // children, so pathologically deep input would overflow the native stack and ABORT the process.
    // Past the limit, report the resource-limit decline and stop descending — a compiler declines on
    // input it cannot handle, never crashes (`self-hosting-and-bootstrap.md`). See `DESCENT_DEPTH_LIMIT`.
    // Mark the walk LIMITED so this (partial) result is not memoized (see `collect_cache`).
    if db.descent_depth >= crate::db::DESCENT_DEPTH_LIMIT {
        trace!(target: "rcdzc::infer", node = id.0, "collect depth limit hit → decline (resource limit)");
        db.collect_limited = true;
        out.push(
            Reject::decline(
                "expression nests too deeply to compile (a recursion/resource limit was reached)",
            )
            .at(id),
        );
        return;
    }
    // MEMO: a node's faults are a pure function of its structure + its parts' (memoized) types, so a node
    // collected once is not re-walked. This is what makes a nested call chain's fault walk LINEAR: without
    // it, `check_application` step 2 re-descends each Apply's cached-but-shared reduced body, whose inner
    // call re-descends its own reduced body — an exponential re-walk. Replaying the cached faults (each
    // already carrying its stamped origin) is exact. Only a subtree collected WITHOUT hitting a limit is
    // cached (a limit-clipped partial walk is not the node's true fault set — tracked by `collect_limited`).
    if let Some(cached) = db.collect_cache.get(&id) {
        out.extend(cached.iter().cloned());
        return;
    }
    db.descent_depth += 1;
    let outer_limited = std::mem::replace(&mut db.collect_limited, false);
    let mut sub: Vec<Reject> = Vec::new();
    collect_node(db, id, &mut sub);
    for reject in &mut sub {
        reject.set_origin_if_absent(id);
    }
    // Cache the node's faults iff its walk was complete (no limit tripped inside it). Propagate the
    // limited flag to the enclosing frame (OR it in) so an ancestor is not cached over a clipped child.
    let this_limited = db.collect_limited;
    if !this_limited {
        db.collect_cache.insert(id, sub.clone());
    }
    db.collect_limited = outer_limited || this_limited;
    db.descent_depth -= 1;
    out.extend(sub);
}

/// Walk the RAW AST subtree at `id` (a `quote`/`quasiquote`/`unquote`/`unquote-splicing` form) and report
/// the coded SYNTAX rejection of every `(unquote …)`/`(unquote-splicing …)` occurrence inside it — the
/// CDZ0003 (outside-a-quasiquote) / CDZ0201 (wrong-arity) checks `resolve::resolve_unquote` produces. The
/// enclosing quote/quasiquote itself declines (its `Ast` value is not built), so these inner defects are
/// invisible to the ordinary type walk; surfacing them here is the "check descends to leaves" discipline
/// (a syntax defect is unconditional well-formedness, like an unbound name in an untaken branch). Walks
/// the AST children directly (the resolved form is a decline, carrying no child structure). The `unquote`'s
/// own OPERANDS are ordinary expressions (`,(+ x 1)`) but they too are inert data until the `Ast` vertical,
/// so only the quoting-form structure is inspected here, not the operand values.
/// Whether `ty` is a type the compiler can PROVE is NOT a list — the predicate the `,@` splice-operand
/// check fires on. A CONSERVATIVE test: it returns `true` only for a concrete non-`List` type (a scalar,
/// text, tuple, record, map, set, sum, quantity), and `false` for a `List` (the good case) OR for any
/// type that is not yet determined — an open `Var`, `Any`, or a nominal wrapper whose underlying shape
/// this walk does not unwrap — so an operand whose type never resolves DECLINES rather than being falsely
/// rejected as a non-list. "Prove it is wrong, never guess it is wrong."
fn provably_not_list(ty: &Ty) -> bool {
    use Ty::*;
    match ty {
        // A concrete NON-list type — a splice of it has no elements.
        Int(_)
        | Bool
        | Unit
        | Record(_)
        | Tuple(_)
        | Map(_, _)
        | Set(_)
        | Bytes
        | String
        | Char
        | Symbol
        | Float(_)
        | Qty { .. }
        | Sum { .. } => true,
        // A list is exactly the good case; an open var / `Any` / anything else is not yet provably wrong.
        List(_) => false,
        _ => false,
    }
}

fn collect_quote_body_syntax(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    // An `(unquote e)` / `(unquote-splicing e)` node: either a SYNTAX/arity defect (report its coded
    // reject), or a WELL-FORMED escape genuinely inside a quasiquote — which MUST evaluate its operand
    // (`metaprogramming.md` §Quasiquote Constructs AST With Selective Evaluation), so the operand's own
    // faults (an unbound name in `` `(a ,(+ b 1)) `` → CDZ0101) are collected NORMALLY. `resolved_of`
    // decides which: a `Poison` with the syntax/arity code is the defect; anything else means it is a
    // well-formed unquote whose operand should be type-checked.
    if matches!(db.ast.head_name(id), Some("unquote" | "unquote-splicing")) {
        match resolved_of(db, id) {
            Resolved::Poison(r)
                if matches!(
                    r.code,
                    Some(Code::UnquoteOutsideQuasiquote) | Some(Code::Malformed)
                ) =>
            {
                let mut r = r;
                r.set_origin_if_absent(id);
                out.push(r);
                return; // a malformed unquote — do not also type-check its (dropped) operands
            }
            // A well-formed unquote inside a quasiquote — its operand IS evaluated, so collect its faults.
            _ => {
                // OWN the head + operand before `collect` (which needs `&mut Db`) — a borrowed `head: &str`
                // / `operand` into `db.ast` would pin `db` immutable across the mutable-borrowing collect.
                let head = db.ast.head_name(id).unwrap().to_string();
                let operand = db.ast.as_form(id, &head).and_then(|t| t.first()).copied();
                if let Some(operand) = operand {
                    let before = out.len();
                    collect(db, operand, out);
                    // `,@` (unquote-splicing) splices the ELEMENTS of a LIST into the parent, so its
                    // operand MUST be a list (`metaprogramming.md` §Quasiquote Constructs AST With
                    // Selective Evaluation: ",@ evaluates <list-expr> to a LIST and splices its
                    // elements"). A splice of a PROVABLY non-list value — a scalar, a string, a tuple —
                    // has no elements to splice, so it is ill-typed (CDZ0201). CONSERVATIVE: only a type
                    // we can PROVE is not a list rejects; a still-open `Var`/`Any`, or a genuine `List`,
                    // does not — so the good `,@(list 1 2 3)` case is untouched, and an operand whose type
                    // never resolves declines (never a false reject). The `,` (unquote) operand embeds as
                    // ONE element and admits any type, so this check is `,@`-only. Only when the operand
                    // was itself fault-free (`before == out.len()`) — else the operand's OWN error (an
                    // unbound name) is the primary one, not a spurious "not a list" on top.
                    if head == "unquote-splicing" && out.len() == before {
                        let ty = type_of(db, operand);
                        if provably_not_list(&ty) {
                            out.push(
                                Reject::coded(
                                    Code::Malformed,
                                    format!(
                                        "unquote-splicing (,@) splices the elements of a list, but its \
                                         operand is {} — a value with no elements to splice",
                                        ty.render_name(&db.name_ctx())
                                    ),
                                )
                                .at(operand),
                            );
                        }
                    }
                }
                return;
            }
        }
    }
    // Descend into every child list — a nested `(unquote …)` / a `(quasiquote …)` deeper in the template.
    if let crate::ast::Struct::List(children) = db.ast.get(id) {
        for child in children.clone() {
            if matches!(db.ast.get(child), crate::ast::Struct::List(_)) {
                collect_quote_body_syntax(db, child, out);
            }
        }
    }
}

/// Validate the IRREFUTABILITY of every binding pattern (`let` LHS) in the subtree at `node`, reporting
/// CDZ0210 (refutable) / CDZ0201 (wrong-shape) / CDZ0102 (non-linear) for a genuinely ill-formed one.
/// SHAPE-ONLY: `check_binding_pattern` against `Ty::Any` classifies refutability from the pattern's shape
/// alone (a literal element / a multi-variant ctor is refutable regardless of the value type), so this
/// NEVER touches a generic/uninstantiated body's TYPE resolution — it can't produce a spurious fault the
/// way a full `collect` of an inline lambda body could. Used to close the gap where a refutable `let`
/// (or a refutable destructuring param, which the desugar moves into a body `let`) inside an INLINE
/// lambda escaped CDZ0210: `collect_node`'s Lambda arm does NOT `collect` an inline lambda body (to avoid
/// double-reporting + spurious generic-body faults), so a refutable binding there was never seen. This
/// narrow walk restores JUST the binding-position irrefutability check, matching what a def-body `let`
/// gets, without the risky full body descent. Does NOT recurse into a NESTED lambda's body (that lambda
/// gets its own `collect_node` Lambda-arm visit).
fn inline_lambda_binding_pattern_faults(db: &mut Db, node: StructId, out: &mut Vec<Reject>) {
    // A `(let <bindings> <body>)` — check each binding's LHS pattern for irrefutability.
    if let Some(tail) = db.ast.as_form(node, "let")
        && let Some(&bindings_occ) = tail.first()
        && let crate::ast::Struct::List(bindings) = db.ast.get(bindings_occ)
    {
        for pair in bindings.clone() {
            if let crate::ast::Struct::List(kv) = db.ast.get(pair)
                && let Some(&lhs) = kv.first()
                && let Err(r) = crate::lower::check_binding_pattern(db, lhs, &crate::ty::Ty::Any)
            {
                out.push(r);
            }
        }
    }
    // Recurse into ALL children, INCLUDING a nested `fn`. A lambda nested inside an inline lambda's body
    // is itself INLINE (never a registered def body), so `collect_node`'s Lambda arm never descends it (it
    // full-collects only a named-def / applied-try body) — stopping at `fn` here would leave a refutable
    // binding one level deeper UNCHECKED (reviewer-found gap in the single-level #1428 fix). Since this
    // walk is SHAPE-ONLY (irrefutability is independent of the value type — safe on a generic/uninstantiated
    // body at ANY depth), recursing into nested-lambda bodies is sound; `dedup_faults` collapses any overlap
    // with a full-collect of a named-def body reached separately.
    if let crate::ast::Struct::List(children) = db.ast.get(node) {
        for child in children.clone() {
            inline_lambda_binding_pattern_faults(db, child, out);
        }
    }
}

/// Whether the lambda `id` is the HEAD of an enclosing application — the immediately-applied form `((fn
/// (…) …) args)`, where the `fn` node is child 0 of an application list. Used by `collect`'s `Resolved::
/// Lambda` arm to check such a lambda's ORIGINAL parented body (so a `?` inside reaches its boundary via
/// the parent walk) rather than relying on the inlined-copy call-site check, whose `?` node is parentless.
fn lambda_heads_an_application(db: &Db, id: StructId) -> bool {
    let Some(parent) = db.parent_of(id) else {
        return false;
    };
    // The parent must be a plain application list (not a `(fn …)`/binder form) with `id` as its head child.
    db.child_ix_of(id) == 0
        && matches!(db.ast.get(parent), crate::ast::Struct::List(_))
        && db.ast.head_name(parent).is_none()
}

/// Whether `id`'s subtree SYNTACTICALLY contains a `(try …)` form — a cheap AST scan (no reduction). Gates
/// the application-heading-lambda body check in `collect`'s `Resolved::Lambda` arm to the ONLY case that
/// needs it (a `?` whose boundary is this lambda): descending into EVERY immediately-applied lambda body
/// re-introduced the O(2^depth) re-reduce a deep capturing-lambda chain (`((fn (a) …((fn (b) …) 1)) 0)`)
/// the `collect` baseline-skip guards against — that chain has no `try`, so this scan skips it. The scan is
/// O(body size) once per applied lambda (linear, not exponential — it walks the AST, never re-reduces).
fn subtree_contains_try_form(db: &Db, id: StructId) -> bool {
    if db.ast.head_name(id) == Some("try") {
        return true;
    }
    match db.ast.get(id) {
        crate::ast::Struct::List(children) => children
            .clone()
            .iter()
            .any(|&c| subtree_contains_try_form(db, c)),
        crate::ast::Struct::Atom(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Slot;
    use crate::testkit::{if_program, scalar_program};

    #[test]
    fn type_of_a_literal_is_a_deferred_int_rendering_as_int64() {
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        // The type of the literal node is a column read — the Stage-0 done-criterion. A bare literal
        // has a DEFERRED width (nothing fixed it), which agrees with `Int64` and renders as `Int64`.
        let t = type_of(&mut db, body);
        assert_eq!(t, Ty::int());
        assert!(t.agrees_with(&Ty::int64()));
        assert_eq!(t.render_name(&db.name_ctx()), "Int64");
    }

    #[test]
    fn type_of_an_if_is_the_branch_type_and_it_checks_clean() {
        let (ast, if_node) = if_program();
        let mut db = Db::load(ast);
        // `(if false 1 2)` : the branches are integers, so the if is an integer.
        assert!(type_of(&mut db, if_node).agrees_with(&Ty::int64()));
        // A well-typed if reports no faults.
        assert!(type_errors(&mut db, if_node).is_empty());
    }

    #[test]
    fn asking_a_type_does_not_fill_the_core_column() {
        // Laziness across modules: solving a type must not have lowered anything.
        let (ast, body) = scalar_program();
        let mut db = Db::load(ast);
        let _ = type_of(&mut db, body);
        assert!(
            matches!(db.core.get(body), Slot::Absent),
            "core filled without demand"
        );
    }

    // ── def_scheme (ANF step 2, sub-increment A1): a def's signature as a value ────────────────────
    //
    // These pin the FOUNDATION only: a fully-determined (annotated) def's scheme is its curried
    // signature and it AGREES with what β-reduction produces at a call; an undetermined (unannotated /
    // recursive) def declines the scheme (defers to β-reduction). No caller reads it yet.

    use crate::testkit::parse;

    /// The def index of `name` in a parsed program.
    fn def_of(db: &Db, name: &str) -> usize {
        db.def_by_name(name).expect("def present")
    }

    #[test]
    fn def_scheme_of_an_annotated_function_is_its_curried_signature() {
        // (def (add (: a Int64) (: b Int64)) (+ a b)) → Int64 -> Int64 -> Int64.
        let ast = parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) 0) (export main))",
        );
        let mut db = Db::load(ast);
        let d = def_of(&db, "add");
        let scheme = def_scheme(&mut db, d).expect("determined scheme");
        let expected = Ty::Fn(
            Box::new(Ty::int64()),
            Box::new(Ty::Fn(Box::new(Ty::int64()), Box::new(Ty::int64()))),
        );
        assert!(
            scheme.ty.agrees_with(&expected),
            "add scheme {} != Int64->Int64->Int64",
            scheme.ty.render_name(&db.name_ctx())
        );
    }

    #[test]
    fn def_scheme_agrees_with_beta_reduction_at_a_call() {
        // The scheme's RESULT (after applying both args) must equal the type β-reduction gives the
        // call `(add 20 22)` — the cross-check that the scheme is a faithful stand-in for inlining.
        let ast = parse(
            "(module m (def (add (: a Int64) (: b Int64)) (+ a b)) (def (main) (add 20 22)) (export main))",
        );
        let mut db = Db::load(ast);
        // β-reduction path: type of the call node in main's body.
        let main_body = db.defs[def_of(&db, "main")].body.expect("main body");
        let call_ty = type_of(&mut db, main_body);
        // scheme path: peel the two Fn arrows.
        let d = def_of(&db, "add");
        let scheme = def_scheme(&mut db, d).expect("determined scheme");
        let result = match scheme.ty {
            Ty::Fn(_, r) => match *r {
                Ty::Fn(_, r2) => *r2,
                other => other,
            },
            other => other,
        };
        assert!(
            call_ty.agrees_with(&result),
            "β-reduced call type {} disagrees with scheme result {}",
            call_ty.render_name(&db.name_ctx()),
            result.render_name(&db.name_ctx())
        );
    }

    #[test]
    fn def_scheme_of_a_nullary_def_is_its_body_type() {
        let ast = parse("(module m (def (main) 42) (export main))");
        let mut db = Db::load(ast);
        let d = def_of(&db, "main");
        let scheme = def_scheme(&mut db, d).expect("nullary scheme");
        assert!(scheme.ty.agrees_with(&Ty::int64()));
    }

    #[test]
    fn def_scheme_declines_an_unannotated_parameter() {
        // `(def (id x) x)` — `x` is unannotated (`Any`), so its signature needs the connected solve
        // (A2). A1 declines the scheme and defers to β-reduction.
        let ast = parse("(module m (def (id x) x) (def (main) (id 5)) (export main))");
        let mut db = Db::load(ast);
        let d = def_of(&db, "id");
        assert!(
            def_scheme(&mut db, d).is_none(),
            "an unannotated param must defer to β-reduction (A2 territory)"
        );
    }

    #[test]
    fn def_scheme_of_an_annotated_recursive_def_is_determined_by_absorption() {
        // KEY FINDING (shrinks A2): an ANNOTATED recursive def types WITHOUT an explicit fixpoint. The
        // self-call `(sum-to …)` returns `Any` (the recursion guard in `apply_type`), and `Any` is
        // ABSORBED by unification/join with the concrete parts — the base case `0` and `(+ n …)` pin
        // the result to Int64. So `def_scheme(sum-to)` = Int64 -> Int64 already, and this is
        // order-independent (the self-call is always `Any` regardless of visit order; the concrete
        // branch determines the type). An explicit recursion fixpoint (A2) is only needed when NO
        // concrete part pins the result — which, for terminating monomorphic recursion, cannot happen
        // (there must be a base case, and it pins the type).
        let ast = parse(
            "(module m (def (sum-to (: n Int64)) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (def (main) (sum-to 3)) (export main))",
        );
        let mut db = Db::load(ast);
        let d = def_of(&db, "sum-to");
        let scheme = def_scheme(&mut db, d).expect("annotated recursive def types via absorption");
        let expected = Ty::Fn(Box::new(Ty::int64()), Box::new(Ty::int64()));
        assert!(
            scheme.ty.agrees_with(&expected),
            "sum-to scheme {} != Int64->Int64",
            scheme.ty.render_name(&db.name_ctx())
        );
    }

    #[test]
    fn def_scheme_of_an_unannotated_recursive_def_is_solved_by_a2() {
        // The ACTUAL corpus `sum-to` has an UNANNOTATED param `(def (sum-to n) …)`. A2's connected solve
        // (`solve_recursive_params`) infers `n : Int64` from its uses (`(= n 0)`, `(+ n …)`), so the def
        // now HAS a scheme `Int64 -> Int64` — where before A2 it declined. (The mechanism the recursive
        // corpus rides.)
        let ast = parse(
            "(module m (def (sum-to n) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (def (main) (sum-to 3)) (export main))",
        );
        let mut db = Db::load(ast);
        let d = def_of(&db, "sum-to");
        let scheme = def_scheme(&mut db, d).expect("A2 solves the unannotated recursive signature");
        let expected = Ty::Fn(Box::new(Ty::int64()), Box::new(Ty::int64()));
        assert!(
            scheme.ty.agrees_with(&expected),
            "sum-to scheme {} != Int64->Int64",
            scheme.ty.render_name(&db.name_ctx())
        );
    }

    #[test]
    fn recursive_param_solve_is_order_independent() {
        // The NON-NEGOTIABLE property (build-order Stage 2 "done when"; the coarse-kind post-mortem): a
        // recursive def's parameter type is the SAME regardless of which node's type is demanded first.
        // Solve `sum-to`'s param via two different first-demands and assert they agree.
        let src = "(module m (def (sum-to n) (if (= n 0) 0 (let ((r (sum-to (+ n -1)))) (+ n r)))) (def (main) (sum-to 3)) (export main))";

        // Order A: demand the def's scheme first (drives the solve from the signature).
        let mut db_a = Db::load(parse(src));
        let da = def_of(&db_a, "sum-to");
        let sa = def_scheme(&mut db_a, da).expect("scheme A");

        // Order B: demand `main`'s body type first (drives the solve from the call site), THEN the scheme.
        let mut db_b = Db::load(parse(src));
        let main_b = db_b.defs[def_of(&db_b, "main")].body.expect("main body");
        let _ = type_of(&mut db_b, main_b);
        let db_b_def = def_of(&db_b, "sum-to");
        let sb = def_scheme(&mut db_b, db_b_def).expect("scheme B");

        assert_eq!(
            sa.ty.render_name(&db_a.name_ctx()),
            sb.ty.render_name(&db_b.name_ctx()),
            "recursive param solve must be order-independent"
        );
    }

    #[test]
    fn rewrapping_a_recursive_results_err_payload_keeps_the_error_type() {
        // REGRESSION (fresh-var collision in `payload_ty_at_instantiation`): a recursive `f` typed
        // `(Result A ?err)` (no branch fixes the error slot, so it stays a free var), matched by a helper
        // `g` that RE-WRAPS `(Result.Err et)`. The `Err` ctor scheme (`∀a b. b -> Result a b`) was
        // instantiated from 0, colliding with the scrutinee's free `?err = ?0` — so `et` solved to `A`
        // (the FIRST type arg) and `g` typed `(Result A A)`, tripping a spurious CDZ0203. Seeding the ctor
        // instantiation PAST the scrutinee's vars keeps them disjoint, so `et` solves to the scrutinee's
        // OWN error var and the whole match types `(Result A ?err)`. Assert the error slot is NOT `A`.
        let src = "(module m \
            (type A AY AN) \
            (type Exp (Num Int64) (If Exp Exp)) \
            (def (f (: e Exp)) (match e ((Exp.Num _) (Result.Ok AY)) ((Exp.If c t) (f c)))) \
            (def (g (: t Exp)) (match (f t) ((Result.Ok tt) (Result.Ok tt)) ((Result.Err et) (Result.Err et)))) \
            (def (main) 0) (export main))";
        let mut db = Db::load(parse(src));
        let d = def_of(&db, "g");
        let scheme = def_scheme(&mut db, d).expect("g has a determined scheme");
        // g : Exp -> (Result A ?err). Peel the one arrow to the result.
        let result = match scheme.ty {
            Ty::Fn(_, r) => *r,
            other => other,
        };
        // The result must be a `Result` whose FIRST arg is `A` and whose SECOND (error) arg is NOT `A` —
        // the collision made both `A`. It should be a free var (`?err`, no branch fixed it).
        match result {
            Ty::Sum { args, .. } if args.len() == 2 => {
                assert_eq!(
                    args[0].render_name(&db.name_ctx()),
                    "A",
                    "ok payload should be A"
                );
                assert!(
                    !matches!(&args[1], Ty::Sum { .. }),
                    "error slot must NOT collapse to the ok type A (collision bug); got {}",
                    args[1].render_name(&db.name_ctx())
                );
            }
            other => panic!(
                "g result should be a 2-arg Result sum, got {}",
                other.render_name(&db.name_ctx())
            ),
        }
    }
}
