//! Unification — the pure Hindley-Milner core: a substitution, unification with occurs-check, and
//! scheme instantiation. This is the machinery the ONE generic application rule uses (`infer`), and
//! that the connected function-parameter solve (`infer::solve_recursive_params`) reuses unchanged.
//!
//! It is a deterministic, order-independent solve over type, width, and sign variables:
//!
//= spec/capabilities/type-system.md#inference-is-principal-type-inference-by-unification
//# Type inference MUST determine types by unification over type variables — solving the equality constraints a program's structure imposes — so that a type is derived from how each binding is used rather than assumed or guessed from a single use site.
//= spec/capabilities/type-system.md#inference-is-principal-type-inference-by-unification
//# The type inference determines for an expression MUST be its principal type: the most general type from which every other valid type of that expression is an instance, so that inference commits to no more than the program's uses require.
//!
//! The solve is a most-general one: `unify` only equates variables the uses force and `instantiate`
//! freshens a scheme's bound variables per use, so the type inferred is the PRINCIPAL type — inference
//! commits to no more than the constraints require, and every other valid typing is an instance of it.
//!
//! A [`Subst`] maps a variable to the type (or width/sign) it has been solved to; `unify` extends it
//! by equating two types, failing (a [`Reject`]) when they cannot be made equal — the conflicting-use
//! type error. `instantiate` freshens a [`Scheme`]'s bound variables so each use is independent, which
//! is what makes an operator generic over the integer type. Nothing here reads a column or an AST node
//! — it is pure over [`Ty`].
//!
//! Because it is pure over [`Ty`] — a non-computational structural type carrying no embedded computation
//! (any type-VALUE computation is reduced in the evaluator (`eval::typeval_of`) BEFORE it reaches a `Ty`,
//! at the bidirectional binder boundary) — the variables unification solves never carry a computation
//! whose reduction it cannot decide.
//= spec/capabilities/type-system.md#inference-and-first-class-types-meet-at-a-bidirectional-boundary
//# Unification-based inference MUST range over a non-computational term core, so that the type variables inference solves never carry a computation whose reduction principal-type inference cannot decide.
//!
//! Because the solution is ONE substitution shared across a binding's uses, a variable solved at one
//! occurrence is applied at every other — a parameter used in one position is typed consistently at
//! every occurrence and every call site (`infer` reads the solved type through this `Subst`). And when
//! two uses impose contradictory constraints the `unify` has no solution: it returns a `Reject` coded
//! for the conflicting-use type error rather than compiling:
//!
//= spec/capabilities/type-system.md#inference-is-principal-type-inference-by-unification
//# Inference MUST propagate a determined type to every occurrence of the binding it constrains, so that a parameter used in one position is typed consistently at every other occurrence and at every call site.
//= spec/capabilities/type-system.md#inference-is-principal-type-inference-by-unification
//# A program for which unification has no solution — a use that imposes contradictory constraints on a type variable — MUST be rejected at compile time with the machine-readable code for the conflicting-use type error, rather than compiled.
//!
//! Unification equates types by EQUALITY — two types either unify or the program is rejected; there is
//! no widening/narrowing arm that would let one type stand in for another, so the type system never
//! inserts an implicit subtyping coercion the program did not write:
//!
//= spec/capabilities/type-system.md#subtyping-is-explicit-or-absent
//# The type system MUST NOT introduce an implicit subtyping coercion that the program did not write.

use crate::diag::{Code, Reject};
use crate::fxhash::FxHashMap;
use crate::ty::{IntTy, NameCtx, Scheme, Sign, Ty, Width};
use tracing::trace;

/// The depth cap on `Subst::apply`'s combined var-chain + structural descent. A cyclic substitution
/// binding (one that bypassed `unify`'s occurs-check — e.g. `?v := (Option ?v)` from an UNDER-APPLIED
/// generic) would otherwise recurse forever and stack-overflow/abort the compiler. This bound is FAR above
/// any real type's depth: a genuine annotation/inferred type nests only a handful of levels, and even a
/// pathologically deep generic is well under this — so a well-formed program never approaches it, while a
/// cycle blows past it and is broken to `Ty::Any` (a clean decline downstream instead of a crash).
///
/// TIED to [`crate::db::DESCENT_DEPTH_LIMIT`] — the compiler-wide native-recursion stack-sizing policy (the
/// worker stack in `host.rs` is sized from it). `apply_depth` is itself NATIVELY recursive (each `chain + 1`
/// is a real frame, and the structural arms recurse on top of that), so the guard MUST fire within that
/// policy: a bound above it (the former `10_000`, ~10× the 1024 policy) risks a hard stack OVERFLOW on a
/// small-stack target (wasm32) BEFORE the guard ever fires — the exact crash the guard exists to prevent.
/// The policy bound is still far above any real type's var-chain depth, so a well-formed program is
/// byte-identical; a cycle is now guaranteed to be broken to `Ty::Any` before the stack is exhausted.
const APPLY_VAR_CHAIN_LIMIT: u32 = crate::db::DESCENT_DEPTH_LIMIT;

/// A substitution: what each type, width, and SIGN variable has been solved to. Applied to a type,
/// it replaces solved variables with their solutions (transitively).
#[derive(Clone, Debug, Default)]
pub struct Subst {
    tys: FxHashMap<u32, Ty>,
    widths: FxHashMap<u32, Width>,
    signs: FxHashMap<u32, Sign>,
}

impl Subst {
    pub fn new() -> Subst {
        Subst::default()
    }

    /// Apply the substitution to a type — replace every solved variable with its solution, following
    /// chains (a variable solved to another variable resolves through). Total and terminating: the
    /// occurs-check normally keeps the variable graph acyclic, but a CYCLIC binding that slipped in through
    /// a path bypassing `unify`'s occurs-check (e.g. a direct `subst` insert during recursive-generic
    /// monomorphization of an UNDER-APPLIED generic — `(: x (Option Box))` bound `?v := (Option ?v)`) would
    /// make the naive `Ty::Var` chain-follow recurse forever and STACK-OVERFLOW/abort `rcdzc-compile` (not
    /// a catchable panic — a crash on ill-formed input). `apply` is the universal read path, so guard it
    /// here with a depth cap on the var-chain: a chain longer than `APPLY_VAR_CHAIN_LIMIT` is a cycle → stop
    /// and yield `Ty::Any` (the cyclic type is genuinely infinite/ill-formed; `Any` lets the fault walk
    /// report a clean decline instead of the compiler aborting). Defense-in-depth: it protects against ANY
    /// occurs-check bypass, present or future, at the one place every type read funnels through.
    pub fn apply(&self, ty: &Ty) -> Ty {
        self.apply_depth(ty, 0)
    }

