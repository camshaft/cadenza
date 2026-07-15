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
use crate::ast::StructId;
use crate::db::Db;
use crate::diag::{Code, Fix, Reject};
use crate::resolve::resolved_of;
use crate::resolved::Resolved;
use crate::ty::{Scheme, Ty};
use crate::unify::{Fresh, Subst};
use tracing::trace;

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
    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "solved type");
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
    if !matches!(t, Ty::Any) && !ty_has_free_var(db, &t) {
        db.types.fill(id, t.clone());
    }
    t
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
            Some(crate::resolved::SegKind::Int { .. } | crate::resolved::SegKind::Bits { .. }) => {
                Ty::int()
            }
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
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    let t = type_of(db, value);
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
            let mut cur =
                tuple_constructor_ty(db, scrutinee).unwrap_or_else(|| type_of(db, scrutinee));
            let mut heads = heads.iter();
            for step in steps.iter() {
                cur = match step {
                    crate::core::PathStep::Payload => {
                        let Some(&head) = heads.next() else {
                            return Ty::Any; // malformed path (fewer heads than Payload steps)
                        };
                        // Over a NOMINAL NEWTYPE the `Payload` step UNWRAPS the tag to its underlying
                        // type (a runtime no-op — the value is unchanged). `(Mk n)` over a `Ty::Nominal {
                        // inner: Int64 }` binds `n : Int64`.
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
                        // A list-pattern element binder — every element of a `List T` has type `T`
                        // (homogeneous), regardless of the index.
                        Ty::List(elem) => (**elem).clone(),
                        _ => return Ty::Any,
                    },
                    // A list-pattern REST binder — the tail sublist is still a `List T` (same type as the
                    // list scrutinee), independent of where the tail starts.
                    crate::core::PathStep::RestFrom(_) => match &cur {
                        Ty::List(_) => cur.clone(),
                        _ => return Ty::Any,
                    },
                };
            }
            cur
        }
        Resolved::If { cond, then_, else_ } => {
            // Reading the children's types is the backward demand: each is a lazy `type_of`.
            let _cond_ty = type_of(db, cond);
            let then_ty = type_of(db, then_);
            let else_ty = type_of(db, else_);
            // The if's type is the join of its branches — `Any` yields the other, and a branch that
            // fixed a deferred integer width contributes it. The cond-is-Bool and branches-agree
            // CHECKS are `type_errors`' job; this fills the value column.
            then_ty.join(&else_ty)
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
            ty
        }
        // `nan` — the canonical NaN Float VALUE (a bare prim naming a value). Types as `Ty::Float` (a
        // bare `nan` grounds to Float64), so `(= nan nan)` type-checks like any float equality.
        Resolved::Prim(crate::resolved::Prim::FloatNan) => Ty::float(),
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
        Resolved::Annot { expr, ty_expr } => match crate::eval::typeval_of(db, ty_expr) {
            Some(annot_ty) => {
                let expr_ty = type_of(db, expr);
                let mut subst = Subst::new();
                let _ = crate::unify::unify(&mut subst, &annot_ty, &expr_ty);
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
    let parent = db.parent_of(id)?;
    let Resolved::Apply { head, args } = resolved_of(db, parent) else {
        return None;
    };
    // The head must be an integer binary operator; the literal must be one of its (exactly two) operands.
    let prim = crate::eval::meta_apply_of(db, head)?;
    if !prim.is_binop() || args.len() != 2 {
        return None;
    }
    let sibling = if args[0] == id {
        args[1]
    } else if args[1] == id {
        args[0]
    } else {
        return None;
    };
    // The sibling's type fixes the shared width only when it is a CONCRETELY-fixed integer (a `UInt64`
    // variable, an annotated operand) — the same "prefer a concrete width over a deferred literal" rule
    // `operand_int_ty` applies at selection. A deferred/var sibling imposes no constraint.
    match type_of(db, sibling) {
        Ty::Int(it) if it.width_is_fixed() => Some(it),
        _ => None,
    }
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
        ));
    }
    None
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
fn int_out_of_range_reject(
    annot_ty: &Ty,
    signed: bool,
    w: u32,
    v: &crate::ast::IntValue,
    ty_expr: StructId,
) -> Reject {
    let reject = Reject::coded(
        Code::IntOutOfRange,
        int_out_of_range_message(annot_ty, signed, w),
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
fn int_out_of_range_message(annot_ty: &Ty, signed: bool, w: u32) -> String {
    match int_width_range(signed, w) {
        Some(range) => format!(
            "integer literal does not fit the annotated type {} (the valid range is {range})",
            annot_ty.render_name(),
        ),
        None => format!(
            "integer literal does not fit the annotated type {}",
            annot_ty.render_name()
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
fn int_width_range(signed: bool, w: u32) -> Option<String> {
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
    match db.ast.as_name(ty_expr).map(str::to_string) {
        // A bare type-CONSTRUCTOR name used with NO argument — `(: xs List)`, `(: m Map)`. `List` IS a
        // type (constructor), so "is a value, not a type" misleads; name the missing argument + the fix,
        // the bare-name twin of the `(List Int64 Int64)` wrong-arity message.
        Some(_) if bare_type_ctor_needs_argument(db, ty_expr).is_some() => {
            let (name, placeholder) = bare_type_ctor_needs_argument(db, ty_expr).unwrap();
            format!(
                "`{name}` is a type constructor — it needs a type argument here, e.g. `({name} {placeholder})`"
            )
        }
        Some(name) => format!(
            "`{name}` is a value, not a type — {lead} requires a type (e.g. annotate `(: value Int64)`)"
        ),
        None => format!("{lead} requires a type, but found a non-type"),
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
fn bare_type_ctor_needs_argument(db: &mut Db, ty_expr: StructId) -> Option<(String, String)> {
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

/// Validate a NON-type-denoting annotation type expression `ty_expr` (one `typeval_of` rejected), pushing
/// each fault. The SHARED core of the three annotation sites — a parameter annotation, a value annotation
/// `(: value T)`, and a let-binder annotation — so all three name a bad type the same way. A RECORD-bearing
/// type (`(Record (name Type)…)`, or a container carrying one) uses the record-aware position split
/// (`push_payload_type_positions` skips field LABELS + descends into each field's TYPE; `validate_type_position`
/// keeps only a genuinely-unknown type name), so a `(Record (x Nonesuch))` names only `Nonesuch`, not the
/// label `x` (the naive value-`collect` mis-resolves labels as unbound names — M125). Otherwise: collect the
/// operand's own faults (an unbound name → CDZ0101), and if none surfaced (a well-formed non-type — a
/// literal, a compound), add the "expected a type" CDZ0203 with `lead` naming the site.
fn validate_non_type_annotation(db: &mut Db, ty_expr: StructId, lead: &str, out: &mut Vec<Reject>) {
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
        out.push(lowercase_type_var_reject(&name, ty_expr, lead));
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
    enrich_nested_lowercase_type_vars(db, ty_expr, lead, out);
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
            out.push(
                Reject::coded(
                    Code::TypeMismatch,
                    non_type_annotation_message(db, ty_expr, lead),
                )
                .at(ty_expr),
            );
        }
    }
}

/// The CDZ0101 reject for a LOWERCASE name used as a would-be type variable in a type-annotation position
/// — the ML/Haskell reflex (`a` read as `∀a`), which Cadenza has no `∀`-binder to scope. Names the actual
/// route to the polymorphism the author wanted: leave the parameter UNANNOTATED. Shared by the top-level
/// bare-name case and the NESTED-position walk (`enrich_nested_lowercase_type_vars`), so `(: x a)` and
/// `(: x (List b))` read identically. `lead` names the site ("a parameter's annotation", etc.).
fn lowercase_type_var_reject(name: &str, at: StructId, lead: &str) -> Reject {
    Reject::coded(
        Code::Unbound,
        format!(
            "unbound name `{name}` — a lowercase name in a type position is not a type variable here \
             ({lead} names an existing type). Cadenza has no `∀`-binder in an annotation; write a \
             GENERIC parameter by leaving it UNANNOTATED — `(def (f x) …)` is already polymorphic in `x` \
             — or annotate a concrete type"
        ),
    )
    .at(at)
}

/// Walk a COMPOUND type-annotation expression's NESTED type positions and emit the rich
/// [`lowercase_type_var_reject`] for each lowercase name that resolves to nothing — so a type-var nested in
/// `(List b)` / `(Tuple a b)` / `(-> a b)` / `(Map k v)` gets the SAME "not a type variable here" guidance
/// the top-level `(: x a)` does, instead of the terse "unbound name `b`" the generic `collect` gives. Only
/// a bare lowercase name resolving to `Poison` is enriched (an uppercase unknown type, or a name that IS a
/// value, keeps its own fault). Recurses through the tail elements of a `(head …)` form (the type-argument
/// positions), NOT the head (`List`/`Map`/`->` are the known ctors); a record-bearing type never reaches
/// here (the caller's record branch returns first), so there are no field LABELS to skip.
fn enrich_nested_lowercase_type_vars(
    db: &mut Db,
    node: StructId,
    lead: &str,
    out: &mut Vec<Reject>,
) {
    // A bare NAME node is a leaf — enrich it if it is a lowercase would-be type var resolving to nothing,
    // then stop (a name has no tail to recurse).
    if let Some(name) = db.ast.as_name(node).map(str::to_string) {
        if name.starts_with(|c: char| c.is_ascii_lowercase())
            && matches!(resolved_of(db, node), Resolved::Poison(_))
        {
            out.push(lowercase_type_var_reject(&name, node, lead));
        }
        return;
    }
    // A compound `(head tail…)` — recurse into the TAIL (the type-argument positions); the head names the
    // constructor, not a type var.
    if let crate::ast::Struct::List(kids) = db.ast.get(node) {
        let kids = kids.clone();
        for &child in kids.iter().skip(1) {
            enrich_nested_lowercase_type_vars(db, child, lead, out);
        }
    }
}

/// An ARITY-specific message when the annotation type is a prelude type CONSTRUCTOR applied to the WRONG
/// number of arguments — `(List Int64 Int64)` (List takes 1), `(Map Int64)` (Map takes 2), `(Set Int64
/// Bool)` (Set takes 1). Such a form reduces to NO type-value (`reduce_ctor` rejects the arity), so the
/// generic `non_type_annotation_message` calls it "a non-type" — misleading, since `List`/`Map`/`Set` ARE
/// type constructors, just misapplied. This names the constructor + its expected vs supplied arity
/// (rustc's "this type takes N generic arguments but M were supplied"). `None` unless `ty_expr` is a
/// `(Head arg…)` application whose head is a known type constructor (`List`/`Set` = 1, `Map` = 2) AND the
/// argument count differs — every other non-type keeps the generic message. Reads the head's ctor
/// identity via `meta_apply_of` (GENERIC — the prim carries the arity, no hard-coded name match on the
/// source spelling).
fn type_ctor_arity_message(db: &mut Db, ty_expr: StructId) -> Option<String> {
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
    //= spec/capabilities/numeric-model.md#an-integer-type-is-indexed-by-a-compile-time-width
    //# The bit width of an integer type MUST be resolved from a compile-time value and MUST NOT be determined by runtime data, so that an integer's width is fixed before the program runs rather than dependent on a value computed at runtime.
    if crate::eval::is_runtime_width_type(db, ty_expr) {
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
    if let Some((signed, w)) = crate::eval::out_of_range_int_width(db, ty_expr) {
        trace!(target: "rcdzc::infer", param = param.0, signed, width = w, "fault: over-ceiling integer width in a parameter annotation (CDZ0302)");
        out.push(
            Reject::coded(
                Code::IntOutOfRange,
                format!(
                    "`{}{w}` is not a valid integer type: a width must be in 1..=64 (a fixed-size \
                     integer wider than 64 bits is reserved to the big-integer layer, and 0 is not a \
                     width)",
                    if signed { "Int" } else { "UInt" }
                ),
            )
            .at(ty_expr),
        );
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
        validate_non_type_annotation(db, ty_expr, "a parameter's annotation", out);
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
    // that runs out of parameters, or an `Any` at this slot, recovers nothing).
    let mut cur = expected_arrow_for_lambda(db, lambda)?;
    for _ in 0..idx {
        match cur {
            Ty::Fn(_, r) => cur = *r,
            _ => return None,
        }
    }
    match cur {
        Ty::Fn(p, _) if !matches!(*p, Ty::Any) => Some(*p),
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

    // Ground each parameter: apply the substitution, FILL any remaining hole (`Any`/free `Var`) from the
    // call-site argument type — a plain unify cannot repair an `Any` (`Any` absorbs), so `fill_holes`
    // merges the determined call-site type into the open positions while keeping the body-pinned parts —
    // then default a still-unsolved NUMERIC variable to the signed-64 integer (a bare literal's default)
    // and leave anything else `Any` (genuinely unconstrained — no call site fixed it).
    for (i, (binder, var)) in param_binders.into_iter().enumerate() {
        // A GENERIC position stays a fresh canonical `Ty::Var` (quantified in the scheme): do not seed it
        // from a call site (which would pin it to ONE type) and do not ground it to `Any`.
        if generic_positions.contains(&i) {
            let gv = Ty::Var(fresh.var());
            trace!(target: "rcdzc::infer", def, binder = binder.0, "A2: recursive param is GENERIC (monomorphized per call site)");
            db.param_types.insert(binder, gv);
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
        trace!(target: "rcdzc::infer", def, binder = binder.0, ty = %grounded.render_name(), "A2: solved recursive param");
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
        let _ = crate::unify::unify(subst, &hvar, &arrow);
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
                        let _ = crate::unify::unify(subst, &param, &at);
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
                            let _ = crate::unify::unify(subst, pvar, &at);
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
                        if !arg_is_param {
                            continue;
                        }
                        if let Some(pt) = callee_param_ty(db, callee, i)
                            && !matches!(pt, Ty::Any | Ty::Var(_))
                        {
                            let at = arg_ty_in_env(db, arg, env, subst, fresh);
                            let _ = crate::unify::unify(subst, &at, &pt);
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
                let _ = crate::unify::unify(subst, &hvar, &arrow);
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
            let _ = crate::unify::unify(subst, &ct, &Ty::Bool);
            collect_param_constraints(db, cond, env, def, subst, fresh);
            collect_param_constraints(db, then_, env, def, subst, fresh);
            collect_param_constraints(db, else_, env, def, subst, fresh);
        }
        // A boolean connective's operands must be Bool — constrain each (a bare-param operand `(and p …)`
        // pins `p` Bool), then descend.
        Resolved::And { lhs, rhs, .. } => {
            for &op in &[lhs, rhs] {
                let t = arg_ty_in_env(db, op, env, subst, fresh);
                let _ = crate::unify::unify(subst, &t, &Ty::Bool);
                collect_param_constraints(db, op, env, def, subst, fresh);
            }
        }
        Resolved::Not { operand } => {
            let t = arg_ty_in_env(db, operand, env, subst, fresh);
            let _ = crate::unify::unify(subst, &t, &Ty::Bool);
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
                    let _ = crate::unify::unify(subst, &st, &pt);
                } else if (scrut_is_param || scrut_is_fn_param_app || scrut_is_scheme_call)
                    && let Some(pt) = pattern_implied_ty(db, *pat, fresh)
                {
                    let _ = crate::unify::unify(subst, &st, &pt);
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
                        let _ = crate::unify::unify(subst, &pvar, &applied);
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
        Resolved::Annot { expr, .. } => collect_param_constraints(db, expr, env, def, subst, fresh),
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
        // Leaves and references contribute no sub-constraints (a bare param ref's constraint comes from
        // the operation that consumes it, handled at the enclosing Apply).
        _ => {}
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
        .as_form(pat, "tuple")
        .or_else(|| db.ast.as_ctor_form(pat, "tuple"))
        .map(<[StructId]>::to_vec)
    {
        let tys: Vec<Ty> = elems
            .iter()
            .map(|&e| pattern_implied_ty(db, e, fresh).unwrap_or_else(|| Ty::Var(fresh.var())))
            .collect();
        return Some(Ty::Tuple(tys.into()));
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
        let _ = crate::unify::unify(&mut subst, &payload_ty, &implied);
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
        // An APPLICATION OF A FN-TYPED PARAMETER being solved — `(f h)` where `f` is in `env`. Its type is
        // `f`'s var peeled by one arrow per argument, read from the LOCAL `subst` (where the arrow was
        // unified in `collect_param_constraints`), NOT from `type_of` — `db.param_types` is still empty
        // mid-solve, so `type_of((f h))` would be `Any` and the result would never link to `f`'s arrow.
        // This is what lets `(+ (f h) …)` flow Int64 back to `f`'s result var, solving the fn param.
        Resolved::Apply { head, args } => {
            // The `list` NAME alias (`(list i)` via `ListNew`) is a list built from its arguments — the
            // element type read through the LOCAL subst, exactly as the `Resolved::List` arm above.
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
                        let _ = crate::unify::unify(&mut local, &param, &at);
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
            if let Some(root) = root {
                return walk_payload_ty(db, subst.apply(&root), &steps, &heads, subst);
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
        let _ = crate::unify::unify(subst, &param, &at);
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
    // The result type is the body's solved type. `Any` here means the body could not be typed without
    // reducing a self-call (recursion) — defer to the fixpoint A2 adds.
    let result = type_of(db, body);
    if matches!(result, Ty::Any) {
        trace!(target: "rcdzc::infer", def, "def_scheme: undetermined result (recursive?) → defer (A2)");
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
        trace!(target: "rcdzc::infer", def, scheme = %ty.render_name(), "def_scheme: determined monomorphic signature");
        return Some(Scheme::mono(ty));
    }
    trace!(target: "rcdzc::infer", def, scheme = %ty.render_name(), quantified = ty_vars.len(), "def_scheme: generalized polymorphic signature (recursive-generic)");
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
        Prim::TupleNew => Ty::Tuple(args.iter().map(|&e| type_of(db, e)).collect()),
        Prim::RecordNew => match crate::resolve::read_record_fields(db, args) {
            Ok(fields) => {
                let mut field_tys = std::collections::BTreeMap::new();
                for (label, &value) in fields.iter() {
                    field_tys.insert(label.clone(), type_of(db, value));
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
        _ => Ty::Any,
    }
}

/// The `Ty::Sum` of the built-in `Ast` prelude sum (a monomorphic sum — no args), or `None` if the
/// declaration is somehow absent. Used to type `ast-splice-lift`'s `(List Ast)` result.
fn ast_sum_ty(db: &Db) -> Option<Ty> {
    let occ = db.type_decls.iter().find(|t| t.name == "Ast")?.occ;
    Some(db.normalize_sum(occ, "Ast".to_string(), Vec::new()))
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
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::RecordExtend | crate::resolved::Prim::RecordWith)
    ) && args.len() == 2
        && let Ty::Record(fields) = type_of(db, args[0])
        && let Some((label, value)) = crate::resolve::record_op_pair(db, args[1])
    {
        let mut out: std::collections::BTreeMap<_, _> =
            fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        out.insert(label, type_of(db, value));
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
    // `Tuple.cat a b` — the concatenation of two tuples' element types (`type-system.md` §Two Tuples Are
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
    // `Tuple.pop t` — element 0 off: `(tuple (. t 0) <rest>)`. Result `(Tuple <e0> (Tuple <rest…>))`. A
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
    // `Unit.in target q` — explicit conversion. The result is a quantity at the TARGET unit (read from
    // arg0 by `unit_of`), carrying q's inner numeric type: `(Unit.in meter (Qty.of 3.0 km))` :
    // `(Qty Float64 meter)`. A dimensional mismatch (target dimension ≠ q's) is reported by
    // `check_application` (CDZ0501); here we fill the value-column type. If the target unit doesn't
    // reduce, or q isn't a quantity, fall through (→ Any, faulted elsewhere).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
        && args.len() == 2
        && let (Some(target), Ty::Qty { inner, .. }) =
            (crate::eval::unit_of(db, args[0]), type_of(db, args[1]))
    {
        return Ty::Qty {
            inner,
            unit: target,
        };
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
            ) && (matches!(a, Ty::BigInt) || matches!(b, Ty::BigInt))
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
            ) && (matches!(a, Ty::Rational) || matches!(b, Ty::Rational))
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
            ) && (matches!(a, Ty::Float(_)) || matches!(b, Ty::Float(_)))
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
                    crate::resolved::Prim::Add | crate::resolved::Prim::Sub => {
                        // The result unit: when both operands are at the SAME unit (scale), keep it (the
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
    trace!(target: "rcdzc::infer", head = head.0, scheme = %scheme.ty.render_name(), args = args.len(), "apply: instantiate head scheme");
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
                let at = crate::unify::freshen_free(&type_of(db, arg), &mut fresh);
                // A unify failure here is a real type fault; ignore it for the VALUE (reported by
                // `type_errors`) and continue with the declared result so the shape stays sane.
                let _ = crate::unify::unify(&mut subst, &param, &at);
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
    applied
}

/// Apply a def SCHEME to `args`, returning the result type — instantiate it with fresh variables, then
/// peel one curried parameter per argument (unifying the arg's type into the parameter). The result is
/// the instantiated return type after substitution. Used to type a RECURSIVE call by its callee's
/// signature (which β-reduction can't type). Mirrors the operator-scheme application in `apply_type`.
fn apply_scheme_to_args(db: &mut Db, scheme: &Scheme, args: &[StructId]) -> Ty {
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
    let generic = !scheme.ty_vars.is_empty();
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
    for at_raw in arg_tys {
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
                let _ = crate::unify::unify(&mut subst, &param, &at);
                cur = *result;
            }
            _ => return Ty::Any,
        }
    }
    subst.apply(&cur)
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
    let _ = crate::unify::unify(&mut subst, &result, scrut_ty);
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
) -> String {
    let subject = match (callee, param) {
        (Some(f), Some(p)) => format!("the argument for `{f}`'s parameter `{p}`"),
        (Some(f), None) => format!("the argument to `{f}`"),
        _ => "this argument".to_string(),
    };
    format!(
        "{subject} is {}, but a value of type {} is expected here{tail}",
        actual.render_with_article(),
        expected.render_name(),
    )
}

fn option_payload_mismatch_hint(expected: &Ty, actual: &Ty) -> Option<String> {
    if let Ty::Sum { name, args, .. } = actual
        && name == "Option"
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
                None => (k.name.clone(), wt, gt),
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
            // Each arg is a `(key value)` two-element entry list — read its two children.
            args.iter()
                .map(|&pair| match db.ast.get(pair) {
                    crate::ast::Struct::List(kids) => match kids.as_slice() {
                        [k, v] => Some((*k, *v)),
                        _ => None,
                    },
                    _ => None,
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
        .map(|k| k.name.as_str())
        .collect();
    let missing: Vec<&str> = want
        .keys()
        .filter(|k| !got.contains_key(*k))
        .map(|k| k.name.as_str())
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

/// The ENTRY node (`(k v)` pair) of the field named `field` in a WRITTEN record literal `expr` — the whole
/// pair, so a delete fix can remove `(field value)` as a unit. `None` if `expr` is not an inline record
/// literal (in the RAW AST) or has no such field. Same raw-AST discipline as [`record_field_key_occ`].
fn record_field_entry_occ(db: &Db, expr: StructId, field: &str) -> Option<StructId> {
    let entries = db
        .ast
        .as_ctor_form(expr, "record")
        .or_else(|| db.ast.as_form(expr, "record"))?;
    for &entry in entries {
        if let crate::ast::Struct::List(kv) = db.ast.get(entry)
            && kv.len() == 2
            && let Some(sym) = crate::resolve::read_key(db, kv[0])
            && sym.name == field
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
    // Both the reserved-symbol head `(record …)` and the bare `record` name-alias spell the same list; the
    // form node we append to is `expr` itself when either parse recognizes it.
    if db.ast.as_ctor_form(expr, "record").is_some() || db.ast.as_form(expr, "record").is_some() {
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
    let arms: Vec<String> = missing
        .iter()
        .map(|k| format!("({} (trap \"TODO\"))", k.name))
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
        .map(|k| k.name.as_str())
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
    if db.ast.as_form(expr, "tuple").is_some() || db.ast.head_ctor(expr) == Some("tuple") {
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
    let entries = db
        .ast
        .as_ctor_form(expr, "record")
        .or_else(|| db.ast.as_form(expr, "record"))?;
    for &entry in entries {
        if let crate::ast::Struct::List(kv) = db.ast.get(entry)
            && kv.len() == 2
            && let Some(sym) = crate::resolve::read_key(db, kv[0])
            && sym.name == field
        {
            return Some(kv[1]);
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
    let entries = db
        .ast
        .as_ctor_form(expr, "record")
        .or_else(|| db.ast.as_form(expr, "record"))?;
    for &entry in entries {
        if let crate::ast::Struct::List(kv) = db.ast.get(entry)
            && kv.len() == 2
            && let Some(sym) = crate::resolve::read_key(db, kv[0])
            && sym.name == field
        {
            return Some(kv[0]);
        }
    }
    None
}

/// The ordered element value-nodes of a directly-written TUPLE (`prim = TupleNew`) or LIST (`ListNew`)
/// literal `expr` — both the `Resolved::Tuple`/`List` primitive form and the `tuple`/`list` NAME-alias
/// application (`Resolved::Apply` of the matching `(meta apply)` prim, whose `args` ARE the elements).
/// `None` when `expr` is not the requested written literal kind.
fn positional_value_nodes(
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
fn record_field_diff_hint(expected: &Ty, actual: &Ty) -> Option<String> {
    let (Ty::Record(want), Ty::Record(got)) = (expected, actual) else {
        return None;
    };
    let missing: Vec<&str> = want
        .keys()
        .filter(|k| !got.contains_key(*k))
        .map(|k| k.name.as_str())
        .collect();
    let extra: Vec<&str> = got
        .keys()
        .filter(|k| !want.contains_key(*k))
        .map(|k| k.name.as_str())
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
                None => (k.name.clone(), wt, gt),
            };
            format!(
                " — field `{path}` should be {}, but this one is {}",
                lw.render_name(),
                lg.render_name()
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
fn tuple_arity_mismatch_hint(expected: &Ty, actual: &Ty) -> Option<String> {
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
                lw.render_name(),
                lg.render_name()
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
fn collection_element_mismatch_hint(expected: &Ty, actual: &Ty) -> Option<String> {
    let axis = |role: &str, want: &Ty, got: &Ty, plural: &str| {
        (!want.agrees_with(got)).then(|| {
            format!(
                " — its {role} should be {}, but {plural} {}",
                want.render_name(),
                got.render_name()
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
fn sum_payload_mismatch_hint(expected: &Ty, actual: &Ty) -> Option<String> {
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
                w.render_name(),
                g.render_name()
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
fn fn_signature_delta_hint(expected: &Ty, actual: &Ty) -> Option<String> {
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
            want_result.render_name(),
            got_result.render_name()
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
fn structural_delta_hint(first: &Ty, other: &Ty) -> Option<String> {
    record_field_diff_hint(first, other)
        .or_else(|| tuple_arity_mismatch_hint(first, other))
        .or_else(|| collection_element_mismatch_hint(first, other))
        .or_else(|| sum_payload_mismatch_hint(first, other))
        .or_else(|| fn_signature_delta_hint(first, other))
        .or_else(|| same_name_distinct_type_hint(first, other))
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
fn same_name_distinct_type_hint(first: &Ty, other: &Ty) -> Option<String> {
    if matches!(first, Ty::Var(_) | Ty::Any) || matches!(other, Ty::Var(_) | Ty::Any) {
        return None;
    }
    if first == other || first.render_name() != other.render_name() {
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
fn peer_type_delta_hint(first: &Ty, other: &Ty) -> Option<String> {
    structural_delta_hint(first, other)
        .or_else(|| option_payload_mismatch_hint(first, other))
        .or_else(|| option_payload_mismatch_hint(other, first))
        .or_else(|| fn_not_applied_hint(first, other))
        .or_else(|| fn_not_applied_hint(other, first))
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
fn fn_not_applied_hint(expected: &Ty, actual: &Ty) -> Option<String> {
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
        expected.render_with_article()
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
fn int_coercion_wrap(expected: &Ty, actual: &Ty) -> Option<(String, String, String)> {
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
        let n = expected.render_name();
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
        let float_name = expected.render_name();
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
    if let Some((prefix, suffix, verb)) = int_coercion_wrap(expected, actual) {
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
    let Some(value) = tail_resume_value(db, arm.body) else {
        return; // no tail resume (abortive / non-tail) — out of scope for this check
    };
    let value_ty = type_of(db, value);
    if !value_ty.agrees_with(&result) {
        trace!(target: "rcdzc::infer", value = value.0, "fault: resume value's type does not match the operation's result type (CDZ0201)");
        let mut reject = Reject::coded(
            Code::Malformed,
            format!(
                "a handler resumes with a value of type {} but the operation's result type is {}",
                value_ty.render_name(),
                result.render_name()
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

/// The VALUE of a tail `(resume value next-state)` in an arm body `node`, if the body IS such a resume at
/// the tail. `None` if the arm does not tail-resume (abortive, or a non-tail resume). Used by the
/// resume-value/result-type check; reads the ORIGINAL (un-substituted) arm body.
fn tail_resume_value(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Resume { value, .. } => Some(value),
        _ => None,
    }
}

/// The NEXT-STATE of a tail `(resume value next-state)` in an arm body `node`. `None` if the arm does not
/// tail-resume. The state-side twin of [`tail_resume_value`].
fn tail_resume_next_state(db: &mut Db, node: StructId) -> Option<StructId> {
    match resolved_of(db, node) {
        Resolved::Resume { next_state, .. } => Some(next_state),
        _ => None,
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
    let Some(next_state) = tail_resume_next_state(db, arm.body) else {
        return; // no tail resume — out of scope
    };
    let next_ty = type_of(db, next_state);
    if !next_ty.agrees_with(&seed_ty) {
        trace!(target: "rcdzc::infer", next_state = next_state.0, "fault: resume next-state type does not match the handler's seed/state type (CDZ0201)");
        let mut reject = Reject::coded(
            Code::Malformed,
            format!(
                "a handler resumes with a next-state of type {} but the handler's state type is {} \
                 (the seed fixes the state type; each resume threads a state of that type)",
                next_ty.render_name(),
                seed_ty.render_name()
            ),
        )
        .at(next_state);
        if let Some(fix) = numeric_text_coercion_fix(db, &seed_ty, &next_ty, next_state) {
            reject = reject.with_fix(fix);
        }
        out.push(reject);
    }
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

/// The surface NAME of a TUPLE row-operation prim (`cat`/`split-at`/`pop`) whose operand(s) must be a
/// `Ty::Tuple`; `None` otherwise. `cat` takes two tuple operands, `split-at`/`pop` one.
fn tuple_row_op_name(prim: Option<crate::resolved::Prim>) -> Option<&'static str> {
    match prim {
        Some(crate::resolved::Prim::TupleCat) => Some("cat"),
        Some(crate::resolved::Prim::TupleSplitAt) => Some("split-at"),
        Some(crate::resolved::Prim::TuplePop) => Some("pop"),
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

/// The diagnostic code for a LIST HOMOGENEITY violation between two element types that do not unify —
/// the same taxonomy line the numeric operators and `if`-branches draw (05-compound-types §A List Is An
/// Ordered Homogeneous Sequence). A HOMOGENEITY conflict is CDZ0201 (a MALFORMED list) in exactly two
/// shapes: two DISTINCT NUMERIC types (`Int64` vs `Float64` — the no-silent-promotion rule), and two
/// SAME-KIND compounds of DIFFERENT SHAPE (records of different field sets, or tuples of different arity
/// — the field set / arity IS the type, so they are genuinely different types the list cannot hold
/// together, which the equality path likewise rejects as incompatible shapes). Every OTHER disagreement
/// (a cross-KIND scalar clash `Int64` vs `Bool`, or a scalar-vs-compound) keeps the generic structural
/// mismatch CDZ0203 the unify produced. Mirrors the `if`-branch / connective taxonomy.
fn list_homogeneity_code(a: &Ty, b: &Ty) -> Code {
    let both_numeric =
        matches!(a, Ty::Int(_) | Ty::Float(_)) && matches!(b, Ty::Int(_) | Ty::Float(_));
    let both_records = matches!(a, Ty::Record(_)) && matches!(b, Ty::Record(_));
    let both_tuples = matches!(a, Ty::Tuple(_)) && matches!(b, Ty::Tuple(_));
    if both_numeric || both_records || both_tuples {
        Code::Malformed
    } else {
        Code::TypeMismatch
    }
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
    let node_count = db.ast.structure.len();
    for id in (0..node_count as u32).map(StructId) {
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
        // "did you mean?". Otherwise DON'T fall back to `did_you_mean`'s raw "closest matches" list: for
        // an ABBREVIATION like `mph`/`kmh`/`rpm` the nearest units by edit distance are semantically
        // unrelated noise (`mph` → `bps`, `mbps`, `bit`) — misleading, and it never says what to DO.
        // Give ACTIONABLE guidance instead — a compound unit is COMPOSED from known units, and a genuinely
        // new named unit is introduced with `Unit.define` (the example carries the actual unknown name, so
        // it is a copy-paste starting point).
        let hint = match crate::diag::suggest::nearest(&name, known.iter()) {
            Some(near) => format!(" — did you mean `{near}`?"),
            None => format!(
                " — compose a compound unit from known units \
                 (e.g. miles per hour is `(Unit./ (Unit.of #\"mile\") (Unit.of #\"hour\"))`), \
                 or declare it with `(Unit.define #\"{name}\" <base-unit> <num> <den>)`"
            ),
        };
        out.push(
            Reject::coded(
                Code::Malformed,
                format!(
                    "unknown unit `{name}` — it is neither a built-in unit nor declared by a \
                     `Unit.define`{hint}"
                ),
            )
            .at(id),
        );
    }
}

fn check_application(
    db: &mut Db,
    app: StructId,
    head: StructId,
    args: &[StructId],
    out: &mut Vec<Reject>,
) {
    // `ast-splice-lift` (the quasiquote-splice desugar's `(intrinsic "ast-splice-lift") e`) requires its
    // operand to be a LIST — `,@` splices the ELEMENTS of a list (`metaprogramming.md`), so an operand
    // that is PROVABLY not a list has no elements to splice → CDZ0201, the same reject the pre-desugar
    // `collect_quote_body_syntax` gave a raw `(unquote-splicing 5)`. CONSERVATIVE (`provably_not_list`):
    // only a concrete non-list rejects; an open/unknown type does not (never a false reject).
    if crate::eval::prim_of(db, head) == Some(crate::resolved::Prim::AstSpliceLift)
        && args.len() == 1
    {
        let ty = type_of(db, args[0]);
        if provably_not_list(&ty) {
            out.push(
                Reject::coded(
                    Code::Malformed,
                    format!(
                        "unquote-splicing (,@) splices the elements of a list, but its operand is {} \
                         — a value with no elements to splice",
                        ty.render_name()
                    ),
                )
                .at(args[0]),
            );
        }
        // The operand's own faults are collected by the caller's `collect(head/operand)`; return so the
        // generic scheme-unify below does not ALSO fault (the intrinsic has no HM scheme).
        return;
    }
    // `(Int64.of b)` / `(UInt N).of b` where `b : BigInt` — the CHECKED NARROWING from the unbounded
    // integer. `CheckedOf`'s HM scheme source is `(Int a)`, which does NOT unify with a `BigInt` — so
    // the generic scheme-unify below would wrongly fault CDZ0203. Skip it for a `BigInt` source: the
    // conversion is well-typed (its result is the target width, filled in `apply_type`), and the fold in
    // `lower` does the range check (an out-of-range constant → CDZ0302). Descend into the source for its
    // own faults, then return. A fixed-width source stays on the scheme path (its `(Int a)` unifies).
    if args.len() == 1
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::CheckedOf)
        && matches!(type_of(db, args[0]), Ty::BigInt)
    {
        collect(db, args[0], out);
        return;
    }
    // `Unit.in target q` — the TARGET unit must share q's DIMENSION (you can convert meters to
    // kilometers, not meters to seconds). A cross-dimension conversion is CDZ0501 (units-of-measure.md
    // §A Dimensional Mismatch Is An Error). Read the target unit + q's unit; descend into q for its own
    // faults, then return (skip the generic scheme-unify — `Unit.in` has no HM scheme).
    if args.len() == 2
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
    {
        if let (Some(target), Ty::Qty { unit: qu, .. }) =
            (crate::eval::unit_of(db, args[0]), type_of(db, args[1]))
            && !target.same_dimension(&qu)
        {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Unit.in target dimension differs from the quantity's (CDZ0501)");
            // NAME both units — the quantity's `qu` and the target `target` — instead of the anonymous
            // "a unit of a different dimension", matching the addition-mismatch message ("meter and
            // second"). The reader sees WHICH conversion is impossible (`meter → second`) rather than only
            // that some dimension differs.
            out.push(Reject::coded(
                Code::DimensionMismatch,
                format!(
                    "converting a {} quantity to {} crosses a dimension boundary — units are never \
                     silently converted across dimensions",
                    qu.render_human(),
                    target.render_human(),
                ),
            ));
        }
        collect(db, args[1], out);
        return;
    }
    // (A `+`/`-`/`*`/`/` over FLOAT operands — including the int/float MIX `(+ 2 2.0)` — is handled by the
    // Float skip+mix arm further below, alongside the BigInt/Rational skips: floating-point arithmetic
    // reuses the ONE arithmetic operator, so there is no operator-swap repair — the fix is the one-shot
    // int→float coercion on the integer operand, the same repair wherever an int/float mismatch surfaces.)
    // The RECORD + TUPLE ROW OPERATIONS have NO HM scheme (a label-list / literal-position operand is not
    // a typed value; a result shape is row-/arity-polymorphic), so SKIP the generic scheme-unify (it would
    // fault the operand). The per-op faults (CDZ0212 absent / CDZ0211 shared / CDZ0201 split-out-of-arity)
    // + operand descent are done in `collect_node`'s Apply arm; nothing to add here beyond stopping the
    // generic path. Matches any arity (`Tuple.pop`/1, the rest/2).
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(
            crate::resolved::Prim::RecordProject
                | crate::resolved::Prim::RecordWithout
                | crate::resolved::Prim::RecordMerge
                | crate::resolved::Prim::RecordExtend
                | crate::resolved::Prim::RecordWith
                | crate::resolved::Prim::RecordPop
                | crate::resolved::Prim::TupleCat
                | crate::resolved::Prim::TupleSplitAt
                | crate::resolved::Prim::TuplePop
        )
    ) {
        return;
    }
    // A COMPARISON between a MAP and a RECORD is a type error — they are DISTINCT KINDS (a record's field
    // set is fixed by its form; a map's key set is a runtime collection), so `(= (map …) (record …))` is
    // CDZ0201, NOT the CDZ0203 that a same-kind SHAPE mismatch (two records with different fields) gets
    // (type-system.md §Structural Values Are Comparable Only When Their Shapes Match — the cross-kind
    // case; collections-and-text.md §A Map Associates Keys With Values). The generic scheme-unify below
    // would report `unify::mismatch` = CDZ0203; catch the map/record kind clash FIRST for the right code.
    // (Two maps of different key SETS do NOT reach here as a mismatch — they are the SAME `Map<K,V>` type,
    // so this fires only on a genuine map-vs-record kind clash.)
    if args.len() == 2
        && matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Compare
            )
        )
    {
        let (a, b) = (type_of(db, args[0]), type_of(db, args[1]));
        if matches!(
            (&a, &b),
            (Ty::Map(_, _), Ty::Record(_)) | (Ty::Record(_), Ty::Map(_, _))
        ) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: comparing a map to a record — distinct kinds (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                "a map and a record are different types (their comparison is a type error)",
            ));
            return;
        }
        // A built-in `=`/`compare` on a value of an ABSTRACT type — one imported handle-only, its
        // constructors withheld — is rejected here (CDZ0202, the nominal-boundary code): built-in
        // structural comparison observes the equality of the module's PRIVATE representation, which the
        // handle-only export withheld. A module that wants its abstract type comparable exports a
        // comparison FUNCTION (`(def (eq (: x T) (: y T)) …)`), the ML discipline — the representation
        // stays hidden and only the operations the module publishes are available. Fires on either
        // operand (a value of the abstract type on one side is enough); within the declaring module (or
        // a concrete importer) the type is not abstract, so ordinary comparison is unaffected.
        //= spec/capabilities/type-system.md#an-abstract-type-s-representation-is-not-observable-across-its-boundary
        //# A built-in structural comparison whose operand is a value of an abstract type — a type whose handle a module made visible without making its constructors visible ([modules-and-namespaces.md](modules-and-namespaces.md) §A Type's Handle And Its Constructors Are Independently Visible) — MUST be rejected outside the declaring module, so that the abstract type's representation is not observed through equality and a module that wants its abstract type compared publishes a comparison operation rather than exposing its structure.
        let abstract_operand = |ty: &Ty, node: StructId| {
            nominal_or_sum_decl(ty).is_some_and(|decl| db.is_abstract_type_at(node, decl))
        };
        if abstract_operand(&a, args[0]) || abstract_operand(&b, args[1]) {
            let ty = if abstract_operand(&a, args[0]) {
                &a
            } else {
                &b
            };
            trace!(target: "rcdzc::infer", head = head.0, "fault: built-in comparison on an abstract type value (CDZ0202)");
            out.push(
                Reject::coded(
                    Code::NominalMismatch,
                    format!(
                        "`{}` is an abstract type here (its constructors are not exported to this \
                         file), so its representation cannot be observed through a built-in comparison \
                         — compare it through a function the module that declares it exports",
                        ty.render_name()
                    ),
                )
                .at(head),
            );
            return;
        }
        // Comparing a SYMBOL to the plain STRING it wraps is a comparison ACROSS THE NOMINAL BOUNDARY —
        // CDZ0202 (17-symbols "a string compared to a symbol is a type error"). A Symbol is a nominal over
        // String; a nominal value never silently compares equal to the untagged shape it was declared
        // distinct from, the same rule the nominal-record-vs-plain-record case pins. Reported HERE for the
        // right code (the generic scheme-unify below would give the plain CDZ0203); fires on either operand
        // order. (A comparison of two DIFFERENT nominal types is likewise rejected — as the generic CDZ0203
        // type mismatch — since two distinct `decl`s never unify.)
        //= spec/capabilities/type-system.md#nominal-types-are-not-comparable-across-their-boundary
        //# A comparison between a nominal value and the underlying structural value of the same shape MUST be rejected by a type-tracking generation, so that a nominal value never silently compares equal to the untagged shape it was declared distinct from.
        //= spec/capabilities/type-system.md#nominal-types-are-not-comparable-across-their-boundary
        //# A comparison whose operands are of two different nominal types MUST be rejected by a type-tracking generation, because the purpose of declaring a type nominal is to give its values an identity that is not interchangeable with a same-shape value of another type.
        if matches!(
            (&a, &b),
            (Ty::Symbol, Ty::String) | (Ty::String, Ty::Symbol)
        ) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: comparing a Symbol to a String across the nominal boundary (CDZ0202)");
            out.push(Reject::coded(
                Code::NominalMismatch,
                "a Symbol and a String are not comparable across the nominal boundary",
            ));
            return;
        }
        // Comparing a NEWTYPE (a nominal single-field sum, erased at runtime) to its UNDERLYING type —
        // `(= (Age 1) 1)` for `(type Age (Age Int64))` — is the SAME nominal-boundary violation as the
        // Symbol-vs-String case, generalized: a nominal value never silently compares equal to the
        // untagged representation it was declared distinct from (`type-system.md` §Nominal Types Are Not
        // Comparable Across Their Boundary). Without this it fell to the generic CDZ0203 "type mismatch",
        // which reads as "unrelated types" and hides that the fix is to WRAP/UNWRAP the nominal. Detect
        // it: one operand is a nominal/sum whose erased inner AGREES WITH the other operand's type. Fires
        // on either order; a nominal-vs-nominal or nominal-vs-unrelated clash is left to the generic path
        // (this is specifically the nominal-vs-its-own-inner boundary).
        // Fires ONLY when the OTHER operand is NOT itself a nominal/sum: this is the nominal-vs-its-own-
        // -UNTAGGED-representation boundary. When both sides are nominal (two instantiations of one
        // generic newtype — `Box Int64` vs `Box Bool` — or two different nominals), the erased inner may
        // be a `Ty::Var` (a generic template) that `agrees_with` ANYTHING, which would wrongly fire here;
        // those distinct-nominal comparisons stay the generic CDZ0203 (a genuinely different type), left
        // to the path below.
        let nominal_inner_vs = |db: &Db, nom: &Ty, other: &Ty| -> bool {
            nominal_or_sum_decl(other).is_none()
                && matches!(nominal_or_sum_decl(nom), Some(decl)
                    if db.newtype_inner.get(&decl).is_some_and(|inner| inner.agrees_with(other)))
        };
        if nominal_inner_vs(db, &a, &b) || nominal_inner_vs(db, &b, &a) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: comparing a newtype to its underlying type across the nominal boundary (CDZ0202)");
            let mut reject = Reject::coded(
                Code::NominalMismatch,
                format!(
                    "{} and {} are not comparable across the nominal boundary (unwrap the nominal to \
                     compare the underlying value)",
                    a.render_name(),
                    b.render_name()
                ),
            );
            // The MECHANICAL unwrap: the newtype operand is `(match <it> ((<Variant> n) n))`, which strips
            // the tag to the underlying value the other side is comparing against. Offer it when the
            // newtype is an ERASABLE SINGLE-VARIANT one (its unwrap is total + unambiguous — one variant,
            // one payload binder). Wrap whichever operand IS the newtype (`a` → `args[0]`, `b` → `args[1]`);
            // if both are (a nominal-vs-nominal never reaches here, guarded above), prefer the first.
            let unwrap_target = if nominal_inner_vs(db, &a, &b) {
                newtype_unwrap_variant(db, &a).map(|v| (args[0], v))
            } else {
                newtype_unwrap_variant(db, &b).map(|v| (args[1], v))
            };
            if let Some((operand, variant)) = unwrap_target {
                reject = reject.with_fix(Fix::wrap_heuristic(
                    operand,
                    "(match ",
                    format!(" (({variant} n) n))"),
                    format!("unwrap the nominal with `(match … (({variant} n) n))`"),
                ));
            }
            out.push(reject);
            return;
        }
    }
    // A built-in arithmetic/comparison/equality operator with a TEXT operand (String/Bytes) against a
    // SCALAR operand (a number, a Bool, a Char) is a CROSS-KIND clash — a malformed operation, CDZ0201,
    // NOT the CDZ0203 a same-kind SHAPE mismatch (two tuples of different arity, Int-vs-Bool) gets. This
    // mirrors the map-vs-record kind clash above: `(+ 1 "two")`, `(< 1 "x")`, `(> "x" 1)` compare a
    // heap-allocated text value against an unboxed scalar — there is no shared order or arithmetic across
    // that kind boundary (type-system.md §Structural Values Are Comparable Only When Their Shapes Match —
    // the cross-kind case; 07-type-system.sexp pins these at CDZ0201, the equality-companion code, while
    // Int-vs-Bool `(< 1 true)` — two scalars — stays the generic CDZ0203). The generic scheme-unify below
    // would report `unify::mismatch` = CDZ0203; catch the text/scalar clash FIRST for the right code. Only
    // fires when EXACTLY one side is text and the other is a definite scalar — String-vs-String (a valid
    // comparison), text-vs-compound, and two scalars all fall through to the generic path unchanged.
    if args.len() == 2
        && matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Compare
                    | crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            )
        )
    {
        let (a, b) = (type_of(db, args[0]), type_of(db, args[1]));
        let is_text = |t: &Ty| matches!(t, Ty::String | Ty::Bytes);
        let is_scalar = |t: &Ty| matches!(t, Ty::Int(_) | Ty::Float(_) | Ty::Bool | Ty::Char);
        // A COMPOUND value — a record, a tuple, a list, a map/set — held against a SCALAR or TEXT operand
        // is the same cross-KIND clash the text-vs-scalar case is: `(= r 5)` compares a heap record to an
        // unboxed integer, `(< t 5)` a tuple to an integer — no shared order/arithmetic/equality across
        // that boundary. The generic scheme-unify would report the opaque "type mismatch: (Record …) and
        // Int64 must be the same type here" (reads like an internal clash); name the kind boundary instead,
        // the compound analogue of the text-vs-scalar message. Only a DEFINITE compound (not an unsolved
        // var/`Any`) against a definite scalar/text — a compound-vs-compound mismatch (two different record
        // shapes) keeps the generic path (its own structural-delta hints, M91/M92, fire there).
        // A USER SUM / NOMINAL is "compound-like" for this cross-kind check: `(+ Color 1)` / `(= Color "x")`
        // holds a tagged heap value (or an erased newtype) against an unboxed scalar or text — no shared
        // arithmetic/order/equality across that boundary, the same clash as a record-vs-int. Included here
        // so a sum/nominal-vs-atom pair reports the clear "different types … kind boundary" (CDZ0201)
        // instead of the generic scheme-unify's phantom "type mismatch: Int64 and Color". (A NEWTYPE vs its
        // OWN INNER — `(= UserId Int64)` — never reaches here: the comparison-only `nominal_inner_vs` block
        // above catches it first with the unwrap fix; and arithmetic on two SAME sums/nominals is caught by
        // the text/compound arithmetic guard below. This adds the remaining sum/nominal-vs-ATOM case.)
        let is_compound = |t: &Ty| {
            matches!(
                t,
                Ty::Record(_)
                    | Ty::Tuple(_)
                    | Ty::List(_)
                    | Ty::Map(..)
                    | Ty::Set(_)
                    | Ty::Sum { .. }
                    | Ty::Nominal { .. }
                    // A first-class TYPE value held against a scalar/text — `(+ Color 1)`, `(< Int64 5)` —
                    // is a cross-kind clash (no shared arithmetic/order between a type and a number). It
                    // otherwise leaked the phantom "type mismatch: Int64 and Type" from the generic scheme.
                    | Ty::Type
            )
        };
        let is_atom = |t: &Ty| is_text(t) || is_scalar(t);
        // A compound's STRUCTURAL KIND tag (Record/Tuple/List/Map/Set) — two compounds of DIFFERENT kinds
        // (`(< (tuple …) (list …))`, `(= r m)` a record vs a map) are as cross-kind as a compound-vs-atom
        // pair: there is no shared order/equality between a tuple and a list. Same-kind compounds (two
        // records, two tuples) return the SAME tag, so they fall through to the generic path where their
        // structural-delta hints (M91/M92, "field `x` should be …", "element 1 …") fire — this only pulls
        // out the genuinely-incomparable DIFFERENT-kind case. A non-compound is `None` (not this test).
        let compound_kind = |t: &Ty| match t {
            Ty::Record(_) => Some(0u8),
            Ty::Tuple(_) => Some(1),
            Ty::List(_) => Some(2),
            Ty::Map(..) => Some(3),
            Ty::Set(_) => Some(4),
            // A user sum/nominal shares ONE tag: two DISTINCT sums (`Color` vs `Shape`) or a sum vs a
            // nominal are the SAME "kind" here, so `different_compound_kinds` does NOT fire for them — they
            // stay on the generic path ("type mismatch: Color and Shape"), a genuine same-kind mismatch. A
            // sum vs a RECORD/LIST/etc. differs in tag → the kind-boundary message, correct.
            Ty::Sum { .. } | Ty::Nominal { .. } => Some(5),
            // A TYPE value gets its own tag: two Types are the SAME kind (a bare `(= Int64 Int64)` is left
            // to its own path — type equality is `Type.eq`, not the generic `=`), but a Type vs a compound
            // differs in tag → the kind-boundary message.
            Ty::Type => Some(6),
            _ => None,
        };
        let different_compound_kinds = match (compound_kind(&a), compound_kind(&b)) {
            (Some(ka), Some(kb)) => ka != kb,
            _ => false,
        };
        // ARITHMETIC on TWO TEXT-or-COMPOUND operands — `(+ s t)` on two Strings/Bytes, `(+ xs ys)` on two
        // Lists, `(+ r q)` on two Records/Tuples/Maps/Sets — whether or not the two operand types match.
        // Comparison (`= < >`) on two texts / two same-kind compounds is VALID or gets its own structural
        // message, so this fires ONLY for the arithmetic prims (Add/Sub/Mul/Div). The earlier cross-kind
        // guard catches a text/compound held against a SCALAR, and two DIFFERENT-kind compounds; what leaks
        // past it is arithmetic on two SAME-framing operands — two texts (String vs Bytes), two same-kind
        // compounds (two records of different fields, two lists of different element types), or two of the
        // identical type. For all of those the generic scheme-unify defaults the operator's `∀a.(Int a)→…`
        // first parameter to `Int64` and reports the other operand as "type mismatch: Int64 and <T> must be
        // the same type here" — a PHANTOM `Int64` the author never wrote, reading as an internal clash.
        // Name the real situation: arithmetic is not defined on text/compound. For `+` on a MATCHED type
        // with a total CONCATENATION op (String/Bytes/List — the Python/JS reflex), rewrite the operator
        // head to that `.concat` (`(+ a b)` → `((. List concat) a b)`), the actionable repair; a mismatched
        // pair or a non-concatenable / non-`+` op gets the honest message with no (forced) fix.
        //
        // SCOPED to text + compound (not the scalar-ish LEAVES Bool/Unit/Symbol): the corpus pins a
        // Bool-in-integer-addition — a body-inferred `(+ true true)` — at CDZ0203 (an argument checked
        // against a body-inferred param type, `09-functions.sexp`), so a leaf scalar stays on that path.
        // EXCLUDES (each has its own correct path): numeric types (valid arithmetic); `Char` (`Char.to-int`
        // coercion wrap fix, below); `Qty` (the dimensional path below); `Bool` (corpus-pinned CDZ0203,
        // above); `Var`/`Any` (unsolved — never a false reject). A user `Sum`/`Nominal` is INCLUDED (see the
        // predicate below): Cadenza has no operator overloading, so arithmetic on one is always a type
        // error, not a possible user-defined `+`.
        let is_arith = matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Mul
                    | crate::resolved::Prim::Div
            )
        );
        // Symbol and Unit join text/compound: adding two Symbols / two Units is as non-numeric as adding
        // two Strings, and reporting the phantom "type mismatch: Int64 and Symbol" (the operator scheme's
        // grounded-to-Int64 first param) is the same leak the text/compound message fixes. Bool is STILL
        // excluded — the corpus pins a body-inferred `(+ true true)` at CDZ0203 (`09-functions.sexp`), so a
        // Bool leaf stays on that path; Symbol/Unit carry no such corpus constraint.
        //
        // A USER SUM / NOMINAL operand joins too: Cadenza has NO operator overloading — the arithmetic
        // prims carry the fixed `∀a. (Int a) → …` scheme, so a `(+ c d)` on two `Color`s (or two `UserId`
        // newtypes) is ALWAYS a type error, never a user-defined `+`. The generic scheme-unify grounds the
        // first param to `Int64` and reports "type mismatch: Int64 and Color" — the SAME phantom leak. Name
        // the real type. (A `Ty::Nominal` over a NUMBER — `UserId` over `Int64` — still gets the honest
        // "arithmetic is not defined on UserId"; unwrapping-then-adding is the author's call, no forced
        // fix.) Comparison (`= < >`) on two same sums/nominals is VALID (structural equality / a derived
        // order), so this stays arithmetic-only, and the earlier `nominal_inner_vs` guard already handles a
        // nominal-vs-its-INNER comparison — this is the two-SAME-framing arithmetic case that leaks past.
        // `Ty::Type` (a first-class TYPE value — `Int64`, a user sum name, `List` used as a value) joins
        // too: arithmetic on a type is always a type error (there is no `+` on types; type EQUALITY is the
        // dedicated `Type.eq`, not the bare `=`/`+`). The generic scheme grounds the first operand to Int64
        // and reports "type mismatch: Int64 and Type" — the same phantom leak (plus a cascading uncoded "a
        // type value has no runtime form" decline). Naming it "arithmetic is not defined on Type" is honest.
        let text_or_compound_ty = |t: &Ty| {
            matches!(
                t,
                Ty::String
                    | Ty::Bytes
                    | Ty::List(_)
                    | Ty::Record(_)
                    | Ty::Tuple(_)
                    | Ty::Map(..)
                    | Ty::Set(_)
                    | Ty::Symbol
                    | Ty::Unit
                    | Ty::Sum { .. }
                    | Ty::Nominal { .. }
                    | Ty::Type
            )
        };
        if is_arith && text_or_compound_ty(&a) && text_or_compound_ty(&b) {
            // A total-concatenation `.concat` exists on these modules → `+` on a MATCHED pair is
            // concatenation intent. A mismatched pair (String vs Bytes, two differing records) gets no fix.
            let concat_module = match &a {
                Ty::String => Some("String"),
                Ty::Bytes => Some("Bytes"),
                Ty::List(_) => Some("List"),
                _ => None,
            };
            trace!(target: "rcdzc::infer", head = head.0, "fault: arithmetic on two text/compound operands — not defined (CDZ0201)");
            // Name the real type(s): one when they match, both when they differ (so a String-vs-Bytes / a
            // records-of-different-fields pair reads honestly, not as a phantom `Int64` clash).
            let subject = if a == b {
                format!(
                    "{} — Cadenza never coerces this to a number",
                    a.render_name()
                )
            } else {
                format!(
                    "{} and {} — Cadenza never coerces these to numbers",
                    a.render_name(),
                    b.render_name()
                )
            };
            let mut reject = Reject::coded(
                Code::Malformed,
                format!("arithmetic is not defined on {subject}"),
            );
            // `+` on a MATCHED concatenable type → rewrite the operator to that module's `concat`.
            if a == b
                && let Some(module) = concat_module
                && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Add)
            {
                reject =
                    reject.with_fix(Fix::replace_heuristic(head, format!("(. {module} concat)")));
            }
            out.push(reject);
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // An `(Option T)` held against its OWN payload `T` — `(+ (List.at xs 0) 1)`, a fallible read used
        // directly — is NOT a kind-boundary clash to relabel: it has a specific, more actionable route
        // (match the Option to handle `None`), attached by the generic-path `option_payload_mismatch_hint`
        // below. Exclude it from the cross-kind sum-vs-atom branch (which the `Ty::Sum` addition to
        // `is_compound` would otherwise capture first), so the "the value is optional; match it" hint wins.
        let is_option_payload_pair = option_payload_mismatch_hint(&a, &b).is_some()
            || option_payload_mismatch_hint(&b, &a).is_some();
        let cross_kind = !is_option_payload_pair
            && ((is_text(&a) && is_scalar(&b))
                || (is_scalar(&a) && is_text(&b))
                || (is_compound(&a) && is_atom(&b))
                || (is_atom(&a) && is_compound(&b))
                || different_compound_kinds);
        if cross_kind {
            trace!(target: "rcdzc::infer", head = head.0, "fault: operands of distinct kinds — not comparable/operable across the boundary (CDZ0201)");
            out.push(Reject::coded(
                Code::Malformed,
                format!(
                    "{} and {} are different types (this operation is not defined across that kind boundary)",
                    a.render_with_article(),
                    b.render_with_article()
                ),
            ));
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // A `Bool` operand against a DIFFERENT scalar kind — a number or a `Char` — `(< true 5)`, `(+ 1
        // true)`, `(= true #\a)`. Both are scalars (so the text/compound cross-kind guard above does not
        // fire), but there is no shared order / arithmetic / equality between a boolean and a number or a
        // character. The generic scheme-unify gives the opaque "type mismatch: Bool and Int64 must be the
        // same type here" — an internal-clash read. Name the boundary instead. KEEP THE CODE `CDZ0203` (NOT
        // the CDZ0201 the text/compound cases take): the corpus pins a two-scalar clash `(< 1 true)` at the
        // general TypeMismatch. ONLY a `Bool`-vs-other-scalar pair — a `Bool` has NO conversion to a number
        // or a char, so it is a genuine dead-end (honest no-fix). A `Char`-vs-number pair is DELIBERATELY
        // excluded: `Char.to-int` is a total conversion, so `(+ #\a 1)` flows on to the numeric-coercion
        // path below, which offers the `(Char.to-int …)` wrap fix (the M96 twin) — naming it a kind boundary
        // here would rob it of that repair. Two different NUMERIC types (Int32 vs Int64, int vs float) are
        // the separate no-promotion path (CDZ0301) handled by `unify::mismatch` below.
        let scalar_kind = |t: &Ty| match t {
            Ty::Bool => Some("a boolean"),
            Ty::Char => Some("a character"),
            Ty::Int(_) | Ty::Float(_) => Some("a number"),
            _ => None,
        };
        if let (Some(ka), Some(kb)) = (scalar_kind(&a), scalar_kind(&b))
            && ka != kb
            && (a == Ty::Bool || b == Ty::Bool)
        {
            trace!(target: "rcdzc::infer", head = head.0, "fault: a Bool against a different scalar kind — not comparable/operable across the boundary (CDZ0203)");
            out.push(Reject::coded(
                Code::TypeMismatch,
                format!(
                    "{} and {} are different types — this operation is not defined between {ka} and {kb}",
                    a.render_with_article(),
                    b.render_with_article(),
                ),
            ));
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
    }
    // DIMENSIONAL check on a binary operator applied to QUANTITIES (units-of-measure.md §A Dimensional
    // Mismatch Is An Error). Fires BEFORE the generic scheme-unify (whose `∀a. (Int a) → …` scheme has no
    // notion of a unit) and `return`s to avoid a duplicate report:
    //  - `+`/`-`/comparison/`=`: the two operands' DIMENSIONS must be EQUAL (Layer 1 = same unit exactly;
    //    conversion between different units of one dimension is Layer 2). A length + a time, or a quantity
    //    + a bare number (no implicit dimensionless coercion), is CDZ0501. The inner numeric types must
    //    ALSO agree (the numeric core is unchanged under a unit — an Int64 meter + a Float64 meter is the
    //    numeric mismatch CDZ0301, reported here so it is not masked).
    //  - `*`/`/`: ALWAYS well-formed on quantities — the dimensions COMPOSE (they are not required equal),
    //    so no dimensional fault; the result unit is computed in `apply_type`. (A quantity × a bare number
    //    is Layer 2 scaling; in Layer 1 a mixed quantity/non-quantity `*`/`/` is left to the generic path.)
    if args.len() == 2 {
        let prim = crate::eval::meta_apply_of(db, head);
        let is_additive = matches!(
            prim,
            Some(
                crate::resolved::Prim::Add
                    | crate::resolved::Prim::Sub
                    | crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Compare
            )
        );
        let is_multiplicative = matches!(
            prim,
            Some(crate::resolved::Prim::Mul | crate::resolved::Prim::Div)
        );
        // `%` (remainder) is BigInt-valid arithmetic like `/`, but it is NOT `is_multiplicative` — the
        // quantity `*`/`/` dimensional skip above must exclude it (a `%` on a quantity has its own rules),
        // so it rides ONLY the BigInt-skip condition below.
        let is_rem = matches!(prim, Some(crate::resolved::Prim::Rem));
        let a0 = type_of(db, args[0]);
        let b0 = type_of(db, args[1]);
        let any_qty = matches!(a0, Ty::Qty { .. }) || matches!(b0, Ty::Qty { .. });
        // `*`/`/` on a quantity is ALWAYS well-formed dimensionally (the dimensions compose, not required
        // equal), so it must NOT reach the generic scheme-unify below (which would unify a `Ty::Qty`
        // against `(Int a)` → a spurious CDZ0203). Skip the fault check, descend for operand faults, and
        // return — the result unit is computed in `apply_type`.
        if is_multiplicative && any_qty {
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // A `+`/`-`/`*`/`/` over BIGINT operands is the unbounded arithmetic — well-typed, but the
        // operator's `∀a. (Int a) → …` scheme does NOT accept a `BigInt`, so the generic scheme-unify
        // below would spuriously reject it CDZ0301. Skip it (both operands are BigInt in the well-typed
        // case; `lower` routes to the runtime bigint op), descend for operand faults, and return. A
        // genuine `BigInt`/fixed MIX still faults: `agrees_with` is false, so `type_of` gave one operand
        // BigInt and the other a fixed Int — the mismatch is caught by the operand check here.
        if (is_additive || is_multiplicative || is_rem)
            && (matches!(a0, Ty::BigInt) || matches!(b0, Ty::BigInt))
        {
            // A mix (one BigInt, one non-BigInt-non-Any) is the no-promotion error CDZ0301.
            let a_big = matches!(a0, Ty::BigInt);
            let b_big = matches!(b0, Ty::BigInt);
            let a_ok = a_big || matches!(a0, Ty::Any);
            let b_ok = b_big || matches!(b0, Ty::Any);
            if !(a_ok && b_ok) {
                let mut reject = Reject::coded(
                    Code::NumericMismatch,
                    format!(
                        "no implicit conversion between numeric types {} and {} — convert explicitly \
                         (Cadenza never silently promotes a numeric type)",
                        a0.render_name(),
                        b0.render_name()
                    ),
                );
                // Offer the total `(BigInt.of …)` wrap on the FIXED-int operand — the one operand that is
                // a fixed integer where the other is a BigInt (`(+ (BigInt.of 5) 3)` → wrap `3`). The same
                // "same repair wherever the mismatch surfaces" the int-width/float coercions give, extended
                // to the BigInt boundary. Only when exactly one side is BigInt and the other a fixed int.
                let (fix_arg, other) = if a_big {
                    (args[1], &b0)
                } else {
                    (args[0], &a0)
                };
                if let Some(fix) = numeric_text_coercion_fix(db, &Ty::BigInt, other, fix_arg) {
                    reject = reject.with_fix(fix);
                }
                out.push(reject);
            }
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // A `+`/`-`/`*`/`/` over RATIONAL operands is exact rational arithmetic — well-typed, but the
        // operator's `∀a. (Int a) → …` scheme does NOT accept a `Rational`, so the generic scheme-unify
        // would spuriously reject it. Skip it (both operands are Rational in the well-typed case; `lower`
        // folds a constant pair), descend for operand faults, return. A genuine `Rational`/other MIX still
        // faults CDZ0301. NOT `%`: exact rational division is total (no remainder), so a `%` over rationals
        // falls through to the scheme, which rejects it (there is no rational `%`). (This SUPERSEDES the
        // earlier "Rational arithmetic is not yet supported" decline — B4-1 now folds it exactly.)
        if (is_additive || is_multiplicative)
            && (matches!(a0, Ty::Rational) || matches!(b0, Ty::Rational))
        {
            // For a COMPARISON (`is_additive` covers `<`/`>`/`=`/… too) the generic path already accepts
            // two Rationals (the `∀a. a→a→Bool` scheme), so only the ARITHMETIC forms need this skip; but
            // a comparison of a Rational against a NON-Rational must still fault. Report a mix (one
            // Rational, one non-Rational-non-Any) as CDZ0301 for BOTH arithmetic and comparison.
            let a_rat = matches!(a0, Ty::Rational);
            let b_rat = matches!(b0, Ty::Rational);
            let a_ok = a_rat || matches!(a0, Ty::Any);
            let b_ok = b_rat || matches!(b0, Ty::Any);
            if !(a_ok && b_ok) {
                let mut reject = Reject::coded(
                    Code::NumericMismatch,
                    format!(
                        "no implicit conversion between numeric types {} and {} — convert explicitly \
                         (Cadenza never silently promotes a numeric type)",
                        a0.render_name(),
                        b0.render_name()
                    ),
                );
                // The Rational twin of the BigInt wrap above: offer `(Rational.of-int …)` on the fixed-int
                // operand where the other side is a Rational (`(+ r 1)` → wrap `1`).
                let (fix_arg, other) = if a_rat {
                    (args[1], &b0)
                } else {
                    (args[0], &a0)
                };
                if let Some(fix) = numeric_text_coercion_fix(db, &Ty::Rational, other, fix_arg) {
                    reject = reject.with_fix(fix);
                }
                out.push(reject);
            }
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // A `+`/`-`/`*`/`/` over FLOAT operands is float arithmetic — the SAME operator as the integer
        // case, dispatched on a `Ty::Float` operand (there is no distinct `+.`). The operator's `∀a. (Int
        // a) → …` scheme does NOT accept a `Float`, so the generic scheme-unify below would spuriously
        // reject two well-typed floats. Skip it (both operands are Float in the well-typed case; `lower`
        // remaps to `Prim::FAdd`… + folds/emits the machine op), descend for operand faults, and return.
        // A genuine FLOAT/other MIX (`(+ 2 2.0)`, `(+ x 1.0)` for an integer `x`) still faults CDZ0301 —
        // reported here, since if it fell through, the scheme-unify would fault only the second operand
        // (its first `(Int a)` param having unified with the leading float's… no — a float never unifies
        // with `(Int a)`, so BOTH would fault, a double-report). This is the numeric-model §An Arithmetic
        // Operator Requires Both Operands To Be One Numeric Type rejection: the mix is caught by the
        // operand disagreement, offering the one-shot int→float coercion on the integer operand (the
        // `(: 3 Float64)` retype for a literal, `(Float64.of-int …)` for a computed int) — the SAME repair
        // wherever an int/float mismatch surfaces. Widths must also agree: a `Float32`/`Float64` mix is a
        // no-silent-promotion CDZ0301 with the `(<Float>.of …)` width wrap (via `numeric_text_coercion_fix`).
        // Comparisons ride the generic `∀a. a→a→Bool` scheme (which accepts two floats) — a float/other
        // comparison mix is faulted below/there — so only the ARITHMETIC forms need this skip; `is_additive`
        // covers comparisons too, so guard the skip to a well-typed both-float pair and let a mix report.
        if (is_multiplicative
            || matches!(
                prim,
                Some(crate::resolved::Prim::Add | crate::resolved::Prim::Sub)
            ))
            && (matches!(a0, Ty::Float(_)) || matches!(b0, Ty::Float(_)))
        {
            // A mix — one operand a float, the other neither a matching-width float nor `Any` — is the
            // no-promotion error CDZ0301. `agrees_with` handles the width check: two `Ty::Float`s agree
            // iff their widths agree (a deferred/var width is compatible), and a float never agrees with a
            // non-float. So the well-typed case is exactly "both floats AND they agree"; anything else is
            // the mix. This is the compile-time rejection of a mixed floating-point/integer application
            // (`(+ 2 2.0)`) — the operator requires both operands to be ONE numeric type, and the fault
            // follows from the operands disagreeing (no silent promotion, no float→int coercion).
            //= spec/capabilities/numeric-model.md#an-arithmetic-operator-requires-both-operands-to-be-one-numeric-type
            //# An arithmetic operator MUST require both of its operands to be the same numeric type, so that an application mixing a floating-point operand with an integer operand is rejected at compile time rather than silently accepting one integer and one floating-point operand or coercing a floating-point operand to an integer.
            let both_float = matches!(a0, Ty::Float(_)) && matches!(b0, Ty::Float(_));
            let a_ok = matches!(a0, Ty::Float(_)) || matches!(a0, Ty::Any);
            let b_ok = matches!(b0, Ty::Float(_)) || matches!(b0, Ty::Any);
            let widths_ok = !both_float || a0.agrees_with(&b0);
            if !(a_ok && b_ok && widths_ok) {
                // Offer a one-shot coercion that conforms the SECOND operand to the FIRST operand's type —
                // the first operand establishes the intended numeric type, the second is retyped to match.
                // So `(+ 2 2.0)` (Int64, Float64) drops the `.0` (`2.0` → `2`, an int context); `(+ 2.0 2)`
                // (Float64, Int64) retypes the int up (`2` → `2.0`); a `Float32`/`Float64` mix wraps the
                // second in the first's `.of`. `numeric_text_coercion_fix` picks the right repair from the
                // (expected = first's type, actual = second's type) pair — and returns `None` when there is
                // no clean one-shot (a non-integer float `2.5` into an int context), leaving the bare
                // CDZ0301. Deterministic and order-consistent (always the second operand), never guessing.
                let (expected, fix_arg, actual) = (a0.clone(), args[1], &b0);
                // A both-float WIDTH mismatch (`Float32`/`Float64`) names the FLOAT domain ("precisions
                // differ … never silently widens or narrows a float") — the message the `Ty::Float`
                // scheme-unify used to give before floating-point arithmetic moved onto the shared `(Int
                // a)`-schemed operator. A float-vs-non-float mix ("no implicit conversion between numeric
                // types") is the ordinary no-promotion wording. Both are CDZ0301 + the same coercion fix.
                let msg = if both_float {
                    let (Ty::Float(fa), Ty::Float(fb)) = (&a0, &b0) else {
                        unreachable!("both_float guarantees two Ty::Float")
                    };
                    format!(
                        "floating-point precisions differ: {}-bit vs {}-bit — convert explicitly \
                         (Cadenza never silently widens or narrows a float)",
                        fa.ground_width(),
                        fb.ground_width(),
                    )
                } else {
                    format!(
                        "no implicit conversion between numeric types {} and {} — convert explicitly \
                         (Cadenza never silently promotes a numeric type)",
                        a0.render_name(),
                        b0.render_name()
                    )
                };
                let mut reject = Reject::coded(Code::NumericMismatch, msg).at(app);
                // Prefer conforming the SECOND operand to the first (the first establishes the intended
                // type). But when that operand has no clean one-shot — a NON-LITERAL float against an int
                // context (`(+ 5 y)`, `y : Float64`: `numeric_text_coercion_fix(Int64, Float64, y)` is
                // `None`, since a runtime float has no int spelling) — the SYMMETRIC repair often IS
                // available: conform the FIRST operand to the second's type (retype the LITERAL `5` → `5.0`).
                // Without this, `(+ 5 y)` offered NO fix while `(+ y 5)` did — an order asymmetry for the
                // identical slip. Try the second-operand coercion first, then fall back to the first, so a
                // literal-int-on-either-side/float-param mix always gets the retype. Deterministic (second
                // preferred, then first); a fix on neither leaves the bare CDZ0301.
                let fix = numeric_text_coercion_fix(db, &expected, actual, fix_arg)
                    .or_else(|| numeric_text_coercion_fix(db, &b0, &a0, args[0]));
                if let Some(fix) = fix {
                    reject = reject.with_fix(fix);
                }
                out.push(reject);
            }
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // A COMPARISON (`< > <= >= = compare`) over a NUMERIC MIX — `(< n 3.0)` for `n : Int64`, or the
        // order-flipped `(< 3 x)` for `x : Float64`. Comparisons ride the generic `∀a. a→a→Bool` scheme
        // (which accepts two equal numeric types), so a mix would fall through to the generic scheme-unify,
        // whose CDZ0301 depends on WHICH operand unified as "expected" — giving the two-way coercion fix in
        // some operand orders (`(< n 3.0)` retypes `3.0`→`3`) but NONE in others (`(< 3 x)` left bare). Fault
        // it HERE with the SAME two-way `numeric_text_coercion_fix` the arithmetic mix uses (M168), so the
        // int/float-literal retype is offered regardless of operand order. Only a definite int-vs-float (or
        // float-width) mix between two NUMERIC operands — a `BigInt`/`Rational`/`Qty` operand is handled by
        // its own block above; an `Any`/`Var` operand is not yet a definite mix; a non-numeric operand is a
        // kind-boundary handled elsewhere. Same CDZ0301 the generic path gives, just with the fix + a
        // stable message.
        let is_comparison = matches!(
            prim,
            Some(
                crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Compare
            )
        );
        let is_fixed_numeric = |t: &Ty| matches!(t, Ty::Int(_) | Ty::Float(_));
        if is_comparison && is_fixed_numeric(&a0) && is_fixed_numeric(&b0) && !a0.agrees_with(&b0) {
            let both_float = matches!(a0, Ty::Float(_)) && matches!(b0, Ty::Float(_));
            let msg = if both_float {
                let (Ty::Float(fa), Ty::Float(fb)) = (&a0, &b0) else {
                    unreachable!("both_float guarantees two Ty::Float")
                };
                format!(
                    "floating-point precisions differ: {}-bit vs {}-bit — convert explicitly \
                     (Cadenza never silently widens or narrows a float)",
                    fa.ground_width(),
                    fb.ground_width(),
                )
            } else {
                format!(
                    "no implicit conversion between numeric types {} and {} — convert explicitly \
                     (Cadenza never silently promotes a numeric type)",
                    a0.render_name(),
                    b0.render_name()
                )
            };
            let mut reject = Reject::coded(Code::NumericMismatch, msg).at(app);
            // Two-way coercion (M168): conform the SECOND operand to the first, else the FIRST to the
            // second — so an int LITERAL retypes to `.0` (or drops it) whichever side it sits on, and a
            // computed int gets the `(<Float>.of-int …)` wrap. A mix with no clean one-shot (a runtime
            // int-vs-float pair) leaves the bare CDZ0301.
            let fix = numeric_text_coercion_fix(db, &a0, &b0, args[1])
                .or_else(|| numeric_text_coercion_fix(db, &b0, &a0, args[0]));
            if let Some(fix) = fix {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        if is_additive {
            let a = a0;
            let b = b0;
            // Only engage when at least one operand is a quantity — two bare numbers take the ordinary
            // numeric path (CDZ0301 etc.), unchanged.
            if matches!(a, Ty::Qty { .. }) || matches!(b, Ty::Qty { .. }) {
                match (&a, &b) {
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
                        // COMPATIBILITY is DIMENSIONAL, not by-unit: two units of one dimension at
                        // DIFFERENT scales (`meter` + `kilometer`) are well-formed and auto-convert; only
                        // a DIMENSION mismatch (`meter` + `second`) is CDZ0501 (units-of-measure.md §A
                        // Dimensional Mismatch Is An Error / §Combining Units Of One Dimension Is
                        // Well-Formed). So gate on `same_dimension` (the exponent map), NOT `==` (which
                        // also compares scale — that distinction is TYPE identity, checked at annotation).
                        //= spec/capabilities/units-of-measure.md#combining-units-of-one-dimension-is-well-formed
                        //# Combining two quantities whose units share a dimension MUST be well-formed even when the units differ, the combination being taken at a common unit of that dimension reached by each operand's exact scale.
                        if !ua.same_dimension(ub) {
                            trace!(target: "rcdzc::infer", head = head.0, "fault: combining quantities of incompatible dimension (CDZ0501)");
                            out.push(Reject::coded(
                                Code::DimensionMismatch,
                                format!(
                                    "{} quantities of incompatible dimension: {} and {} — {} requires \
                                     equal dimensions (units are never silently converted across \
                                     dimensions)",
                                    additive_op_gerund(prim),
                                    ua.render_human(),
                                    ub.render_human(),
                                    additive_op_noun(prim),
                                ),
                            ));
                        } else {
                            // Same dimension — the INNER numeric types must still agree (no promotion
                            // under a unit): unify them and report a numeric mismatch as CDZ0301.
                            let mut subst = Subst::new();
                            if let Err(mut reject) = crate::unify::unify(&mut subst, ia, ib) {
                                // The SAME numeric mismatch a bare `(+ 5 3.0)` gets — so it should offer the
                                // SAME coercion fix (drop the `.0`, or `<Float>.of-int …`), just applied to
                                // the INNER value of the offending quantity rather than the whole `(Qty.of
                                // …)`. The inner value is the FIRST argument of each operand's `Qty.of`
                                // application; retype whichever inner the coercion bridges (`(Qty.of 5 …) +
                                // (Qty.of 3.0 …)` → `5` becomes `5.0`), mirroring the bare-numeric path's
                                // one-shot repair. Only attaches when the inner value node is recoverable
                                // (a directly-written `(Qty.of n u)` operand) and a coercion applies.
                                let inner_a = qty_of_value_arg(db, args[0]);
                                let inner_b = qty_of_value_arg(db, args[1]);
                                let fix = inner_a
                                    .and_then(|n| numeric_text_coercion_fix(db, ib, ia, n))
                                    .or_else(|| {
                                        inner_b
                                            .and_then(|n| numeric_text_coercion_fix(db, ia, ib, n))
                                    });
                                if let Some(fix) = fix {
                                    reject = reject.with_fix(fix);
                                }
                                out.push(reject);
                            }
                        }
                        for &arg in args {
                            collect(db, arg, out);
                        }
                        return;
                    }
                    // A quantity combined additively with a NON-quantity (a bare number) — no implicit
                    // dimensionless coercion (the numeric core's no-silent-promotion discipline applied to
                    // dimensions). CDZ0501.
                    _ => {
                        trace!(target: "rcdzc::infer", head = head.0, "fault: combining a quantity with a non-quantity (CDZ0501)");
                        let mut reject = Reject::coded(
                            Code::DimensionMismatch,
                            format!(
                                "{} a quantity and a plain number: {} and {} — a quantity has a \
                                 dimension a bare number lacks, and there is no implicit \
                                 dimensionless coercion",
                                additive_op_gerund(prim),
                                a.render_name(),
                                b.render_name(),
                            ),
                        );
                        // The mechanical repair: give the BARE number the SAME unit as the quantity operand,
                        // `(Qty.of <n> <unit>)` — then both sides are quantities of one dimension and the
                        // add is well-formed. The unit is recoverable from the quantity operand's type
                        // (`Unit::render` is the re-parseable `(Unit.base #"…")` surface), and `Qty.of`
                        // grounds the bare number to it. HEURISTIC — the author may instead have meant the
                        // quantity's magnitude (`Qty.value`), but giving the bare number the sibling's unit
                        // is the direct resolution of "these are not the same dimension". Fire only when
                        // EXACTLY one operand is the quantity (the other the bare number this wraps).
                        let bare_and_unit = match (&a, &b) {
                            (Ty::Qty { unit, .. }, _) if !matches!(b, Ty::Qty { .. }) => {
                                args.get(1).map(|&n| (n, unit.render()))
                            }
                            (_, Ty::Qty { unit, .. }) if !matches!(a, Ty::Qty { .. }) => {
                                args.first().map(|&n| (n, unit.render()))
                            }
                            _ => None,
                        };
                        if let Some((bare, unit_src)) = bare_and_unit {
                            reject = reject.with_fix(Fix::wrap_heuristic(
                                bare,
                                "(Qty.of ",
                                format!(" {unit_src})"),
                                format!("give the number the same unit: `(Qty.of … {unit_src})`"),
                            ));
                        }
                        out.push(reject);
                        for &arg in args {
                            collect(db, arg, out);
                        }
                        return;
                    }
                }
            }
        }
    }
    // `Map.insert m k v` — the inserted KEY must agree with the map's key type AND the inserted VALUE with
    // its value type (collections-and-text.md §A Map Associates Keys With Values — keys of ONE type with
    // values of ONE type). A mismatch is CDZ0201 (a map homogeneity violation, exactly as the `(map …)`
    // literal is — coded Malformed), NOT the CDZ0203 the generic scheme-unify below would give. Read the
    // map OPERAND's solved `Ty::Map(k, v)` and compare the arg types via `agrees_with` (the same
    // structural agreement the literal's homogeneity check uses); a still-unsolved map operand
    // (`Ty::Any`/a var) is skipped (its own fault, if any, surfaces elsewhere).
    if args.len() == 3
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::MapInsert)
        && let Ty::Map(kt, vt) = type_of(db, args[0])
    {
        let key_ty = type_of(db, args[1]);
        let val_ty = type_of(db, args[2]);
        if !kt.agrees_with(&key_ty) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Map.insert key type disagrees with the map's key type (CDZ0201)");
            // Name the map's KEY type and the inserted key's type (like the map-literal heterogeneity
            // message), and offer the int-literal→float retype when that bridges the clash — the
            // Map.insert twin of the map-literal check (M75).
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a map associates keys of one type, but this key's type differs: the map's keys are \
                     {}, this key is {}",
                    kt.render_name(),
                    key_ty.render_name()
                ),
            );
            if let Some(fix) = float_literal_retype_fix(db, args[1], &key_ty, &kt) {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
        }
        if !vt.agrees_with(&val_ty) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Map.insert value type disagrees with the map's value type (CDZ0201)");
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a map associates values of one type, but this value's type differs: the map's \
                     values are {}, this value is {}",
                    vt.render_name(),
                    val_ty.render_name()
                ),
            );
            if let Some(fix) = float_literal_retype_fix(db, args[2], &val_ty, &vt) {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
        }
        if !kt.agrees_with(&key_ty) || !vt.agrees_with(&val_ty) {
            // Descend into the operands for their own faults, then stop (do NOT run the generic
            // scheme-unify, which would ALSO report the same mismatch as a CDZ0203 duplicate).
            for &a in args {
                collect(db, a, out);
            }
            return;
        }
    }
    // `Set.of list` — the set is HOMOGENEOUS: its elements (the list's) must share one type
    // (collections-and-text.md §A Set Is A Collection Of Unique Elements — elements of ONE type). A
    // mismatch is CDZ0201 (a SET homogeneity violation — the corpus codes it like the map homogeneity
    // cases), NOT the CDZ0203 the list-element unify would give on its own. So check the list argument's
    // element types HERE and, on a mismatch, report CDZ0201 + descend into the elements, stopping before
    // the generic path lets the inner list emit CDZ0203. (A homogeneous list flows through unchanged.)
    if args.len() == 1 && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::SetOf)
    {
        // Read the list argument's element occurrences — the `(list …)` string-head form, the `list`
        // name-alias application, or a `Resolved::List`.
        let list = args[0];
        let elems: Vec<StructId> = match resolved_of(db, list) {
            Resolved::List { elems } => elems.to_vec(),
            Resolved::Apply { head: lh, args: la }
                if crate::eval::meta_apply_of(db, lh) == Some(crate::resolved::Prim::ListNew) =>
            {
                la.to_vec()
            }
            _ => Vec::new(), // not a visible list literal — the generic path handles it
        };
        let mut subst = Subst::new();
        // Capture the FIRST clashing element (occurrence + type) so the message can name the two types and
        // offer the int-literal→float retype fix, like the list-homogeneity check — the set twin of M57.
        let mut clash: Option<(StructId, Ty)> = None;
        let first_pair = elems.first().map(|&f| (f, type_of(db, f)));
        if let Some((_, first_ty)) = &first_pair {
            for &e in elems.iter().skip(1) {
                let et = type_of(db, e);
                if crate::unify::unify(&mut subst, first_ty, &et).is_err() {
                    clash = Some((e, et));
                    break;
                }
            }
        }
        if let (Some((first, first_ty)), Some((e, et))) = (&first_pair, &clash) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Set.of elements do not share one type (CDZ0201)");
            let delta = peer_type_delta_hint(first_ty, et).unwrap_or_default();
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a set contains elements of one type, but the elements differ: {} and {}{delta}",
                    first_ty.render_name(),
                    et.render_name()
                ),
            );
            if let Some(fix) = float_literal_retype_fix(db, *first, first_ty, et)
                .or_else(|| float_literal_retype_fix(db, *e, et, first_ty))
            {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
            for &el in &elems {
                collect(db, el, out);
            }
            return;
        }
    }
    // A LIST constructor (`list` alias) applied — its arguments are its ELEMENTS, and a list is
    // HOMOGENEOUS: every element must share one type (collections-and-text.md §A List Is A Homogeneous
    // Sequence). Unify each element's type against the first; a mismatch (`(list 1 true)`) is CDZ0203. The
    // `list` NAME alias resolves to a `Resolved::Apply` (this path), NOT `Resolved::List` (the symbol
    // form), so this check — not the `Resolved::List` `collect` arm — is what catches a mixed name-alias
    // list. (`ListNew` has no `(meta t)` scheme, so the generic check below would silently pass it.)
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::ListNew)
    ) {
        let mut subst = Subst::new();
        if let Some(&first) = args.first() {
            let first_ty = type_of(db, first);
            for &e in args.iter().skip(1) {
                let et = type_of(db, e);
                if crate::unify::unify(&mut subst, &first_ty, &et).is_err() {
                    // The unify reports the generic CDZ0203. But list homogeneity draws the SAME taxonomy
                    // line the numeric operators and `if`-branches do (05-compound-types): a HOMOGENEITY
                    // violation between two DISTINCT NUMERIC types (`(list 1 2.5)` — Int64 vs Float64, the
                    // no-silent-promotion rule) or two SAME-KIND-DIFFERENT-SHAPE compounds (`(list (record
                    // (a 1)) (record (b 2)))` — records of different field sets; `(list (tuple 1 2) (tuple 1
                    // 2 3))` — tuples of different arity, where the field set / arity IS the type) is a
                    // MALFORMED list (CDZ0201), not the generic structural mismatch (CDZ0203) a cross-KIND
                    // scalar clash (`(list 1 true)` — Int64 vs Bool) is. Reclassify exactly those two shapes;
                    // every other element disagreement keeps the unify's CDZ0203.
                    let code = list_homogeneity_code(&first_ty, &et);
                    trace!(target: "rcdzc::infer", head = head.0, ?code, "fault: list elements differ in type");
                    // An INT-LITERAL-vs-FLOAT clash has the same one-shot repair the annotation site gives
                    // (`(: 3 Float64)` → `3.0`): rewrite the integer literal as a float literal so the list
                    // unifies at the float type. The literal may be on EITHER side — the FIRST element
                    // (`(list 1 2.0)`, fix `first`) or THIS one (`(list 1.0 2)`, fix `e`); offer the fix on
                    // whichever side is the int literal (a computed integer expression yields no fix).
                    // When the two element types are the SAME structured kind but differ INSIDE (records of
                    // one field's type, tuples of one position, a nested collection axis), name the specific
                    // differing sub-part instead of rendering two whole compounds — the join-site reuse of
                    // the annotation-mismatch per-member hints.
                    let delta = peer_type_delta_hint(&first_ty, &et).unwrap_or_default();
                    // Anchor at the OUTLIER element `e` (the one that broke homogeneity against the
                    // established first-element type), not the whole `(list …)` — the squiggle lands on
                    // `"three"` in `(list 1 2 "three" 4 5)` rather than the entire list, so the reader sees
                    // exactly which element is off. (Without `.at`, `collect` stamps the coarse list node.)
                    let mut reject = Reject::coded(
                        code,
                        format!(
                            "list elements must share one type: {} and {}{delta}",
                            first_ty.render_name(),
                            et.render_name()
                        ),
                    )
                    .at(e);
                    if let Some(fix) = float_literal_retype_fix(db, first, &first_ty, &et)
                        .or_else(|| float_literal_retype_fix(db, e, &et, &first_ty))
                        // A record-field TYPO in one element vs the other (`(list (record (foo 1)) (record
                        // (fooo 2)))`) — rename the misspelled key, whichever element carries it. A peer join
                        // has no fixed "expected", so try BOTH orderings: treat the FIRST as the target shape
                        // (the outlier `e` has the typo), then the outlier as the target (the first has it).
                        .or_else(|| record_field_typo_fix(db, &first_ty, &et, e))
                        .or_else(|| record_field_typo_fix(db, &et, &first_ty, first))
                    {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
            }
        }
        for &e in args {
            collect(db, e, out);
        }
        return;
    }
    // A MAP constructor (`map` alias) applied — its arguments are its ENTRY PAIRS `(key value)`, NOT
    // curried arguments and NOT ordinary sub-expressions (a `(a 1)` entry is the pair a↦1, so it must
    // NOT be checked as "apply `a` to `1`"). Read each entry's RAW `(key value)` children, check key/
    // value homogeneity (all keys one type, all values one type — CDZ0201) + duplicate-const-key, then
    // `collect` faults from each KEY and each VALUE (as values, in scope). The `map` NAME alias resolves
    // to a `Resolved::Apply` (this path), so this — not the `Resolved::Map` `collect` arm — catches the
    // name-alias map's faults; the two share the same rules.
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(crate::resolved::Prim::MapNew)
    ) {
        // Read the entry pairs' (key, value) child occurrences (a malformed entry is faulted at resolve).
        let entries: Vec<(StructId, StructId)> = args
            .iter()
            .filter_map(|&e| match db.ast.get(e) {
                crate::ast::Struct::List(items) if items.len() == 2 => Some((items[0], items[1])),
                _ => None,
            })
            .collect();
        let mut ksubst = Subst::new();
        let mut vsubst = Subst::new();
        if let Some(&(fk, fv)) = entries.first() {
            let (fkt, fvt) = (type_of(db, fk), type_of(db, fv));
            for &(k, v) in entries.iter().skip(1) {
                let kt = type_of(db, k);
                if crate::unify::unify(&mut ksubst, &fkt, &kt).is_err() {
                    // Name the two clashing key types (like the list-homogeneity message) and, for an
                    // int-literal-vs-float clash, offer the same `3.0` retype fix — the map-key twin of the
                    // list/if/match "same repair wherever the same mismatch surfaces" (M57). The
                    // structural-delta hint names the SPECIFIC differing sub-part when the two key types are
                    // same-kind compounds (a record field / tuple position / sum payload) — the peer-join
                    // hint the list/if/match/set sites carry.
                    let delta = peer_type_delta_hint(&fkt, &kt).unwrap_or_default();
                    let mut reject = Reject::coded(
                        Code::Malformed,
                        format!(
                            "a map associates keys of one type, but the keys differ: {} and {}{delta}",
                            fkt.render_name(),
                            kt.render_name()
                        ),
                    );
                    if let Some(fix) = float_literal_retype_fix(db, fk, &fkt, &kt)
                        .or_else(|| float_literal_retype_fix(db, k, &kt, &fkt))
                    {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
                let vt = type_of(db, v);
                if crate::unify::unify(&mut vsubst, &fvt, &vt).is_err() {
                    let delta = peer_type_delta_hint(&fvt, &vt).unwrap_or_default();
                    let mut reject = Reject::coded(
                        Code::Malformed,
                        format!(
                            "a map associates values of one type, but the values differ: {} and {}{delta}",
                            fvt.render_name(),
                            vt.render_name()
                        ),
                    );
                    if let Some(fix) = float_literal_retype_fix(db, fv, &fvt, &vt)
                        .or_else(|| float_literal_retype_fix(db, v, &vt, &fvt))
                    {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
            }
        }
        if let Some(reject) = map_duplicate_const_key(db, &entries) {
            out.push(reject);
        }
        for (k, v) in entries {
            collect(db, k, out);
            collect(db, v, out);
        }
        return;
    }
    // A NULLARY variant CONSTRUCTOR applied to the unit value — `(None unit)` / `(Nil ())` — is the
    // canonical construction of a nullary variant (core-semantics.md §Construction MUST Be Via
    // Application). Its ctor `(meta t)` is the bare sum (no arrow — `variant_payload_type` is `None`), so
    // the generic "instantiate the scheme and apply each arg" check below would see a non-function head
    // and wrongly fault "cannot apply a value of type <Sum>". Recognize it here: a variant ctor
    // (`(meta variant)` present) with no payload type, applied to one argument, is a well-formed nullary
    // construction — the argument is the unit payload. (A NON-unit argument is an arity error the arg's
    // own type surfaces; a nullary variant's payload type is unit, checked when it matters at the escape.)
    if crate::eval::variant_disc_of(db, head).is_some()
        && crate::eval::variant_payload_type(db, head).is_none()
    {
        // A nullary variant's argument type IS Unit (core-semantics.md §A Sum Type Constructor Is A
        // Single-Arity Function: "A nullary variant MUST be a constructor whose argument type is Unit").
        // So the ONE argument must be `unit`; a NON-unit payload — `(None 5)`, `(Opt.Nn (tuple 1 2))` —
        // is a malformed construction, NOT a silently-discarded payload. Without this check the argument
        // vanished and the value rendered `(Nn unit)`, fabricating a variant (a soundness hole — an
        // ill-typed program accepted). Reject CDZ0201, the SAME code the corpus assigns a constructor
        // applied to a wrong-type payload (the nullary-Unit and the unary declared-payload cases in
        // 05-compound-types both pin CDZ0201 — a malformed construction, not a plain type mismatch).
        // Check the supplied argument's type against Unit, then descend for the argument's own faults.
        for &arg in args {
            let at = type_of(db, arg);
            if !at.agrees_with(&Ty::Unit) {
                trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, at = %at.render_name(), "fault: nullary variant applied to a non-unit payload (CDZ0201)");
                out.push(Reject::coded(
                    Code::Malformed,
                    format!(
                        "a nullary variant takes the unit value, but a payload of type {} was applied",
                        at.render_name()
                    ),
                ));
            }
            // Descend for the argument's OWN faults (an unbound name in `(None (frob))`).
            collect(db, arg, out);
        }
        return;
    }
    // A LAMBDA head β-reduces. Its faults do NOT surface on their own: the outer `collect` walks the
    // ORIGINAL call (head + argument occurrences), never the reduced body, and β-reduction ERASES the
    // parameter↔argument relationship (the parameter's annotation is dropped when its argument is
    // substituted). So a mistyped argument — `(f 5)` to a `(: x Bool)` parameter, or to a bare
    // parameter the body uses as a Bool/Int — goes unreported and the emitter later produces invalid
    // wasm. Check the call here, at the call site, in two parts:
    if crate::eval::lambda_body(db, head).is_some() {
        // (1) Unify each argument against its PARAMETER's declared type. An annotated parameter
        //     (`(: x Bool)`) has a definite type the argument must agree with; a bare parameter types
        //     `Any` (its body-inferred type isn't a signature here) and unifies with anything, so this
        //     catches the annotated-parameter mismatch (case A) precisely, without over-rejecting.
        //
        //     A REFERENCED parameter's argument is ALSO checked by step (2): substituting the argument
        //     into the body puts it under the parameter's SYNTHESIZED `(: arg paramtype)` annotation, whose
        //     `Annot` check reports the SAME conflict as an annotation-context CDZ0203 — carrying the
        //     actionable retype/coercion fix (`(: 3 Float64)` → `3.0`), exactly as a direct annotation
        //     does. That report is the AUTHORITATIVE one (annotation semantics — the arg must satisfy the
        //     declared type). So a step-(1) unify fault at a param step (2) WILL cover is a REDUNDANT twin
        //     with a different, fix-less code (CDZ0301 for a numeric mix) — the M58 dual-producer shape,
        //     but across codes so `dedup_faults`'s (code, node) rule can't collapse it. BUFFER step (1)'s
        //     faults with their param index and flush only those step (2) will NOT cover (an UNREFERENCED
        //     param — step (2) is silent — or a callee whose reduction declines, e.g. recursion). The
        //     "covered" test mirrors step (3)'s exactly (`reduced_ok && param_is_referenced`).
        let mut arg_faults: Vec<(usize, Reject)> = Vec::new();
        if let Some(params) = crate::eval::lambda_params_of(db, head) {
            let mut subst = Subst::new();
            for (i, (&param_occ, &arg)) in params.iter().zip(args.iter()).enumerate() {
                let pt = type_of(db, param_occ);
                let at = type_of(db, arg);
                if let Err(reject) = crate::unify::unify(&mut subst, &pt, &at) {
                    trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, "apply: argument conflicts with parameter annotation (type fault)");
                    // REWORD to the call-ARGUMENT phrasing (M106): this fault is a wrong-typed call
                    // argument, but the raw unify gave "type mismatch: Int64 and Bool must be the same type
                    // here" — the same defect the synthesized-parameter-annotation path (step 2) reports as
                    // "this argument is a Bool, but a value of type Int64 is expected here". Step (1) is the
                    // SOLE reporter for an UNREFERENCED param (step 2 is silent) and a RECURSIVE callee (the
                    // reduction declines), so without this those two cases keep the raw unify wording while a
                    // referenced-param arg reads nicely — an inconsistency for one defect. Here we ALSO have
                    // the callee + parameter names (step 2 has only the annotation node), so name them when
                    // known. Keep the reject's CODE (CDZ0301 for a numeric mix, CDZ0203 otherwise) — only the
                    // MESSAGE is reworded — and append the structural-delta hint for a same-kind compound.
                    let callee = callee_head_name(db, head);
                    let param = db
                        .ast
                        .as_name(crate::eval::param_name_occ(db, param_occ))
                        .map(str::to_string);
                    let tail = structural_delta_hint(&pt, &at).unwrap_or_default();
                    let reject = Reject {
                        message: call_argument_mismatch_message(
                            callee.as_deref(),
                            param.as_deref(),
                            &pt,
                            &at,
                            &tail,
                        ),
                        ..reject
                    };
                    // A value of the parameter sum's PAYLOAD type where the SUM is expected — `(f 5)` to a
                    // `(: o (Option Int64))` parameter. Offer the rustc-flagship "wrap in `Some`" repair:
                    // WRAP the argument in the matching constructor `(Some 5)`. General over any sum (reads
                    // the expected sum's own variants), forced-choice only (ambiguous → no suggestion), and
                    // the wrap type-checks in one shot. Heuristic — the intent (which construction) is a guess.
                    let tagged = if let Some(variant) = wrap_variant_for(db, &pt, &at) {
                        reject.with_fix(Fix::wrap_heuristic(
                            arg,
                            format!("({variant} "),
                            ")",
                            format!("wrap the value in `{variant}`"),
                        ))
                    } else if let Some((prefix, suffix, verb)) = total_conversion_wrap(&pt, &at) {
                        // A total prelude conversion bridges the mismatch at the CALL SITE — `String` where
                        // `Bytes` is expected → `(String.to-bytes …)`. The text-model twin of the numeric
                        // coercion wraps; heuristic, `--verify-fixes` upgrades it.
                        reject.with_fix(Fix::wrap_heuristic(arg, prefix, suffix, verb))
                    } else if let Some(fix) = record_field_typo_fix(db, &pt, &at, arg)
                        .or_else(|| record_field_add_fix(db, &pt, &at, arg))
                        .or_else(|| record_field_delete_fix(db, &pt, &at, arg))
                        .or_else(|| tuple_element_add_fix(db, &pt, &at, arg))
                        .or_else(|| tuple_element_delete_fix(db, &pt, &at, arg))
                    {
                        // A RECORD-literal field-set repair: a misspelled field is RENAMED (first — a rename
                        // is the minimal edit); a pure OMISSION gets the missing fields ADDED with `(trap
                        // "TODO")` placeholders; a lone SURPLUS field is DELETED. The construction analogue of
                        // rustc's "missing field `y`" / "no field `z`" with an applicable edit. The TUPLE
                        // analogue does the same by POSITION — too few elements get `(trap "TODO")` appended,
                        // one too many gets the trailing element deleted.
                        reject.with_fix(fix)
                    } else {
                        reject
                    };
                    arg_faults.push((i, tagged));
                }
            }
            // OVER-APPLICATION: more arguments than the lambda's arity, and the body's result is not
            // itself a function to absorb them. `((fn (x) (+ x 1)) 5 9)` is a type error, not a decline —
            // the SAME CDZ0201 the corpus assigns an over-applied CONSTRUCTOR ("over-applying a constructor
            // is a type error, not a silent argument drop"). Without this, `apply_lambda` returns
            // Err("applied more arguments…") and the reduced body is never collected, so the extra args
            // silently graded as a to-do. A body whose result IS a function (a curried lambda returning a
            // lambda) legitimately absorbs the extra args — so gate on the reduced result NOT being an
            // arrow (checked via the full-application result type below).
            if args.len() > params.len() {
                // Type the lambda fully applied to its own arity; if the result is not a function, the
                // surplus args over-apply it.
                let applied_ty = apply_type(db, head, &args[..params.len()]);
                if !matches!(applied_ty, Ty::Fn(_, _) | Ty::Any) {
                    trace!(target: "rcdzc::infer", head = head.0, arity = params.len(), args = args.len(), "fault: over-applied lambda (CDZ0203)");
                    // CDZ0203 (`TypeMismatch`) — the SAME code an over-applied CONSTRUCTOR and a
                    // scheme-typed over-application use (applying the fully-consumed value, which is not a
                    // function, to a further argument). Keeps the over-application taxonomy uniform.
                    // The mechanical repair: DELETE the FIRST surplus argument (`args[params.len()]`) — the
                    // fixpoint removes each extra in turn until the arity matches. Anchor the fault + fix at
                    // the surplus arg so `cdz fix` edits it. Heuristic: the author may instead have meant a
                    // different callee; removing the extra is the direct resolution of "too many arguments".
                    let mut reject = Reject::coded(
                        Code::TypeMismatch,
                        format!(
                            "applied {} arguments to a function of arity {} — it is not a function after \
                             its arguments are consumed",
                            args.len(),
                            params.len()
                        ),
                    );
                    if let Some(&surplus) = args.get(params.len()) {
                        reject = reject
                            .at(surplus)
                            .with_fix(crate::diag::Fix::delete_heuristic(
                                surplus,
                                "remove the extra argument",
                            ));
                    }
                    out.push(reject);
                }
            }
        }
        // (2) Collect the REDUCED body's own faults. Substituting the concrete arguments turns a
        //     body use of a bare parameter into a use of the actual argument value, so a use at a
        //     conflicting type — `(if 5 1 2)`, `(+ true true)` — now faults where the unreduced
        //     `(if x 1 2)` (x : Any) did not. This is what turns the case-B/C MISCOMPILE (invalid
        //     wasm) into a reported rejection. Runs under the reduction guard; a recursive callee
        //     declines the reduction (no reduced body) and is checked by its own def collection.
        let mut reduced_ok = false;
        if let Some(mut guard) = db.enter_reduction() {
            let g = guard.db();
            if let Ok(Some(reduced)) = crate::eval::apply_lambda(g, head, args) {
                // Collect the reduced body's faults into a LOCAL vec first so a fault anchored to a
                // SYNTHESIZED node (β-reduction mints fresh nodes for the substituted body — a use of a
                // bare parameter becomes a use of the argument VALUE, on a node with no source span) can
                // be re-anchored to the CALL SITE. Without this, `(helper true)` where `helper`'s body is
                // `(+ x 1)` reduces to `(+ true 1)` on spanless nodes, so its CDZ0203 reported with NO
                // location (a bare `cdz:`/`file:` prefix). The call `(helper true)` IS the source the
                // author must fix — anchoring the reduced-body fault to `app` (when it landed on a
                // non-user, hence spanless, node) restores its `file:line:col`. A fault already anchored
                // to a real user node (e.g. inside an argument sub-expression) keeps its own, more precise
                // anchor.
                let mut body_faults = Vec::new();
                collect(g, reduced, &mut body_faults);
                // A fault the callee's UNREDUCED body ALREADY has does NOT depend on this call's arguments —
                // a non-exhaustive `(match c …)` or an unbound name in the body faults regardless of what is
                // passed. The callee's OWN def-body check (`compile::collect_faults`) reports it once, at the
                // definition, WITH its fix (e.g. the CDZ0210 insert-arms). Re-surfacing it here — re-anchored
                // to the CALL SITE and stripped of its fix — is a DUPLICATE: the same defect reported twice,
                // the second copy worse (no fix, points at the caller not the buggy match). So keep only the
                // faults β-reduction INTRODUCED — the ones absent from the unreduced body, which are exactly
                // the argument-induced faults this check exists to catch (`(if x …)` → `(if 5 …)`,
                // `(+ x 1)` → `(+ true 1)`). Diff by (code, message): a renumbering-invariant identity, the
                // same key `dedup_faults` uses.
                //
                // GUARD: only compute the baseline when the reduced body actually HAS faults to filter — an
                // empty `body_faults` (the well-typed common case) has nothing to diff, so skip the baseline
                // collect entirely. This matters for a NESTED-LAMBDA chain `((fn a0) ((fn a1) …) 1) 0)`: the
                // reduced body and the unreduced callee body BOTH contain the inner nested application, so
                // collecting BOTH re-reduced the inner chain twice per level → O(2^depth) (a depth-20 chain:
                // ~28s). The reduced-body collect alone is unavoidable, but the baseline was pure waste when
                // there are no faults to filter — and a well-typed deep chain has none. (A body WITH faults
                // still computes the baseline, so a genuine callee-defect is still de-duplicated exactly.)
                let baseline: std::collections::HashSet<(Option<crate::diag::Code>, String)> =
                    if body_faults.is_empty() {
                        std::collections::HashSet::new()
                    } else {
                        match crate::eval::lambda_body(g, head) {
                            Some(callee_body) => {
                                let mut unreduced = Vec::new();
                                collect(g, callee_body, &mut unreduced);
                                unreduced.into_iter().map(|f| (f.code, f.message)).collect()
                            }
                            None => std::collections::HashSet::new(),
                        }
                    };
                for mut f in body_faults {
                    if baseline.contains(&(f.code, f.message.clone())) {
                        continue; // the callee's own defect — already reported at the definition, with its fix
                    }
                    let on_user_node = f.at.is_some_and(|o| g.is_user_node(o));
                    if !on_user_node {
                        f.at = Some(app);
                    }
                    out.push(f);
                }
                reduced_ok = true;
            }
        } else {
            // The reduction depth limit was hit — the reduced body was NOT collected here, so this
            // application's fault set is INCOMPLETE. Mark the walk limited so it is not memoized (a
            // shallower entry would reduce and collect the body). See `collect_cache`.
            db.collect_limited = true;
        }
        // (3) Descend the ARGUMENTS for their OWN faults (an unbound name / malformed application in an
        //     argument, whether or not the body uses it). When the reduction SUCCEEDED (`reduced_ok`), a
        //     USED argument's faults are already in the reduced body (step 2), so descend only the DEAD
        //     arguments — the ones the body does not reference, hence absent from the reduced body. This
        //     is what keeps a deep call chain `(f (f … (f 0)))` LINEAR: `f` uses its parameter, so no
        //     argument is dead and no argument is re-descended (the O(N³) came from re-walking every
        //     argument here AND in the reduced body, compounded per level). When the reduction did NOT
        //     produce a body (a recursive callee, the depth limit), NOTHING covered the arguments, so
        //     descend them ALL — matching the pre-change behavior for that case.
        //
        //     The SAME `covered` test flushes step (1)'s buffered arg-mismatch faults: a fault at a param
        //     step (2) covered (referenced + reduced) is the redundant twin of step (2)'s annotation-context
        //     CDZ0203 — drop it; a fault at an UNCOVERED param (unreferenced / reduction declined) is the
        //     SOLE report — keep it.
        let params = crate::eval::lambda_params_of(db, head).unwrap_or_default();
        // The set of parameters the body references, computed in ONE walk (was a full-body scan PER
        // argument → O(args × body) = O(N²) for a WIDE application; now O(body) once + O(1) per arg).
        // Only needed when the reduction succeeded — that is the only branch that skips a covered arg.
        let referenced = if reduced_ok {
            crate::eval::lambda_body(db, head)
                .map(|body| referenced_binders(db, body))
                .unwrap_or_default()
        } else {
            std::collections::HashSet::new()
        };
        for (i, &arg) in args.iter().enumerate() {
            let covered = reduced_ok && params.get(i).is_some_and(|&p| referenced.contains(&p));
            if !covered {
                collect(db, arg, out);
            }
        }
        for (i, reject) in arg_faults {
            let covered = reduced_ok && params.get(i).is_some_and(|&p| referenced.contains(&p));
            if !covered {
                out.push(reject);
            }
        }
        return;
    }
    // An ARITHMETIC operator applied at an arity OTHER than 2, over a NON-integer numeric operand
    // (`(+ 1.0 2.0 3.0)`, `(+ x 1.0 2.0)` for a Float `x`) — the well-typed arity-2 case took the
    // Float/BigInt/Rational skip arm above (guarded on `args.len() == 2`), but an over/under-application
    // falls through here to the generic `∀a. (Int a) → …` scheme-unify, which faults each Float/BigInt/
    // Rational arg CDZ0301 (they never unify with `(Int a)`). That is a SPURIOUS type-mismatch masking
    // the real ARITY error — the emit-path lowering reports the clean CDZ0201 "+ takes exactly 2
    // operands" (with the delete/complete fix). So skip the generic unify for this shape, descend for the
    // operands' own faults, and return; the arity CDZ0201 is the sole report. (An arity-2 integer/mixed
    // application is unaffected — it is handled above or wants the generic unify's numeric report.)
    if args.len() != 2
        && let Some(prim) = crate::eval::meta_apply_of(db, head)
        && prim.is_arith()
        && args
            .iter()
            .any(|&a| matches!(type_of(db, a), Ty::Float(_) | Ty::BigInt | Ty::Rational))
    {
        trace!(target: "rcdzc::infer", head = head.0, "fault: skip generic unify for a non-arity-2 arithmetic op over a non-int numeric (the arity CDZ0201 is the real fault)");
        for &arg in args {
            collect(db, arg, out);
        }
        return;
    }
    let mut fresh = Fresh::new();
    let scheme = match crate::eval::scheme_of(db, head, &mut fresh) {
        Some(s) => s,
        // No scheme: the head is not a function-valued built-in/def. It may still be applyable via one
        // of the paths above (lambda / constructor / compound alias — all handled + returned already),
        // so reaching here means the head is a PLAIN VALUE. If its type is a DEFINITE non-function —
        // applying a scalar `(5 3)`, a Bool `(true 1)`, a Float `(3.5 1)` — that is a MALFORMED
        // application (`core-semantics.md` §Applying A Function Binds Its Parameter To Its Argument: a
        // non-function has no defined result), the CDZ0201 the corpus assigns (09-functions "applying a
        // non-function/boolean/float is a type error"). Reject it here rather than letting lowering
        // decline "value is not applyable" (which grades as a to-do, not the type error it is). Guarded
        // on `is_definite_non_function` so an UNDETERMINED head (`Ty::Any` — a not-yet-modeled construct,
        // an unresolved var) still falls through to a clean decline, never a spurious reject.
        None => {
            // A head with a `(meta apply)` PRIMITIVE is applyable via that primitive even though it has
            // no type SCHEME — the compound-value constructors (`tuple`/`record`/`list` aliases build the
            // compound), a type constructor (`(Int 64)`), etc. Those are NOT "applying a non-function";
            // only a head that is neither scheme-typed NOR a `(meta apply)` primitive AND whose type is a
            // definite non-function is the malformed `(5 3)` / `(true 1)` / `(3.5 1)` case.
            if !args.is_empty() && crate::eval::meta_apply_of(db, head).is_none() {
                let ht = type_of(db, head);
                if is_definite_non_function(&ht) {
                    // When the head is a bare name that DECLARES a type or effect, name the CATEGORY —
                    // `(E 5)` for an effect `E`, `(Color 5)` for a type `Color`. Rendering `ht.render_name()`
                    // for an effect leaks its SYNTHESIZED record type verbatim (`(Record (foo (Record (apply
                    // Any) …)) …)`) — an internal representation dumped at the user, the leaky-message
                    // anti-pattern. Say "`E` is an effect, not a function" (the apply-position analogue of
                    // the M74 export-a-type category message). A non-name head (a literal `(5 3)`, a value)
                    // keeps the type-named message — the type IS the useful fact there.
                    let name = db.ast.as_name(head).map(str::to_string);
                    let name_category = name.as_deref().and_then(|n| {
                        if db.type_decl_by_name(n).is_some() {
                            Some((n.to_string(), "a type"))
                        } else if db.effect_decl_by_name(n).is_some() {
                            Some((n.to_string(), "an effect"))
                        } else {
                            None
                        }
                    });
                    // A bare name resolving to a NULLARY FUNCTION def — `(def (g) …)` applied `(g 5)`. A
                    // nullary def resolves its name straight to its body VALUE (a `Ref`), so `g` IS that
                    // value and `(g 5)` genuinely applies a non-function — but the author wrote `g` with a
                    // `()` signature and CALLED it, so "cannot apply a value of type Int64" hides both the
                    // name and the real cause (it takes no arguments). Distinguish it from a plain value def
                    // `(def v …)` by its SIGNATURE shape: a nullary FUNCTION's `sig_occ` is a list `(g)`, a
                    // value def's is a bare name. Name it + say it takes no arguments (the nullary companion
                    // of the over-application naming — M99/M105). A value def keeps the type-named message
                    // (its value IS the fact — `v` names nothing more useful than its type).
                    let nullary_fn = name
                        .as_deref()
                        .filter(|_| name_category.is_none())
                        .and_then(|n| {
                            let idx = db.def_by_name(n)?;
                            let sig = db.defs[idx].sig_occ;
                            matches!(db.ast.get(sig), crate::ast::Struct::List(_))
                                .then(|| n.to_string())
                        });
                    trace!(target: "rcdzc::infer", head = head.0, ty = %ht.render_name(), "fault: applying a non-function value (CDZ0201)");
                    let message = match (name_category, nullary_fn) {
                        (Some((name, cat)), _) => {
                            format!(
                                "`{name}` is {cat}, not a function — it cannot be applied to arguments"
                            )
                        }
                        (None, Some(name)) => format!(
                            "`{name}` takes no arguments, but {} {} applied — call it as `({name})`, without arguments",
                            args.len(),
                            if args.len() == 1 { "was" } else { "were" }
                        ),
                        (None, None) => format!(
                            "{} {} — it is not a function",
                            crate::diag::NOT_A_FUNCTION_PREFIX,
                            ht.render_name()
                        ),
                    };
                    out.push(Reject::coded(Code::Malformed, message));
                }
            }
            return;
        }
    };
    let mut cur = crate::unify::instantiate(&scheme, &mut fresh);
    let mut subst = Subst::new();
    for (arg_index, &arg) in args.iter().enumerate() {
        let applied = subst.apply(&cur);
        match applied {
            Ty::Fn(param, result) => {
                // Freshen the arg's free variables past the head's instantiation before unifying — see
                // the same step in `apply_type`. Without it, an under-constrained arg (a bare nullary
                // variant `(None) : Option ?0`) shares variable numbers with the head scheme and the
                // occurs-check spuriously FAULTS a well-typed `(Some (None))` (CDZ0203 "infinite type").
                let at = crate::unify::freshen_free(&type_of(db, arg), &mut fresh);
                if let Err(reject) = crate::unify::unify(&mut subst, &param, &at) {
                    trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, "apply: argument conflicts with parameter (type fault)");
                    // A wrong-type payload to a VARIANT CONSTRUCTOR — `(T.Mk "x")` for `(Mk Int64)` — is a
                    // MALFORMED construction (`CDZ0201`), the code the corpus assigns a constructor applied
                    // to a wrong-type/wrong-arity payload (the typed-payload companion of the nullary-Unit
                    // check above). It flows through this SAME instantiated-and-substituted unify as any
                    // application — so a GENERIC/nested construction (`(Ok (Err 9))`, `(Some (None))`) is
                    // NOT over-rejected — only the diagnostic CODE is specialized: a variant-ctor head
                    // reclassifies the unify mismatch from the generic `CDZ0203` to the structural
                    // `CDZ0201`. A non-ctor head (an ordinary function, an operator) keeps `CDZ0203`.
                    let sparam = subst.apply(&param);
                    let sat = subst.apply(&at);
                    if crate::eval::variant_disc_of(db, head).is_some() {
                        // A wrong-type payload to a VARIANT CONSTRUCTOR (CDZ0201). When the mismatch is a
                        // NUMERIC/TEXT one a total conversion repairs — `(Mk a)` with `a:Int8`, payload
                        // Int64 → `(Int64.of a)`; `(Mk 3.0)`, payload Int64 → `3`; `(Mk s)` s:String,
                        // payload Bytes → `(String.to-bytes s)` — offer the SAME coercion fix the operator/
                        // argument position does (the D33 lesson: the same repair wherever the same mismatch
                        // surfaces). No coercion (e.g. Bool payload) → the bare reject.
                        // Anchor at the offending ARGUMENT (the wrong-type payload value), not the whole
                        // ctor application — the squiggle lands on `"x"` in `(T.Mk "x")`. When the payload
                        // is a RECORD whose field-set differs, add the structural field-diff tail (which
                        // fields are missing/extra, or which field's type clashes) so the reader is not left
                        // to diff two whole record renders.
                        let delta = structural_delta_hint(&sparam, &sat).unwrap_or_default();
                        let mut reject = Reject::coded(
                            Code::Malformed,
                            format!(
                                "a variant constructor's payload has declared type {}, but a value of \
                                 type {} was applied{delta}",
                                sparam.render_name(),
                                sat.render_name()
                            ),
                        )
                        .at(arg);
                        // Prefer a numeric/text coercion fix; else a record-field RENAME (a misspelled
                        // field key in the supplied record literal — the construction twin of the
                        // member-access typo fix).
                        if let Some(fix) = numeric_text_coercion_fix(db, &sparam, &sat, arg)
                            .or_else(|| record_field_typo_fix(db, &sparam, &sat, arg))
                        {
                            reject = reject.with_fix(fix);
                        }
                        out.push(reject);
                    } else if let (Some(da), Some(db_)) =
                        (nominal_or_sum_decl(&sparam), nominal_or_sum_decl(&sat))
                        && da != db_
                        && same_sum_shape(db, da, db_)
                    {
                        // Two DISTINCT NOMINAL types (both sums) of the SAME STRUCTURAL SHAPE unified in the
                        // same position — the classic is comparing them, `(= (A.Mk 1) (B.Mk 1))` for
                        // same-shape sums `A`/`B` (the `=` operator is `∀a. a → a → Bool`, so its two
                        // operands unify against one `a` and the nominal difference conflicts here). A
                        // nominal type's identity is its declaration, so this is a comparison ACROSS THE
                        // NOMINAL BOUNDARY — `CDZ0202`, ill-typed, NOT the `false` an untagged structural
                        // comparison would give (`type-system.md` §Nominal Types Are Not Comparable Across
                        // Their Boundary). Two sums of DIFFERENT shape (disjoint variant names — `Option` vs
                        // `Result`) are unrelated types, the plain `CDZ0203` mismatch, so the shape guard
                        // keeps that case on the generic code (the corpus draws exactly this line).
                        out.push(Reject::coded(
                            Code::NominalMismatch,
                            format!(
                                "comparing distinct nominal types {} and {} across the nominal boundary",
                                sparam.render_name(),
                                sat.render_name()
                            ),
                        ));
                    } else if let Some(fix) = numeric_text_coercion_fix(db, &sparam, &sat, arg) {
                        // A NUMERIC/TEXT mismatch a total conversion repairs — int→float (`of-int`),
                        // int-width (`.of`), int-valued-float-literal drop (`2.0`→`2`), String→Bytes
                        // (`to-bytes`), or Char→Int64 (`Char.to-int`, the `(+ #\a 1)` case that is
                        // deliberately not caught by the bool/kind-boundary branch so it can flow here for
                        // the wrap). REWORD the raw unify LEAD ("type mismatch: Int64 and Char must be the
                        // same type here, but differ" — an internal-clash read) to the arg-site phrasing the
                        // member-op / effect-op / annotation siblings use, keeping the code + the coercion
                        // fix. (Only when a coercion applies; a non-coercible clash keeps the raw message via
                        // the branches below.) The D33 "same repair wherever the same mismatch surfaces"
                        // lesson, now with the same READABLE lead too.
                        out.push(
                            Reject {
                                message: format!(
                                    "this argument is {}, but a value of type {} is expected here",
                                    sat.render_with_article(),
                                    sparam.render_name()
                                ),
                                ..reject
                            }
                            .with_fix(fix),
                        );
                    } else if let Some(variant) = wrap_variant_for(db, &sparam, &sat) {
                        // A value of the sum's PAYLOAD type where the SUM itself is expected — `5 : Int64`
                        // where `(Option Int64)` is required, in an OPERATOR/ctor argument position (e.g.
                        // `(= o 5)` for `o : (Option Int64)` — the `=` scheme grounds its first operand, so
                        // the second is checked against `(Option Int64)`). The rustc-flagship repair: WRAP
                        // the value in the matching constructor — `(Some 5)`. `wrap_variant_for` picks the
                        // sum's UNIQUE single-payload variant whose payload equals the actual type (general
                        // over any sum, not hard-coded to Option/Some — it reads the expected sum's own
                        // variant set), so the wrap type-checks in one shot. HEURISTIC: wrapping resolves the
                        // mismatch, but WHICH variant the author meant is a guess when a value could be the
                        // payload of more than one construction — an ambiguous match returns None, so we
                        // only suggest when the choice is forced. REWORD the raw unify lead ("type mismatch:
                        // (Option Int64) and Int64 must be the same type here, but differ" — an internal-
                        // clash read) to the readable arg-site phrasing the coercion / option-payload
                        // siblings use, keeping the code + the wrap fix.
                        out.push(
                            Reject {
                                message: format!(
                                    "this argument is {}, but a value of type {} is expected here",
                                    sat.render_with_article(),
                                    sparam.render_name()
                                ),
                                ..reject
                            }
                            .with_fix(Fix::wrap_heuristic(
                                arg,
                                format!("({variant} "),
                                ")",
                                format!("wrap the value in `{variant}`"),
                            )),
                        );
                    } else if let Some(hint) = option_payload_mismatch_hint(&sparam, &sat) {
                        // The INVERSE of the wrap-variant case: the ARGUMENT is `(Option T)` where the
                        // param wants the bare payload `T` — a fallible read (`(+ ((. List at) xs i) 1)`)
                        // used directly. No total unwrap exists (an Option is matched, not unwrapped), so no
                        // fix; append the actionable "match it" hint to the unify message so the diagnostic
                        // says how to fix it, not just that two types differ.
                        out.push(Reject {
                            message: format!("{}{hint}", reject.message),
                            ..reject
                        });
                    } else if crate::eval::effect_op_of(db, head).is_some() {
                        // A wrong-type argument PERFORMED to an effect operation — `(E.put true)` where
                        // `put`'s declared type is `(-> Int64 Unit)`. The head is the op VALUE (a `(. E put)`
                        // member access), so the generic unify mismatch ("Int64 and Bool must be the same
                        // type here") does not say WHAT the author got wrong — it reads like an internal
                        // clash, not "you performed `put` with the wrong argument". Name the operation and
                        // its declared argument type, the perform-site analogue of the variant-ctor payload
                        // message above (which does the same for `(T.Mk "x")`). The op name is the head's
                        // member key `(. E put)` → `put`; fall back to "this operation" if unreadable.
                        let op = db
                            .ast
                            .as_form(head, ".")
                            .and_then(|t| t.get(1).copied())
                            .and_then(|k| db.ast.as_name(k))
                            .map(|n| format!("`{n}`"))
                            .unwrap_or_else(|| "this operation".to_string());
                        let mut reject = Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "operation {op} expects an argument of type {}, but a value of type {} \
                                 was performed",
                                sparam.render_name(),
                                sat.render_name()
                            ),
                        );
                        if let Some(fix) = numeric_text_coercion_fix(db, &sparam, &sat, arg) {
                            reject = reject.with_fix(fix);
                        }
                        out.push(reject);
                    } else if let Some((module, member)) = member_op_head_name(db, head) {
                        // A wrong-type argument to a NAMED PRELUDE MEMBER OP — `(List.push xs true)`,
                        // `(Int64.of s)`, `(String.slice s true 2)`. The head is a `(. Module member)`
                        // access, so the generic unify mismatch ("Int64 and Bool must be the same type
                        // here") reads like an internal clash — it does not say WHICH operation wanted
                        // WHAT. Name the operation and its expected argument type, the prelude-op analogue
                        // of the sibling's effect-op perform message + the variant-ctor payload message.
                        // (Only fires when the head is a `(. Module member)` with both parts names — a bare
                        // operator `+`/`<` reads fine already and takes the earlier float/numeric branches;
                        // a user-fn call takes the annotation-mismatch path, never this generic else.) The
                        // coercion fix (`(Int64.of …)` etc.) still rides along when one applies.
                        let mut reject = Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "`{module}.{member}` expects an argument of type {}, but a value of \
                                 type {} was given",
                                sparam.render_name(),
                                sat.render_name()
                            ),
                        );
                        if let Some(fix) = numeric_text_coercion_fix(db, &sparam, &sat, arg) {
                            reject = reject.with_fix(fix);
                        }
                        out.push(reject);
                    } else if let Some(hint) = fn_not_applied_hint(&sparam, &sat) {
                        // The ARGUMENT is an UNAPPLIED function where a non-function param is wanted — a
                        // partial application `(+ (h 1) 2)` (h takes 2, applied to 1) passed to `+`, which
                        // wants an Int64. This falls through the bare-operator path to the generic unify
                        // "type mismatch: Int64 and (-> Int64 Int64) must be the same type here, but differ"
                        // — an INTERNAL-CLASS read that never says the argument is simply a function you
                        // forgot to finish calling. REPLACE the raw unify LEAD with the arg-site phrasing the
                        // annotation / member-op / effect-op siblings use ("this argument is …, but a value
                        // of type … is expected here"), then append the "apply N more argument(s)" hint
                        // (`fn_not_applied_hint`). No mechanical fix (which values were meant is unknown), so
                        // the hint is a tail only. Keeps the reject's CODE.
                        out.push(Reject {
                            message: format!(
                                "this argument is a function value, but a value of type {} is expected \
                                 here{hint}",
                                sparam.render_name()
                            ),
                            ..reject
                        });
                    } else {
                        out.push(reject);
                    }
                }
                cur = *result;
            }
            // The head's arrow ran out but there are still arguments — the applied value is not a
            // function. TWO distinct situations, distinguished by how many arguments were already
            // consumed:
            //   • `arg_index > 0` — the head DID accept its parameters (each earlier arg unified against
            //     an `Ty::Fn` param), then a further argument is applied to the fully-consumed result. That
            //     is OVER-APPLICATION — e.g. `(+ 1 2 3)`, where `+` consumes 2 then `3` over-applies the
            //     `Int64` sum. Report it with the over-application taxonomy (the same phrasing as an
            //     over-applied lambda/constructor), carrying `OVER_APPLICATION_MARKER` so `dedup_faults`
            //     drops the emit-path decline. Naming the arity turns the opaque "cannot apply a value of
            //     type Int64" into "applied 3 arguments to a function of arity 2".
            //   • `arg_index == 0` — the head is a scheme-typed value that was NEVER a function, applied to
            //     an argument (`(x 3)` for `x : Int`). That is the genuine not-a-function case; keep the
            //     `NOT_A_FUNCTION_PREFIX` message.
            other => {
                if arg_index > 0 {
                    trace!(target: "rcdzc::infer", head = head.0, arity = arg_index, args = args.len(), "apply: over-applied a scheme-typed head (CDZ0203)");
                    // `arg_index` args were consumed before the arrow ran out, so `args[arg_index]` is the
                    // FIRST surplus — DELETE it (the fixpoint removes each extra in turn). Anchor + fix there.
                    // NAME the operation when the head is a `(. Module member)` — `(List.push xs 1 2)` reads
                    // "`List.push` takes 2 arguments, but 3 were given" instead of the anonymous "a function
                    // of arity 2", the over-application companion of the M95 wrong-type-arg member-op message.
                    // A bare VARIANT CONSTRUCTOR (`(Mk 1 2 3)`) reads as well as the member-access spelling
                    // `(. P Mk)` (handled by `member_op_head_name`) — name it too. `name_of` prefers the
                    // dotted member spelling, then falls back to a bare ctor name; either yields the "`X`
                    // takes N arguments, but M were given" phrasing (so `MEMBER_OVER_APPLICATION_MARKER`
                    // still deduplicates the emit-path decline). An ordinary over-applied function/operator
                    // keeps the anonymous "function of arity N".
                    let name_of = member_op_head_name(db, head)
                        .map(|(m, k)| format!("{m}.{k}"))
                        .or_else(|| variant_ctor_head_name(db, head));
                    let message = match name_of {
                        Some(name) => format!(
                            "`{name}` takes {arg_index} argument{}, but {} {} given",
                            if arg_index == 1 { "" } else { "s" },
                            args.len(),
                            if args.len() == 1 { "was" } else { "were" },
                        ),
                        None => format!(
                            "applied {} arguments to a function of arity {} — it is not a function after \
                             its arguments are consumed",
                            args.len(),
                            arg_index
                        ),
                    };
                    let mut reject = Reject::coded(Code::TypeMismatch, message);
                    if let Some(&surplus) = args.get(arg_index) {
                        reject = reject
                            .at(surplus)
                            .with_fix(crate::diag::Fix::delete_heuristic(
                                surplus,
                                "remove the extra argument",
                            ));
                    }
                    out.push(reject);
                } else {
                    trace!(target: "rcdzc::infer", head = head.0, ty = %other.render_name(), "apply: applied a non-function (type fault)");
                    // A head that DENOTES A TYPE applied in call position (`(Color 5)`, `(Option 5)`,
                    // `(Int64 5)`) — name the CATEGORY, mirroring the M75 effect/type message (and the M74
                    // export-a-type message): "`Color` is a type, not a function". A type belongs in an
                    // ANNOTATION `(: value Color)`, not a call — say so. The discriminator is GENERIC:
                    // `typeval_of(head)` succeeds iff the head reduces to a type-value (a user `(type …)`
                    // name, a prelude type like `Int64`/`Option`), so no hard-coded name list is needed
                    // (the no-keys-outside-the-prelude rule). Names the head's SOURCE spelling. Any other
                    // scheme-typed non-function head keeps the type-named message (the type IS the fact).
                    // Read the head's source name first (releasing the `&db.ast` borrow), THEN check it
                    // denotes a type via `typeval_of` (which needs `&mut db`) — a name AND a type-value.
                    let head_name = db.ast.as_name(head).map(str::to_string);
                    let head_typeval = crate::eval::typeval_of(db, head);
                    let type_name = head_name.filter(|_| head_typeval.is_some());
                    // A type applied to arguments where a function was expected. Distinguish the common
                    // sum-ANNOTATION slip — a MONOMORPHIC type given type arguments (`(: t (T Int64))` where
                    // `(type T (Leaf Int64) …)` takes no parameters) — from a type used in value call
                    // position (`(T 5)`). A type-value that is a sum/nominal with ZERO declared parameters,
                    // applied to args, is over-applied: say it takes no type parameters and spell the fix
                    // (`T`, not `(T Int64)`), rather than the generic "not in call position" (which reads
                    // wrong when the type IS in annotation position, just over-applied). Read the declared
                    // param count off the type-value's decl (a GENERIC type given args is handled correctly
                    // elsewhere; only a nullary-param type reaches here with args).
                    let mono_type_params = match &head_typeval {
                        Some(crate::ty::Ty::Sum { decl, .. })
                        | Some(crate::ty::Ty::Nominal { decl, .. }) => {
                            db.type_decl_by_occ(*decl).map(|d| d.params.len())
                        }
                        _ => None,
                    };
                    // A monomorphic type applied to arguments carries the concrete fix the message names:
                    // REPLACE the whole `(T …)` application with the bare type `T` (strip the spurious
                    // arguments). `app` is the application node; the head's source name is the replacement.
                    // Only for the `Some(0)` case — a type given args where it takes none, whose repair is
                    // unambiguous ("write `T`"). The generic call-position case (`(T 5)`) has no single edit
                    // (delete args? annotate? — the author's intent is unclear), so it stays message-only.
                    let (message, mono_fix) = match (&type_name, mono_type_params) {
                        // A monomorphic type (0 declared params) applied to arguments — most often the
                        // sum-annotation slip `(: t (T Int64))` where `T` takes no parameters. Name the
                        // exact fix (`T`, not `(T …)`) without asserting a context, since the same
                        // over-application can appear in value call position (`(T 5)`) too.
                        (Some(name), Some(0)) => (
                            format!(
                                "`{name}` is a type that takes no type parameters — write `{name}`, not \
                                 `({name} …)` (a type belongs in an annotation `(: value {name})`, not \
                                 applied to arguments)"
                            ),
                            // HEURISTIC (not verified): replacing `(T …)` with the bare type `T` clears the
                            // fault in the COMMON case — an ANNOTATION slip `(: t (T Int64))`, where `T` is
                            // exactly what belongs. But the SAME over-application can appear in VALUE call
                            // position (`(Color 5)`), where a bare type name is still not a value → the
                            // replace trades this error for a clearer "a type is not a value" one (no
                            // miscompile). Since `check_application` runs on the reduced value graph and
                            // cannot cheaply tell annotation from value position here, the fix stays
                            // heuristic — right in the common case, harmless (error→clearer-error) otherwise.
                            Some(crate::diag::Fix::replace_heuristic(app, name.clone())),
                        ),
                        // A GENERIC type (≥1 declared parameter) applied with a NON-TYPE argument —
                        // `(Option 5)`, `(Box 5)`. Its type-argument position wants a TYPE, not a value; the
                        // generic "not a function" reads as if `Option` were being called, missing that this
                        // is a type constructor whose argument is wrong. Name that — the sum twin of List/Set's
                        // "the element type must be a type" (`non_type_argument_message`, which only covers the
                        // prim ctors). No forced fix (the author supplies the intended type argument).
                        (Some(name), Some(p)) if p >= 1 => (
                            format!(
                                "`{name}` is a type constructor — its type argument must be a type, but a \
                                 value appears here (write `({name} <Type>)`, e.g. `({name} Int64)`)"
                            ),
                            None,
                        ),
                        (Some(name), _) => (
                            format!(
                                "`{name}` is a type, not a function — a type appears in an annotation \
                                 `(: value {name})`, not in call position"
                            ),
                            None,
                        ),
                        (None, _) => (
                            format!(
                                "{} {}",
                                crate::diag::NOT_A_FUNCTION_PREFIX,
                                other.render_name()
                            ),
                            None,
                        ),
                    };
                    let mut reject = Reject::coded(Code::TypeMismatch, message);
                    if let Some(fix) = mono_fix {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
                return;
            }
        }
    }
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
    crate::diag::suggest::nearest(name, fields.keys().map(|k| k.name.as_str()))
}

fn nominal_or_sum_decl(ty: &Ty) -> Option<StructId> {
    match ty {
        Ty::Sum { decl, .. } | Ty::Nominal { decl, .. } => Some(*decl),
        _ => None,
    }
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
pub(crate) fn member_category(db: &Db, operand: StructId, key: &str) -> (String, &'static str) {
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
    ("record".to_string(), "field")
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
        let fields = crate::eval::record_field_names(db, operand);
        let suggestion = crate::diag::suggest::nearest(&key.name, &fields);
        let hint = crate::diag::suggest::did_you_mean(&key.name, &fields, 3);
        if let Some(k) = &cache_key {
            db.no_field_suggestion
                .insert(k.clone(), (suggestion.clone(), hint.clone()));
        }
        (suggestion, hint)
    };
    // The key occurrence is the second child of the `(. operand key)` form — the node the fix rewrites.
    let key_occ = db.ast.as_form(member, ".").and_then(|t| t.get(1).copied());
    // NAME the operand's real category (effect / module / type / record) instead of always "record" —
    // `(. E emt)` reads "effect `E` has no operation `emt`", `(. List nonesuch)` "the `List` module has no
    // member `nonesuch`". The `has no <word> \`key\`` shape stays the shared dedup invariant.
    let (subject, member_word) = member_category(db, operand, &key.name);
    let reject = Reject::coded(
        Code::Malformed,
        format!("{subject} has no {member_word} `{}`{hint}", key.name),
    );
    match (suggestion, key_occ) {
        (Some(field), Some(occ)) => reject.with_fix(Fix::replace_heuristic(occ, field)),
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
    if name == "eval"
        && db
            .parent_of(id)
            .and_then(|p| db.ast.as_form(p, "eval").map(|t| (t, p)))
            .is_some_and(|(_, p)| {
                matches!(db.ast.get(p), crate::ast::Struct::List(kids) if kids.first() == Some(&id))
            })
    {
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
                                        ty.render_name()
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

fn collect_node(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
    // A `do` SEQUENCING block resolves to a `Ref` to its LAST form (its value), so the `Resolved::Ref`
    // arm below descends only into that. But the INTERMEDIATE forms are still evaluated (their value
    // discarded), so an ill-typed or provably-trapping intermediate must be caught — descend into EVERY
    // form here (the last one is also covered by the `Ref` descent, harmlessly). Read off the raw AST
    // head since the resolved form has already collapsed to the last form's `Ref`.
    if db.ast.head_name(id) == Some("do") && db.ast.as_form(id, "do").is_some() {
        // A `do` block that is ITSELF malformed — EMPTY (`(do)` has no value form) or ending in a
        // DECLARATION (`(do (def x 5))` yields nothing) — resolves to a coded `Poison` (`resolve_do`).
        // That is a well-formedness fault of the `do` node, so it must surface wherever `collect` visits
        // it — including a def BODY that IS the malformed `(do)`. Without this, `(def (g) (do))` PASSED
        // `cdz check` (the resolve-poison reached only the emit-path lowering walk, which runs on
        // nullary-exported bodies alone) while `compile` rejected it — the same "check misses a
        // resolve/lower-only reject on a param/nullary body" hole M81's `match_pattern_fault` closed for
        // pattern faults. Surface the coded poison here (anchored at the `do` node), then STILL descend
        // the forms below so an ill-typed intermediate is also reported; `dedup_faults` collapses this
        // against any copy the emit walk surfaces at the same node. Compute the poison BEFORE re-borrowing
        // `db.ast` for the forms list (`resolved_of` needs `&mut db`).
        if let Resolved::Poison(r) = resolved_of(db, id)
            && r.code.is_some()
        {
            let mut r = r;
            r.set_origin_if_absent(id);
            out.push(r);
        }
        let forms: Vec<StructId> = db.ast.as_form(id, "do").unwrap_or(&[]).to_vec();
        for f in forms {
            // A do-local `(def …)` is a DECLARATION, not a value expression — resolving it as one would
            // decline. A VALUE declaration `(def x V)` (or nullary `(def (x) V)`) has its value `V`
            // type-checked eagerly, exactly like a `let` binding's value (a value binding is a fault
            // whether or not the name is used). A FUNCTION declaration `(def (f p…) BODY)` is a lambda:
            // its body is checked on CALL (β-reduced at a reference), like a `let`-bound lambda, so it is
            // not descended into here.
            if db.ast.head_name(f) == Some("def") {
                if let Some(value) = crate::resolve::do_value_def_value(db, f) {
                    collect(db, value, out);
                }
                continue;
            }
            // A do-local `(type …)` / `(effect …)` / `(module …)` is a DECLARATION, not a value
            // expression — its record is synthesized at load (`db::collect_nested_decls` / the module
            // scan) and its names resolve through the ordinary decl paths. There is nothing to type-check
            // as a value (resolving the form as one would decline, e.g. "`module` is not an expression
            // here"), so skip it, like a `def`. (A module member's own body IS checked on demand through
            // the member access that reaches it, exactly like a def called through its name.)
            if matches!(
                db.ast.head_name(f),
                Some("type") | Some("effect") | Some("module")
            ) {
                continue;
            }
            collect(db, f, out);
        }
        return;
    }
    // A `quote`/`quasiquote`/`unquote`/`unquote-splicing` form resolves to a DECLINE (its `Ast` value is
    // not yet built), so the ordinary `Resolved` arms below stop at that decline and never see the body.
    // But a SYNTAX defect inside the quoted structure is UNCONDITIONAL well-formedness (like an unbound
    // name in an untaken branch): an `unquote`/`,@` OUTSIDE a quasiquote (CDZ0003) or a wrong-arity one
    // (CDZ0201) must be reported even though the enclosing quote/quasiquote itself declines. So descend
    // into the RAW body subtree here, collecting each nested `(unquote …)`/`(unquote-splicing …)`'s coded
    // reject (its own `collect` → the `Poison` arm reports the coded syntax/arity rejection). Walk the raw
    // AST children (the resolved form collapsed to a decline, carrying no children).
    if matches!(
        db.ast.head_name(id),
        Some("quote" | "quasiquote" | "unquote" | "unquote-splicing")
    ) {
        collect_quote_body_syntax(db, id, out);
        return;
    }
    // A bare integer literal defaulted to a NARROW fixed-width type by a `(pragma default-integer <T>)`
    // must satisfy the SAME literal-fit range check an explicit `(: v T)` annotation runs — else the pragma
    // silently admits an out-of-range value into a narrow-typed slot (`(pragma default-integer Int8) (def
    // (x) 300)` gave `x : Int8 = 300`, a soundness hole the explicit `(: 300 Int8)` correctly rejects
    // CDZ0302). The pragma records each such literal → its `<T>` type-expression occurrence in
    // `default_int_literals` — the exact `(value, ty_expr)` pair `literal_width_fault` checks — so reuse it,
    // giving the pragma default the annotation path's fit-check. A WIDENING default (Int64/BigInt) never
    // faults (every literal fits); only a narrowing one (Int8/UInt8/…) rejects an out-of-range literal.
    if let Some(&ty_expr) = db.default_int_literals.get(&id)
        && let Some(reject) = literal_width_fault(db, id, ty_expr)
    {
        out.push(reject);
    }
    match resolved_of(db, id) {
        Resolved::If { cond, then_, else_ } => {
            let cond_ty = type_of(db, cond);
            if !cond_ty.agrees_with(&Ty::Bool) {
                trace!(target: "rcdzc::infer", node = id.0, cond_ty = %cond_ty.render_name(), "fault: if condition not Bool (CDZ0203)");
                // Anchor at the CONDITION expression, not the whole `(if …)` — the squiggle then lands on
                // the non-Bool value (`5` in `(if 5 …)`), the actual culprit. (Without `.at`, `collect`
                // stamps the coarse `if`-node default.) The branch-mismatch reject below stays at the `if`
                // node: it concerns the RELATIONSHIP between both branches, not one sub-node.
                out.push(
                    Reject::coded(
                        Code::TypeMismatch,
                        format!("if condition must be Bool, found {}", cond_ty.render_name()),
                    )
                    .at(cond),
                );
            }
            // Both branches are type-checked HERE (each `type_of`'d, and they must agree) EVEN THOUGH only
            // the condition-selected branch runs — so an unevaluated branch can never carry a deferred type
            // error, exactly as a boolean connective's shielded operand is still checked.
            //= spec/capabilities/core-semantics.md#conditionals-evaluate-one-branch
            //# Every branch of a conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated branch cannot carry a deferred error.
            let then_ty = type_of(db, then_);
            let else_ty = type_of(db, else_);
            if !then_ty.agrees_with(&else_ty) {
                // Two DISTINCT NUMERIC types (an `Int64` branch and a `Float64` branch) are the no-silent-
                // promotion rule (numeric-model.md #Numeric Types Do Not Silently Promote) — a MALFORMED
                // program (CDZ0201), NOT the structural type mismatch (CDZ0203) a cross-KIND disagreement
                // (Int vs Bool, a compound vs a scalar, two tuples of different arity) is. So an `if` whose
                // branches are both numeric but of different numeric type is CDZ0201; every other branch
                // disagreement stays CDZ0203. (02-binding "a conditional with integer and floating-point
                // branches is a type error" wants CDZ0201; the tuple-arity/kind cases want CDZ0203.)
                let both_numeric = matches!(then_ty, Ty::Int(_) | Ty::Float(_))
                    && matches!(else_ty, Ty::Int(_) | Ty::Float(_));
                let code = if both_numeric {
                    Code::Malformed
                } else {
                    Code::TypeMismatch
                };
                trace!(target: "rcdzc::infer", node = id.0, then_ty = %then_ty.render_name(), else_ty = %else_ty.render_name(), ?code, "fault: if branches differ");
                let delta = peer_type_delta_hint(&then_ty, &else_ty).unwrap_or_default();
                let mut reject = Reject::coded(
                    code,
                    format!(
                        "if branches differ: {} vs {}{delta}",
                        then_ty.render_name(),
                        else_ty.render_name()
                    ),
                );
                // An INT-LITERAL-vs-FLOAT branch clash has the same one-shot repair the list-element and
                // annotation sites give (`(: 3 Float64)` → `3.0`): rewrite the integer-literal branch as a
                // float literal so both branches unify at the float type. The literal may be EITHER branch
                // (`(if b 1 2.0)`, fix `then_`; `(if b 1.0 2)`, fix `else_`); offer on whichever is the int
                // literal (a computed integer branch yields no fix).
                if let Some(fix) = float_literal_retype_fix(db, then_, &then_ty, &else_ty)
                    .or_else(|| float_literal_retype_fix(db, else_, &else_ty, &then_ty))
                    // A record-field TYPO in one branch vs the other — rename the misspelled key on whichever
                    // branch carries it (both orderings, the peer-join twin of the list-element record typo).
                    .or_else(|| record_field_typo_fix(db, &then_ty, &else_ty, else_))
                    .or_else(|| record_field_typo_fix(db, &else_ty, &then_ty, then_))
                {
                    reject = reject.with_fix(fix);
                }
                out.push(reject);
            }
            collect(db, cond, out);
            collect(db, then_, out);
            collect(db, else_, out);
        }
        // A boolean connective — each operand must be Bool (core-semantics.md §Boolean Connectives
        // Short-Circuit: each operand is type-checked as a Bool whether or not it is evaluated). A non-Bool
        // operand is a MALFORMED program (CDZ0201) — the operand simply is not the required type, like a
        // binary operator's operand, NOT the structural-shape mismatch (CDZ0203) a cross-kind disagreement
        // is (02-binding "a boolean connective with a non-boolean operand is a type error" wants CDZ0201).
        // Then descend for each operand's own faults. Both operands are checked here EVEN THOUGH the right
        // is short-circuited at run time, so an unevaluated operand can never carry a deferred type error —
        // exactly as every branch of a conditional is type-checked whether or not it is taken.
        //= spec/capabilities/core-semantics.md#boolean-connectives-short-circuit
        //# Each operand of a boolean connective MUST be type-checked as a boolean whether or not it is evaluated, so that an unevaluated operand cannot carry a deferred error, exactly as every branch of a conditional is type-checked.
        Resolved::And { lhs, rhs, is_and } => {
            let op = if is_and { "and" } else { "or" };
            for &operand in &[lhs, rhs] {
                let t = type_of(db, operand);
                if !t.agrees_with(&Ty::Bool) {
                    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "fault: connective operand not Bool (CDZ0201)");
                    // Anchor at the offending OPERAND, not the whole `(and …)`/`(or …)`.
                    out.push(
                        Reject::coded(
                            Code::Malformed,
                            format!("`{op}` operand must be Bool, found {}", t.render_name()),
                        )
                        .at(operand),
                    );
                }
                collect(db, operand, out);
            }
        }
        Resolved::Not { operand } => {
            let t = type_of(db, operand);
            if !t.agrees_with(&Ty::Bool) {
                trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(), "fault: not operand not Bool (CDZ0201)");
                // Anchor at the offending OPERAND, not the whole `(not …)`.
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!("`not` operand must be Bool, found {}", t.render_name()),
                    )
                    .at(operand),
                );
            }
            collect(db, operand, out);
        }
        // Member access: the operand must be a record, and it must have the named field. Both faults
        // are compile-time rejections (a non-record operand, or an absent field, has no defined result —
        // never an unspecified value or a runtime trap). A built-in module is CLOSED the same way a user
        // record is: it carries every field it will ever have, and an unrealized field DECLINES when
        // projected rather than being absent (`prelude.rs` — there is no open-module rule). (Projection
        // of a PRESENT field to the field's value is realized at the `type_of`/`lower` Member arm — the
        // `Member::Field` case here is the well-formed no-fault path.)
        //= spec/capabilities/core-semantics.md#member-access-projects-a-record-field
        //# Member access applied to a value that is not a record MUST be rejected at compile time with the machine-readable code for a type error rather than produce an unspecified value or a runtime trap.
        //= spec/capabilities/core-semantics.md#member-access-projects-a-record-field
        //# Member access naming a field the record does not contain MUST be rejected at compile time with the machine-readable code for a required field that is absent rather than produce an unspecified value or a runtime trap, so that a projection cannot name a field the operand's type never held.
        Resolved::Member { operand, key } => {
            // Project via the evaluator (reduces refs / a ctor-built module), so a missing field on a
            // built module is caught too. A poison operand reports its OWN fault (via the descent
            // below), so we don't add a redundant "not a record" for it.
            let operand_is_poison = matches!(resolved_of(db, operand), Resolved::Poison(_));
            match crate::eval::member_value(db, operand, &key) {
                crate::eval::Member::Field(_) => {}
                crate::eval::Member::NoField => {
                    // The evaluator reduced the operand to a CONCRETE record lacking `key`. Usually that IS
                    // the fault — but when the operand's TYPE (annotation-wins, see `Resolved::Ref`) DOES
                    // carry `key`, the value/type disagreement is an annotated let-binder whose initializer
                    // contradicts its annotation (`(: r (Record (foo …))) (record (fooo …))`) — already
                    // reported once at the binder (CDZ0203). Suppress the member fault so the two-diagnostic
                    // CASCADE (rename the value's field vs rename the body's use) does not fire; a body use
                    // types against what the author DECLARED, so under the declared type the access is valid.
                    let declared_has_key = matches!(
                        type_of(db, operand).strip_nominal(),
                        Ty::Record(fields) if fields.contains_key(&key)
                    );
                    if !declared_has_key {
                        trace!(target: "rcdzc::infer", node = id.0, key = %key.name, "fault: record has no such field (CDZ0201)");
                        out.push(no_field_reject(db, id, operand, &key))
                    }
                }
                // The operand did not reduce to a compile-time-visible record. Before rejecting, check
                // its TYPE: a RUNTIME record (a call result, an `if` selection) carries a record type,
                // and the access is well-formed iff that type has the field — the field read lowers to
                // an `arr-get` at the field's sorted index (mirrors a tuple projection on a runtime
                // tuple). Only a genuine non-record operand, or a record type lacking the field, faults.
                crate::eval::Member::NotRecord if !operand_is_poison => {
                    // A NOMINAL newtype over a record is erased to that record at run time, so member
                    // access sees through the tag (`strip_nominal`): the access is well-formed iff the
                    // inner record type has the field. A nominal over a NON-record stays a non-record
                    // fault (the `other` arm).
                    match type_of(db, operand).strip_nominal() {
                        Ty::Record(fields) if fields.contains_key(&key) => {}
                        Ty::Record(_) => {
                            trace!(target: "rcdzc::infer", node = id.0, key = %key.name, "fault: runtime record has no such field (CDZ0201)");
                            out.push(no_field_reject(db, id, operand, &key))
                        }
                        // An UNCONSTRAINED operand (`Any`) — a bare (unannotated) parameter of a
                        // non-recursive def, whose type is not known until the def inlines at a call
                        // site with a concrete argument. `Any` means "no constraint" (it agrees with
                        // everything), so a field read on it is NOT a fault HERE — the real check runs
                        // on the reduced body at the call site (where the argument's record type flows
                        // in). Rejecting here spuriously fails a well-typed program `(def (get-x r) (. r
                        // x))`, exactly as arithmetic on an `Any` param (`(+ r 1)`) does not fault.
                        Ty::Any => {}
                        // A TUPLE accessed by NAME — `(. t x)` on a `(Tuple …)`. A tuple IS a member-access
                        // operand, just by POSITION not name (`type-system.md` §A Tuple Is Accessed By
                        // Position): the generic "requires a record" reads as a dead end when the real fix
                        // is a numeric index. Name the tuple's arity + spell the index form, so the reader
                        // reaches for `(. t 0)` rather than thinking a tuple is unindexable.
                        Ty::Tuple(elems) => {
                            trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: named member access on a tuple (CDZ0201)");
                            let last = elems.len().saturating_sub(1);
                            out.push(Reject::coded(
                                Code::Malformed,
                                format!(
                                    "a tuple is accessed by position, not by name `{}` — use a numeric \
                                     index `(. <tuple> N)` with N in 0..={last} (this tuple has {} \
                                     element{})",
                                    key.name,
                                    elems.len(),
                                    if elems.len() == 1 { "" } else { "s" },
                                ),
                            ))
                        }
                        // A COLLECTION or TEXT value accessed by NAME — `(. xs foo)` on a `(List …)`, a
                        // `(Map …)`, `(Set …)`, `String`, `Bytes`. These are NOT field-bearing records: a
                        // field NAME reads nothing off them. Their operations live on the type MODULE and
                        // take the value as the FIRST argument — `(. List at)`/`Map.lookup`/`Set.contains`/
                        // `String.len` — so the generic "requires a record" reads as a dead end when the fix
                        // is a module operation. Name the module + the value-first call form so the reader
                        // reaches for `((. List at) xs …)` (rustc's "method not found; the operations are on
                        // the type"). Only for a value with such a module; other non-records keep the plain
                        // message.
                        other => {
                            // Redirect a NAMED member access on a non-record to the way its kind IS used,
                            // instead of the dead-end "requires a record":
                            //  • COLLECTION/TEXT (`(List …)`/`(Map …)`/`(Set …)`/`String`/`Bytes`) — not a
                            //    field read; its operations live on the type MODULE, value-first (`(. List
                            //    at) xs …`).
                            //  • SUM (`(Option …)`, a user sum) — its payload is reached by MATCHING each
                            //    variant, not by field access; spell a `(match <value> …)` template.
                            //  • any other non-record (a scalar) has no such route → the plain message.
                            let module = collection_or_text_module(other);
                            let match_tmpl = if module.is_none() {
                                sum_match_hint(db, other)
                            } else {
                                None
                            };
                            let ty_name = other.render_name();
                            let reject = if let Some(module) = module {
                                trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: named member access on a collection/text value (CDZ0201)");
                                Reject::coded(
                                    Code::Malformed,
                                    format!(
                                        "a {ty_name} value has no field `{}` — its operations live on the \
                                         `{module}` module and take the value as the first argument, e.g. \
                                         `((. {module} <op>) <value> …)`",
                                        key.name,
                                    ),
                                )
                            } else if let Some(tmpl) = match_tmpl {
                                trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: named member access on a sum value (CDZ0201)");
                                Reject::coded(
                                    Code::Malformed,
                                    format!(
                                        "a {ty_name} value has no field `{}` — a sum's payload is reached \
                                         by matching its variants, not by field access, e.g. `{tmpl}`",
                                        key.name,
                                    ),
                                )
                            } else {
                                trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: member access on a non-record (CDZ0201)");
                                Reject::coded(
                                    Code::Malformed,
                                    format!("member access requires a record, found {ty_name}"),
                                )
                            };
                            out.push(reject)
                        }
                    }
                }
                crate::eval::Member::NotRecord => {}
            }
            collect(db, operand, out);
        }
        // Tuple projection `(. t N)`: the operand must be a TUPLE, and `N` must be within its static
        // arity. Both faults are CDZ0201, decided at COMPILE TIME — an out-of-arity index is NOT a
        // runtime trap (`type-system.md` §A Tuple Is Split At A Position Into A Prefix And A Suffix;
        // [[an-out-of-arity-tuple-index-traps]]). A poison operand reports its own fault via the descent.
        Resolved::Proj { operand, index } => {
            let operand_is_poison = matches!(resolved_of(db, operand), Resolved::Poison(_));
            // NOT collapsible into a guarded arm (`clippy::collapsible_match`): an IN-RANGE tuple
            // projection must match `Ty::Tuple` and produce NO fault. Guarding the arm with `if index
            // >= len` would let the in-range case fall through to the `_ if !operand_is_poison` arm
            // below, which would spuriously report "requires a tuple, found (Tuple …)".
            #[allow(clippy::collapsible_match)]
            match type_of(db, operand) {
                Ty::Tuple(elems) => {
                    if index >= elems.len() {
                        trace!(target: "rcdzc::infer", node = id.0, index, arity = elems.len(), "fault: tuple index out of arity (CDZ0201)");
                        out.push(Reject::coded(
                            Code::Malformed,
                            format!(
                                "tuple index {index} is out of range for a {}-element tuple",
                                elems.len()
                            ),
                        ));
                    }
                }
                // An UNCONSTRAINED operand (`Any`) — a bare (unannotated) parameter of a non-recursive
                // def, whose tuple type is not known until the def inlines at a call site with a
                // concrete argument. `Any` means "no constraint", so a projection on it is NOT a fault
                // HERE — the real check runs on the reduced body at the call site (where the argument's
                // tuple type flows in). Rejecting here spuriously fails a well-typed `(def (fst t) (. t
                // 0))`, exactly as arithmetic on an `Any` param does not fault.
                Ty::Any => {}
                // A non-tuple operand (that is not itself a poison) — projecting a position of a
                // non-tuple has no defined result.
                _ if !operand_is_poison => {
                    trace!(target: "rcdzc::infer", node = id.0, operand = operand.0, "fault: tuple projection on a non-tuple (CDZ0201)");
                    out.push(Reject::coded(
                        Code::Malformed,
                        format!(
                            "tuple projection requires a tuple, found {}",
                            type_of(db, operand).render_name()
                        ),
                    ));
                }
                _ => {}
            }
            collect(db, operand, out);
        }
        // A tuple literal: descend into each element for its own faults.
        Resolved::Tuple { elems } => {
            for &e in elems.iter() {
                collect(db, e, out);
            }
        }
        // A list literal is HOMOGENEOUS: every element must share ONE type (collections-and-text.md §A
        // List Is A Homogeneous Sequence). Unify each element's type against the first — a mismatch (a
        // `(list 1 true)`) is CDZ0203, the same class as `if` branches disagreeing. Then descend into
        // each element for its own faults.
        Resolved::List { elems } => {
            let mut subst = Subst::new();
            if let Some(&first) = elems.first() {
                let first_ty = type_of(db, first);
                for &e in elems.iter().skip(1) {
                    let et = type_of(db, e);
                    if let Err(reject) = crate::unify::unify(&mut subst, &first_ty, &et) {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: list elements differ in type (CDZ0203)");
                        out.push(reject);
                    }
                }
            }
            for &e in elems.iter() {
                collect(db, e, out);
            }
        }
        // A map literal is HOMOGENEOUS on BOTH axes: all keys share one type AND all values share one
        // type (collections-and-text.md §A Map Associates Keys With Values — keys of ONE type with values
        // of ONE type). Unify each key against the first key and each value against the first value — a
        // mismatch (`(map (a 1) (b true))` values, or `(map (j 1) (k 2))` with `j`:Int/`k`:Bool keys) is
        // CDZ0201 (a map is ill-typed, not merely a shape mismatch — coded Malformed like the list-of-
        // homogeneous). Independent of the key-is-a-value rule (the keys are ordinary value occurrences).
        // Then descend into each key and value for its own faults. The DUPLICATE-CONSTANT-KEY check is
        // separate (a repeated compile-time-constant key is CDZ0201 — a runtime-computed duplicate is a
        // runtime overwrite, not a reject); it lives in `map_duplicate_const_key` below.
        Resolved::Map { entries } => {
            let mut ksubst = Subst::new();
            let mut vsubst = Subst::new();
            if let Some(&(fk, fv)) = entries.first() {
                let (fkt, fvt) = (type_of(db, fk), type_of(db, fv));
                for &(k, v) in entries.iter().skip(1) {
                    let kt = type_of(db, k);
                    if crate::unify::unify(&mut ksubst, &fkt, &kt).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: map keys differ in type (CDZ0201)");
                        out.push(Reject::coded(
                            Code::Malformed,
                            "a map associates keys of one type (its keys do not share a type)",
                        ));
                    }
                    let vt = type_of(db, v);
                    if crate::unify::unify(&mut vsubst, &fvt, &vt).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: map values differ in type (CDZ0201)");
                        out.push(Reject::coded(
                            Code::Malformed,
                            "a map associates values of one type (its values do not share a type)",
                        ));
                    }
                }
            }
            // A repeated COMPILE-TIME-CONSTANT key makes the association ambiguous → CDZ0201.
            if let Some(reject) = map_duplicate_const_key(db, &entries) {
                trace!(target: "rcdzc::infer", node = id.0, "fault: map has a duplicate constant key (CDZ0201)");
                out.push(reject);
            }
            for &(k, v) in entries.iter() {
                collect(db, k, out);
                collect(db, v, out);
            }
        }
        // A `(bin …)` construction: check STATIC well-formedness (CDZ0220 — decidable from the segment
        // list alone), then descend into each segment's value slot for its own faults. Well-formedness:
        // the running bit-cursor from `bits` segments must close to a whole byte before any byte-aligned
        // segment (int/bytes) and at the end (`binary-syntax`: the whole `bin` is byte-aligned); an
        // unsized `(bytes b)` (no dependent size) is only legal as the FINAL segment. A non-const `bits`
        // width already became a CDZ0220 `Poison` at resolve.
        Resolved::Bin { segs } => {
            let mut bit_cursor: u32 = 0; // open bits since the last byte boundary
            for (i, seg) in segs.iter().enumerate() {
                match &seg.kind {
                    crate::resolved::SegKind::Bits { k } => bit_cursor += k,
                    crate::resolved::SegKind::Int { .. } => {
                        if !bit_cursor.is_multiple_of(8) {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                format!(
                                    "a bin integer segment must start on a byte boundary, but {} of \
                                     bit-fields precede it — {}",
                                    open_bits_phrase(bit_cursor),
                                    bits_to_byte_boundary_hint(bit_cursor),
                                ),
                            ));
                        }
                    }
                    crate::resolved::SegKind::Bytes { size } => {
                        if !bit_cursor.is_multiple_of(8) {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                format!(
                                    "a bin bytes segment must start on a byte boundary, but {} of \
                                     bit-fields precede it — {}",
                                    open_bits_phrase(bit_cursor),
                                    bits_to_byte_boundary_hint(bit_cursor),
                                ),
                            ));
                        }
                        // An UNSIZED bytes segment (splice-all / bind-rest) is legal only as the last
                        // segment — a non-final unsized bytes has no defined boundary.
                        if size.is_none() && i + 1 != segs.len() {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                "a non-final bin bytes segment must have an explicit size (bytes b n)",
                            ));
                        }
                    }
                    // A `utf8` segment is byte-aligned (a string is a byte sequence); it is always sized,
                    // so there is no non-final-unsized fault to check (unlike `bytes`).
                    crate::resolved::SegKind::Utf8 { .. } => {
                        if !bit_cursor.is_multiple_of(8) {
                            out.push(Reject::coded(
                                Code::IllFormedBinary,
                                format!(
                                    "a bin utf8 segment must start on a byte boundary, but {} of \
                                     bit-fields precede it — {}",
                                    open_bits_phrase(bit_cursor),
                                    bits_to_byte_boundary_hint(bit_cursor),
                                ),
                            ));
                        }
                    }
                }
                collect(db, seg.slot, out);
                // A segment's VALUE must match its KIND: an `Int`/`Bits` segment takes an integer, a
                // `Utf8` segment a String, a `Bytes` segment a Bytes. An int segment fed a String
                // (`(bin (u8 "x"))`) was silently accepted at BUILD (the int-segment lowering never
                // type-checked its slot), emitting garbage bytes; a `Bytes` slot mismatch was caught only
                // at compile ("not a Bytes value"). Check the slot's type here (so it surfaces in `cdz
                // check` too), flagging ONLY a DEFINITE wrong kind — an `Any`/`Var` slot (a binder, an
                // unreduced param, or — crucially — a bin PATTERN's binder, which types as the decoded
                // value) is never flagged, so a valid `(bin …)` pattern/construction stays clean.
                let (want, ok): (&str, bool) = match &seg.kind {
                    crate::resolved::SegKind::Int { .. }
                    | crate::resolved::SegKind::Bits { .. } => {
                        ("an integer", !definite_non_int(&type_of(db, seg.slot)))
                    }
                    crate::resolved::SegKind::Utf8 { .. } => (
                        "a String",
                        !definite_conflicts_with(&type_of(db, seg.slot), &Ty::String),
                    ),
                    crate::resolved::SegKind::Bytes { .. } => (
                        "a Bytes",
                        !definite_conflicts_with(&type_of(db, seg.slot), &Ty::Bytes),
                    ),
                };
                if !ok {
                    out.push(
                        Reject::coded(
                            Code::IllFormedBinary,
                            format!(
                                "a bin {} segment takes {want} value, but this value is {}",
                                seg_kind_name(&seg.kind),
                                type_of(db, seg.slot).render_name()
                            ),
                        )
                        .at(seg.slot),
                    );
                }
                match &seg.kind {
                    crate::resolved::SegKind::Bytes { size: Some(n) } => collect(db, *n, out),
                    crate::resolved::SegKind::Utf8 { size } => collect(db, *size, out),
                    _ => {}
                }
            }
            // The whole form must be byte-aligned: any open bits at the end are ill-formed.
            if !bit_cursor.is_multiple_of(8) {
                out.push(Reject::coded(
                    Code::IllFormedBinary,
                    format!(
                        "a bin form's bit-fields must close to a whole number of bytes, but they total \
                         {bit_cursor} bit{} — {}",
                        plural_s(bit_cursor),
                        bits_to_byte_boundary_hint(bit_cursor),
                    ),
                ));
            }
        }
        // Descend into the new binding/aggregate forms for their own faults.
        Resolved::Let { bindings, body } => {
            for (lhs, value) in bindings {
                // A binding LHS may be an irrefutable PATTERN (`(tuple a b)`), not just a bare name. It is
                // a BINDING POSITION — no alternative arm — so it must be irrefutable, and its shape must
                // agree with the value's type. Validate it against the value's type (a refutable pattern →
                // CDZ0210, a wrong-arity/non-tuple shape → CDZ0201, a non-linear pattern → CDZ0102, a
                // not-yet-supported record/single-variant/list pattern → decline). A bare-name LHS is the
                // trivial irrefutable pattern and validates cheaply. (The binder REFERENCES resolve to a
                // `SumPayload` reading the element out of `value`; this only ensures the binding is
                // well-formed so an ill-formed one faults instead of silently miscompiling.)
                let value_ty = type_of(db, value);
                if let Err(mut r) = crate::lower::check_binding_pattern(db, lhs, &value_ty) {
                    // An ANNOTATED let-binder whose annotation disagrees with the init value's type —
                    // `(let (((: x Float64) 3)) …)`, `(let (((: x Int64) n)) …)` with `n : Int8`,
                    // `(let (((: x Bytes) s)) …)` with `s : String`, `(let (((: x (Option Int64)) 5)) …)` —
                    // is the let-binding analogue of the value/param annotation mismatch (D33) and the
                    // argument mismatch: it has the SAME one-shot repair the direct annotation `(: value T)`
                    // offers. Attach it HERE (where the init `value` node is in hand —
                    // `check_binding_pattern` recurses over patterns without it), keyed on the binder's
                    // annotation type. `numeric_text_coercion_fix` bundles the numeric/text coercions
                    // (int-literal→float retype `3`→`3.0`, int→float `of-int`, int-width `.of`, int-valued-
                    // float literal drop `3.0`→`3`, String→Bytes `to-bytes`); the sum-wrap (`5` where an
                    // `(Option Int64)` is annotated → `(Some 5)`) is the separate structural repair. Before
                    // this the let-binder offered ONLY the int-width `.of` case — so `(: x Float64) 3` and
                    // the rest declined a fix the annotation site already gave.
                    if r.code == Some(Code::TypeMismatch)
                        && let Some(ann) = db.ast.as_form(lhs, ":")
                        && ann.len() == 2
                        && let Some(annot_ty) = crate::eval::typeval_of(db, ann[1])
                    {
                        if let Some(fix) =
                            numeric_text_coercion_fix(db, &annot_ty, &value_ty, value)
                        {
                            r = r.with_fix(fix);
                        } else if let Some(variant) = wrap_variant_for(db, &annot_ty, &value_ty) {
                            r = r.with_fix(Fix::wrap_heuristic(
                                value,
                                format!("({variant} "),
                                ")",
                                format!("wrap the value in `{variant}`"),
                            ));
                        } else if let Some(fix) = qty_coercion_fix(&annot_ty, &value_ty, value) {
                            // A bare number bound to a `(Qty …)`-annotated binder → wrap it in `(Qty.of …
                            // <unit>)` with the unit from the annotation, the let-binder twin of the value/
                            // argument `Qty.of` wrap.
                            r = r.with_fix(fix);
                        } else if let Some(fix) =
                            record_field_typo_fix(db, &annot_ty, &value_ty, value)
                        {
                            // A MISSPELLED FIELD in a record literal bound to a `(: r (Record …))` binder —
                            // `(let (((: r (Record (foo Int64))) (record (fooo 1)))) …)` — gets the same
                            // `fooo`→`foo` key rename the argument + value-annotation sites give (a confident
                            // single extra↔missing pairing over a directly-written literal). The let-binder
                            // twin of the record-field-typo rename.
                            r = r.with_fix(fix);
                        }
                    }
                    out.push(r);
                }
                // An ANNOTATED binder `((: name T) value)` constrains the bound value's TYPE — and, when
                // `T` is a NARROW integer width and the value is an integer LITERAL, RANGE-CHECKS it,
                // exactly as a value annotation `(: value T)` does (CDZ0302 for a literal outside the
                // width). `check_binding_pattern` above only checks type AGREEMENT (`(: a Bool) 5` →
                // CDZ0203), and a deferred literal agrees with any int width, so an out-of-range narrow
                // literal (`(: a Int8) 200`) slipped through and ran to a value the type cannot hold. The
                // binder annotation must apply its width fit-check to the bound value, the let-binder
                // analogue of the parameter-substitution fit-check.
                if let Some(ann) = db.ast.as_form(lhs, ":")
                    && ann.len() == 2
                    && let Some(reject) = literal_width_fault(db, value, ann[1])
                {
                    out.push(reject);
                }
                // VALIDATE the binder annotation TYPE itself — the let-binding analogue of
                // `param_annotation_faults` (and the value-annotation / variant-payload / effect-op checks).
                // `check_binding_pattern` above only checks the annotation AGREES with the value's type; an
                // UNKNOWN annotation type (`(let (((: x Nonesuch) 5)) …)`) resolves to nothing and agrees
                // vacuously, so it slipped through and typed `x` as `Any`. A garbage annotation type — an
                // unbound name (→ CDZ0101, recursing into `(List Nonesuch)`), or a well-formed non-type (a
                // literal `(: x 5)` → CDZ0203) — must be rejected here, exactly as a parameter annotation is.
                if let Some(ann) = db.ast.as_form(lhs, ":").map(<[_]>::to_vec)
                    && ann.len() == 2
                    && crate::eval::typeval_of(db, ann[1]).is_none()
                {
                    // The SHARED annotation-type validator (M125): a record-bearing binder type
                    // (`(: r (Record (x Nonesuch)))`) names only the bad field TYPE, not the label `x`
                    // (the naive value-`collect` mis-resolved labels), and a bound-value-misused-as-a-type
                    // gets the category message the parameter + value sites use — the let-binder is not the
                    // odd one out.
                    validate_non_type_annotation(db, ann[1], "a binder's annotation", out);
                }
                collect(db, value, out);
            }
            collect(db, body, out);
        }
        // A match: every arm body must AGREE in type with the first (like an `if`'s branches), and the
        // scrutinee + bodies are checked for their own faults. (Exhaustiveness — a scalar match with no
        // covering arm — is checked in `lower::lower_match`, where the pattern shapes are classified;
        // the arms-agree check is the type fault here.)
        Resolved::Match { scrutinee, arms } => {
            if let Some((_, first_body)) = arms.first() {
                let first_ty = type_of(db, *first_body);
                for (_, body) in arms.iter().skip(1) {
                    let bt = type_of(db, *body);
                    if !first_ty.agrees_with(&bt) {
                        let delta = peer_type_delta_hint(&first_ty, &bt).unwrap_or_default();
                        let mut reject = Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "match arms differ: {} vs {}{delta}",
                                first_ty.render_name(),
                                bt.render_name()
                            ),
                        );
                        // An INT-LITERAL-vs-FLOAT arm clash has the same one-shot repair the if-branch,
                        // list-element, and annotation sites give (`(: 3 Float64)` → `3.0`): rewrite the
                        // integer-literal arm body as a float literal so the arms unify at the float type.
                        // The literal may be the FIRST arm (`(match x (0 1) (_ 2.0))`, fix `first_body`) or
                        // THIS one (`(match x (0 1.0) (_ 2))`, fix `body`); offer on whichever is the int
                        // literal (a computed integer arm body yields no fix).
                        if let Some(fix) = float_literal_retype_fix(db, *first_body, &first_ty, &bt)
                            .or_else(|| float_literal_retype_fix(db, *body, &bt, &first_ty))
                            // A record-field TYPO in one arm body vs another — rename the misspelled key on
                            // whichever arm carries it (both orderings, the peer-join twin at the match site).
                            .or_else(|| record_field_typo_fix(db, &first_ty, &bt, *body))
                            .or_else(|| record_field_typo_fix(db, &bt, &first_ty, *first_body))
                        {
                            reject = reject.with_fix(fix);
                        }
                        out.push(reject);
                    }
                }
            }
            // EXHAUSTIVENESS is well-formedness — a non-exhaustive match is a compile-time error
            // (`core-semantics.md` §Matching Is Exhaustive Or Rejected) whether or not the enclosing def
            // is reached, exactly as an unbound name in an uncalled sibling is. The emit path lowers only
            // reached (and, for the standalone walk, nullary-exported) bodies, so `check` missed a
            // non-exhaustive match on a function PARAMETER; surface it here, where `collect_node` visits
            // EVERY match in every body. `match_nonexhaustive_fault` returns only the CDZ0210 (carrying
            // its "add the missing arm" fix), never a not-yet-lowerable decline — so this adds the
            // actionable fix to `check`/`--json`/`fix` without raising false alarms.
            if let Some(r) = crate::lower::match_nonexhaustive_fault(db, id) {
                out.push(r);
            }
            // PATTERN-HEAD WELL-FORMEDNESS: a MISTYPED variant pattern head (`((C.Gren) …)` on `(type C
            // Red Green)`), a foreign-sum variant, a payload-arity mismatch, or a non-linear binder is a
            // CODED fault (CDZ0201 "record has no field `Gren` — did you mean `Green`?" carrying a replace
            // fix, CDZ0203, CDZ0102). It was produced ONLY by the emit-path lowering walk
            // (`collect_reached_poisons`), which runs on nullary-EXPORTED bodies alone — so a variant typo
            // in ANY parameterized function's match silently PASSED `cdz check` (exit 0, no diagnostic)
            // while `compile` rejected it, hiding the very "did you mean?" fix from the fast check path.
            // Surface it here, where `collect` visits EVERY match in every body — the pattern-fault twin of
            // the exhaustiveness accessor above. `match_pattern_fault` returns only a CODED non-CDZ0210
            // pattern fault (never a not-yet-lowerable decline, never the exhaustiveness code the accessor
            // already reports), so it adds the actionable fix to `check`/`--json`/`fix` with no false
            // alarm. A scrutinee/body poison it also bubbles up is independently collected below and shares
            // its (code, node, message), so `dedup_faults` collapses the duplicate; a genuine pattern-head
            // fault is produced by no other `collect` path, so it appears exactly once.
            if let Some(r) = crate::lower::match_pattern_fault(db, id) {
                out.push(r);
            }
            // SCRUTINEE / PATTERN TYPE COMPATIBILITY: a VARIANT-constructor pattern (`(C.Red)`, `(Some x)`)
            // over a scrutinee whose type is a DEFINITE NON-SUM — a scalar `(match 5 ((C.Red) …))`, a Bool,
            // a Float, a String — is a type confusion: the scrutinee has no variants to dispatch on. Reject
            // CDZ0203 (the same code `(: 5 Bool)` gets) rather than letting `lower_match` DECLINE it ("a
            // match pattern that is not a scalar literal or `_`"), which grades a genuine type error as a
            // to-do. Guarded on a DEFINITE non-sum scrutinee (`definite_non_sum_scalar`) so an undetermined
            // scrutinee (`Any` / an unsolved var — a not-yet-inferred param) still declines cleanly, never a
            // spurious reject; a mismatched-SUM pattern (a foreign variant over a different sum) is caught by
            // `pattern_constraints` in lowering with its own message, so this only adds the SCALAR case.
            let st = type_of(db, scrutinee);
            if definite_non_sum_scalar(&st) {
                for (pat, _) in &arms {
                    if pattern_is_variant_ctor(db, *pat) {
                        out.push(Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "a variant pattern cannot match a scrutinee of type {} — it is not a sum",
                                st.render_name()
                            ),
                        ));
                        break;
                    }
                }
            }
            collect(db, scrutinee, out);
            for (pat, body) in &arms {
                // A GUARDED arm `(guard <pattern> <cond>)`: the guard condition is a boolean predicate
                // gating the arm (`core-semantics.md` §A Guard Refines A Pattern With A Boolean Test), so
                // it must have type Bool — exactly like an `if` condition. A non-Bool guard (`(guard x (+
                // x 1))`) was NOT checked (nor descended into): it compiled, using an Int64 as a branch
                // condition. Type-check the cond against Bool (CDZ0203, the `if`-condition message) and
                // descend into it so a fault INSIDE the guard (an unbound name, a type error) also surfaces.
                if let Some(g) = db.ast.as_form(*pat, "guard").filter(|g| g.len() == 2) {
                    let cond = g[1];
                    let cond_ty = type_of(db, cond);
                    if !cond_ty.agrees_with(&Ty::Bool) {
                        trace!(target: "rcdzc::infer", node = cond.0, cond_ty = %cond_ty.render_name(), "fault: guard condition not Bool (CDZ0203)");
                        out.push(
                            Reject::coded(
                                Code::TypeMismatch,
                                format!(
                                    "guard condition must be Bool, found {}",
                                    cond_ty.render_name()
                                ),
                            )
                            .at(cond),
                        );
                    }
                    collect(db, cond, out);
                }
                collect(db, *body, out);
            }
        }
        Resolved::Record { fields } => {
            for (_, &value) in fields.iter() {
                collect(db, value, out);
            }
        }
        // Application faults, by the ONE rule: instantiate the head's `(meta t)` scheme and unify each
        // argument's type into the curried parameter positions. A unify FAILURE is the conflicting-use
        // type error (a non-integer where an integer is required, two operands of different widths —
        // no silent promotion). One check for every operator; no arith-specific logic. A head with no
        // `(meta t)` (a type constructor / not-yet-typed value) is not checked here — its own fault, if
        // any, surfaces via the head descent.
        //
        // Because an operator's parameter types must unify with the operand types (no widening rule sits
        // between them), two operands of different numeric type is a type error rather than an implicit
        // promotion, and a result's type is exactly what the operator's scheme + operand types give:
        //= spec/capabilities/numeric-model.md#numeric-types-do-not-silently-promote
        //# An operation on two numeric values of different types MUST require an explicit conversion rather than promote one operand implicitly.
        //= spec/capabilities/numeric-model.md#numeric-types-do-not-silently-promote
        //# The type of an arithmetic result MUST be determined by the operand types and the operation, not by an implicit widening the author did not write.
        Resolved::Apply { head, args } => {
            check_application(db, id, head, &args, out);
            // OPERATOR-ARITY WELL-FORMEDNESS: a fixed-arity binary operator applied to a count other than 2
            // (an over-application `(+ n 1 2)`/`(< n 1 2)`/`(+ x 1.0 2.0)`, or an under-application `(+ n)`)
            // has a clear operator-specific CDZ0201 "+ takes exactly 2 operands" — but it is produced ONLY
            // by the emit-path lowering walk (`collect_reached_poisons`, nullary-exported bodies), so `check`
            // on a PARAMETERIZED body saw only `check_application`'s GENERIC CDZ0203 ("applied N arguments to
            // a function of arity M …") for the over-application and NOTHING for the under-application, while
            // `compile` rejected both. Surface the operator message here, where `collect` visits every
            // application — the binop-arity twin of the pattern-fault accessor. `binop_arity_fault` fires
            // purely on the argument COUNT (the language has no unary operator form), so it is no false alarm
            // over an unreached body; on the over-application its CDZ0201 delete fix targets the same surplus
            // node as the CDZ0203, so `dedup_faults` keeps this clearer message and drops the generic sibling.
            if let Some(r) = crate::lower::binop_arity_fault(db, id) {
                out.push(r);
            }
            // Descend into the HEAD (an unbound head like `frobnicate` is a scope error caught here)
            // and each operand for their own faults.
            collect(db, head, out);
            // A RECORD value-constructor alias applied — `(record (x 1) (y 2))`. Its arguments are
            // `(key value)` FIELD PAIRS, not expressions, so descending into a pair as an expression
            // would resolve the key `x` as an unbound name. Instead validate the field shape (a
            // malformed pair / duplicate field is CDZ0201) and descend into each field's VALUE only —
            // exactly what the symbol-headed `({} …)` form does via `resolve_record`.
            if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordNew)
            ) {
                match crate::resolve::read_record_fields(db, &args) {
                    Ok(fields) => {
                        for (_, &value) in fields.iter() {
                            collect(db, value, out);
                        }
                    }
                    Err(reject) => out.push(reject),
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::MapNew)
            ) {
                // A MAP value-constructor alias applied — `(map (k v) …)`. Its arguments are `(key value)`
                // ENTRY PAIRS, not expressions: descending into a pair as an expression would resolve the
                // key `a` as applied to the value (the `(a 1)`→"cannot apply Int64" fault). `check_application`
                // above already validated homogeneity + descended into each key/value; here we only handle
                // the MALFORMED-entry fault (a non-`(key value)` entry) so it is reported — the well-formed
                // entries' faults are already collected. Do NOT `collect` the raw entry pairs.
                for &entry in args.iter() {
                    match db.ast.get(entry) {
                        crate::ast::Struct::List(items) if items.len() == 2 => {}
                        // A wrong-arity entry — a SURPLUS element gets the shared delete fix, too few is
                        // message-only (mirrors `resolve_map`; this path handles the `(map …)` NAME alias).
                        crate::ast::Struct::List(items) => {
                            let items = items.clone();
                            out.push(crate::resolve::fixed_arity_reject(
                                entry,
                                &items,
                                2,
                                "a map entry is a (key value) pair",
                            ));
                        }
                        _ => out.push(Reject::coded(
                            Code::Malformed,
                            "a map entry is a (key value) pair",
                        )),
                    }
                }
            } else if let Some(rec_op) = record_row_op_name(crate::eval::meta_apply_of(db, head))
                && !args.is_empty()
                && {
                    // A record row op over a DEFINITE NON-RECORD operand — `(Record.project n (x))` for
                    // `n : Int64` — is a kind error, exactly like member access on a non-record. It was
                    // check-INVISIBLE (the per-op field checks below only fire for a `Ty::Record` operand,
                    // silently skipping a non-record) and on compile gave a MISLEADING "a record row
                    // operation over a runtime record is not yet built" (the operand is not a record at
                    // all). `merge` checks BOTH operands; the rest check `args[0]`. Report "requires a
                    // record, found <T>" — the row-op twin of the member-access message — and skip the
                    // field checks (a non-record has no fields to check). `Any`/`Var` (an unconstrained
                    // param) is NOT definite, so it is not flagged (its real type flows in at the call site).
                    let operands: &[StructId] = if rec_op == "merge" { &args } else { &args[..1] };
                    operands
                        .iter()
                        .any(|&o| definite_non_record(&type_of(db, o)))
                }
            {
                let operands: &[StructId] = if rec_op == "merge" { &args } else { &args[..1] };
                for &o in operands {
                    let ot = type_of(db, o);
                    if definite_non_record(&ot) {
                        out.push(
                            Reject::coded(
                                Code::Malformed,
                                format!(
                                    "`Record.{rec_op}` requires a record, found {}",
                                    ot.render_name()
                                ),
                            )
                            .at(o),
                        );
                    }
                    collect(db, o, out);
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordProject | crate::resolved::Prim::RecordWithout)
            ) && args.len() == 2
            {
                // `Record.project r (a c)` / `Record.without r (b)` — the FIRST operand `r` is an ordinary
                // expression (descend it); the SECOND is a LITERAL field-name list `(a c)` — LABELS, not an
                // expression, so descending it would resolve `a` as applied to `c`. Instead read its labels
                // and check each names a field the operand record HOLDS: a named field ABSENT from `r`'s
                // record type is CDZ0212 — for `project` (§A Record Is Restricted To A Named Set Of Its
                // Fields, 2nd sentence) AND `without` (§A Record Is Reduced By Dropping A Named Set …, 2nd
                // sentence), the same absent-field check. A malformed label list is CDZ0201.
                collect(db, args[0], out);
                match (
                    type_of(db, args[0]),
                    crate::resolve::record_op_labels(db, args[1]),
                ) {
                    (Ty::Record(fields), Some(labels)) => {
                        // A DUPLICATE label in the projection list `(a a)` is the SAME malformedness a
                        // record LITERAL with a duplicate field `(record (a 1) (a 2))` is rejected for
                        // (CDZ0201) — a record's fields are a fixed SET of names (`type-system.md` §A Record
                        // Is Restricted To A Named Set Of Its Fields), so a label named twice is ill-formed,
                        // not silently deduplicated to one field. Checked here (the projection's
                        // well-formedness site, beside the absent-field check) so it holds whether or not the
                        // projection is reached. Reported at the first REPEAT, matching the record-literal
                        // message/code.
                        let mut seen: std::collections::HashSet<&crate::resolved::Symbol> =
                            std::collections::HashSet::new();
                        for label in &labels {
                            if !seen.insert(label) {
                                out.push(
                                    Reject::coded(
                                        Code::Malformed,
                                        format!(
                                            "record names field `{}` more than once",
                                            label.name
                                        ),
                                    )
                                    .at(args[1]),
                                );
                                break;
                            }
                        }
                        // The record's own field NAMES — the closed set a dropped/projected label must
                        // name, so a near-miss is a "did you mean?" (the same closed-set suggestion a
                        // member access `(. r k)` gets — a mistyped `Record.without r (alfa)` for a field
                        // `alpha` should point at it, not just say "no field `alfa`"). The label LIST's
                        // child nodes align positionally with `labels` (both read from `args[1]`), so a
                        // near-miss carries a REPLACE fix on the SPECIFIC label occurrence — the same
                        // fix `no_field_reject` attaches for a member access, not just the message hint.
                        let field_names: Vec<&str> =
                            fields.keys().map(|k| k.name.as_str()).collect();
                        let label_nodes: Vec<StructId> = match db.ast.get(args[1]) {
                            crate::ast::Struct::List(items) => items.clone(),
                            _ => Vec::new(),
                        };
                        for (i, label) in labels.iter().enumerate() {
                            if !fields.contains_key(label) {
                                // The confident single (tier 1) drives the REPLACE fix; the two-tier
                                // `did_you_mean` message adds the closest-matches LIST when there is no
                                // confident typo, so a FAR label (`Record.without r (zzzzz)`) tells the
                                // author what fields the record actually has instead of a dead-end "no
                                // field" — the row-op twin of `no_field_reject`'s member-access enrichment.
                                let near = crate::diag::suggest::nearest(
                                    &label.name,
                                    field_names.iter().copied(),
                                );
                                let hint = crate::diag::suggest::did_you_mean(
                                    &label.name,
                                    field_names.iter().copied(),
                                    3,
                                );
                                let msg = format!(
                                    "{}`{}`{hint}",
                                    crate::diag::NO_FIELD_PREFIX,
                                    label.name
                                );
                                let mut reject = Reject::coded(Code::AbsentField, msg).at(args[1]);
                                // Attach the replace fix on THIS label's occurrence when a near field
                                // exists AND its node is known — `label_nodes[i]` is the i-th label of
                                // `args[1]` (positionally aligned with `labels`). Only tier 1 (a confident
                                // single) carries a fix; a tier-2 list of options is not one mechanical edit.
                                if let (Some(near), Some(&node)) = (near, label_nodes.get(i)) {
                                    reject = reject
                                        .with_fix(crate::diag::Fix::replace_heuristic(node, near));
                                }
                                out.push(reject);
                            }
                        }
                    }
                    // A non-record operand, or a malformed label list — the operand's own fault (a non-record
                    // type) surfaces via the `collect(args[0])` above; a malformed label list is CDZ0201.
                    (_, None) => out.push(Reject::coded(
                        Code::Malformed,
                        "the second operand is a list of field names, e.g. `(a c)`",
                    )),
                    _ => {}
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordMerge)
            ) && args.len() == 2
            {
                // `Record.merge a b` — BOTH operands are ordinary record expressions (descend each). Their
                // field sets MUST be DISJOINT: a name present in BOTH is CDZ0211 (`type-system.md` §Two
                // Records Are Combined Only When Their Field Sets Are Disjoint, 2nd sentence) — the combined
                // record never chooses which operand's value a shared field takes. A non-record operand's
                // own fault surfaces via the descent.
                collect(db, args[0], out);
                collect(db, args[1], out);
                if let (Ty::Record(a), Ty::Record(b)) = (type_of(db, args[0]), type_of(db, args[1]))
                {
                    for k in a.keys() {
                        if b.contains_key(k) {
                            out.push(
                                Reject::coded(
                                    Code::PresentField,
                                    format!("both records share the field `{}`", k.name),
                                )
                                .at(id),
                            );
                        }
                    }
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordExtend | crate::resolved::Prim::RecordWith)
            ) && args.len() == 2
            {
                // `Record.extend r (z v)` / `Record.with r (z v)` — the FIRST operand `r` is a record
                // expression (descend it); the SECOND is a `(z v)` pair whose NAME is a label and whose
                // VALUE is an ordinary expression (descend the value, NOT the whole pair — descending it
                // would resolve `z` as applied to `v`). `extend` REQUIRES `z` ABSENT (a present field →
                // CDZ0211, never a silent overwrite); `with` REQUIRES `z` PRESENT (an absent field →
                // CDZ0212, stays distinct from `extend`). A malformed pair is CDZ0201.
                let is_extend = crate::eval::meta_apply_of(db, head)
                    == Some(crate::resolved::Prim::RecordExtend);
                collect(db, args[0], out);
                match (
                    type_of(db, args[0]),
                    crate::resolve::record_op_pair(db, args[1]),
                ) {
                    (Ty::Record(fields), Some((label, value))) => {
                        collect(db, value, out);
                        let present = fields.contains_key(&label);
                        // The operation-KEY occurrence of the `(. Record extend|with)` head — the node an
                        // OPERATOR-SWAP fix rewrites (`extend`→`with` or `with`→`extend`). The message
                        // already NAMES the sibling op to use; the fix makes that one-token swap applyable
                        // (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix — the
                        // row-op analogue of `float_sibling_operator`). `None` if the head is not `(. R op)`.
                        let op_key_occ = db.ast.as_form(head, ".").and_then(|t| t.get(1).copied());
                        if is_extend && present {
                            // `extend` REQUIRES an absent field; the field is present → the author means
                            // `with` (replace). Swap the op: `Record.extend`→`Record.with`, VERIFIED — the
                            // present field is exactly `with`'s precondition, so the swap clears the fault
                            // without introducing another (unlike a heuristic near-miss guess).
                            let mut reject = Reject::coded(
                                Code::PresentField,
                                format!(
                                    "record already has field `{}` (use `Record.with` to replace)",
                                    label.name
                                ),
                            )
                            .at(args[1]);
                            if let Some(occ) = op_key_occ {
                                reject = reject.with_fix(crate::diag::Fix::replace_verified(
                                    occ,
                                    "with".to_string(),
                                    "replace `extend` with `with` to update the existing field",
                                ));
                            }
                            out.push(reject);
                        } else if !is_extend && !present {
                            // A did-you-mean over the record's own fields — a mistyped `with` field is
                            // the closed-set case (like `without`/`project`, M51): `Record.with r (alpa …)`
                            // for a field `alpha` should point at it. The complementary-op hint
                            // (`Record.extend` to ADD) stays: the suggestion and the add-hint are distinct
                            // advice (fix the typo vs. genuinely a new field).
                            let near = nearest_record_field(db, &fields, &label.name);
                            let did = near
                                .as_ref()
                                .map(|n| format!(" (did you mean `{n}`?)"))
                                .unwrap_or_default();
                            let mut reject = Reject::coded(
                                Code::AbsentField,
                                format!(
                                    "record has no field `{}` to update{did} (use `Record.extend` to add)",
                                    label.name
                                ),
                            )
                            .at(args[1]);
                            // TWO possible repairs, and which is right depends on intent: (a) the label is a
                            // TYPO of a real field → rewrite the label (the near-miss case, like `without`/
                            // `project`, M63); (b) the field genuinely does NOT exist → the author means
                            // `extend` (ADD it), so swap the op `with`→`extend`. Prefer the label typo-fix
                            // when a near field exists (the likelier intent — a present-looking field name);
                            // else offer the operator swap (VERIFIED — an absent field is exactly `extend`'s
                            // precondition). The `(z v)` pair's FIRST child is the label node `z`.
                            let label_node = match db.ast.get(args[1]) {
                                crate::ast::Struct::List(items) => items.first().copied(),
                                _ => None,
                            };
                            match (&near, label_node, op_key_occ) {
                                (Some(near), Some(label_node), _) => {
                                    reject = reject.with_fix(crate::diag::Fix::replace_heuristic(
                                        label_node,
                                        near.clone(),
                                    ));
                                }
                                (None, _, Some(occ)) => {
                                    reject = reject.with_fix(crate::diag::Fix::replace_verified(
                                        occ,
                                        "extend".to_string(),
                                        "replace `with` with `extend` to add the new field",
                                    ));
                                }
                                _ => {}
                            }
                            out.push(reject);
                        }
                    }
                    (_, None) => out.push(Reject::coded(
                        Code::Malformed,
                        "the second operand is a `(name value)` field pair, e.g. `(z 5)`",
                    )),
                    _ => {}
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordPop)
            ) && args.len() == 2
            {
                // `Record.pop r z` — the FIRST operand `r` is a record expression (descend it); the SECOND
                // is a BARE field NAME (a label, not an expression). An absent field is CDZ0212 (a static
                // label, never a runtime None). A non-label second operand is CDZ0201.
                collect(db, args[0], out);
                let label = crate::resolve::read_label(db, args[1]);
                match (type_of(db, args[0]), label) {
                    (Ty::Record(fields), Some(label)) if !fields.contains_key(&label) => {
                        // A did-you-mean over the record's own fields (the closed-set case, as
                        // `without`/`project`/`with` — a mistyped popped field should point at the real one).
                        let near = nearest_record_field(db, &fields, &label.name);
                        let did = near
                            .as_ref()
                            .map(|n| format!(" (did you mean `{n}`?)"))
                            .unwrap_or_default();
                        let mut reject = Reject::coded(
                            Code::AbsentField,
                            format!("record has no field `{}` to pop{did}", label.name),
                        )
                        .at(args[1]);
                        // `Record.pop`'s field operand is a BARE name (`args[1]` itself), so the near-miss
                        // replace fix rewrites `args[1]` directly — the same closed-set fix `without`/
                        // `project`/`with` labels get (M63). No near → message only.
                        if let Some(near) = &near {
                            reject = reject.with_fix(crate::diag::Fix::replace_heuristic(
                                args[1],
                                near.clone(),
                            ));
                        }
                        out.push(reject);
                    }
                    (_, None) => out.push(Reject::coded(
                        Code::Malformed,
                        "the second operand is a field name, e.g. `z`",
                    )),
                    _ => {}
                }
            } else if let Some(tup_op) = tuple_row_op_name(crate::eval::meta_apply_of(db, head))
                && !args.is_empty()
                && {
                    // A tuple row op over a DEFINITE NON-TUPLE operand — `(Tuple.pop n)` for `n : Int64` —
                    // is a kind error, the tuple twin of the record-row-op check. It was check-INVISIBLE
                    // and compiled to a MISLEADING "Tuple.<op> over a runtime tuple is not yet built" (the
                    // operand is not a tuple at all). `cat` checks BOTH operands; `split-at`/`pop` check
                    // `args[0]` (`split-at`'s `args[1]` is a position literal, not a tuple). Report
                    // "requires a tuple, found <T>" (the tuple-projection message already exists for `(. n
                    // N)`); `Any`/`Var` (an unconstrained param) is not flagged.
                    let operands: &[StructId] = if tup_op == "cat" { &args } else { &args[..1] };
                    operands
                        .iter()
                        .any(|&o| definite_non_tuple(&type_of(db, o)))
                }
            {
                let operands: &[StructId] = if tup_op == "cat" { &args } else { &args[..1] };
                for &o in operands {
                    let ot = type_of(db, o);
                    if definite_non_tuple(&ot) {
                        out.push(
                            Reject::coded(
                                Code::Malformed,
                                format!(
                                    "`Tuple.{tup_op}` requires a tuple, found {}",
                                    ot.render_name()
                                ),
                            )
                            .at(o),
                        );
                    }
                    collect(db, o, out);
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::TupleSplitAt)
            ) && args.len() == 2
            {
                // `Tuple.split-at t k` — descend `t`; the position `k` is a compile-time literal (not an
                // expression to descend). A `k` OUTSIDE the operand's static arity range `0..=arity` is a
                // type error (CDZ0201) — the same static-bounds rule an out-of-arity `(. x N)` gets
                // (`type-system.md` §A Tuple Is Split At A Position …, 2nd sentence). A non-literal `k`, or
                // a non-tuple operand, falls through (declines/faulted elsewhere).
                //= spec/capabilities/type-system.md#a-tuple-is-split-at-a-position-into-a-prefix-and-a-suffix
                //# A split position that is not within the operand tuple's static arity range MUST be rejected at compile time as a type error, consistent with a positional tuple access whose index is out of the tuple's static arity being rejected, so that a split can never name a position the tuple does not have.
                collect(db, args[0], out);
                if let Ty::Tuple(elems) = type_of(db, args[0])
                    && let Resolved::Int(k) = resolved_of(db, args[1])
                    && k.to_i64()
                        .is_none_or(|k| !(0..=elems.len() as i64).contains(&k))
                {
                    out.push(
                        Reject::coded(
                            Code::Malformed,
                            format!(
                                "split position is outside the tuple's arity 0..={}",
                                elems.len()
                            ),
                        )
                        .at(args[1]),
                    );
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::TupleCat | crate::resolved::Prim::TuplePop)
            ) {
                // `Tuple.cat a b` / `Tuple.pop t` — both operands (cat) / the sole operand (pop) are
                // ordinary tuple expressions; descend each. No positional literal to guard (cat has none;
                // pop's arity-≥1 requirement — an empty tuple is `unit`, not a tuple — surfaces as the
                // generic non-tuple fault). No per-op reject here.
                for &arg in args.iter() {
                    collect(db, arg, out);
                }
            } else if crate::eval::lambda_body(db, head).is_some() {
                // A LAMBDA head — `check_application` already collected the REDUCED body (step 2), which
                // contains each argument substituted at its use sites, so the used arguments' faults are
                // ALREADY gathered. Re-descending the raw arguments here would DOUBLE the walk of every
                // argument subtree — and on a deep call chain `(f (f … (f 0)))` (where each argument is
                // itself the next call) that doubling, compounded per level, is what made the fault walk
                // O(N³) (each of the N `collect(arg)` frames restarts a full O(N) descent, whose own
                // `type_of` adds another factor). Skip it. A DEAD argument (one the body never uses, so it
                // is NOT in the reduced body) still needs checking — `check_application` descends those
                // (only the un-substituted args) so their faults are not lost.
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::SetOf)
            ) {
                // `Set.of list` — `check_application` above already checked the list's element HOMOGENEITY
                // as a SET fault (CDZ0201) and, on a mismatch, descended into the elements + returned. So
                // descending into the list ARG here would RE-derive the list's own CDZ0203 (a redundant,
                // wrong-coded duplicate of the fault already reported). Instead descend into each ELEMENT
                // directly (a nested per-element fault still surfaces), never the list node as a whole.
                let list = args[0];
                let elems: Vec<StructId> = match resolved_of(db, list) {
                    Resolved::List { elems } => elems.to_vec(),
                    Resolved::Apply { head: lh, args: la }
                        if crate::eval::meta_apply_of(db, lh)
                            == Some(crate::resolved::Prim::ListNew) =>
                    {
                        la.to_vec()
                    }
                    // Not a visible list literal (a runtime list operand) — collect it normally.
                    _ => {
                        collect(db, list, out);
                        Vec::new()
                    }
                };
                for &e in &elems {
                    collect(db, e, out);
                }
            } else {
                for &arg in args.iter() {
                    collect(db, arg, out);
                }
            }
        }
        // A type annotation `(: expr T)`: UNIFY the asserted type `T` against `expr`'s type. A failure
        // is the conflicting-use type error (`(: true Int64)` — Bool asserted as Int64 → CDZ0203); a
        // success grounds a deferred width harmlessly. If `T` is not a type this stage reduces, decline
        // the CHECK (no false reject) — the expr still type-checks on its own via the descent below.
        Resolved::Annot { expr, ty_expr } => {
            // A RUNTIME WIDTH is forbidden: `(: 5 (UInt n))` with `n` a runtime value (a parameter, a
            // call result) puts runtime data in a type-determining position, which the type system
            // forbids — an integer type's width MUST be a compile-time natural (`numeric-model.md §An
            // Integer Type Is Indexed By A Compile-Time Width`; `type-system.md §Generics Are
            // Type-Valued Parameters` — a type-valued parameter is resolved at compile time, never from
            // runtime data). This is checked on the ANNOTATION's un-inlined body, so `(def (mk n) (: 5
            // (UInt n)))` rejects (CDZ0302) even though a constant call site `(mk 8)` would fold `n` — a
            // width must be non-dependent regardless of how it happens to be called.
            let runtime_width = crate::eval::is_runtime_width_type(db, ty_expr);
            if runtime_width {
                trace!(target: "rcdzc::infer", node = id.0, "fault: integer width from runtime data (CDZ0302)");
                out.push(Reject::coded(
                    Code::IntOutOfRange,
                    "an integer width must be a compile-time natural, not runtime data",
                ));
            }
            // A TYPE CONSTRUCTOR applied at the WRONG arity in the annotation position — a prelude `(: 5
            // (List Int64 Int64))` or a user generic sum `(: b (Box Int64 Bool))`. Checked FIRST, before
            // the type is used below, because a generic sum REDUCES to a `Ty::Sum` (silently dropping the
            // extra arg) so `typeval_of` succeeds and the "not a type" branch never fires — the arity fault
            // would be lost. `type_ctor_arity_message` returns `None` for a correct arity / a non-ctor.
            if !runtime_width && let Some(msg) = type_ctor_arity_message(db, ty_expr) {
                trace!(target: "rcdzc::infer", node = id.0, "fault: type constructor applied at the wrong arity (CDZ0203)");
                out.push(Reject::coded(Code::TypeMismatch, msg));
                collect(db, expr, out);
                return;
            }
            if let Some(annot_ty) = crate::eval::typeval_of(db, ty_expr) {
                // A FLOAT type annotating with a NON-ADMITTED width reduces (via the `Float` constructor)
                // to the sentinel `Ty::Float(Fixed(0))` — a `(: 1.5 (Float 16))` / `(: 1.5 (Float 48))`.
                // A float width outside {32,64} is rejected CDZ0302 (numeric-model.md §A Floating-Point
                // Type Is Indexed By A Compile-Time Width), the float analogue of an out-of-range integer
                // width; caught here at the annotation, before the grounding/unify below (which would
                // otherwise let the sentinel slip through against a deferred literal).
                //= spec/capabilities/numeric-model.md#a-floating-point-type-is-indexed-by-a-compile-time-width
                //# A floating-point bit width that is outside the set the numeric model admits MUST be rejected at compile time with the machine-readable diagnostic for the unsatisfied width constraint, rather than accepted or trapped at runtime.
                if let Ty::Float(ft) = &annot_ty
                    && let crate::ty::Width::Fixed(w) = ft.width
                    && !crate::ty::ADMITTED_FLOAT_WIDTHS.contains(&w)
                {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: float width not in the admitted set (CDZ0302)");
                    out.push(Reject::coded(
                        Code::IntOutOfRange,
                        "a floating-point width must be one of the admitted IEEE widths (32 or 64)",
                    ));
                }
                // An OUT-OF-CEILING / zero INTEGER width `(: e (UInt 65))` is ill-formed at the annotation
                // regardless of what `e` is — the integer analogue of the float admitted-set check just
                // above. `reduce_ctor` clamps such a width to the sentinel 0 (so `annot_ty` is `Int0`), and
                // the literal-fit check below would only fire for a CONCRETE literal and would name the
                // misleading clamped `UInt0`; catch it HERE by reading the ORIGINAL width, so a NON-literal
                // value (`(: x (UInt 65))`) is rejected too and the message names the written width. Same
                // CDZ0302 + wording as the parameter-annotation path (`param_annotation_faults`).
                if let Some((signed, w)) = crate::eval::out_of_range_int_width(db, ty_expr) {
                    trace!(target: "rcdzc::infer", node = id.0, signed, width = w, "fault: over-ceiling integer width in a value annotation (CDZ0302)");
                    out.push(Reject::coded(
                        Code::IntOutOfRange,
                        format!(
                            "`{}{w}` is not a valid integer type: a width must be in 1..=64 (a fixed-size \
                             integer wider than 64 bits is reserved to the big-integer layer, and 0 is not \
                             a width)",
                            if signed { "Int" } else { "UInt" }
                        ),
                    ));
                }
                // A bare integer LITERAL annotated with an integer type is a GROUNDING, not a
                // unification: the literal has no intrinsic signedness/width to conflict, so the
                // annotation fixes its type (`(: 200 UInt8)` : UInt8) subject only to a RANGE CHECK —
                // a literal outside the width is rejected (CDZ0302), never truncated. (This is
                // "Annotations Constrain": the annotation determines the literal's type rather than
                // clashing with the signed-64 default a bare literal would otherwise take.)
                if let (Resolved::Int(v), Ty::Int(it)) = (resolved_of(db, expr), &annot_ty) {
                    // Skip the SENTINEL width 0 (`(: 5 (UInt 65))` clamps to `Int0`): the ill-formed-width
                    // check just above already reported it naming the written width, so a "literal does not
                    // fit UInt0" here would double-report and mislead.
                    if let crate::ty::Width::Fixed(w) = it.width
                        && w != 0
                        && !v.fits_width(it.ground_signed(), w)
                    {
                        trace!(target: "rcdzc::infer", node = id.0, annot_ty = %annot_ty.render_name(), "fault: literal does not fit annotated width (CDZ0302)");
                        out.push(int_out_of_range_reject(
                            &annot_ty,
                            it.ground_signed(),
                            w,
                            &v,
                            ty_expr,
                        ));
                    }
                } else if matches!(resolved_of(db, expr), Resolved::Int(_))
                    && matches!(annot_ty, Ty::BigInt)
                {
                    // A bare integer LITERAL annotated `BigInt` is a GROUNDING — the same "Annotations
                    // Constrain" rule as `(: 200 UInt8)`, but there is NO range check: `BigInt` is
                    // unbounded, so EVERY literal fits (`(: 100000000000000000000 BigInt)` — a value no
                    // fixed width holds — is exactly an exact BigInt). The literal's `IntValue` already
                    // carries the arbitrary-precision magnitude; only the static type widens to
                    // `Ty::BigInt` (the annot node's type, from `type_of`'s `Annot` arm). No fault.
                    trace!(target: "rcdzc::infer", node = id.0, "integer literal annotated BigInt — grounds to BigInt (unbounded, always fits)");
                } else if matches!(resolved_of(db, expr), Resolved::Int(_) | Resolved::Float(_))
                    && matches!(annot_ty, Ty::Rational)
                {
                    // A numeric LITERAL annotated `Rational` is a GROUNDING — the same "Annotations
                    // Constrain" rule as `(: 200 UInt8)` / `(: N BigInt)`, but with NO range check and
                    // no truncation: an EXACT rational holds any literal. An integer `k` grounds to the
                    // exact `k/1`; a decimal `significand·10^exp` grounds to the exact `significand /
                    // 10^|exp|` (LOSSLESS — a `Decimal` is captured exactly, so `0.5` is precisely `1/2`,
                    // never a rounded float). `lower`'s `Annot` arm folds the literal to the normalized
                    // `Core::ConstRational`; here we only suppress the CDZ0203 the generic unify below
                    // (`Rational` vs the literal's deferred int/float type) would otherwise report.
                    trace!(target: "rcdzc::infer", node = id.0, "numeric literal annotated Rational — grounds to the exact rational (always fits)");
                } else if let (
                    Ty::Qty {
                        inner: ai,
                        unit: au,
                    },
                    Ty::Qty {
                        inner: ei,
                        unit: eu,
                    },
                ) = (&annot_ty, &type_of(db, expr))
                    && au != eu
                {
                    // A DIMENSIONAL annotation conflict — the value derives one dimension but the
                    // annotation asserts another (`(: (* (Qty 2 meter) (Qty 3 meter)) (Qty Float64
                    // meter))` — the product is meter², annotated meter). This is the dimensional
                    // specialization of the annotation conflict: CDZ0501, not the CDZ0203 the generic
                    // unify below would give (units-of-measure.md §A Dimensional Mismatch Is An Error;
                    // the erasure fence's dimensional case). The inner types are irrelevant when the
                    // units already disagree — the dimension is the conflict.
                    let _ = (ai, ei);
                    trace!(target: "rcdzc::infer", node = id.0, "fault: annotation at a dimension the expression does not derive (CDZ0501)");
                    out.push(Reject::coded(
                        Code::DimensionMismatch,
                        format!(
                            "this expression has dimension {} but is annotated {} — the annotation \
                             must match the dimension the expression derives",
                            eu.render_human(),
                            au.render_human(),
                        ),
                    ));
                } else {
                    // A non-literal value has a real type that must AGREE with the annotation — unify,
                    // and report a genuine conflict (`(: true Int64)`) as CDZ0203. The annotation is an
                    // ADDITIONAL CONSTRAINT unified with the inferred type, never an override: a conflict
                    // is a rejection, not a silent replacement of the inferred type.
                    //= spec/capabilities/type-system.md#annotations-constrain-never-contradict
                    //# An explicit type annotation MUST participate in inference as an additional constraint unified with the type the system infers, rather than override it.
                    //= spec/capabilities/type-system.md#annotations-constrain-never-contradict
                    //# A program whose annotation cannot be unified with the type inference determines MUST be rejected rather than have the annotation silently replace the inferred type.
                    // The same check realizes the compile-time half of core-semantics §Types Are First-Class
                    // Values: the annotation is validated against the expression's static type here, and a
                    // mismatch is rejected before the program runs (the runtime-inspection half — a Type as a
                    // value inspected at runtime — is not realized; the seed erases types).
                    //= spec/capabilities/core-semantics.md#types-are-first-class-values
                    //# The compiler MUST validate a type annotation against the annotated expression's static type at compile time.
                    //= spec/capabilities/core-semantics.md#types-are-first-class-values
                    //# The compiler MUST reject a program in which a type annotation's declared type does not match the annotated expression's static type before that program runs.
                    let expr_ty = type_of(db, expr);
                    let mut subst = Subst::new();
                    if crate::unify::unify(&mut subst, &annot_ty, &expr_ty).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, annot_ty = %annot_ty.render_name(), expr_ty = %expr_ty.render_name(), "fault: annotation type mismatch (CDZ0203)");
                        // This arm reports BOTH a genuine value annotation `(: value T)` the author wrote AND
                        // a CALL ARGUMENT checked via the parameter's SYNTHESIZED `(: arg paramtype)` wrap
                        // (`substituted_arg` — step (2) of the lambda-application check). For a call argument
                        // the word "annotation" is misleading: the author wrote no annotation, they passed a
                        // wrong-typed value to a parameter. The two are distinguishable: the synthesized wrap
                        // is a SPANLESS (non-user) node whose `expr` is the USER-written argument, whereas a
                        // genuine `(: value T)` is a user node (and a β-copied in-body annotation has a
                        // non-user `expr` too — deduped against its user twin). So `id` non-user + `expr` user
                        // ⟹ a call argument. Phrase it as "this argument is a Bool, but … expects Int64"
                        // instead of "annotation type Int64 does not match value type Bool".
                        let is_call_arg = !db.is_user_node(id) && db.is_user_node(expr);
                        let mismatch_lead = |verb_tail: &str| {
                            if is_call_arg {
                                format!(
                                    "this argument is {}, but a value of type {} is expected here{verb_tail}",
                                    expr_ty.render_with_article(),
                                    annot_ty.render_name(),
                                )
                            } else {
                                format!(
                                    "annotation type {} does not match value type {}{verb_tail}",
                                    annot_ty.render_name(),
                                    expr_ty.render_name(),
                                )
                            }
                        };
                        // The annotation mismatch has a MECHANICAL REPAIR in several cases, each making the
                        // value type-check in ONE shot (the annotation position mirrors the argument
                        // position's coercion fixes). The two LITERAL-RETYPE repairs (a `replace`, not a
                        // wrap) are checked first:
                        //  • an INTEGER-VALUED FLOAT LITERAL annotated an INTEGER — `(: 3.0 Int64)` → DROP
                        //    the fractional form, REPLACE `3.0` with `3` (a non-integer / out-of-range float
                        //    → no `int_text` → no fix; truncating is the author's choice); and
                        //  • an INTEGER LITERAL annotated a FLOAT — `(: 3 Float64)` → ADD the fractional
                        //    form, REPLACE `3` with `3.0` (the exact mirror; a bignum past i128 → no fix).
                        // Then the two WRAP repairs: sum single-payload ctor ("wrap in `Some`"), and the
                        // `(<AnnotInt>.of value)` int-width coercion.
                        let literal_retype: Option<(String, &'static str)> =
                            if let Ty::Int(expected_int) = &annot_ty
                                && let crate::ast::Struct::Atom(lid) = db.ast.get(expr)
                                && let crate::ast::Leaf::Float(dec) = db.ast.leaf(*lid).clone()
                            {
                                // integer-valued float annotated Int → drop the `.0`.
                                integer_text_of_float_literal(&dec, *expected_int)
                                    .map(|t| (t, "drop the fractional form"))
                            } else if matches!(&annot_ty, Ty::Float(_))
                                && let crate::ast::Struct::Atom(lid) = db.ast.get(expr)
                                && let crate::ast::Leaf::Int { value, .. } =
                                    db.ast.leaf(*lid).clone()
                                && let Some(n) = value.to_i128()
                            {
                                // integer literal annotated Float → add the `.0` (make it a float literal).
                                Some((format!("{n}.0"), "make it a float literal"))
                            } else {
                                None
                            };
                        if let Some((text, verb)) = literal_retype {
                            out.push(
                                Reject::coded(
                                    Code::TypeMismatch,
                                    mismatch_lead(&format!(" — {verb} (`{text}`)")),
                                )
                                .at(id)
                                .with_fix(Fix::replace_heuristic(expr, text)),
                            );
                        } else {
                            // Compute the wrap `(prefix, suffix, verb, msg_tail)` from whichever applies.
                            let wrap: Option<(String, String, String, String)> = if let Some(ctor) =
                                wrap_variant_for(db, &annot_ty, &expr_ty)
                            {
                                Some((
                                    format!("({ctor} "),
                                    ")".to_string(),
                                    format!("wrap in `({ctor} …)`"),
                                    format!(" — wrap the value in `{ctor}`"),
                                ))
                            } else if let (Ty::Float(_), Ty::Int(actual_int)) =
                                (&annot_ty, &expr_ty)
                            {
                                // A NON-literal integer expression annotated a float — `(: n Float64)`
                                // with `n : Int64` — cannot become a float LITERAL (handled above for a
                                // literal), so convert it with `(<Float>.of-int …)`, widening a narrower
                                // int to Int64 first (`of-int : Int64 → Float`). Mirrors
                                // `numeric_text_coercion_fix`'s int→float branch, which the ARGUMENT site
                                // already offered — the annotation site had NO int→float wrap for a
                                // non-literal, so `(: n Float64)` declined a fix the arg site gave.
                                let f = annot_ty.render_name();
                                if actual_int.ground_width() == 64 && actual_int.ground_signed() {
                                    Some((
                                        format!("({f}.of-int "),
                                        ")".to_string(),
                                        format!("convert the integer to {f} with `{f}.of-int`"),
                                        format!(" — convert with `({f}.of-int …)`"),
                                    ))
                                } else {
                                    Some((
                                        format!("({f}.of-int (Int64.of "),
                                        "))".to_string(),
                                        format!("convert to {f} with `{f}.of-int (Int64.of …)`"),
                                        format!(" — convert with `({f}.of-int (Int64.of …))`"),
                                    ))
                                }
                            } else if let Some((prefix, suffix, verb)) =
                                int_coercion_wrap(&annot_ty, &expr_ty)
                            {
                                let n = annot_ty.render_name();
                                Some((
                                    prefix,
                                    suffix,
                                    verb,
                                    format!(" — convert with `({n}.of …)`"),
                                ))
                            } else if let Some((prefix, suffix, verb)) =
                                total_conversion_wrap(&annot_ty, &expr_ty)
                            {
                                // A total prelude conversion bridges the mismatch — `String` where
                                // `Bytes` is annotated → `(String.to-bytes …)`. The heuristic wrap fix
                                // applies it; the message tail names the verb inline. (This is the
                                // annotation-context twin of the CALL-SITE arg check's total-conversion
                                // wrap — a `(g s)` to a `Bytes` param now reports the SAME CDZ0203 with
                                // this fix, since the arg check defers a REFERENCED param to this arm.)
                                let tail = format!(" — {verb}");
                                Some((prefix, suffix, verb, tail))
                            } else {
                                None
                            };
                            // A BARE NUMBER where a QUANTITY is annotated — `(: 5 (Qty Int64 meter))`, or a
                            // bare `5` passed to a `(Qty …)` parameter — gets the `(Qty.of <n> <unit>)` wrap
                            // (unit read from the EXPECTED quantity type), the annotation twin of the
                            // dimensional-mismatch site's `Qty.of` wrap. The MECHANICAL fix is offered ONLY
                            // for a CALL ARGUMENT (`is_call_arg`): its `expr` is the user-written argument
                            // node, whose wrap payload the parse-based `fix_edits` builder splices cleanly. A
                            // DIRECT value annotation `(: 5 (Qty …))` gets the TAIL only — its wrap payload
                            // (carrying a nested `(Unit.base …)` surface) mis-splices the WRAP_HOLE into the
                            // nested member access, so the fix is withheld there (message still points the way).
                            let qty_fix = if wrap.is_none() && is_call_arg {
                                qty_coercion_fix(&annot_ty, &expr_ty, expr)
                            } else {
                                None
                            };
                            let qty_tail = if wrap.is_none()
                                && let Ty::Qty { inner, unit } = &annot_ty
                                && expr_ty.agrees_with(inner)
                                && !matches!(&expr_ty, Ty::Qty { .. })
                            {
                                format!(
                                    " — give the number the required unit, e.g. `(Qty.of … {})`",
                                    unit.render()
                                )
                            } else {
                                String::new()
                            };
                            // When NO conversion wrap bridges the mismatch, one common shape still has an
                            // actionable explanation: the value is an `(Option T)` used where its PAYLOAD
                            // `T` is expected — a fallible read (`List.at`, `String.at`) whose optional
                            // result was used directly. There is no TOTAL unwrap (an `Option` is eliminated
                            // only by matching its `None` case — the author's choice), so no mechanical fix;
                            // but the message can say WHY + how to fix it ("the value is optional — match it
                            // to handle the absent (`None`) case") instead of only naming two types. Tail
                            // only (no fix), and only when the wrap chain found nothing.
                            let option_tail = if wrap.is_none() {
                                option_payload_mismatch_hint(&annot_ty, &expr_ty)
                            } else {
                                None
                            };
                            // Two RECORDS that differ only in their FIELD SET — the value is missing a
                            // field the type requires, or carries one the type has no place for. Naming
                            // both full record types (`(Record (x Int64) (y Int64))` vs `(Record (x
                            // Int64))`) buries the actual difference; name the specific missing/extra
                            // fields instead (rustc's "missing field `y`" / "no field `z`"). Tail only.
                            let record_tail = if wrap.is_none() && option_tail.is_none() {
                                record_field_diff_hint(&annot_ty, &expr_ty)
                                    .or_else(|| tuple_arity_mismatch_hint(&annot_ty, &expr_ty))
                            } else {
                                None
                            };
                            // The value is an UNAPPLIED function where a non-function is annotated — a
                            // partial application `(h 1)` or a bare fn name `h` used as a value. The
                            // "annotation Int64 does not match value (-> Int64 Int64)" render never says
                            // the value is simply a function you forgot to finish calling; name the slip
                            // and how many arguments remain. Tail only (no mechanical fix — which argument
                            // values were meant is unknown), after the wrap/option/record chain found none.
                            let fn_tail =
                                if wrap.is_none() && option_tail.is_none() && record_tail.is_none()
                                {
                                    fn_not_applied_hint(&annot_ty, &expr_ty)
                                } else {
                                    None
                                };
                            // Same collection KIND (both List/Map/Set) but an element/key/value TYPE
                            // differs — `(Map String Int64)` where `(Map Int64 Int64)` is annotated. Name
                            // the differing AXIS instead of leaving the reader to diff two full renders, the
                            // collection twin of the record/tuple per-member hint. Tail only (no fix — the
                            // repair is retyping the elements). Last in the chain, after the others found none.
                            let collection_tail = if wrap.is_none()
                                && option_tail.is_none()
                                && record_tail.is_none()
                                && fn_tail.is_none()
                            {
                                collection_element_mismatch_hint(&annot_ty, &expr_ty)
                            } else {
                                None
                            };
                            // Same SUM type whose payload type-arg differs — `(Option Float64)` vs `(Option
                            // Int64)`. Names the payload axis, the sum twin of the collection-axis hint;
                            // last in the chain after the others found none.
                            let sum_tail = if wrap.is_none()
                                && option_tail.is_none()
                                && record_tail.is_none()
                                && fn_tail.is_none()
                                && collection_tail.is_none()
                            {
                                sum_payload_mismatch_hint(&annot_ty, &expr_ty)
                            } else {
                                None
                            };
                            // Two FUNCTION types that differ in ARITY or RESULT — `(-> Int64 Bool)` where
                            // `(-> Int64 Int64)` is expected (a callback of the wrong return type), or a
                            // 2-arg fn where a 1-arg is wanted. `fn_not_applied_hint` (fn_tail) only covers
                            // an UNAPPLIED function; two genuinely-different signatures fall through it, and
                            // the curried arrow render (`(-> Int64 (-> Int64 Int64))`) is hard to diff by eye.
                            // Name the differing part (result / arity), the function twin of the collection/
                            // sum axis hints. Last in the chain (a same-arity PARAMETER difference surfaces at
                            // the inner position on its own, so this only adds the result/arity axis).
                            let fn_sig_tail = if wrap.is_none()
                                && option_tail.is_none()
                                && record_tail.is_none()
                                && fn_tail.is_none()
                                && collection_tail.is_none()
                                && sum_tail.is_none()
                            {
                                fn_signature_delta_hint(&annot_ty, &expr_ty)
                            } else {
                                None
                            };
                            // LAST resort: two DISTINCT types that RENDER to the SAME name — "an Int64,
                            // but a value of type Int64 is expected" — which happens when a user type
                            // SHADOWS a prelude type name (`(type Int64 (A))`). None of the structural
                            // hints fire (the names match, so there is no field/axis/arity delta to name),
                            // so without this the reader sees a bare self-contradiction. Explain that the
                            // names collide via a shadowing declaration. After every structural hint.
                            let same_name_tail = if wrap.is_none()
                                && option_tail.is_none()
                                && record_tail.is_none()
                                && fn_tail.is_none()
                                && collection_tail.is_none()
                                && sum_tail.is_none()
                            {
                                same_name_distinct_type_hint(&annot_ty, &expr_ty)
                            } else {
                                None
                            };
                            let tail = wrap
                                .as_ref()
                                .map(|w| w.3.clone())
                                .or(option_tail)
                                .or(record_tail.clone())
                                .or(fn_tail)
                                .or(collection_tail.clone())
                                .or(sum_tail.clone())
                                .or(fn_sig_tail)
                                .or((!qty_tail.is_empty()).then(|| qty_tail.clone()))
                                .or(same_name_tail)
                                .unwrap_or_default();
                            let mut reject =
                                Reject::coded(Code::TypeMismatch, mismatch_lead(&tail));
                            if let Some((prefix, suffix, verb, _)) = wrap {
                                reject = reject
                                    .with_fix(Fix::wrap_heuristic(expr, prefix, suffix, verb));
                            } else if let Some(fix) = qty_fix {
                                // A CALL ARGUMENT's bare-number→quantity `(Qty.of … <unit>)` wrap (built via
                                // `qty_coercion_fix`, not the `wrap` tuple; the arg node splices cleanly).
                                reject = reject.with_fix(fix);
                            } else if record_tail.is_some()
                                || collection_tail.is_some()
                                || sum_tail.is_some()
                            {
                                // A MISSPELLED FIELD in a written record literal passed where a specific
                                // record type is expected — `(g (record (fooo 1)))` for a `(: r (Record (foo
                                // Int64)))` param — is a one-shot RENAME (`fooo` → `foo`), the argument-
                                // position twin of the member-access `(. r fooo)` did-you-mean fix. Tried
                                // FIRST (a field-SET typo, not a leaf-type coercion); `record_field_typo_fix`
                                // fires only on a confident single extra↔missing pairing over a directly-
                                // written literal, so it never mis-guesses an ambiguous multi-field slip.
                                // Otherwise the same-shape compound whose single differing leaf is a numeric
                                // literal gets the coercion fix (`(record (x 5))` vs `(Record (x Float64))` →
                                // retype `5`→`5.0`), via `compound_inner_coercion_fix` (M116). A non-literal /
                                // non-numeric leaf yields None (message only).
                                if let Some(fix) =
                                    record_field_typo_fix(db, &annot_ty, &expr_ty, expr)
                                        .or_else(|| {
                                            compound_inner_coercion_fix(
                                                db, expr, &annot_ty, &expr_ty,
                                            )
                                        })
                                        // A pure field-SET diff over a written record literal — ADD the missing
                                        // fields (`(field (trap "TODO"))` placeholders) or DELETE a lone surplus one,
                                        // the construction analogue of rustc's applicable "add/remove field" edit.
                                        .or_else(|| {
                                            record_field_add_fix(db, &annot_ty, &expr_ty, expr)
                                        })
                                        .or_else(|| {
                                            record_field_delete_fix(db, &annot_ty, &expr_ty, expr)
                                        })
                                        // The TUPLE-arity analogue: too few elements get `(trap "TODO")`
                                        // appended, one too many gets the trailing element deleted.
                                        .or_else(|| {
                                            tuple_element_add_fix(db, &annot_ty, &expr_ty, expr)
                                        })
                                        .or_else(|| {
                                            tuple_element_delete_fix(db, &annot_ty, &expr_ty, expr)
                                        })
                                {
                                    reject = reject.with_fix(fix);
                                }
                            }
                            out.push(reject);
                        }
                    }
                }
            } else if !runtime_width {
                // The TYPE OPERAND does not denote a type — an unbound name, an integer/compound VALUE,
                // an arbitrary expression, or a non-constructor type applied to arguments (`(Int64 Int64)`).
                // The type position REQUIRES a type, so this is REJECTED, not dropped-and-ignored. The
                // SHARED validator (M125): a record-bearing annotation type (`(: v (Record (x Nonesuch)))`)
                // names only the bad field TYPE, not the label `x` (the naive value-`collect` mis-resolved
                // labels); an unbound name → CDZ0101; a well-formed non-type → the "expected a type" CDZ0203.
                // (A RUNTIME WIDTH also makes `typeval_of` return None but is already reported CDZ0302
                // above — excluded here so it is not double-faulted.)
                trace!(target: "rcdzc::infer", node = id.0, "fault: annotation type position is not a type");
                validate_non_type_annotation(
                    db,
                    ty_expr,
                    "the type position of an annotation",
                    out,
                );
            }
            // A FLOAT LITERAL annotated `Float32` that OVERFLOWS the Float32 range — `(: 1.0e300 Float32)`
            // — is finite as the literal's default `Float64` but rounds to `±inf` in `Float32`, a value
            // with no written form (CDZ0302, `numeric-model.md` §A Floating-Point Literal That Denotes No
            // Representable Value Is Malformed) — the float analogue of an out-of-range integer literal.
            // Checked independently of the annot-agreement branches above (a deferred float literal unifies
            // fine with `Float32`, so the mismatch path never fires), via the shared `literal_width_fault`
            // (the same fit-check the let-binder runs), so a value annotation surfaces it in `cdz check`.
            if let Some(reject) = literal_width_fault(db, expr, ty_expr) {
                out.push(reject);
            }
            collect(db, expr, out);
        }
        // EFFECT CONTROL FORMS: descend into every executed sub-expression so a fault inside is caught
        // regardless of whether lowering can yet run the form. A handler's init, each arm's PERFORM
        // (the op projection — a perform-argument type mismatch surfaces here as an ordinary application
        // check via the op value's `(meta t)` arrow) and body, and the handled body all participate in
        // well-formedness. The arm's param/state binders are binder occurrences (not collected as
        // values). This is what makes a wrong-type perform argument a CDZ0203 even while the handler
        // itself declines to run (E1a).
        Resolved::Handle { init, arms, body } => {
            collect(db, init, out);
            // A HANDLER BINDS EACH OPERATION AT MOST ONCE. A handler's arms ARE its effect's operation set
            // (like a record's fields or an effect's op declarations — a FIXED set), so binding the same
            // operation twice — `(handle E s ((emit …) (emit …)) …)` — is the same closed-set
            // ill-formedness a duplicate record field (CDZ0201) or duplicate effect-op declaration is:
            // the second arm is dead (the first discharges the op), never reached. Reject CDZ0201 with a
            // delete fix on the redundant arm, the effect-handler analogue of the duplicate-field/op/export
            // family. Keyed by `(effect-decl, op-name)` (`arm_op_identity`) so two effects each declaring
            // `emit` never false-collide; an UNDECLARED-op arm has no identity (its own CDZ0403 fires
            // below), so it never participates here.
            let mut seen_arm_ops: std::collections::HashSet<(u32, String)> =
                std::collections::HashSet::new();
            for arm in arms.iter() {
                if let Some(identity) = crate::effects::arm_op_identity(db, arm.op)
                    && !seen_arm_ops.insert(identity.clone())
                {
                    // The op-key occurrence carries the arm's op-name source span (the desugar-synthesized
                    // projection is spanless) — anchor there, like the CDZ0403 undeclared-op report.
                    let anchor = crate::effects::arm_op_key_occ(db, arm.op).unwrap_or(arm.op);
                    let mut reject = Reject::coded(
                        Code::Malformed,
                        format!(
                            "operation `{}` is handled more than once in this handler (a handler binds \
                             each of its effect's operations at most once)",
                            identity.1
                        ),
                    )
                    .at(anchor);
                    // Delete the redundant `(op (params…) state body)` arm form — the enclosing list of the
                    // op-key occurrence's projection. The op key is `k` in `(. E k)`; its projection's
                    // parent is the arm form.
                    if let Some(key_occ) = crate::effects::arm_op_key_occ(db, arm.op)
                        && let Some(proj) = db.parent_of(key_occ)
                        && let Some(arm_form) = db.parent_of(proj)
                        && matches!(db.ast.get(arm_form), crate::ast::Struct::List(_))
                    {
                        reject = reject.with_fix(crate::diag::Fix::delete_heuristic(
                            arm_form,
                            format!("remove the duplicate `{}` arm", identity.1),
                        ));
                    }
                    out.push(reject);
                }
                // A HANDLER ARM NAMES AN UNDECLARED OPERATION (CDZ0403). If the arm's op is `(. E k)`
                // where `E` is an effect but `k` is not one of its declared operations, that is a
                // closed-set violation (`capabilities-and-effects.md` §A Handler Arm Names An Operation
                // Its Effect Declares) — CDZ0403, NOT the generic "record has no field" (CDZ0201) the
                // member projection would otherwise emit. When it fires, skip the generic `collect` of the
                // op (which would add the CDZ0201 duplicate).
                if crate::effects::arm_op_names_undeclared_operation(db, arm.op) {
                    // Anchor at the op-KEY occurrence, NOT `arm.op`: the desugar synthesizes the arm's op
                    // projection `(. E k)` (spanless), so `.at(arm.op)` maps to no source and the error
                    // loses its `file:line:col`; the key child carries the arm's op-name span. Fall back
                    // to the projection only if the key child is somehow absent.
                    let anchor = crate::effects::arm_op_key_occ(db, arm.op).unwrap_or(arm.op);
                    // Name the effect's DECLARED operations, two-tier (the effect-op analogue of
                    // `no_field_reject`): a confident typo → `` — did you mean `op`? `` + a replace fix on
                    // the mistyped key; a FAR miss → `` — closest matches: `a`, `b` `` listing the effect's
                    // ops (a closed set), so a mistyped arm never dead-ends at "does not declare" with no
                    // hint of what the effect actually offers. Only the confident single carries a fix.
                    match crate::effects::declared_op_hint(db, arm.op) {
                        Some((key_occ, hint, single)) => {
                            let mut reject = Reject::coded(
                                Code::HandlerUndeclaredOp,
                                format!(
                                    "this handler arm names an operation its effect does not declare{hint}"
                                ),
                            )
                            .at(key_occ);
                            if let Some(candidate) = single {
                                reject =
                                    reject.with_fix(Fix::replace_heuristic(key_occ, candidate));
                            }
                            out.push(reject);
                        }
                        None => out.push(
                            Reject::coded(
                                Code::HandlerUndeclaredOp,
                                "this handler arm names an operation its effect does not declare",
                            )
                            .at(anchor),
                        ),
                    }
                } else {
                    collect(db, arm.op, out);
                    // A HANDLER ARM BINDS ITS OPERATION'S PARAMETERS (CDZ0201). A declared op has a fixed
                    // parameter arity (its `(-> P… R)` arrow); an arm that binds the WRONG number of
                    // parameter binders is ill-formed the way a function applied at the wrong arity is.
                    // Before this: too FEW binders was SILENTLY ACCEPTED (the fold substituted a
                    // defaulted/absent binder), too MANY surfaced only the leaky "not yet reducible by the
                    // tail-resumptive fold" feature-decline — neither named the real defect. Name the
                    // operation and the expected/actual counts (the arm analogue of the over/under-
                    // application arity message). The helper honors the ELIDED-UNIT convention (`(-> Unit R)`
                    // accepts a 0- OR 1-binder arm) and returns `None` for an undeclared op (its CDZ0403
                    // above is the fault) or a malformed op with no type. Anchored at the op-key occurrence
                    // (the arm's op-name span), like the CDZ0403 report.
                    if let Some((op_name, expected, actual)) =
                        crate::effects::arm_param_arity_mismatch(db, arm)
                    {
                        let anchor = crate::effects::arm_op_key_occ(db, arm.op).unwrap_or(arm.op);
                        out.push(
                            Reject::coded(
                                Code::Malformed,
                                format!(
                                    "handler arm for operation `{op_name}` binds {actual} \
                                     parameter{} but the operation declares {expected} \
                                     (an arm binds exactly its operation's parameters)",
                                    if actual == 1 { "" } else { "s" },
                                ),
                            )
                            .at(anchor),
                        );
                    }
                    // A handler arm's OPERATION-PARAMETER list is a binder position, LINEAR like a def's or
                    // a lambda's — `(two (x x) s …)` binds `x` twice, silently reading one and shadowing
                    // the other (the same miscompile a duplicate def/lambda param was). The same CDZ0102 +
                    // rename fix, wherever a parameter list is written (the M121 sibling-site sweep — this
                    // was the remaining binder-list form that skipped the check).
                    param_list_linearity_faults(db, &arm.params, out);
                }
                // RESUME-VALUE / RESULT-TYPE CHECK. The value a handler resumes with — `(resume value
                // state)` — is returned to the perform site, so it MUST have the operation's declared
                // RESULT type (`capabilities-and-effects.md` §Performing An Operation Is Typed And
                // Contributes To The Row — a perform 'yields the operation's declared result type', so an
                // operation is typed exactly as a function application whose body must return the declared
                // type). A mismatch — `(resume true s)` for an `(-> Int64 Int64)` op — is CDZ0201 (the
                // result-type companion of the perform-argument check). Without this the fold silently
                // substitutes the mistyped value as the perform's result (a type-confusion miscompile).
                check_resume_result_type(db, arm, out);
                // NEXT-STATE / SEED-TYPE CHECK. A handler folds a STATE across the operations its body
                // performs, threading it purely (`capabilities-and-effects.md` §Discharging An Operation
                // Produces … The Next State Carried Forward). The state's type is fixed by the handle's
                // SEED (`init`); the NEXT state in `(resume value next-state)` continues that fold, so it
                // MUST have the seed's type. A mismatch — `(resume 5 "x")` under an Int64 seed — would
                // change the state's type mid-fold; it was SILENTLY ACCEPTED (the fold dropped the type
                // discrepancy, a type-confusion miscompile), the state-side companion of the resume-VALUE
                // check above. CDZ0201, anchored at the next-state.
                check_resume_next_state_type(db, init, arm, out);
                collect(db, arm.body, out);
            }
            // A HANDLER MUST DISCHARGE ITS EFFECT'S WHOLE OPERATION SET (CDZ0405,
            // `capabilities-and-effects.md` §A Handler Discharges Its Effect). A `handle E` names one
            // effect whose operations are a closed set, so — like a match over a sum — it must bind EVERY
            // operation; a handler missing one leaves that operation of the effect it claims to discharge
            // without a home. Only checked when NO arm named an undeclared operation (a CDZ0403 arm makes
            // the discharged effect ambiguous, so its own fault is the one to report). Names the omitted
            // operations AND carries an "add the missing arm" fix — a template arm per omission appended
            // to the arms LIST (the sibling of `non_exhaustive_sum_reject`'s missing-match-arm fix).
            let has_undeclared = arms
                .iter()
                .any(|arm| crate::effects::arm_op_names_undeclared_operation(db, arm.op));
            if !has_undeclared {
                let missing = crate::effects::handler_missing_operations(db, &arms);
                if !missing.is_empty() {
                    out.push(non_exhaustive_handler_reject(db, id, &missing));
                }
            }
            collect(db, body, out);
        }
        Resolved::Resume { value, next_state } => {
            collect(db, value, out);
            collect(db, next_state, out);
        }
        Resolved::Host { body, .. } => {
            // The effect names are label occurrences (an effect name resolves to its record); descend
            // into the delegated body for its faults.
            collect(db, body, out);
        }
        // A scope-error poison (an unbound name) is UNCONDITIONAL well-formedness — report it here,
        // where the walk descends into EVERY position (including an `if`'s branches), so an unbound
        // name in an untaken branch or an uncalled definition is still rejected (`core-semantics.md`
        // §Binding Is Lexical — not gated on reachability). A DECLINE or a compile-provable TRAP
        // poison is NOT reported here: a decline is not a well-formedness fault, and a trap is
        // reachability-gated (the trap-poison walk in `compile` handles it, skipping untaken branches).
        Resolved::Poison(r) => {
            if r.code == Some(Code::Unbound) {
                trace!(target: "rcdzc::infer", node = id.0, "fault: unbound name reported (CDZ0101)");
                out.push(enrich_unbound(db, id, r));
            } else if matches!(
                r.code,
                // A LEXICAL / FORM well-formedness poison a node resolves WHOLE to — a malformed numeric
                // literal (`0o17`, `12abc`, an over-i64 bare literal), a float outside the Float64 range, an
                // unrecognized string escape (CDZ0001), a char naming a non-scalar (CDZ0002), or a
                // malformed `(bin …)` form whose segment list does not resolve (`(bits v -1)` — a
                // non-natural bit-field width, CDZ0220). Like an unbound name these are UNCONDITIONAL
                // well-formedness (a defect of the token/form itself, independent of whether the definition
                // is reached), but `collect_node`'s poison arm only surfaced `Unbound` — so such a defect in
                // a PARAMETERIZED or non-exported body PASSED `cdz check` while `compile` (on a reached
                // body) rejected it, the same "check misses a resolve-only reject on an unreached body" hole
                // M81's pattern accessor / the `(do)`-block poison close. Surface the coded poison here,
                // anchored at the node; `dedup_faults` collapses it against any copy the emit walk produces
                // at the same node on a reached body.
                Some(Code::Malformed | Code::BadEscape | Code::BadChar | Code::IllFormedBinary)
            ) {
                trace!(target: "rcdzc::infer", node = id.0, code = ?r.code, "fault: lexical well-formedness poison reported");
                let mut r = r;
                r.set_origin_if_absent(id);
                out.push(r);
            }
        }
        // A ref's target-node fault is reported when that node is collected on its own. A bare
        // intrinsic value and the atomic leaves have no sub-faults. A `SumPayload` (a variant binder's
        // payload read) has no sub-fault of its own — the scrutinee's faults surface at the match.
        // A bare integer literal whose width DEFAULTED (nothing annotated/inferred it, so it grounds to
        // the default signed `Int64`) but whose VALUE does not fit signed-64 is a MALFORMED literal
        // (CDZ0201) — `9223372036854775808` (Int64.max+1), `0xFFFFFFFFFFFFFFFF` (fits UNSIGNED-64 but a
        // bare literal is signed): a number with no width to blame, not a name and not an out-of-range-
        // for-a-chosen-width (which stays CDZ0302, checked at the annotation in the `Annot` arm). 01-
        // literals "an out-of-range integer literal is a malformed literal, not an unbound name".
        Resolved::Int(v) => {
            // A literal that is the direct operand of an annotation `(: LIT T)` is checked against `T`
            // by the `Annot` arm (CDZ0302 on an over-width annotated literal); its own deferred type
            // here would misfire. So skip an annotated literal — only a literal with NO width in sight
            // (defaulted to `Int64`) is judged malformed here.
            let annotated = db
                .parent_of(id)
                .and_then(|p| db.ast.as_form(p, ":"))
                .and_then(|t| t.first().copied())
                == Some(id);
            // A literal that is an OPERAND of an integer binary op takes its sibling's CONCRETE type
            // (numeric-model.md §a constraint on a literal takes precedence): `(& x 0xFFFF…FFFF)` with
            // `x : UInt64` fixes the literal to UInt64, whose range holds 2^64-1 — so it is NOT malformed,
            // even though the same value overflows the signed-64 DEFAULT. Only UInt64 exposes this gap (it
            // has representable values above i64::MAX); UInt8/16/32 literals always fit i64, and the
            // selection path already grounds the literal to the shared width. Fit against the CONTEXTUAL
            // type when there is one, else the Int64 default.
            let context = literal_binop_context_ty(db, id);
            let (signed, width) = match context {
                Some(it) => (it.ground_signed(), it.ground_width()),
                None => (true, crate::ty::DEFAULT_INT_WIDTH),
            };
            if !annotated
                && let Ty::Int(it) = type_of(db, id)
                && !it.width_is_fixed()
                && !v.fits_width(signed, width)
            {
                // Name the type the value actually overflowed: the CONTEXTUAL type when one fixed it (a
                // literal too big even for its `UInt64` operand), else the Int64 default (a bare literal
                // with no width in sight). Both are CDZ0201 — a number with no annotation to blame.
                let ty_name = context
                    .map(|it| Ty::Int(it).render_name())
                    .unwrap_or_else(|| "Int64".to_string());
                trace!(target: "rcdzc::infer", node = id.0, "fault: integer literal exceeds its width (malformed, CDZ0201)");
                // Name the valid RANGE the literal overflowed (as the annotated-width CDZ0302 does), so a
                // bare huge literal explains WHAT it exceeded rather than a terse "out of range". When the
                // overflowed type is the Int64 DEFAULT (a bare literal, no context), also note it is the
                // widest fixed integer — the honest current story (the numeric model reserves wider values
                // to a big-integer layer not yet constructible), so the author knows the value simply has
                // no representable fixed type rather than expecting a `--from`-style flag.
                // A bare literal overflowing the signed-Int64 default that STILL FITS UNSIGNED 64 (`2^63 ..
                // 2^64-1`, e.g. `18446744073709551615`) has a concrete fixed type — `UInt64` — it just is
                // not the default a bare literal takes. The mechanical repair: ANNOTATE it `(: <lit>
                // UInt64)` (the annotation grounds the literal to UInt64, whose range holds it). A value
                // PAST 2^64-1 fits no FIXED width — but `BigInt` holds an integer literal of ANY magnitude
                // (`(: <lit> BigInt)` grounds the literal to the arbitrary-precision type, a TOTAL one-shot
                // repair), so it gets a fix too. Both only in the BARE case (`context.is_none()` — a
                // literal fixed by a UInt64/other operand has already picked its type). `fits_u64` chooses
                // between the UInt64 and BigInt fix so the message names the tightest type that holds it.
                let bare = context.is_none();
                let fits_u64 = bare && v.fits_width(false, 64);
                let past_fixed = bare && !fits_u64; // no fixed width holds it — BigInt does
                // A literal that is the ARGUMENT of `BigInt.of` — `(BigInt.of 999…)`. `BigInt.of` widens a
                // FIXED integer (`∀a. (Int a) → BigInt`), so a literal too big for every fixed width can
                // NEVER be its argument — and the annotate-`(: … BigInt)` wrap would cascade (it produces
                // `(BigInt.of (: … BigInt))`, a BigInt where a fixed int is wanted). The real repair is to
                // DROP the redundant `BigInt.of` and write the value as a `BigInt`-annotated literal
                // directly. We cannot cheaply spell that replacement here (the arbitrary-precision literal's
                // decimal text is not reconstructable from the magnitude at this layer), so — honest-no-fix
                // — we name the repair in the MESSAGE and offer NO cascading wrap fix.
                let in_bigint_of = past_fixed
                    && db.parent_of(id).is_some_and(|p| {
                        matches!(resolved_of(db, p), Resolved::Apply { head, .. }
                            if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::BigIntOf))
                    });
                let msg = match int_width_range(signed, width) {
                    Some(range) if fits_u64 => format!(
                        "integer literal is out of range for {ty_name} (the valid range is {range}) — \
                         it fits `UInt64`; annotate it `(: … UInt64)`"
                    ),
                    Some(range) if in_bigint_of => format!(
                        "integer literal is out of range for {ty_name} (the valid range is {range}) — \
                         `BigInt.of` widens a fixed-size integer, so it cannot hold this value; write the \
                         literal directly as a `BigInt` with `(: … BigInt)` instead of `(BigInt.of …)`"
                    ),
                    Some(range) if past_fixed => format!(
                        "integer literal is out of range for {ty_name} (the valid range is {range}; no \
                         fixed-size integer is wider) — it fits `BigInt`; annotate it `(: … BigInt)`"
                    ),
                    Some(range) => format!(
                        "integer literal is out of range for {ty_name} (the valid range is {range})"
                    ),
                    None => format!("integer literal is out of range for {ty_name}"),
                };
                let mut reject = Reject::coded(Code::Malformed, msg);
                if fits_u64 {
                    reject = reject.with_fix(Fix::wrap_heuristic(
                        id,
                        "(: ",
                        " UInt64)",
                        "annotate the literal `UInt64` (its range holds this value)",
                    ));
                } else if past_fixed && !in_bigint_of {
                    // `BigInt` holds an integer literal of any magnitude — the total repair for a value no
                    // fixed width can represent. The annotation grounds the literal to the big-integer type.
                    // NOT offered inside `(BigInt.of …)`: there the wrap cascades (see `in_bigint_of`), so
                    // that case carries the drop-the-wrapper message with no fix.
                    reject = reject.with_fix(Fix::wrap_heuristic(
                        id,
                        "(: ",
                        " BigInt)",
                        "annotate the literal `BigInt` (it holds an integer of any magnitude)",
                    ));
                }
                out.push(reject);
            }
        }
        Resolved::Prim(_)
        | Resolved::Ref { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::Param { .. }
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Float(_)
        | Resolved::Unit
        | Resolved::TypeVal(_) => {}
        // An anonymous LAMBDA `(fn (params…) body)`: check its parameter list is LINEAR, exactly as a
        // top-level def's is (`(fn (x x) …)` shadowed the first `x` and silently bound nothing). The body
        // is checked at the application site (β-reduction), so only the param-linearity is added here.
        Resolved::Lambda { params, .. } => {
            param_list_linearity_faults(db, &params, out);
        }
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
        assert_eq!(t.render_name(), "Int64");
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
            scheme.ty.render_name()
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
            call_ty.render_name(),
            result.render_name()
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
            scheme.ty.render_name()
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
            scheme.ty.render_name()
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
            sa.ty.render_name(),
            sb.ty.render_name(),
            "recursive param solve must be order-independent"
        );
    }
}