    fn apply_depth(&self, ty: &Ty, chain: u32) -> Ty {
        // GROUND FAST-PATH: a type with no substitutable variable of any kind is an `apply` fixpoint, so
        // returning `ty.clone()` is correct — and for an Rc-shared `Record`/`Tuple`/`Sum` that clone is a
        // refcount bump, NOT the deep `.iter().map(apply).collect()` rebuild (+ its BTreeMap drop) the arms
        // below would do. `unify` applies BOTH operands on entry, so a wide GROUND record unified per call
        // site (a wide-record arg passed to a function called N times) rebuilt its whole field map every
        // call → O(width × calls); this makes it O(1) per call. `is_ground` walks once (O(width)) and
        // short-circuits, replacing an allocate-and-rebuild with a read-only check + pointer clone.
        if ty.is_ground() {
            return ty.clone();
        }
        match ty {
            // The var-chain follow is the ONLY unbounded recursion (structural arms below descend a FINITE
            // type). A chain longer than the limit means a cyclic binding bypassed the occurs-check — break
            // it with `Ty::Any` rather than overflow. The limit is far above any real solved-var chain
            // depth (union-find chains are short; even a pathological deep generic is well under it), so a
            // well-formed program is byte-identical.
            Ty::Var(v) if chain >= APPLY_VAR_CHAIN_LIMIT => {
                trace!(target: "rcdzc::unify", var = *v, "apply: var-chain depth limit hit → Any (cyclic substitution — occurs-check bypass)");
                Ty::Any
            }
            Ty::Var(v) => match self.tys.get(v) {
                Some(t) => self.apply_depth(t, chain + 1),
                None => Ty::Var(*v),
            },
            Ty::Int(it) => Ty::Int(IntTy {
                sign: self.apply_sign(it.sign),
                width: self.apply_width(it.width),
            }),
            // A float's width may be a `Var` (an operator generic over the float width), resolved the
            // SAME way an integer's width is — floats reuse the integer width machinery.
            Ty::Float(ft) => Ty::Float(crate::ty::FloatTy {
                width: self.apply_width(ft.width),
            }),
            // Structural arms thread `chain` through — a cyclic binding `?v := (Option ?v)` re-enters a
            // `Ty::Var` one structural level down, so the chain counter must keep climbing across the
            // descent (resetting it per structural node would let the cycle loop forever). A well-formed
            // (finite) type still terminates on its own structure long before the limit.
            Ty::Fn(p, r) => Ty::Fn(
                Box::new(self.apply_depth(p, chain + 1)),
                Box::new(self.apply_depth(r, chain + 1)),
            ),
            Ty::Cont { resume, answer } => Ty::Cont {
                resume: Box::new(self.apply_depth(resume, chain + 1)),
                answer: Box::new(self.apply_depth(answer, chain + 1)),
            },
            Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
                fields
                    .iter()
                    .map(|(k, t)| (k.clone(), self.apply_depth(t, chain + 1)))
                    .collect(),
            )),
            Ty::Tuple(elems) => Ty::Tuple(
                elems
                    .iter()
                    .map(|t| self.apply_depth(t, chain + 1))
                    .collect(),
            ),
            // A list substitutes into its element type (a deferred `List ?0` — the empty-list case).
            Ty::List(elem) => Ty::List(Box::new(self.apply_depth(elem, chain + 1))),
            // A map substitutes into its key AND value types (a deferred `Map ?0 ?1` — the empty-map case).
            Ty::Map(k, v) => Ty::Map(
                Box::new(self.apply_depth(k, chain + 1)),
                Box::new(self.apply_depth(v, chain + 1)),
            ),
            // A set substitutes into its element type (a deferred `Set ?0` — an empty set).
            Ty::Set(elem) => Ty::Set(Box::new(self.apply_depth(elem, chain + 1))),
            // A sum's identity is its `decl`, but a GENERIC instantiation carries type ARGS that may hold
            // unsolved variables (a deferred payload — `Option ?0`), so substitute into each arg. A
            // monomorphic sum has empty args, so this is a cheap clone of the name/decl.
            Ty::Sum { decl, args } => Ty::Sum {
                decl: *decl,
                args: args
                    .iter()
                    .map(|t| self.apply_depth(t, chain + 1))
                    .collect(),
            },
            // A quantity substitutes into its INNER numeric type (a deferred inner width — `(Qty ?0 u)` —
            // solves); the unit is a concrete compile-time value, carried unchanged.
            Ty::Qty { inner, unit } => Ty::Qty {
                inner: Box::new(self.apply_depth(inner, chain + 1)),
                unit: unit.clone(),
            },
            // A nominal substitutes into its type ARGS (a generic `Box ?0` — the deferred instantiation)
            // AND its `inner` machine-rep hint, so a solved var flows into both; `decl`/`name` unchanged.
            Ty::Nominal { decl, args, inner } => Ty::Nominal {
                decl: *decl,
                args: args
                    .iter()
                    .map(|t| self.apply_depth(t, chain + 1))
                    .collect(),
                inner: std::rc::Rc::new(self.apply_depth(inner, chain + 1)),
            },
            Ty::Bool
            | Ty::Unit
            | Ty::Bytes
            | Ty::String
            | Ty::Char
            | Ty::Symbol
            | Ty::BigInt
            | Ty::Rational
            | Ty::Type
            | Ty::Any => ty.clone(),
        }
    }

    /// Apply the substitution to a width — resolve a solved width variable.
    fn apply_width(&self, w: Width) -> Width {
        match w {
            Width::Var(v) => match self.widths.get(&v) {
                Some(&sol) => self.apply_width(sol),
                None => Width::Var(v),
            },
            other => other,
        }
    }

    /// Apply the substitution to a sign — resolve a solved sign variable.
    fn apply_sign(&self, s: Sign) -> Sign {
        match s {
            Sign::Var(v) => match self.signs.get(&v) {
                Some(&sol) => self.apply_sign(sol),
                None => Sign::Var(v),
            },
            other => other,
        }
    }
}

/// Unify two types, extending `subst` so both become equal, or return the conflicting-use type error.
/// Order-independent: `unify(a, b)` and `unify(b, a)` reach the same solution. `Ty::Any` (a poison's
/// type) unifies with anything so a "no" never induces a spurious conflict.
pub fn unify(subst: &mut Subst, a: &Ty, b: &Ty, ncx: &NameCtx) -> Result<(), Reject> {
    let a = subst.apply(a);
    let b = subst.apply(b);
    match (&a, &b) {
        // A poison type never conflicts.
        (Ty::Any, _) | (_, Ty::Any) => Ok(()),
        // Two variables / a variable and a type: bind the variable (occurs-check first).
        (Ty::Var(v), Ty::Var(w)) if v == w => Ok(()),
        (Ty::Var(v), t) | (t, Ty::Var(v)) => {
            if occurs(subst, *v, t) {
                trace!(target: "rcdzc::unify", var = *v, ty = %t.render_name(ncx), "occurs-check failed (infinite type)");
                return Err(Reject::coded(
                    Code::TypeMismatch,
                    "a type would contain itself (infinite type)",
                ));
            }
            trace!(target: "rcdzc::unify", var = *v, solved = %t.render_name(ncx), "bind type variable");
            subst.tys.insert(*v, t.clone());
            Ok(())
        }
        (Ty::Bool, Ty::Bool) | (Ty::Unit, Ty::Unit) | (Ty::Type, Ty::Type) => Ok(()),
        // Bytes is a leaf — two bytes unify reflexively (no element type to descend into).
        (Ty::Bytes, Ty::Bytes) => Ok(()),
        // Integers unify on BOTH axes — sign and width. A variable/deferred axis resolves to the
        // other's fixed value; two DIFFERENT fixed values conflict (no implicit promotion — neither a
        // width nor a signedness silently changes). Unifying the sign first lets a `mismatch` name the
        // conflict; a deferred literal (`Deferred` on both axes) grounds to whatever it meets.
        (Ty::Int(ia), Ty::Int(ib)) => {
            unify_sign(subst, ia.sign, ib.sign, &a, &b, ncx)?;
            unify_width(subst, ia.width, ib.width, NumericKind::Int)
        }
        (Ty::Fn(pa, ra), Ty::Fn(pb, rb)) => {
            unify(subst, pa, pb, ncx)?;
            unify(subst, ra, rb, ncx)
        }
        // Two records unify (hence compare) only at an IDENTICAL field-name set, two tuples only at equal
        // arity, two sums only at the same decl (variant set) — a shape mismatch is a `mismatch` (CDZ0203),
        // rejected as a type error rather than answered `false`. Since `=` unifies its operand types, a
        // comparison of differing-shape structural values is caught here, not computed.
        //= spec/capabilities/type-system.md#structural-values-are-comparable-only-when-their-shapes-match
        //# Two records MUST be comparable only when their sets of field names are identical, two tuples only when their lengths are identical, and two sums only when their variant sets are identical, because values of different shapes have no meaningful equality.
        //= spec/capabilities/type-system.md#structural-values-are-comparable-only-when-their-shapes-match
        //# A comparison of two structural values whose shapes differ MUST be rejected by a type-tracking generation as a type error rather than reported as unequal, so that a shape mismatch is caught rather than answered.
        (Ty::Record(fa), Ty::Record(fb)) => {
            if fa.len() != fb.len() {
                return Err(mismatch(&a, &b, ncx));
            }
            for (k, ta) in fa.iter() {
                match fb.get(k) {
                    Some(tb) => unify(subst, ta, tb, ncx)?,
                    None => return Err(mismatch(&a, &b, ncx)),
                }
            }
            Ok(())
        }
        // Tuples unify at EQUAL ARITY, position by position — a different arity is an irreconcilable
        // type (no structural subtyping), so it is the conflicting-use error.
        (Ty::Tuple(ea), Ty::Tuple(eb)) => {
            if ea.len() != eb.len() {
                return Err(mismatch(&a, &b, ncx));
            }
            for (ta, tb) in ea.iter().zip(eb.iter()) {
                unify(subst, ta, tb, ncx)?;
            }
            Ok(())
        }
        // Two lists unify iff their ELEMENT types unify — this is what makes every element of `(list …)`
        // share one type (a mixed list fails here) and solves a deferred element (an empty `(list)` : List
        // ?0 unified against a `List Int64`).
        (Ty::List(ea), Ty::List(eb)) => unify(subst, ea, eb, ncx),
        // Two maps unify iff their KEY types unify AND their VALUE types unify — a map's identity is
        // `Map<K,V>`, parametric in the key and value types (its key SET is runtime data, not part of the
        // type). This solves a deferred key/value (an empty `Map.empty` : `Map ?0 ?1` unified against a
        // `Map Int64 Int64`) and makes `Map Int64 Int64` conflict with `Map Int64 Bool`. Crucially it does
        // NOT compare key sets, so two maps with different keys unify (well-typed comparison → `false`).
        (Ty::Map(ka, va), Ty::Map(kb, vb)) => {
            unify(subst, ka, kb, ncx)?;
            unify(subst, va, vb, ncx)
        }
        // Two sets unify iff their ELEMENT types unify — `Set Int64` conflicts with `Set Bool`; solves a
        // deferred element (an empty set `Set ?0` unified against `Set Int64`). Does NOT compare element
        // sets (runtime data), so two sets with different elements unify (well-typed comparison → `false`).
        (Ty::Set(a), Ty::Set(b)) => unify(subst, a, b, ncx),
        // Two sums unify iff they are the SAME declaration AND their type ARGS unify pairwise — a sum's
        // identity is its declaration OCCURRENCE (`type-system.md` §158/§160), NOT its name (two `(type
        // Foo …)` declared separately are DISTINCT types even with the same name), together with its
        // instantiation: `Option Int64` and `Option Bool` share a declaration but their args conflict, so
        // they do NOT unify (§the head agrees but the payload does not). A monomorphic sum has empty args
        // on both sides, so this reduces to the decl check (the case a sibling added so annotating a
        // parameter with its own monomorphic sum type is not rejected "cannot unify N with N"). Unifying
        // the args also SOLVES a deferred payload (a generic instantiation against a concrete one).
        (
            Ty::Sum {
                decl: da, args: aa, ..
            },
            Ty::Sum {
                decl: db, args: ab, ..
            },
        ) => {
            if da != db || aa.len() != ab.len() {
                return Err(mismatch(&a, &b, ncx));
            }
            for (x, y) in aa.iter().zip(ab.iter()) {
                unify(subst, x, y, ncx)?;
            }
            Ok(())
        }
        // Two nominals unify iff they are the SAME declaration AND their type ARGS unify pairwise — the
        // exact `Ty::Sum` rule. `decl` is the identity (distinct declarations conflict even with identical
        // shape); `args` is the instantiation, so `Box Int64` and `Box Bool` conflict on their arg unify.
        // NOT `inner` — a RECURSIVE nominal's inner diverges by derivation path (`Ty::Sum{decl}` back-edge
        // vs `Ty::Nominal{decl}`), so unifying it would make `Lst` conflict with `Lst`; unifying `args`
        // (empty for a monomorphic recursive newtype → decl-equality suffices) is the μ-type-safe rule. A
        // `Ty::Nominal` NEVER unifies with a bare `inner` (`(Nominal, Int)` → the `mismatch` below), so a
        // nominal value cannot cross its own boundary as its underlying type (§Nominal Types Are Not
        // Comparable Across Their Boundary).
        (
            Ty::Nominal {
                decl: da, args: aa, ..
            },
            Ty::Nominal {
                decl: db, args: ab, ..
            },
        ) => {
            if da != db || aa.len() != ab.len() {
                return Err(mismatch(&a, &b, ncx));
            }
            for (x, y) in aa.iter().zip(ab.iter()) {
                unify(subst, x, y, ncx)?;
            }
            Ok(())
        }
        // A RECURSIVE NOMINAL, FOLDED vs UNFOLDED. A recursive newtype `(type Lst (Mk (Option (Tuple Int64
        // Lst))))` presents its μ back-edge as a bare `Ty::Sum{decl}` (the derivation-path divergence noted
        // in the Nominal arm above) while the DECLARED / value form is `Ty::Nominal{decl}` — SAME `decl`.
        // So a recursive field PROJECTED out of the compound (`(. p 1) : Ty::Sum{decl:Lst}`) must unify
        // with the folded declared param (`Ty::Nominal{decl:Lst}`) — they are the one recursive type, just
        // reached by different paths (fold vs unfold). Match on EQUAL `decl` (a nominal's back-edge reuses
        // its own declaration occurrence; a genuine sum's decl is a distinct `(type …)` occurrence, so this
        // never conflates `Option` with `Lst`) + args, exactly as the two same-variant arms do. Without
        // this, the canonical recursive-newtype traversal (`count`/`length`/`sum` recursing on the tail)
        // was rejected CDZ0203 "Lst and Lst differ".
        (
            Ty::Sum {
                decl: da, args: aa, ..
            },
            Ty::Nominal {
                decl: db, args: ab, ..
            },
        )
        | (
            Ty::Nominal {
                decl: da, args: aa, ..
            },
            Ty::Sum {
                decl: db, args: ab, ..
            },
        ) if da == db && aa.len() == ab.len() => {
            for (x, y) in aa.iter().zip(ab.iter()) {
                unify(subst, x, y, ncx)?;
            }
            Ok(())
        }
        // `String` is monomorphic — it unifies only with itself (no element/arg to recurse on).
        (Ty::String, Ty::String) => Ok(()),
        // `Char` is monomorphic — it unifies only with itself.
        (Ty::Char, Ty::Char) => Ok(()),
        // `Symbol` is monomorphic — it unifies only with itself (not with the `String` it wraps: the
        // nominal boundary, which falls to `mismatch` below).
        (Ty::Symbol, Ty::Symbol) => Ok(()),
        // `BigInt` is monomorphic — it unifies only with itself, NEVER with a fixed-width `Ty::Int` (no
        // silent promotion: an `Int64`/`BigInt` mix falls to `mismatch` below, CDZ0301).
        (Ty::BigInt, Ty::BigInt) => Ok(()),
        // `Rational` is monomorphic — it unifies only with itself, NEVER with an integer or a `BigInt` (no
        // silent promotion: a `Rational`/integer mix falls to `mismatch` below, CDZ0301).
        (Ty::Rational, Ty::Rational) => Ok(()),
        // Two floats unify iff their WIDTHS unify — reusing the integer `unify_width` (a width variable is
        // a width variable). So `Float32`/`Float64` are distinct (two fixed widths conflict → CDZ0301),
        // a deferred/variable float width solves. A float does NOT unify with `Ty::Int` (it falls to the
        // `mismatch` below, coded CDZ0301 as two-different-numeric), so `(+ 2 2.0)` is rejected.
        (Ty::Float(fa), Ty::Float(fb)) => {
            unify_width(subst, fa.width, fb.width, NumericKind::Float)
        }
        // Two quantities unify iff their UNITS are EQUAL and their INNER numeric types unify. A unit is a
        // concrete compile-time value (never a variable), so it is not solved by unification — it is a
        // side condition, exactly like a sum's `decl`. Unequal units are a DIMENSIONAL mismatch, but the
        // dimensional diagnostic (CDZ0501) is raised by the units post-check (`units.rs`) which has the
        // operator context; here (the plain HM seam — e.g. an annotation `(: e (Qty T u))` unifying its
        // declared unit against the derived one) an unequal unit is the general `mismatch`. Unifying the
        // inner types keeps the numeric core's no-promotion rule (an `Int64`-quantity vs a `Float64`-
        // quantity conflicts CDZ0301 through the inner `unify`). A quantity never unifies with a
        // non-quantity (a meter vs a bare number) — that falls to the `mismatch` below.
        (
            Ty::Qty {
                inner: ia,
                unit: ua,
            },
            Ty::Qty {
                inner: ib,
                unit: ub,
            },
        ) => {
            if ua != ub {
                return Err(mismatch(&a, &b, ncx));
            }
            unify(subst, ia, ib, ncx)
        }
        _ => Err(mismatch(&a, &b, ncx)),
    }
}

/// Unify two signednesses — a variable takes the other; a deferred sign is compatible with anything
/// (it grounds later); two DIFFERENT fixed signs conflict (a signed and an unsigned integer are
/// distinct types — no silent promotion). `a`/`b` are the enclosing integer types, for the error.
fn unify_sign(
    subst: &mut Subst,
    sa: Sign,
    sb: Sign,
    a: &Ty,
    b: &Ty,
    ncx: &NameCtx,
) -> Result<(), Reject> {
    let sa = resolve_sign(subst, sa);
    let sb = resolve_sign(subst, sb);
    match (sa, sb) {
        (Sign::Var(v), Sign::Var(w)) if v == w => Ok(()),
        // A sign variable meeting a DEFERRED literal sign stays UNBOUND (mirrors `unify_width`): binding
        // it to `Deferred` would freeze the var and ignore a later concrete sign, so `(+ 1 n)` with `n :
        // UInt8` (unsigned) would wrongly ground the shared sign to the signed default. Leaving it open
        // lets `n`'s `Fixed(false)` bind it — operand order no longer changes the result's signedness.
        (Sign::Var(_), Sign::Deferred) | (Sign::Deferred, Sign::Var(_)) => Ok(()),
        (Sign::Var(v), other) | (other, Sign::Var(v)) => {
            trace!(target: "rcdzc::unify", var = v, solved = ?other, "bind sign variable");
            subst.signs.insert(v, other);
            Ok(())
        }
        // A deferred (literal) sign takes whatever it meets.
        (Sign::Deferred, _) | (_, Sign::Deferred) => Ok(()),
        (Sign::Fixed(x), Sign::Fixed(y)) if x == y => Ok(()),
        (Sign::Fixed(x), Sign::Fixed(y)) => {
            trace!(target: "rcdzc::unify", lhs = x, rhs = y, "sign conflict (CDZ0301, no silent promotion)");
            Err(mismatch(a, b, ncx))
        }
    }
}

fn resolve_sign(subst: &Subst, s: Sign) -> Sign {
    match s {
        Sign::Var(v) => match subst.signs.get(&v) {
            Some(&sol) => resolve_sign(subst, sol),
            None => Sign::Var(v),
        },
        other => other,
    }
}

/// Unify two widths — a variable/deferred width takes the other; two different fixed widths conflict.
fn unify_width(subst: &mut Subst, a: Width, b: Width, kind: NumericKind) -> Result<(), Reject> {
    let a = resolve_width(subst, a);
    let b = resolve_width(subst, b);
    match (a, b) {
        (Width::Var(v), Width::Var(w)) if v == w => Ok(()),
        // A width variable meeting a DEFERRED literal width stays UNBOUND — `Deferred` is "unconstrained,"
        // not a solution, so binding the var to it would freeze the var and IGNORE a later concrete width.
        // `(+ 1 n)` with `n : UInt8` instantiates `+`'s param to `Var(w)`; unifying the deferred `1` first
        // must NOT set `w = Deferred` (then `n`'s `Fixed(8)` meeting the now-`Deferred` `w` is a no-op and
        // the result grounds to the i64 default — the const-LEFT-operand invalid-wasm bug). Leaving `w`
        // open lets `n`'s `Fixed(8)` bind it, so `(+ 1 n)` solves UInt8 exactly as `(+ n 1)` does —
        // operand order no longer changes the result width.
        (Width::Var(_), Width::Deferred) | (Width::Deferred, Width::Var(_)) => Ok(()),
        (Width::Var(v), other) | (other, Width::Var(v)) => {
            trace!(target: "rcdzc::unify", var = v, solved = ?other, "bind width variable");
            subst.widths.insert(v, other);
            Ok(())
        }
        // A deferred (literal) width takes whatever it is unified with.
        (Width::Deferred, _) | (_, Width::Deferred) => Ok(()),
        (Width::Fixed(x), Width::Fixed(y)) if x == y => Ok(()),
        (Width::Fixed(x), Width::Fixed(y)) => {
            trace!(target: "rcdzc::unify", lhs = x, rhs = y, "width conflict (CDZ0301, no silent promotion)");
            // The message names the RIGHT numeric domain: an integer width vs a floating-point precision.
            // A `Float32`/`Float64` mismatch used to read "integer widths differ … never silently widens
            // or narrows an INTEGER", which is wrong (the values are floats) — `kind` picks the wording.
            let msg = match kind {
                NumericKind::Int => format!(
                    "integer widths differ: {x}-bit vs {y}-bit — convert explicitly (Cadenza never \
                     silently widens or narrows an integer)"
                ),
                NumericKind::Float => format!(
                    "floating-point precisions differ: {x}-bit vs {y}-bit — convert explicitly \
                     (Cadenza never silently widens or narrows a float)"
                ),
            };
            Err(Reject::coded(Code::NumericMismatch, msg))
        }
    }
}

/// Which numeric domain a width belongs to — so a width conflict names the right thing (an integer's
/// WIDTH vs a float's PRECISION). Threaded into [`unify_width`] by its two callers (the `Ty::Int` and
/// `Ty::Float` unify arms); nothing else about the width algebra depends on it.
#[derive(Clone, Copy)]
enum NumericKind {
    Int,
    Float,
}

fn resolve_width(subst: &Subst, w: Width) -> Width {
    match w {
        Width::Var(v) => match subst.widths.get(&v) {
            Some(&sol) => resolve_width(subst, sol),
            None => Width::Var(v),
        },
        other => other,
    }
}

/// Whether variable `v` occurs in `t` (after substitution) — the occurs-check that keeps the variable
/// graph acyclic, so binding `v := t` cannot create an infinite type.
fn occurs(subst: &Subst, v: u32, t: &Ty) -> bool {
    // WALK the type, resolving each `Ty::Var` through the substitution chain in place — do NOT
    // `subst.apply(t)` (which deep-REBUILDS the whole type into a fresh tree) at every node. The old code
    // applied the full substitution at the root AND again at each recursive descent, so one occurs-check
    // over a size-`N` type was O(N²); driven by a deeply-nested generic-sum value (each enclosing `(Some
    // x)` unifies its `?0` against the O(depth) type below it), that made type-checking O(N³). Following
    // the var chain here — the standard union-find `walk` — keeps a solved variable resolvable while
    // making the check O(N): every node is visited once, and a `Ty::Var` costs only its chain length
    // (bounded by the acyclic-substitution invariant the occurs-check itself maintains). The RESULT is
    // identical: `v` occurs in `subst.apply(t)` iff, resolving each variable of `t` through `subst`, some
    // position reaches the (unsolved) variable `v`.
    match t {
        // Resolve the variable: a solved one continues into its solution (chasing the chain, as
        // `apply` did via `self.apply(t)`); an unsolved one is compared to `v`.
        Ty::Var(w) => match subst.tys.get(w) {
            Some(sol) => occurs(subst, v, sol),
            None => v == *w,
        },
        Ty::Fn(p, r) => occurs(subst, v, p) || occurs(subst, v, r),
        Ty::Cont { resume, answer } => occurs(subst, v, resume) || occurs(subst, v, answer),
        Ty::Record(fields) => fields.values().any(|ft| occurs(subst, v, ft)),
        Ty::Tuple(elems) => elems.iter().any(|ft| occurs(subst, v, ft)),
        // A GENERIC sum's type ARGS may hold a variable (a deferred payload) — check each. A monomorphic
        // sum has empty args, so no variable occurs in it (like the ground types).
        Ty::Sum { args, .. } => args.iter().any(|ft| occurs(subst, v, ft)),
        // A nominal's underlying type may hold the variable (a generic `Box ?0`).
        // A nominal's var occurs in its type ARGS (like `Ty::Sum`). NOT `inner` — a recursive nominal's
        // inner holds a `Ty::Sum{decl}` back-edge; walking it is redundant and its own-decl cycle is not
        // a variable occurrence. `args` is the axis a fresh instantiation var lives in.
        Ty::Nominal { args, .. } => args.iter().any(|t| occurs(subst, v, t)),
        // A list's element type may hold the variable (`List ?0`).
        Ty::List(elem) => occurs(subst, v, elem),
        // A map's key or value type may hold the variable (`Map ?0 ?1` — the empty map).
        Ty::Map(k, val) => occurs(subst, v, k) || occurs(subst, v, val),
        // A set's element type may hold the variable (`Set ?0` — an empty set).
        Ty::Set(elem) => occurs(subst, v, elem),
        // A quantity's INNER numeric type may hold the variable (`Qty ?0 u`); the unit never does.
        Ty::Qty { inner, .. } => occurs(subst, v, inner),
        Ty::Int(_)
        | Ty::Float(_)
        | Ty::Bool
        | Ty::Unit
        | Ty::Bytes
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Type
        | Ty::Any => false,
    }
}

/// The conflicting-use type error for two irreconcilable types. The CODE distinguishes the KIND of
/// conflict: two DIFFERENT NUMERIC types (a width or signedness mismatch — `Int32` vs `Int64`, signed
/// vs unsigned) is the numeric-model's no-silent-promotion rule (CDZ0301); ANY OTHER conflict (a
/// non-numeric type where another is required — `Bool` vs `Int64`) is the general type mismatch
/// (CDZ0203). This matches the corpus: CDZ0301 is reserved for two-different-numeric, everything else
/// is CDZ0203.
fn mismatch(a: &Ty, b: &Ty, ncx: &NameCtx) -> Reject {
    trace!(target: "rcdzc::unify", lhs = %a.render_name(ncx), rhs = %b.render_name(ncx), "unify FAILED (conflicting use)");
    // Both sides NUMERIC (an integer of any width/sign, a float, or a BigInt) but DIFFERENT — the
    // no-silent-promotion rule (CDZ0301, `numeric-model.md` §Numeric Types Do Not Silently Promote).
    // Covers a width/sign mismatch (`Int32` vs `Int64`), an integer↔float mix (`Int64` vs `Float64`,
    // `(+ 2 2.0)`), AND a `BigInt`↔fixed-int mix (`(+ (BigInt.of n) 1)`) — the `unify` arm for `BigInt`
    // states it "falls to `mismatch` … CDZ0301", so `BigInt` must count as numeric here (else it fell to
    // the generic CDZ0203, missing the "convert explicitly with `BigInt.of`" story). A non-numeric
    // conflict (`Bool` vs `Int64`) stays the general CDZ0203.
    let is_numeric = |t: &Ty| matches!(t, Ty::Int(_) | Ty::Float(_) | Ty::BigInt | Ty::Rational);
    let both_numeric = is_numeric(a) && is_numeric(b);
    if both_numeric {
        // Two different numeric types — the no-silent-promotion rule (CDZ0301). Name the RULE and point
        // at the explicit conversion, since the repair is never an implicit coercion: an int↔float mix
        // converts with `<Float>.of-int`, a width/sign mix with `<IntN>.of` (`numeric-model.md` §Numeric
        // Types Do Not Silently Promote). This is the message; the operand-anchored `wrap` fix (D7/D8)
        // rides alongside where inference has the AST node.
        return Reject::coded(
            Code::NumericMismatch,
            format!(
                "no implicit conversion between numeric types {} and {} — convert explicitly (Cadenza \
                 never silently promotes a numeric type)",
                a.render_name(ncx),
                b.render_name(ncx),
            ),
        );
    }
    // Name the two conflicting types in the house term ("type mismatch"), NOT the HM-algorithm verb
    // "unify" — a Cadenza author never meets the word "unify", and reporting a raw "unification failure"
    // is precisely the naive-HM leak the reporting discipline forbids (`type-errors-report-the-minimal-
    // conflict.md`: report the conflicting requirements, not the algorithm step that noticed them). The
    // two types are stated SYMMETRICALLY ("A and B must be the same type here, but differ") because the
    // caller's argument order is not a reliable expected-vs-found orientation — some call sites pass
    // (annotation, expr), others (expr, expected) — so an "expected X, found Y" phrasing would lie about
    // direction half the time. The precise blame (which site imposes which) rides in `related` spans.
    Reject::coded(
        Code::TypeMismatch,
        format!(
            "type mismatch: {} and {} must be the same type here, but differ",
            a.render_name(ncx),
            b.render_name(ncx),
        ),
    )
}

/// A source of fresh variable numbers — one counter for a whole inference solve, so no two fresh
/// variables collide. Type and width variables share the counter's space (they are distinguished by
/// which map they land in), which is harmless and keeps one source of truth.
#[derive(Debug, Default)]
pub struct Fresh {
    next: u32,
}

impl Fresh {
    pub fn new() -> Fresh {
        Fresh { next: 0 }
    }

    /// A fresh variable number.
    pub fn var(&mut self) -> u32 {
        let n = self.next;
        self.next += 1;
        n
    }

    /// Reserve a CONTIGUOUS block of `n` fresh numbers, returning the first. Equivalent to calling
    /// [`var`] `n` times (the block is `[base, base+n)`), but in one bump — what `instantiate` uses to
    /// map a scheme's bound-var list to a fresh block without a per-var map. `base` is meaningful only
    /// when `n > 0` (a zero-length reservation returns the current counter and bumps nothing).
    ///
    /// [`var`]: Fresh::var
    pub fn reserve(&mut self, n: u32) -> u32 {
        let base = self.next;
        self.next += n;
        base
    }
}

/// Instantiate a scheme: replace each bound type/width/sign variable with a FRESH one, so this use is
/// independent of every other. Returns the freshened type. This is what makes `+`'s `∀a. (Int a) →
/// (Int a) → (Int a)` apply at a fresh `a` each time — generic over the integer type.
pub fn instantiate(scheme: &Scheme, fresh: &mut Fresh) -> Ty {
    // The bound variables map to a CONTIGUOUS block of fresh numbers, so no per-var hashmap is needed:
    // `fresh.var()` returns sequential ids, and the old code called it for each ty var (in order), then
    // each width var, then each sign var — assigning `ty_vars[i] → base+i`,
    // `width_vars[j] → base+|ty|+j`, `sign_vars[l] → base+|ty|+|width|+l`. So a bound var's fresh number
    // is `axis_base + its INDEX in the scheme's var list` (a linear scan over the already-`&[u32]` list
    // — no allocation). `instantiate` was called ~226k times on a deep call chain, each building THREE
    // 1-entry `FxHashMap`s (~677k allocs, the bulk of a manydefs profile's malloc/free); the block
    // scheme is alloc-free. Numbering is byte-IDENTICAL to the map version (same call order), including
    // the case where a scheme's `ty_vars` and `width_vars` share a number `n` — it renames to `base+i`
    // in type position and `base+|ty|+j` in width position exactly as two separate maps did.
    let ty_base = fresh.reserve(scheme.ty_vars.len() as u32);
    let width_base = fresh.reserve(scheme.width_vars.len() as u32);
    let sign_base = fresh.reserve(scheme.sign_vars.len() as u32);
    let m = Rename {
        ty_vars: &scheme.ty_vars,
        ty_base,
        width_vars: &scheme.width_vars,
        width_base,
        sign_vars: &scheme.sign_vars,
        sign_base,
    };
    rename(&scheme.ty, &m)
}

/// The (allocation-free) instantiation mapping: per axis, the scheme's bound-var list and the base
/// fresh number its block starts at. A bound var renames to `base + its index in the list`; a var not
/// in the list is FREE and kept. Linear scan — the lists are tiny (an operator scheme has one var per
/// axis), so this beats a hashmap and allocates nothing.
struct Rename<'a> {
    ty_vars: &'a [u32],
    ty_base: u32,
    width_vars: &'a [u32],
    width_base: u32,
    sign_vars: &'a [u32],
    sign_base: u32,
}

impl Rename<'_> {
    fn ty_var(&self, v: u32) -> u32 {
        match self.ty_vars.iter().position(|&x| x == v) {
            Some(i) => self.ty_base + i as u32,
            None => v,
        }
    }
    fn width_var(&self, v: u32) -> u32 {
        match self.width_vars.iter().position(|&x| x == v) {
            Some(i) => self.width_base + i as u32,
            None => v,
        }
    }
    fn sign_var(&self, v: u32) -> u32 {
        match self.sign_vars.iter().position(|&x| x == v) {
            Some(i) => self.sign_base + i as u32,
            None => v,
        }
    }
}

/// Rename the bound variables of a type per the fresh mapping (a variable not in an axis list is free
/// and kept).
fn rename(ty: &Ty, m: &Rename) -> Ty {
    match ty {
        Ty::Var(v) => Ty::Var(m.ty_var(*v)),
        Ty::Int(it) => Ty::Int(IntTy {
            sign: rename_sign(it.sign, m),
            width: rename_width(it.width, m),
        }),
        // A float's width variable is renamed the SAME way an integer's is (shared `Width` machinery).
        Ty::Float(ft) => Ty::Float(crate::ty::FloatTy {
            width: rename_width(ft.width, m),
        }),
        Ty::Fn(p, r) => Ty::Fn(Box::new(rename(p, m)), Box::new(rename(r, m))),
        Ty::Cont { resume, answer } => Ty::Cont {
            resume: Box::new(rename(resume, m)),
            answer: Box::new(rename(answer, m)),
        },
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), rename(t, m)))
                .collect(),
        )),
        Ty::Tuple(elems) => Ty::Tuple(elems.iter().map(|t| rename(t, m)).collect()),
        // A list scheme's element type may hold a bound variable (a `(fn (a) … (List a))` scheme) — rename.
        Ty::List(elem) => Ty::List(Box::new(rename(elem, m))),
        // A set scheme's element type may hold a bound variable (a `(fn (a) … (Set a))` op scheme) — rename.
        Ty::Set(elem) => Ty::Set(Box::new(rename(elem, m))),
        // A map scheme's key/value types may hold bound variables (a `(fn (k v) … (Map k v))` op scheme)
        // — rename each.
        Ty::Map(k, v) => Ty::Map(Box::new(rename(k, m)), Box::new(rename(v, m))),
        // A GENERIC sum scheme's type ARGS may hold bound variables (a `(fn (a) … (Option a))` variant
        // ctor scheme) — rename each. A monomorphic sum has empty args; nothing to rename.
        Ty::Sum { decl, args } => Ty::Sum {
            decl: *decl,
            args: args.iter().map(|t| rename(t, m)).collect(),
        },
        // A quantity scheme's INNER type may hold a bound variable (a `(fn (T) … (Qty T u))` op scheme) —
        // rename it; the unit is a concrete value, carried unchanged.
        Ty::Qty { inner, unit } => Ty::Qty {
            inner: Box::new(rename(inner, m)),
            unit: unit.clone(),
        },
        // A generic nominal scheme's ARGS (and its `inner` hint) may hold a bound variable (a `(fn (a) …
        // (Box a))` ctor scheme) — rename both; `decl`/`name` identity is unchanged.
        Ty::Nominal { decl, args, inner } => Ty::Nominal {
            decl: *decl,
            args: args.iter().map(|t| rename(t, m)).collect(),
            inner: std::rc::Rc::new(rename(inner, m)),
        },
        Ty::Bool
        | Ty::Unit
        | Ty::Bytes
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Type
        | Ty::Any => ty.clone(),
    }
}

fn rename_width(w: Width, m: &Rename) -> Width {
    match w {
        Width::Var(v) => Width::Var(m.width_var(v)),
        other => other,
    }
}

fn rename_sign(s: Sign, m: &Rename) -> Sign {
    match s {
        Sign::Var(v) => Sign::Var(m.sign_var(v)),
        other => other,
    }
}

/// Replace EVERY free variable of `ty` (type, width, and sign axes) with a fresh one from `fresh`,
/// consistently (the same old variable maps to the same new one). Unlike [`instantiate`], which
/// freshens only a scheme's explicitly-BOUND variables, this freshens ALL variables — used to make a
/// value's inferred type variable-DISJOINT from another solve before the two are unified.
///
/// WHY THIS IS NEEDED: each `apply_type`/scheme instantiation uses a PRIVATE `Fresh` counting from 0
/// (a solve's variables are local to it), so two independent solves both produce `?0`, `?1`, …. When
/// one solve's RESULT flows into another and they unify — e.g. typing `(Some (None))`, where `Some`'s
/// scheme instantiates to `(-> ?0 (Option ?0))` and the argument `(None)` independently types as
/// `(Option ?0)` — the numerically-equal `?0`s ALIAS, so unifying the parameter `?0` against the
/// argument `(Option ?0)` produces `?0 = Option ?0` and the occurs-check spuriously rejects a
/// well-typed value (CDZ0203 "infinite type"). Freshening the argument's free variables past the
/// head's counter first (`?0` → `?1`) makes them disjoint: `?0 = Option ?1`, no cycle.
pub fn freshen_free(ty: &Ty, fresh: &mut Fresh) -> Ty {
    let mut map: crate::fxhash::FxHashMap<u32, u32> = crate::fxhash::FxHashMap::default();
    let mut wmap: crate::fxhash::FxHashMap<u32, u32> = crate::fxhash::FxHashMap::default();
    let mut smap: crate::fxhash::FxHashMap<u32, u32> = crate::fxhash::FxHashMap::default();
    freshen_free_go(ty, fresh, &mut map, &mut wmap, &mut smap)
}

/// [`freshen_free`], but a whole-type variable in `rigid` is PRESERVED (left unchanged) instead of
/// renamed. Implemented by pre-seeding the rename map with the identity (`v → v`) for each rigid var, so
/// `map.entry(v).or_insert_with(…)` returns `v` — no recursion-signature change, and a NON-rigid var still
/// freshens exactly as before. Used ONLY by `compute_def_scheme`'s body solve to keep the def's own
/// parameter type vars tied across the recursive-call arg-freshen in `apply_type` (the recursive-generic
/// producer element tie). Width/sign vars are NOT made rigid — a generic parameter is generic over its
/// WHOLE type, and the tie is a whole-type-var relationship; the numeric-axis maps stay independent.
pub fn freshen_free_except(
    ty: &Ty,
    fresh: &mut Fresh,
    rigid: &crate::fxhash::FxHashSet<u32>,
) -> Ty {
    let mut map: crate::fxhash::FxHashMap<u32, u32> = crate::fxhash::FxHashMap::default();
    for &v in rigid {
        map.insert(v, v);
    }
    let mut wmap: crate::fxhash::FxHashMap<u32, u32> = crate::fxhash::FxHashMap::default();
    let mut smap: crate::fxhash::FxHashMap<u32, u32> = crate::fxhash::FxHashMap::default();
    freshen_free_go(ty, fresh, &mut map, &mut wmap, &mut smap)
}

fn freshen_free_go(
    ty: &Ty,
    fresh: &mut Fresh,
    map: &mut crate::fxhash::FxHashMap<u32, u32>,
    wmap: &mut crate::fxhash::FxHashMap<u32, u32>,
    smap: &mut crate::fxhash::FxHashMap<u32, u32>,
) -> Ty {
    match ty {
        Ty::Var(v) => {
            let n = *map.entry(*v).or_insert_with(|| fresh.var());
            Ty::Var(n)
        }
        Ty::Int(it) => Ty::Int(IntTy {
            sign: match it.sign {
                Sign::Var(v) => Sign::Var(*smap.entry(v).or_insert_with(|| fresh.var())),
                other => other,
            },
            width: match it.width {
                Width::Var(v) => Width::Var(*wmap.entry(v).or_insert_with(|| fresh.var())),
                other => other,
            },
        }),
        // A float's free width variable is freshened the SAME way an integer's is (shared `Width`
        // machinery / `wmap`), so a generic float operator's scheme instantiates a fresh width per use.
        Ty::Float(ft) => Ty::Float(crate::ty::FloatTy {
            width: match ft.width {
                Width::Var(v) => Width::Var(*wmap.entry(v).or_insert_with(|| fresh.var())),
                other => other,
            },
        }),
        Ty::Fn(p, r) => Ty::Fn(
            Box::new(freshen_free_go(p, fresh, map, wmap, smap)),
            Box::new(freshen_free_go(r, fresh, map, wmap, smap)),
        ),
        Ty::Cont { resume, answer } => Ty::Cont {
            resume: Box::new(freshen_free_go(resume, fresh, map, wmap, smap)),
            answer: Box::new(freshen_free_go(answer, fresh, map, wmap, smap)),
        },
        Ty::Record(fields) => Ty::Record(std::rc::Rc::new(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), freshen_free_go(t, fresh, map, wmap, smap)))
                .collect(),
        )),
        Ty::Tuple(elems) => Ty::Tuple(
            elems
                .iter()
                .map(|t| freshen_free_go(t, fresh, map, wmap, smap))
                .collect(),
        ),
        Ty::List(elem) => Ty::List(Box::new(freshen_free_go(elem, fresh, map, wmap, smap))),
        Ty::Set(elem) => Ty::Set(Box::new(freshen_free_go(elem, fresh, map, wmap, smap))),
        Ty::Map(k, v) => Ty::Map(
            Box::new(freshen_free_go(k, fresh, map, wmap, smap)),
            Box::new(freshen_free_go(v, fresh, map, wmap, smap)),
        ),
        Ty::Sum { decl, args } => Ty::Sum {
            decl: *decl,
            args: args
                .iter()
                .map(|t| freshen_free_go(t, fresh, map, wmap, smap))
                .collect(),
        },
        // A quantity's free INNER variables are freshened (a generic `(Qty T u)` op scheme instantiates a
        // fresh inner per use); the unit is a concrete value, unchanged.
        Ty::Qty { inner, unit } => Ty::Qty {
            inner: Box::new(freshen_free_go(inner, fresh, map, wmap, smap)),
            unit: unit.clone(),
        },
        Ty::Nominal { decl, args, inner } => Ty::Nominal {
            decl: *decl,
            args: args
                .iter()
                .map(|t| freshen_free_go(t, fresh, map, wmap, smap))
                .collect(),
            inner: std::rc::Rc::new(freshen_free_go(inner, fresh, map, wmap, smap)),
        },
        Ty::Bool
        | Ty::Unit
        | Ty::Bytes
        | Ty::String
        | Ty::Char
        | Ty::Symbol
        | Ty::BigInt
        | Ty::Rational
        | Ty::Type
        | Ty::Any => ty.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::{IntTy, Sign, Ty, Width};

    /// Test-local `unify` over an empty [`NameCtx`] — these unit tests exercise the solve, not the
    /// name-rendering of a diagnostic, so no declared types are needed.
    fn unify(s: &mut Subst, a: &Ty, b: &Ty) -> Result<(), Reject> {
        super::unify(s, a, b, &NameCtx::new(&[]))
    }

    #[test]
    fn unifies_a_var_with_a_concrete_type() {
        let mut s = Subst::new();
        unify(&mut s, &Ty::Var(0), &Ty::Bool).unwrap();
        assert_eq!(s.apply(&Ty::Var(0)), Ty::Bool);
    }

    #[test]
    fn unify_is_order_independent() {
        let mut a = Subst::new();
        unify(&mut a, &Ty::Var(0), &Ty::int64()).unwrap();
        let mut b = Subst::new();
        unify(&mut b, &Ty::int64(), &Ty::Var(0)).unwrap();
        assert_eq!(a.apply(&Ty::Var(0)), b.apply(&Ty::Var(0)));
    }

    #[test]
    fn width_var_unifies_then_fixes() {
        // (Int w) unified with Int64 fixes w := 64.
        let wv = Ty::Int(IntTy {
            sign: Sign::Fixed(true),
            width: Width::Var(5),
        });
        let mut s = Subst::new();
        unify(&mut s, &wv, &Ty::int64()).unwrap();
        assert_eq!(s.apply(&wv), Ty::int64());
    }

    #[test]
    fn different_fixed_widths_conflict() {
        let i32t = Ty::Int(IntTy::fixed(true, 32));
        let i64t = Ty::int64();
        let mut s = Subst::new();
        assert!(unify(&mut s, &i32t, &i64t).is_err());
    }

    #[test]
    fn apply_ground_fast_path_is_identity_and_correct() {
        // `apply` has a ground fast-path (returns the input unchanged) — pin its CORRECTNESS. A ground type
        // (no ty/width/sign var) must come back equal under ANY substitution, and a NON-ground type must
        // still be fully applied (the fast-path must not swallow a substitutable var). The subtle case:
        // `is_ground` must be FALSE for a `Ty::Int` carrying a `Width::Var` (else the fast-path would skip
        // the width solve) — `width_var_unifies_then_fixes` above covers the apply, this pins the classify.
        // A ground compound is a fixpoint: applying a populated subst returns an EQUAL type.
        let ground = Ty::Tuple(vec![Ty::Bool, Ty::int64(), Ty::String].into());
        assert!(ground.is_ground(), "a tuple of concrete leaves is ground");
        let mut s = Subst::new();
        unify(&mut s, &Ty::Var(7), &Ty::Bool).unwrap(); // a non-empty subst
        assert_eq!(
            s.apply(&ground),
            ground,
            "apply is identity on a ground type"
        );
        // A `Width::Var` int is NOT ground (must still be applied) — the fast-path must not claim it.
        let width_var = Ty::Int(IntTy {
            sign: Sign::Fixed(true),
            width: Width::Var(9),
        });
        assert!(
            !width_var.is_ground(),
            "an int with a Width::Var is NOT ground (apply must still solve the width)"
        );
        // A tuple CONTAINING a var is not ground, and apply still substitutes into it.
        let mixed = Ty::Tuple(vec![Ty::Bool, Ty::Var(3)].into());
        assert!(
            !mixed.is_ground(),
            "a tuple with a var element is not ground"
        );
        let mut s2 = Subst::new();
        unify(&mut s2, &Ty::Var(3), &Ty::int64()).unwrap();
        assert_eq!(
            s2.apply(&mixed),
            Ty::Tuple(vec![Ty::Bool, Ty::int64()].into()),
            "apply substitutes into a non-ground compound"
        );
    }

    #[test]
    fn different_signs_conflict() {
        // Int8 and UInt8 differ only in sign — they must NOT unify (no silent promotion).
        let i8t = Ty::Int(IntTy::fixed(true, 8));
        let u8t = Ty::Int(IntTy::fixed(false, 8));
        let mut s = Subst::new();
        assert!(unify(&mut s, &i8t, &u8t).is_err());
    }

    #[test]
    fn a_deferred_sign_grounds_to_what_it_meets() {
        // A bare literal (deferred sign + width) unifies with UInt8 — the annotation grounds it. The
        // deferred axes don't conflict; the literal takes the unsigned-8 type.
        let lit = Ty::Int(IntTy::deferred());
        let u8t = Ty::Int(IntTy::fixed(false, 8));
        let mut s = Subst::new();
        assert!(unify(&mut s, &lit, &u8t).is_ok());
    }

    #[test]
    fn occurs_check_rejects_infinite_type() {
        // v = (-> v v) would be infinite.
        let mut s = Subst::new();
        let f = Ty::Fn(Box::new(Ty::Var(0)), Box::new(Ty::Var(0)));
        assert!(unify(&mut s, &Ty::Var(0), &f).is_err());
    }

    #[test]
    fn occurs_check_follows_the_substitution_chain_through_a_solved_var() {
        // The occurs-check must resolve variables THROUGH the substitution, not only look at the
        // syntactic type it is handed — the property the O(N) `walk` rewrite of `occurs` must preserve.
        // Solve `?1 = ?0`, then try `?0 = (-> ?1 Bool)`: syntactically `?0` does not appear in the body,
        // but `?1` RESOLVES to `?0`, so binding `?0` here would make `?0 = (-> ?0 Bool)` — an infinite
        // type. `occurs` must chase `?1 → ?0` and reject. (The old deep-`apply` did this by rebuilding the
        // whole type first; the walk does it by following the var chain in place — same verdict.)
        let mut s = Subst::new();
        assert!(unify(&mut s, &Ty::Var(1), &Ty::Var(0)).is_ok()); // ?1 = ?0
        let body = Ty::Fn(Box::new(Ty::Var(1)), Box::new(Ty::Bool));
        assert!(
            unify(&mut s, &Ty::Var(0), &body).is_err(),
            "occurs must follow ?1 → ?0 and reject the cycle"
        );
    }

    #[test]
    fn occurs_check_accepts_a_deep_nested_sum_without_cycle() {
        // A DEEPLY-nested generic-sum type — the shape whose occurs-check was O(size²) (making a deep
        // `(Some (Some … 5))` value O(N³) to type-check). Binding a fresh `?0` to a size-`N` nested sum
        // that does NOT contain `?0` must succeed (no cycle) — and, post-fix, in O(N). Build `Sum(Sum(…(?1)))`
        // 500 deep and unify `?0` against it: `?0` does not occur, so it binds cleanly.
        let mut inner = Ty::Var(1);
        for _ in 0..500 {
            inner = Ty::Sum {
                decl: crate::ast::StructId(0),
                args: std::rc::Rc::from([inner]),
            };
        }
        let mut s = Subst::new();
        assert!(
            unify(&mut s, &Ty::Var(0), &inner).is_ok(),
            "a deep nested sum not containing ?0 binds without a spurious cycle"
        );
        // And the SAME deep sum WITH `?0` at its core is correctly rejected as infinite.
        let mut cyclic = Ty::Var(0);
        for _ in 0..500 {
            cyclic = Ty::Sum {
                decl: crate::ast::StructId(0),
                args: std::rc::Rc::from([cyclic]),
            };
        }
        let mut s2 = Subst::new();
        assert!(
            unify(&mut s2, &Ty::Var(0), &cyclic).is_err(),
            "a deep nested sum containing ?0 at its core is an infinite type"
        );
    }

    #[test]
    fn instantiate_freshens_bound_vars() {
        // ∀a. a -> a  instantiates to  ?n -> ?n  for a fresh n (not var 0).
        let scheme = Scheme {
            ty_vars: vec![0],
            width_vars: vec![],
            sign_vars: vec![],
            ty: Ty::Fn(Box::new(Ty::Var(0)), Box::new(Ty::Var(0))),
        };
        let mut fresh = Fresh::new();
        let a = instantiate(&scheme, &mut fresh);
        let b = instantiate(&scheme, &mut fresh);
        // Two instantiations use DIFFERENT fresh variables (independent uses).
        assert_ne!(a, b);
        // Each is a self-consistent `?n -> ?n`.
        if let Ty::Fn(p, r) = &a {
            assert_eq!(p, r);
        } else {
            panic!("expected a function type");
        }
    }

    #[test]
    fn freshen_free_renames_free_vars_disjoint_and_consistent() {
        // `freshen_free` maps EVERY free var to a fresh one, consistently. This is what makes an
        // under-constrained value's type variable-disjoint from a head scheme before unifying — the fix
        // for the `(Some (None))` spurious occurs-check. `(Option ?0)` freshened past a `Fresh` already
        // at 1 becomes `(Option ?1)`, so unifying a head's `?0` against it gives `?0 = Option ?1`, not
        // the cyclic `?0 = Option ?0`.
        let opt0 = Ty::Sum {
            decl: crate::ast::StructId(0),
            args: std::rc::Rc::from([Ty::Var(0)]),
        };
        let mut fresh = Fresh::new();
        let _ = fresh.var(); // advance to 1 (as a head instantiation would have reserved ?0)
        let freshened = freshen_free(&opt0, &mut fresh);
        // The free `?0` renamed to a var != 0 (disjoint from the head's `?0`).
        match &freshened {
            Ty::Sum { args, .. } => match &args[..] {
                [Ty::Var(v)] => assert_ne!(*v, 0, "the free var must be renamed away from 0"),
                other => panic!("expected one Var arg, got {other:?}"),
            },
            other => panic!("expected a Sum, got {other:?}"),
        }
        // Unifying a head param `?0` against the freshened arg does NOT cycle (occurs-check passes).
        let mut subst = Subst::new();
        assert!(
            unify(&mut subst, &Ty::Var(0), &freshened).is_ok(),
            "unifying ?0 with a freshened (Option ?k), k != 0, must not trip the occurs-check"
        );
        // Consistency: the SAME free var maps to ONE fresh var throughout.
        let paired = Ty::Fn(Box::new(Ty::Var(7)), Box::new(Ty::Var(7)));
        let mut f2 = Fresh::new();
        if let Ty::Fn(p, r) = freshen_free(&paired, &mut f2) {
            assert_eq!(p, r, "one source var must map to one fresh var");
        } else {
            panic!("expected a function type");
        }
    }

    #[test]
    fn instantiate_numbers_axes_as_contiguous_blocks() {
        // The alloc-free block scheme must reproduce the old per-axis numbering: each ty var → its
        // index in `ty_vars`, each width var → `|ty_vars| + its index`, each sign var → `|ty_vars| +
        // |width_vars| + its index`. This mirrors an operator scheme like `+`'s
        // `∀(ty a=0, width a=0, sign s=1). (Int^s_a) → (Int^s_a) → (Int^s_a)` — note ty var 0 and width
        // var 0 SHARE the source number 0 but live on different axes, so they must rename to DIFFERENT
        // fresh numbers (0 in ty position, 1 in width position) exactly as two separate maps did.
        let scheme = Scheme {
            ty_vars: vec![0],
            width_vars: vec![0],
            sign_vars: vec![1],
            // A type mentioning all three axes: a bare `Var(0)` (ty), and an `Int` whose width is
            // `Var(0)` and sign is `Var(1)`.
            ty: Ty::Fn(
                Box::new(Ty::Var(0)),
                Box::new(Ty::Int(IntTy {
                    sign: Sign::Var(1),
                    width: Width::Var(0),
                })),
            ),
        };
        let mut fresh = Fresh::new();
        let inst = instantiate(&scheme, &mut fresh);
        // ty block = [0,1) → 0; width block = [1,2) → 1; sign block = [2,3) → 2.
        let Ty::Fn(p, r) = &inst else {
            panic!("expected a function type");
        };
        assert_eq!(
            **p,
            Ty::Var(0),
            "the ty var renames to the ty block base (0)"
        );
        let Ty::Int(it) = &**r else {
            panic!("expected an Int result");
        };
        assert_eq!(
            it.width,
            Width::Var(1),
            "the width var renames to the width block base (1)"
        );
        assert_eq!(
            it.sign,
            Sign::Var(2),
            "the sign var renames to the sign block base (2)"
        );
        // And the whole reservation advanced the counter past all three (next fresh is 3).
        assert_eq!(fresh.var(), 3);
    }

    #[test]
    fn a_partway_failing_compound_unify_leaves_a_partial_binding() {
        // TEETH for the PR#462 call-seed soundness fix (`compute_def_scheme`/`solve_recursive_params`
        // grounding): `unify` mutates `subst` IN PLACE as it descends a compound, binding earlier
        // components BEFORE it hits a later mismatch — so a FAILED unify does NOT roll back its partial
        // work. Here `(-> ?0 Bool)` vs `(-> String Int64)` binds `?0 = String` (the domains unify) and
        // THEN fails on `Bool` vs `Int64` (the codomains conflict). The call returns `Err`, yet `?0` is
        // now bound. A caller that does `let _ = unify(&mut subst, …)` and reads `subst` afterward would
        // see that stale `?0 = String` — the inconsistent-substitution hazard the reviewer flagged. This
        // test PINS the premise (so nobody "optimizes" the trial-clone away believing unify rolls back).
        let a = Ty::Fn(Box::new(Ty::Var(0)), Box::new(Ty::Bool));
        let b = Ty::Fn(Box::new(Ty::String), Box::new(Ty::int64()));
        let mut s = Subst::new();
        assert!(unify(&mut s, &a, &b).is_err(), "the codomains conflict");
        assert_eq!(
            s.apply(&Ty::Var(0)),
            Ty::String,
            "the DOMAIN binding survives the failed unify — unify does NOT roll back (the hazard)"
        );

        // THE REMEDY the grounding path uses: unify a TRIAL CLONE and commit only on `Ok`. A failing
        // unify then leaves the original `subst` PRISTINE — no stale binding flows to later inference.
        let mut committed = Subst::new();
        let mut trial = committed.clone();
        if unify(&mut trial, &a, &b).is_ok() {
            committed = trial; // not taken — the unify fails
        }
        assert_eq!(
            committed.apply(&Ty::Var(0)),
            Ty::Var(0),
            "trial-clone-commit-on-Ok leaves the committed subst untouched after a failed unify"
        );
    }
}
