//! `infer::application` — application type-checking (`check_application`), extracted verbatim from
//! `infer.rs` to keep the parent module under the source-size limit. Behavior + API unchanged: the
//! item stays crate-private and is re-imported into `infer` via `use application::*`.

use super::*;

/// The fault (if any) for NEGATING an operand of type `t` — shared by the unary `-` (`Prim::Sub` arity-1)
/// and `Num.neg` (`Prim::Neg`) checks. A DEFINITELY-UNSIGNED integer has no negation (its domain has no
/// sign): `CDZ0310` `UnsignedNegation`, a COMPILE-TIME reject rather than the const-overflow / runtime trap
/// it degrades to today. A NON-number operand is `CDZ0201` `Malformed` (Cadenza never coerces to a number).
/// A SIGNED number (`Int`/`Float`/`Rational`/`BigInt`/`Qty`), an undetermined `Any`, or a not-yet-fixed
/// sign (`Deferred`/`Var` — may still ground to signed) is NOT a fault here (`None`).
fn negate_operand_fault(db: &mut Db, t: &Ty, _at: StructId) -> Option<Reject> {
    // A DEFINITELY-unsigned integer (a FIXED unsigned sign; a Deferred/Var sign may still resolve to
    // signed, so it is not rejected). Negation is undefined on an unsigned domain.
    if let Ty::Int(it) = t
        && it.sign == crate::ty::Sign::Fixed(false)
    {
        trace!(target: "rcdzc::infer", ty = %t.render_name(&db.name_ctx()), "fault: negation of an unsigned integer (CDZ0310)");
        return Some(Reject::coded(
            Code::UnsignedNegation,
            format!(
                "negation is not defined on {} — an unsigned integer has no sign to negate; use a \
                 signed integer type (e.g. Int64) if you need negative values",
                t.render_name(&db.name_ctx())
            ),
        ));
    }
    // A NON-number operand: negation is not defined (never coerced to a number). Same CDZ0201 the binary
    // arithmetic-on-a-non-number reject uses.
    if !matches!(
        t,
        Ty::Int(_) | Ty::Float(_) | Ty::Rational | Ty::BigInt | Ty::Qty { .. } | Ty::Any
    ) {
        trace!(target: "rcdzc::infer", ty = %t.render_name(&db.name_ctx()), "fault: negation of a non-numeric operand (CDZ0201)");
        return Some(Reject::coded(
            Code::Malformed,
            format!(
                "negation is not defined on {} — Cadenza never coerces this to a number",
                t.render_name(&db.name_ctx())
            ),
        ));
    }
    None
}

pub(crate) fn check_application(
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
                        ty.render_name(&db.name_ctx())
                    ),
                )
                .at(args[0]),
            );
        }
        // The operand's own faults are collected by the caller's `collect(head/operand)`; return so the
        // generic scheme-unify below does not ALSO fault (the intrinsic has no HM scheme).
        return;
    }
    // `ast-lift` (the quasiquote desugar's `(intrinsic "ast-lift") e` around a runtime active-unquote
    // operand) accepts ANY operand type (`∀a. a → Ast`) — the lift is by the operand's inferred type at
    // lower. So it imposes NO operand constraint here; return so the generic scheme-unify does not fault
    // (the intrinsic has no HM scheme). The operand's own faults are collected by the caller.
    if crate::eval::prim_of(db, head) == Some(crate::resolved::Prim::AstLift) && args.len() == 1 {
        return;
    }
    // ABSTRACT-TYPED MAP/SET KEY — CDZ0202. A CHAMP Map/Set keyed by a value of an ABSTRACT type (imported
    // handle, ctors withheld) observes the equality of the module's PRIVATE representation at insert/lookup
    // (`champ_eq`/value-eq over the key spine) — the SAME `type-system.md §An Abstract Type's Representation
    // Is Not Observable Across Its Boundary` violation as a direct `(=)`, an INDIRECT route (breaker,
    // concierge-ruled). Values stay legal to HOLD (payloads); only the key-EQUALITY-observation rejects.
    // Gate the collection-CONSTRUCTION prims (you cannot obtain an abstract-keyed collection without building
    // one): read the RESULT type's key/elem — `Ty::Map(k, _)` / `Ty::Set(k)` — and reject if `k` is abstract
    // AT THIS SITE. Reuses the direct-eq predicate shape (`nominal_or_sum_decl` + `is_abstract_type_at`);
    // gating on `is_abstract_type_at` (NOT bare visibility) excludes a prelude/concrete/own key (an Int64 /
    // prelude-Option key never flags) — only a genuinely handle-only imported key type. A comparison FUNCTION
    // the module exports stays the sanctioned route.
    //= spec/capabilities/type-system.md#an-abstract-type-s-representation-is-not-observable-across-its-boundary
    //# A built-in structural comparison whose operand is a value of an abstract type — a type whose handle a module made visible without making its constructors visible ([modules-and-namespaces.md](modules-and-namespaces.md) §A Type's Handle And Its Constructors Are Independently Visible) — MUST be rejected outside the declaring module, so that the abstract type's representation is not observed through equality and a module that wants its abstract type compared publishes a comparison operation rather than exposing its structure.
    if matches!(
        crate::eval::meta_apply_of(db, head),
        Some(
            // CONSTRUCTION — the key type is the RESULT's key (you build an abstract-keyed collection).
            crate::resolved::Prim::SetOf
                | crate::resolved::Prim::SetInsert
                | crate::resolved::Prim::MapNew
                | crate::resolved::Prim::MapInsert
                // LOOKUP / MEMBERSHIP / SET-ALGEBRA — the collection arrives as an ARGUMENT (a param, an
                // import, a returned collection), so a construction gate alone is bypassed; these prims
                // STILL compare/hash the stored keys structurally (`champ_eq`) at lookup/membership/union,
                // observing the abstract key rep the same way. The key type is read off the OPERAND's
                // `Ty::Set(k)`/`Ty::Map(k,_)`, so collect from every argument below.
                | crate::resolved::Prim::SetContains
                | crate::resolved::Prim::SetRemove
                | crate::resolved::Prim::SetUnion
                | crate::resolved::Prim::SetIntersection
                | crate::resolved::Prim::SetDifference
                | crate::resolved::Prim::MapLookup
                | crate::resolved::Prim::MapRemove
                | crate::resolved::Prim::MapSwap
                | crate::resolved::Prim::MapTake
        )
    ) {
        // The key type can appear as the RESULT's key (construction prims yield the collection) OR as an
        // ARGUMENT's key (lookup/membership/algebra prims take the collection). Collect candidate key
        // types from both and reject if ANY contains an abstract type at this site. A `Ty::Set(k)`/
        // `Ty::Map(k,_)` in either position contributes its key.
        let key_of = |t: &Ty| -> Option<Ty> {
            match t {
                Ty::Map(k, _) => Some((**k).clone()),
                Ty::Set(k) => Some((**k).clone()),
                _ => None,
            }
        };
        let mut key_tys: Vec<Ty> = Vec::new();
        let result_ty = type_of(db, app);
        if let Some(k) = key_of(&result_ty) {
            key_tys.push(k);
        }
        for &a in args.iter() {
            let at = type_of(db, a);
            if let Some(k) = key_of(&at) {
                key_tys.push(k);
            }
        }
        // Anchor at `head` (the prim application's head) — `push_unhashable_key_fault` builds the CDZ0202
        // (abstract key) / CDZ0216 (function key) rejects; the native `#set`/`#map` LITERAL arms in `collect`
        // call the SAME helper anchored at the literal node, so the two surfaces enforce one constraint.
        for k in &key_tys {
            if push_unhashable_key_fault(db, head, k, out) {
                return;
            }
        }
    }
    // `(trap MESSAGE)` — the abort primitive `trap : ∀a. String → a`. Its message MUST be a String; a
    // non-String message (`(trap 5)`, `(trap true)`) is a type error. But `trap`'s RESULT is the polymorphic
    // `a` (it inhabits any type), so the generic scheme-unify grounds the operand parameter to `String` and
    // reports the OPAQUE "type mismatch: String and Int64 must be the same type here" — reads like an
    // internal clash, not "the message you passed is not a String". Name the real fault (the operand-type
    // twin of the member-op "`String.at` expects an argument of type Int64, but a value of type String was
    // given" message). Fires only on a definite non-String message (a `Var`/`Any` operand is unsolved —
    // never a false reject); the wrong-ARITY `(trap "a" "b")` keeps its coded CDZ0203 (the over-application
    // path). Descend into the operand for its own faults, then return so the scheme-unify does not re-fault.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Trap) && args.len() == 1
    {
        let msg_ty = type_of(db, args[0]);
        if !matches!(msg_ty, Ty::String | Ty::Any | Ty::Var(_)) {
            trace!(target: "rcdzc::infer", head = head.0, ty = %msg_ty.render_name(&db.name_ctx()), "fault: trap message is not a String (CDZ0203)");
            out.push(
                Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "`trap`'s message must be a String, but a value of type {} was given — \
                         `trap` aborts with a text message, e.g. `(trap \"reason\")`",
                        msg_ty.render_name(&db.name_ctx())
                    ),
                )
                .at(args[0]),
            );
        }
        collect(db, args[0], out);
        return;
    }
    // `(read TEXT)` — the reader primitive `read : String → Ast` (parses re-readable text back into an
    // `Ast`). Its operand MUST be a String; a non-String operand (`(read 5)`, `(read true)`) is a type
    // error. The `trap` sibling: the generic scheme-unify grounds the operand parameter to `String` and
    // reports the OPAQUE "type mismatch: String and Int64 must be the same type here" (the `read` RESULT is
    // the fixed `Ast`, but the OPERAND-side unify still leaks the phantom `String` clash — unlike `print :
    // Ast → String`, whose distinctive `Ast` operand type the checker names directly). Name the real fault,
    // exactly as the `trap` arm above. Definite non-String only (`Var`/`Any` skip); a wrong-arity `(read)`
    // / `(read a b)` keeps its own decline. Descend the operand, then return (the scheme-unify would only
    // re-report the phantom clash).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Read) && args.len() == 1
    {
        let arg_ty = type_of(db, args[0]);
        if !matches!(arg_ty, Ty::String | Ty::Any | Ty::Var(_)) {
            trace!(target: "rcdzc::infer", head = head.0, ty = %arg_ty.render_name(&db.name_ctx()), "fault: read operand is not a String (CDZ0203)");
            out.push(
                Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "`read`'s argument must be a String, but a value of type {} was given — \
                         `read` parses text back into an `Ast`, e.g. `(read \"(+ 1 2)\")`",
                        arg_ty.render_name(&db.name_ctx())
                    ),
                )
                .at(args[0]),
            );
        }
        collect(db, args[0], out);
        return;
    }
    // BUILT-IN OPERATION applied at FEWER args than it takes — a PARTIAL APPLICATION as an UNCONSUMED
    // value (`(String.slice s 0)` — slice takes 3 — as a dead/unexported def body). `lower` rejects a
    // reached one (`BUILTIN_WRONG_ARITY_DECLINE`, "a built-in operation must be applied to exactly its
    // arguments — a partial application … is not yet built"), but an unexported/dead partial reaches
    // NEITHER `collect_reached_poisons` (nullary+exported only) NOR the lower path — so it was accepted by
    // BOTH `cdz check` AND `cdz compile` (shipped unflagged). This is the ONE pass over every body, so it
    // surfaces the hole. Co-designed with v-inference (owns this seam + the operation-vs-ctor semantics).
    //
    // The completion test is LOCAL, done in O(1) via the parent, so this per-node check never false-flags
    // an inner partial that an OUTER application saturates (the tick-108 regression class): fire ONLY when
    // `app` is a SPINE-TOP — its parent, peeled through ref/annot wrappers, is NOT an `Apply` feeding `app`
    // as its head (if it were, the parent brings more args → not partial here). A generic OPERATION prim is
    // NOT partially applicable (needs a runtime closure, unbuilt); a CONSTRUCTOR is (curried spine completes
    // / eta-lifts) — so exclude a ctor-headed spine that reaches its payload arity or eta-lifts. `Var`/`Any`
    // arities and non-prim heads (user fn / module member currying — legitimate) never reach the reject.
    // NB: the gate is `is_builtin_partial_application` ALONE — no immediate-head `meta_apply_of` pre-gate.
    // The predicate flattens the spine to its BOTTOM head and gates on THAT, so the nested-parens surface
    // `((String.slice s) 0)` (immediate head an `Apply`, `meta_apply_of` None) reaches the same test the
    // flat `(String.slice s 0)` does. A pre-gate on the immediate head would skip the nested form (the
    // PR#491 hole). The predicate returns fast-false for a non-op bottom head, so this is not costly.
    if is_builtin_partial_application(db, app, head, args) {
        // Name the operation off the spine's bottom `(. Module member)` head — the flat `(String.slice s
        // 0)` has that head directly; the nested `((String.slice s) 0)` reaches it by peeling the AST head
        // children. Peel on the RAW AST (a `List` whose first child is the deeper head), NOT the resolved
        // form (whose bottom is the op-prim value, not the `(. …)` node `member_op_head_name` reads).
        let mut name_head = app;
        while let crate::ast::Struct::List(kids) = db.ast.get(name_head) {
            match kids.first() {
                Some(&h) if matches!(db.ast.get(h), crate::ast::Struct::List(_)) => name_head = h,
                _ => break,
            }
        }
        let named = member_op_head_name(db, name_head)
            .map(|(m, k)| format!("`{m}.{k}`"))
            .unwrap_or_else(|| "a built-in operation".to_string());
        trace!(target: "rcdzc::infer", app = app.0, head = head.0, "fault: built-in operation partially applied as an unconsumed value (surfaced in check)");
        out.push(
            Reject::declined(
                crate::diag::DeclineId::PrimAsValueNeedsClosure,
                format!(
                    "{named} is applied at the wrong arity — a built-in operation must be applied to \
                     exactly its arguments; a partial application, which would require a synthesized \
                     runtime closure, is not supported"
                ),
            )
            .at(app),
        );
        for &arg in args {
            collect(db, arg, out);
        }
        return;
    }
    // `Qty.of <value> <unit>` — the SECOND argument must be a compile-time UNIT expression (`Unit.one`,
    // `(Unit.base #"m")`, `(Unit.of #"m")`, or a `Unit.*`/`Unit./`/`Unit.^` composition), read by
    // `eval::unit_of`. A second arg that is NOT a unit-builder form at all — a plain literal `(Qty.of 5
    // 5)`, a string `(Qty.of 5 "m")`, a tuple `(Qty.of 5 (tuple 1 2))` — made `unit_of` return `None`,
    // and `type_of`'s `Qty.of` arm SILENTLY fell through to `Any`, so `cdz check` passed — a quantity with
    // no real unit slipped by. Reject it (CDZ0201), the value-position twin of the `Qty`-TYPE unit message
    // (M213). GUARDS: (a) skip an arg whose head IS a `Unit.*` builder prim — a `(Unit.of #"zorks")` with
    // an UNKNOWN unit name is a real unit form with a bad name, handled by `check_unknown_units` with a
    // did-you-mean, so leave it be (else we'd shadow that richer message); (b) skip if the arg is itself
    // faulty (a bare unbound `(Qty.of 5 meter)` → its own CDZ0101). So this fires only on a well-formed
    // value that is plainly not a unit expression.
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyOf)
        && args.len() == 2
        && crate::eval::unit_of(db, args[1]).is_none()
    {
        let before = out.len();
        if is_unit_builder_form(db, args[1]) {
            // The unit arg IS a `Unit.*`/`Unit./`/`Unit.^` (or `Unit.of`) builder form, yet `unit_of`
            // declined — so a builder OPERAND is malformed (a non-unit factor, a non-int exponent) OR the
            // `Unit.of` names an unknown unit (handled by `check_unknown_units` with a did-you-mean, so we
            // leave THAT be — `check_unit_composition` only fires on the arithmetic composers). Name the bad
            // operand rather than skipping the whole form (the M235 check-miss: a malformed composition
            // slipped by `check` and leaked "no machine representation" at compile).
            check_unit_composition(db, args[1], out);
        } else {
            // Not a unit-builder form at all — a plain literal `(Qty.of 5 5)`, a string `(Qty.of 5 "m")`, a
            // tuple. Descend for the arg's own faults, then add the not-a-unit reject if it is otherwise
            // fault-free (else its own error is the primary one).
            collect(db, args[1], out);
            if out.len() == before {
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        "`Qty.of`'s second argument must be a UNIT expression, but this value is not one — \
                         write a unit, e.g. `(Unit.base #\"meter\")` for a base unit, `Unit.one` for the \
                         dimensionless unit, or a `Unit.*`/`Unit./`/`Unit.^` composition",
                    )
                    .at(args[1]),
                );
            }
        }
        // Descend into the VALUE arg for its own faults; the unit arg was handled above. Return so the
        // generic scheme-unify does not also fault (`Qty.of`'s unit is not an HM-typed argument).
        collect(db, args[0], out);
        return;
    }
    // A `Qty.of` whose UNIT is VALID but whose MAGNITUDE is NON-NUMERIC — `(Qty.of (tuple true) (Unit.base
    // #"gram"))`. `Qty.of`'s scheme `∀a. a → Unit → (Qty a u)` does NOT constrain the magnitude numeric, and
    // the result-type synth wraps whatever `a` is, so a compound/bool/string magnitude slips past `check`
    // to emit — where `Qty.pow`/scale assume a scalar magnitude and mis-width the boxed compound (an i32/i64
    // mismatch → INVALID WASM, v-rb reroute). A quantity's magnitude must be a numeric scalar; reject it here
    // (reject-don't-miscompile), the numeric analogue of the not-a-unit reject above. Only when the magnitude
    // is CONCRETELY non-numeric (an unsolved `Var`/`Any` is skipped — `agrees_with`/`ty_has_free_var` gate)
    // and otherwise fault-free (its own error, if any, is the primary one).
    if crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyOf)
        && args.len() == 2
        && crate::eval::unit_of(db, args[1]).is_some()
    {
        let before = out.len();
        collect(db, args[0], out); // the magnitude's own faults first
        if out.len() == before {
            let mt = type_of(db, args[0]);
            if !matches!(
                mt,
                Ty::Int(_) | Ty::Float(_) | Ty::Rational | Ty::BigInt | Ty::Any
            ) && !ty_has_free_var(db, &mt)
            {
                trace!(target: "rcdzc::infer", head = head.0, ty = %mt.render_name(&db.name_ctx()), "fault: a Qty magnitude is not numeric (CDZ0201)");
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "a quantity's magnitude must be a numeric value (Int, Float, Rational, or \
                             BigInt) — `Qty.of` here is applied to a `{}`",
                            mt.render_name(&db.name_ctx())
                        ),
                    )
                    .at(args[0]),
                );
            }
        }
        return;
    }
    // (Prefix negation `(- e)` is deprecated: arity-1 `Sub` is NOT handled here as negation — it CURRIES
    // like `(+ 1)` (a partial subtraction) and takes the ordinary binary scheme-unify below, which types
    // it as the curried arrow. `Num.neg`/`T.neg` (`Prim::Neg`, next arm) are the negation replacement and
    // carry the operand check.)
    // `(Num.neg e)` / a per-type `<T>.neg` (`Prim::Neg`, a UNARY negation over the number shape — the
    // generic `∀a. a → a` front `Num.neg` #7023 backs; the scheme is unconstrained, so the operand type is
    // checked HERE). Same operand rule as unary `-`: an UNSIGNED integer has no negation (CDZ0310), a
    // NON-number operand is CDZ0201 (Cadenza never coerces to a number) — the shared `negate_operand_fault`
    // (v-compiler-primitives co-design step-3 + operator's explicit unsigned-static-reject requirement;
    // code CDZ0310 assigned by v-deferral-declines). A signed-number operand is well-typed; descend for its
    // own faults and return so the ∀a scheme-unify adds no phantom.
    if args.len() == 1 && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Neg) {
        let t = type_of(db, args[0]);
        if let Some(reject) = negate_operand_fault(db, &t, args[0]) {
            out.push(reject);
        }
        collect(db, args[0], out);
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
    //= spec/capabilities/units-of-measure.md#an-explicit-conversion-unwraps-to-a-bare-number
    //# The chosen unit MUST share the quantity's dimension, so that a conversion across dimensions — a length into a duration — remains an error rather than silently producing a number.
    // `Qty.value q` recovers a quantity's underlying number — its operand MUST be a quantity. A NON-quantity
    // argument (commonly a bare number that is the result of a PRIOR `Unit.in`/`as`, which UNWRAPS to a bare
    // number — so `(Qty.value (Unit.in inch (Qty.of 5 foot)))` applies `Qty.value` to the bare `60`, NOT a
    // quantity) makes `apply_type`'s `QtyValue` arm return `Ty::Any` ("faulted elsewhere") — but nothing
    // faulted it, so the `Any`-typed result slipped past `cdz check` and declined only at the backend
    // ("function return type has no machine representation"), a check-vs-compile gap. Reject it here with a
    // coded CDZ0501 that names the operand's type + the repair: a conversion result is ALREADY the bare
    // number, so drop the `Qty.value`. Guarded to skip a genuine quantity + unsolved types (`Any`/`Var`),
    // which fault (or resolve) elsewhere. Mirrors the `Unit.in`-non-quantity reject just below.
    if args.len() == 1
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::QtyValue)
    {
        // Collect the OPERAND's own faults FIRST (into a temp), so an inner error is the primary
        // diagnostic — e.g. `(Qty.value (% (Qty …) (Qty …)))`: the `%` on a quantity is the real cause and
        // its type leaks to a bare `Int64`, which would otherwise trip THIS check with a misleading
        // "Qty.value … not a quantity" ahead of the honest "% not defined on quantities". Only emit the
        // non-quantity reject when the operand is itself well-typed (no inner fault) AND resolves to a
        // genuine non-quantity — the `(Qty.value 60)` / `(Qty.value (Unit.in …))` shape this targets.
        let mut operand_faults = Vec::new();
        collect(db, args[0], &mut operand_faults);
        let operand_clean = operand_faults.is_empty();
        out.append(&mut operand_faults);
        let operand_ty = type_of(db, args[0]);
        if operand_clean && !matches!(operand_ty, Ty::Qty { .. } | Ty::Any | Ty::Var(_)) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Qty.value operand is not a quantity (CDZ0501)");
            // The message names the operand's REAL type and says it is not a quantity — GENERALLY, since a
            // non-quantity operand can be ANY type (a `Bool`/`String`/`Record`, not only a bare number). The
            // "a conversion already unwrapped it" note is a REPAIR HINT that only applies when the operand is
            // itself NUMERIC (the common `(Qty.value (Unit.in …))` mistake, where the conversion result is
            // already the bare number) — so append it only for a numeric operand, not for e.g. a `Bool`
            // (where "a plain number" was self-contradictory: "this operand is a Bool — a plain number").
            let numeric = matches!(
                operand_ty.strip_nominal(),
                Ty::Int(_) | Ty::Float(_) | Ty::BigInt | Ty::Rational
            );
            let hint = if numeric {
                " (an `as`/`in` conversion already UNWRAPS to a bare number, so its result needs no \
                 `Qty.value` — drop it)"
            } else {
                ""
            };
            out.push(Reject::coded(
                Code::DimensionMismatch,
                format!(
                    "`Qty.value` recovers a quantity's number, but this operand is {}, which is not a \
                     quantity{hint}",
                    operand_ty.render_with_article(&db.name_ctx()),
                ),
            ));
        }
        return;
    }
    if args.len() == 2
        && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::UnitIn)
    {
        let operand_ty = type_of(db, args[1]);
        if let (Some(target), Ty::Qty { unit: qu, .. }) =
            (crate::eval::unit_of(db, args[0]), &operand_ty)
            && !target.same_dimension(qu)
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
        } else if !matches!(operand_ty, Ty::Qty { .. } | Ty::Any | Ty::Var(_)) {
            // The second operand is NOT a quantity — a bare number (commonly the result of a PRIOR
            // `Unit.in`/`as`, which UNWRAPS to a bare number). `Unit.in`/`as` converts a QUANTITY, not a
            // plain number, so this is rejected at COMPILE time (CDZ0501) rather than leaking the terse
            // backend "Unit.in of a non-quantity" decline at lowering. The common cause is CHAINING two
            // conversions (`(Unit.in cm (Unit.in mm x))`): the inner `Unit.in` already stripped the unit,
            // so the outer sees a bare number — re-wrap with `Qty.of` (`(Unit.in cm (Qty.of (Unit.in mm x)
            // millimeter))`) if the intermediate is meant to carry a unit. Names the operand's type so the
            // reader sees it is a plain `T`, not a quantity.
            trace!(target: "rcdzc::infer", head = head.0, "fault: Unit.in second operand is not a quantity (CDZ0501)");
            // Name the operand's REAL type + "not a quantity" GENERALLY (the operand can be ANY non-quantity
            // type — a `Bool`/`String`/`Record`, not only a bare number). The "a conversion unwrapped it /
            // chain re-wrap with `Qty.of`" note is a REPAIR HINT that only applies when the operand is
            // NUMERIC (the common `(Unit.in cm (Unit.in mm x))` chaining mistake) — append it only then, not
            // for e.g. a `Bool` (where "a plain number" was self-contradictory). Mirrors the `Qty.value` fix.
            let numeric = matches!(
                operand_ty.strip_nominal(),
                Ty::Int(_) | Ty::Float(_) | Ty::BigInt | Ty::Rational
            );
            let hint = if numeric {
                " (a conversion UNWRAPS to a bare number, so chaining `Unit.in (Unit.in …)` feeds the \
                 second one a bare number); wrap it with `Qty.of` if it should carry a unit"
            } else {
                ""
            };
            out.push(Reject::coded(
                Code::DimensionMismatch,
                format!(
                    "`Unit.in`/`as` converts a QUANTITY, but this operand is {}, which is not a quantity{hint}",
                    operand_ty.render_with_article(&db.name_ctx()),
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
    // generic path. Matches any arity (`Tuple.remove`/1, the rest/2).
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
                | crate::resolved::Prim::TupleSize
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
        // A built-in comparison observes the WHOLE operand structure, so an abstract type CONTAINED in a
        // compound operand (`(= (tuple (mk k) 1) …)` with `Temp` abstract-here) is observed by its private
        // rep exactly as a bare abstract operand is — the compound-operand generalization of the bare check
        // (PR#890, the direct-eq sibling of the map/set-key gap). Recurse via `key_ty_contains_abstract_at`
        // (the shared "type contains abstract at site" walk); a concrete/prelude/own constituent never
        // flags. Returns the first abstract type found (for the message).
        let abstract_operand = |ty: &Ty, node: StructId| key_ty_contains_abstract_at(db, node, ty);
        if let Some(ty) = abstract_operand(&a, args[0]).or_else(|| abstract_operand(&b, args[1])) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: built-in comparison on an abstract type value (CDZ0202)");
            out.push(
                Reject::coded(
                    Code::NominalMismatch,
                    format!(
                        "`{}` is an abstract type here (its constructors are not exported to this \
                         file), so its representation cannot be observed through a built-in comparison \
                         — compare it through a function exported by the module that declares it",
                        ty.render_name(&db.name_ctx())
                    ),
                )
                .at(head),
            );
            return;
        }
        // NOTE: a direct `(=)`/`<`/`compare` whose operand is a bare FUNCTION value is ALREADY rejected
        // below (the `Ty::Fn` operand arm → CDZ0203 "this operation is not defined on a function value",
        // with a forgot-to-apply hint). That path is more precise (distinguishes a partial application from
        // a genuine fn) and owns the direct-comparison-operand case, so CDZ0216 is NOT re-emitted here —
        // it is scoped to the Map/Set KEY position (above), where no operator-operand arm applies. (A fn
        // NESTED in a compound compared by `=` is a rare edge the CDZ0203 arm's top-level check doesn't
        // walk; left to that arm's evolution rather than duplicating the walk under a second code.)
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
            // The MECHANICAL repair: intern the STRING operand into a Symbol with the total `Symbol.of`
            // (`String → Symbol`), bringing both sides to `Symbol` so the comparison type-checks — the
            // Symbol twin of the newtype-unwrap fix just below (a nominal-boundary clash whose bridge is a
            // total conversion). Wrap whichever operand is the plain String (`a` is String → `args[0]`,
            // else `args[1]`). Heuristic: the author might instead have meant to compare as strings
            // (`Symbol.to-string` on the other side), so an agent confirms the direction before applying.
            let string_operand = if matches!(a, Ty::String) {
                args[0]
            } else {
                args[1]
            };
            out.push(
                Reject::coded(
                    Code::NominalMismatch,
                    "a Symbol and a String are not comparable across the nominal boundary",
                )
                .with_fix(Fix::wrap_heuristic(
                    string_operand,
                    "(Symbol.of ",
                    ")",
                    "intern the String into a Symbol with `(Symbol.of …)`",
                )),
            );
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
                    a.render_name(&db.name_ctx()),
                    b.render_name(&db.name_ctx())
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
    // A BITWISE / SHIFT operator (`& | ^ << >>`) on a NON-INTEGER operand — `(& true false)`, `(<< c 1)`,
    // `(| "a" "b")`. These carry the `∀a. (Int a) → (Int a) → (Int a)` scheme, so a non-Int operand makes
    // the generic scheme-unify ground the type var to `Int64` and report the opaque "type mismatch: Int64
    // and Bool must be the same type here" — an internal-clash read that hides the real fault (a bitwise op
    // needs integers). This block is NOT in the comparison/arith list below (those share numeric coercion
    // hints a bitwise op has no analogue for), so it would otherwise leak the phantom clash. Name it: a
    // bitwise/shift op is integer-only. For a BOOL operand, add the likely-intent hint — `and`/`or` are the
    // boolean connectives (a `&`/`|` on Bools is the C/Python habit). Fires only on a DEFINITE non-Int
    // operand (a `Var`/`Any`/`Int` never a false reject); a shift's COUNT is also `(Int a)` so a non-Int
    // count is caught too. CDZ0203 (an operand-type fault), like the arith cross-kind messages.
    if args.len() == 2 {
        let bitwise = matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::BitAnd
                    | crate::resolved::Prim::BitOr
                    | crate::resolved::Prim::BitXor
                    | crate::resolved::Prim::Shl
                    | crate::resolved::Prim::Shr
            )
        );
        if bitwise {
            // Skip any NUMERIC or open operand: a bitwise op is Int-only, but a FLOAT/Rational/BigInt
            // operand is a NUMERIC mismatch the generic scheme-unify already reports as the specific
            // CDZ0301 "no implicit conversion between numeric types" (pinned: `(& 2 2.0)` cites the
            // Float64↔Int mismatch, NOT this message). Fire ONLY on a DEFINITE NON-NUMERIC operand
            // (Bool/Char/String/compound); a `Var`/`Any` is unsolved (never a false reject).
            let is_numeric_or_open = |t: &Ty| {
                matches!(
                    t,
                    Ty::Int(_) | Ty::Float(_) | Ty::Rational | Ty::BigInt | Ty::Any | Ty::Var(_)
                )
            };
            let (a, b) = (type_of(db, args[0]), type_of(db, args[1]));
            let bad = if !is_numeric_or_open(&a) {
                Some((args[0], a))
            } else if !is_numeric_or_open(&b) {
                Some((args[1], b))
            } else {
                None
            };
            if let Some((bad_arg, bad_ty)) = bad {
                // For a BOOL operand, add the likely-intent hint AND — when the operator has a boolean
                // connective twin — a heuristic Replace on the OPERATOR HEAD so the suggestion is
                // machine-applyable: `&` → `and`, `|` → `or` (`(& true false)` → `(and true false)`).
                // `^`/`<<`/`>>` have NO boolean analogue, so they keep the message-only hint (no fix). The
                // fix anchors `head` (the operator occurrence), NOT `bad_arg` (the operand the message points
                // at) — the repair swaps the operator, not the value.
                let mut fix = None;
                let hint = if bad_ty == Ty::Bool {
                    let connective = match crate::eval::meta_apply_of(db, head) {
                        Some(crate::resolved::Prim::BitAnd) => Some("and"),
                        Some(crate::resolved::Prim::BitOr) => Some("or"),
                        _ => None, // BitXor / Shl / Shr — no boolean connective to swap in
                    };
                    if let Some(connective) = connective {
                        fix = Some(crate::diag::Fix::replace_heuristic(head, connective));
                    }
                    " — for boolean logic use `and`/`or`, not the bitwise operators"
                } else {
                    ""
                };
                trace!(target: "rcdzc::infer", head = head.0, ty = %bad_ty.render_name(&db.name_ctx()), "fault: bitwise/shift op on a non-integer operand (CDZ0203)");
                let mut reject = Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "a bitwise/shift operator needs integer operands, but a value of type {} \
                         was given{hint}",
                        bad_ty.render_name(&db.name_ctx())
                    ),
                )
                .at(bad_arg);
                if let Some(fix) = fix {
                    reject = reject.with_fix(fix);
                }
                out.push(reject);
                for &arg in args {
                    collect(db, arg, out);
                }
                return;
            }
        }
    }
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
                    // `%` (Rem) is integer arithmetic like `+`/`-`; include it so a non-numeric operand
                    // gets the "arithmetic is not defined on X" message rather than the phantom
                    // "type mismatch: Int64 and X" clash. (A `%` on a Qty is declined earlier, before here.)
                    | crate::resolved::Prim::Rem
            )
        )
    {
        let (a, b) = (type_of(db, args[0]), type_of(db, args[1]));
        // A FUNCTION-VALUED operand — an UNAPPLIED / partially-applied function where a value is wanted:
        // `(+ (String.slice s) 1)` (`String.slice s` is `(-> Int64 (-> Int64 (Option String)))`, applied to
        // one of its three arguments), `(< (add 1) 2)`. The operator's `∀a.(Int a)→…` / `∀a.a→a→Bool` scheme
        // defaults the type variable to the OTHER operand's type, then this operand fails to unify — the
        // generic scheme-unify reports the opaque "type mismatch: Int64 and (-> Int64 (-> Int64 (Option
        // String))) must be the same type here" (reads as an internal clash, and BURIES the real cause: an
        // operand that is a function, not a value). No arithmetic/order/equality is defined on a function
        // (`type-system.md` — a function is not data), so this is a genuine kind boundary the numeric/
        // cross-kind arms below miss (none of them match a `Ty::Fn`). Name it, and when the function fully
        // APPLIED would yield the other operand's type — a call the author forgot to finish, `(+ (add 1) 2)`
        // — append the SAME `fn_not_applied_hint` the call-argument / annotation sites use ("apply it to N
        // more argument(s) to get T"); when it would not (`String.slice` fully applied is `(Option String)`,
        // never a number), the base message stands alone (honest — "just call it" is not the fix).
        if matches!(a, Ty::Fn(_, _)) || matches!(b, Ty::Fn(_, _)) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: an operand is a function value — not operable/comparable (CDZ0203)");
            // Prefer the suggest-NEG hint when an operand is a partial subtraction `(- e)` (likely a
            // negation the user wrote as `-e`) over the generic "apply it to N more arguments".
            let mut neg = crate::infer::construct::suggest_neg_hint(db, args[0]);
            if neg.is_none() {
                neg = crate::infer::construct::suggest_neg_hint(db, args[1]);
            }
            let hint = neg
                .or_else(|| fn_not_applied_hint(&b, &a, &db.name_ctx()))
                .or_else(|| fn_not_applied_hint(&a, &b, &db.name_ctx()))
                .unwrap_or_default();
            out.push(Reject::coded(
                Code::TypeMismatch,
                format!(
                    "this operation is not defined on a function value — one operand is a function, \
                     not a value{hint}"
                ),
            ));
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // Bare `=` on TWO TYPE-VALUES (`Ty::Type` — a first-class type used as a value: `Int64`, a user sum
        // name). A type is ERASED at run time (`type-system.md` — types are checked then erased; a type-value
        // has no runtime representation), and there is a DEDICATED compile-time `Type.eq` (`Prim::TypeEq`, a
        // DISTINCT prim that folds two type-values to a constant `Bool`, e.g. `(Type.eq (Type.of x) Int64)`,
        // and never reaches this `Eq` block). So bare `=` — a RUNTIME structural comparison — is the wrong
        // tool for two type-values: refuse it and point at `Type.eq` (v-deferral-declines corpus-deprecation
        // BUCKET-2 — a correct-reject, not a should-work), rather than letting the two same-kind `Ty::Type`
        // operands unify and fall to a later shape-less/uncoded decline. GATED narrowly on `Eq` with BOTH
        // operands `Ty::Type`: a Type-vs-SCALAR `=` (different kinds) stays the cross-kind boundary reject
        // below (its own message), and arithmetic/order on a type-value (`(+ Color 1)`, `(+ Int64 Int64)`)
        // keeps the "kind boundary" / "not defined on Type" paths — those are not equality, so `Type.eq` is
        // not the fix for them.
        if matches!(a, Ty::Type)
            && matches!(b, Ty::Type)
            && crate::eval::meta_apply_of(db, head) == Some(crate::resolved::Prim::Eq)
        {
            trace!(target: "rcdzc::infer", head = head.0, "fault: bare `=` on two type values — use Type.eq (CDZ0203)");
            out.push(Reject::coded(
                Code::TypeMismatch,
                "bare `=` is not defined on a type value — a type is erased at run time and is not data; \
                 use `Type.eq` for compile-time type equality (e.g. `(Type.eq (Type.of x) Int64)`)"
                    .to_string(),
            ));
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // `Symbol` counts as a TEXT-like atom here: it is an interned string value, comparable/equatable
        // only to another `Symbol` (like `String` to `String`), and cross-kind to a number/char/bool/
        // compound. Including it makes a `Symbol`-vs-scalar or `Symbol`-vs-compound pair take the named
        // cross-kind boundary message (CDZ0201) instead of the opaque generic scheme-unify "type mismatch:
        // Symbol and Int64 must be the same type here" it fell through to (Bool/Char/String were already
        // named; Symbol was the missing scalar-adjacent kind). A `Symbol`-vs-`String` pair is caught
        // EARLIER by the CDZ0202 nominal-boundary arm, and `Symbol`-vs-`Symbol` is same-kind (not
        // cross-kind), so neither is disturbed.
        let is_text = |t: &Ty| matches!(t, Ty::String | Ty::Bytes | Ty::Symbol);
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
                    // `%` (Rem) is integer arithmetic — a non-numeric operand (`(% "a" "b")`) is "arithmetic
                    // is not defined on String", the same as `+` (no `.concat` fix — that is `+`-only below).
                    | crate::resolved::Prim::Rem
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
                    a.render_name(&db.name_ctx())
                )
            } else {
                format!(
                    "{} and {} — Cadenza never coerces these to numbers",
                    a.render_name(&db.name_ctx()),
                    b.render_name(&db.name_ctx())
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
        let is_option_payload_pair = option_payload_mismatch_hint(&db.name_ctx(), &a, &b).is_some()
            || option_payload_mismatch_hint(&db.name_ctx(), &b, &a).is_some();
        let cross_kind = !is_option_payload_pair
            && ((is_text(&a) && is_scalar(&b))
                || (is_scalar(&a) && is_text(&b))
                || (is_compound(&a) && is_atom(&b))
                || (is_atom(&a) && is_compound(&b))
                || different_compound_kinds);
        if cross_kind {
            trace!(target: "rcdzc::infer", head = head.0, "fault: operands of distinct kinds — not comparable/operable across the boundary (CDZ0201)");
            // An `Ast` operand in an ARITHMETIC/comparison position is a distinctive category error, not a
            // generic cross-type clash: `Ast` is COMPILE-TIME METADATA (a quoted / reified syntax tree),
            // never a runtime number. The generic "a Ast and an Int64 are different types" reads as an
            // ordinary user type mismatch and hides the real situation. Most often this arises from `eval`
            // of a template with a runtime SPLICE — `(eval (quasiquote (+ (unquote (quote …)) 1)))`: the
            // spliced `(quote …)` reconstructs to an `Ast` value the surrounding `+` cannot consume (eval
            // reconstructs source statically and cannot see through a runtime-spliced Ast subtree). Name
            // that — the message the breaker probe asked for (corpus-bugfix issue). A NON-`Ast` cross-kind
            // pair keeps the generic boundary message.
            let ast_operand =
                a.render_name(&db.name_ctx()) == "Ast" || b.render_name(&db.name_ctx()) == "Ast";
            let message = if ast_operand {
                "an `Ast` value is compile-time metadata (a quoted or reified syntax tree), not a runtime \
                 value, so this operation is not defined on it — this often arises from `eval` of a \
                 template with a runtime splice (the spliced `(quote …)`/`Ast.*` reconstructs to an `Ast` \
                 the surrounding expression cannot consume): evaluate the spliced expression directly, or \
                 bind the template and match on it rather than computing with it"
                    .to_string()
            } else {
                format!(
                    "{} and {} are different types (this operation is not defined across that kind boundary)",
                    a.render_with_article(&db.name_ctx()),
                    b.render_with_article(&db.name_ctx())
                )
            };
            out.push(Reject::coded(Code::Malformed, message));
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
                    a.render_with_article(&db.name_ctx()),
                    b.render_with_article(&db.name_ctx()),
                ),
            ));
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // A CHAR against a NUMBER in a COMPARISON / EQUALITY (`(< c 1)`, `(= c 5)`, `(> #\a 0)`) — the
        // comparison/equality twin of the ARITHMETIC `(+ #\a 1)` case. Arithmetic flows to the numeric-
        // coercion path, which offers the `(Char.to-int …)` wrap fix; comparison/equality instead falls to
        // the generic scheme-unify → the opaque "type mismatch: Char and Int64 must be the same type here"
        // (an internal-clash read), with NO repair. `Char.to-int : Char → Int64` is TOTAL, so the SAME wrap
        // fits — name the kind boundary + carry the `(Char.to-int …)` fix on the CHAR operand, at parity
        // with arithmetic. (`Bool` was handled above — a boolean has no numeric conversion; a `Char` does.)
        let is_compare_or_eq = matches!(
            crate::eval::meta_apply_of(db, head),
            Some(
                crate::resolved::Prim::Lt
                    | crate::resolved::Prim::Gt
                    | crate::resolved::Prim::Le
                    | crate::resolved::Prim::Ge
                    | crate::resolved::Prim::Eq
                    | crate::resolved::Prim::Compare
            )
        );
        // The CHAR operand AND the NUMERIC SIBLING's type — the fix depends on whether the sibling is an
        // INTEGER or a FLOAT. `Char.to-int : Char → Int64` yields an `Int64`, which type-checks a
        // comparison against an INTEGER sibling directly; but a FLOAT sibling needs a further
        // `Float{W}.of-int` step, because Cadenza never implicitly promotes Int64 → Float (a bare
        // `(Char.to-int c)` against a `Float64` re-fails CDZ0301, so the old one-size fix was non-working
        // for the float case).
        let (char_arg, sibling) = if a == Ty::Char && matches!(b, Ty::Int(_) | Ty::Float(_)) {
            (Some(args[0]), b.clone())
        } else if b == Ty::Char && matches!(a, Ty::Int(_) | Ty::Float(_)) {
            (Some(args[1]), a.clone())
        } else {
            (None, Ty::Any)
        };
        if is_compare_or_eq && let Some(char_arg) = char_arg {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Char compared to a number — not comparable, offer Char.to-int (CDZ0203)");
            // A FLOAT sibling needs `Float{W}.of-int` around the `Char.to-int` (else the Int64 it yields
            // still can't compare to the float — Cadenza has no implicit Int→Float promotion). An INTEGER
            // sibling takes the plain `Char.to-int` wrap. The float MODULE name follows the sibling's
            // ground width (`Float32`/`Float64`), so the wrapped value matches the sibling's exact type.
            let reject = Reject::coded(
                Code::TypeMismatch,
                "a character and a number are not comparable directly — a `Char` is not a number; \
                 convert it to its scalar value with `Char.to-int` first"
                    .to_string(),
            )
            .at(char_arg);
            let reject = if let Ty::Float(ft) = sibling {
                let module = format!("Float{}", ft.ground_width());
                reject.with_fix(Fix::wrap_heuristic(
                    char_arg,
                    format!("({module}.of-int (Char.to-int "),
                    "))",
                    format!(
                        "convert the char to its Int64 scalar value then to a float with \
                         `{module}.of-int`"
                    ),
                ))
            } else {
                reject.with_fix(Fix::wrap_heuristic(
                    char_arg,
                    "(Char.to-int ",
                    ")",
                    "convert the char to its Int64 scalar value with `Char.to-int`",
                ))
            };
            out.push(reject);
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
        // `%` (remainder) on a QUANTITY operand is NOT DEFINED — the units surface enumerates `+`/`-`/`*`/
        // `/`/comparison, not `%`, and unlike those `%` has no dimensional rule wired here. Reject it with
        // a CLEAN, intentional decline naming that, rather than letting it fall through to the generic
        // scheme-unify below — which unifies the operator's `∀a. (Int a) → …` scheme against the `Ty::Qty`
        // and leaks a confusing "type mismatch: Int64 and (Qty Int64 meter) must be the same type" (the
        // `Int64` is the scheme's, an internal detail the author never wrote). Whether `%` SHOULD be a
        // quantity operation (a same-dimension remainder `7m % 3m = 1m` is arithmetically sensible) is a
        // language-design call held for the operator; until then this is the correct shipped behavior — a
        // clear decline, not a leaked scheme mismatch. (`is_rem` was already excluded from the `*`/`/`
        // dimensional skip; this replaces the fall-through with an explicit message.)
        if is_rem && any_qty {
            // `%` (remainder) on QUANTITIES is a SAME-DIMENSION INTEGER operation — `7m % 3m = 1m`
            // (operator ruling 2026-08-28: same-dimension mod is well-formed). It mirrors `+`/`-`
            // dimensionally (same dimension in, SAME unit out — a remainder does not compose units like
            // `*`/`/`) and is defined only for an INTEGER/BigInt inner numeric type, exactly like the
            // bare `%` (a float/rational has no remainder — exact division is total). `apply_type` gives
            // the same-unit `(Qty T u)` result; `lower` erases each `Qty.of` to its magnitude and runs the
            // inner integer `%`. Faults: cross-dimension → CDZ0501; a float/rational inner → clean decline
            // (like bare float `%`); a quantity mixed with a bare number → CDZ0501 (no dimensionless coercion).
            match (&a0, &b0) {
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
                    if !ua.same_dimension(ub) {
                        trace!(target: "rcdzc::infer", head = head.0, "fault: remainder of quantities of incompatible dimension (CDZ0501)");
                        out.push(Reject::coded(
                            Code::DimensionMismatch,
                            format!(
                                "taking the remainder of quantities of incompatible dimension: {} and {} \
                                 — a remainder requires equal dimensions (units are never silently \
                                 converted across dimensions)",
                                ua.render_human(),
                                ub.render_human(),
                            ),
                        ));
                    } else if matches!(**ia, Ty::Float(_) | Ty::Rational)
                        || matches!(**ib, Ty::Float(_) | Ty::Rational)
                    {
                        // A remainder is an INTEGER operation — a float/rational has none (exact/floating
                        // arithmetic is total). Reject with the SAME code a bare float `%` gets (CDZ0301):
                        // the numeric type does not support the operator, not a not-yet-implemented decline.
                        trace!(target: "rcdzc::infer", head = head.0, "reject: remainder is not defined on a floating/rational quantity (CDZ0301)");
                        out.push(Reject::coded(
                            Code::NumericMismatch,
                            "remainder (%) is not defined on a floating-point or rational quantity — a \
                             remainder is an integer operation; use an integer quantity, or recover the \
                             numeric value with `Qty.value` first"
                                .to_string(),
                        ));
                    } else {
                        // Same dimension, integer inner — the INNER numeric types must still agree (no
                        // promotion under a unit), the same check `+`/`-` make; report a mismatch CDZ0301
                        // with the one-shot inner-value coercion fix.
                        let mut subst = Subst::new();
                        if let Err(mut reject) =
                            crate::unify::unify(&mut subst, ia, ib, &db.name_ctx())
                        {
                            let inner_a = qty_of_value_arg(db, args[0]);
                            let inner_b = qty_of_value_arg(db, args[1]);
                            let fix = inner_a
                                .and_then(|n| numeric_text_coercion_fix(db, ib, ia, n))
                                .or_else(|| {
                                    inner_b.and_then(|n| numeric_text_coercion_fix(db, ia, ib, n))
                                });
                            if let Some(fix) = fix {
                                reject = reject.with_fix(fix);
                            }
                            out.push(reject);
                        }
                    }
                }
                // A quantity mixed with a BARE number — `7m % 3` — has no dimensionless coercion, exactly
                // like `7m + 3`: CDZ0501.
                _ => {
                    trace!(target: "rcdzc::infer", head = head.0, "fault: remainder mixes a quantity and a bare number (CDZ0501)");
                    out.push(Reject::coded(
                        Code::DimensionMismatch,
                        format!(
                            "taking the remainder of a quantity and a bare number: {} and {} — a \
                             remainder requires two quantities of equal dimension (recover the numeric \
                             value with `Qty.value` first)",
                            a0.render_name(&db.name_ctx()),
                            b0.render_name(&db.name_ctx()),
                        ),
                    ));
                }
            }
            for &arg in args {
                collect(db, arg, out);
            }
            return;
        }
        // `*`/`/` on a quantity is ALWAYS well-formed DIMENSIONALLY (the dimensions compose, not required
        // equal), so it must NOT reach the generic scheme-unify below (which would unify a `Ty::Qty`
        // against `(Int a)` → a spurious CDZ0203). But the INNER NUMERIC TYPES must still agree: scaling a
        // `(Qty Float64 meter)` by a bare `Int64` `1` is the SAME no-silent-promotion error a bare
        // `(* 5.0 1)` gets (CDZ0301), NOT a silent success — without this check the mismatch reached
        // `lower`, which emitted the `1` as an `i64` into an `f64` multiply → INVALID wasm (a miscompile:
        // `cdz check` passed, wasm-tools rejected it). Unify the two operands' inner numeric types (a
        // quantity contributes its `inner`, a bare number contributes itself) and report a mismatch as
        // CDZ0301 with the same one-shot coercion fix the additive arm offers (`1` → `1.0`), applied to
        // the offending operand's inner value. Then skip the generic path (dimensions handled in
        // `apply_type`), descend for operand faults, and return.
        if is_multiplicative && any_qty {
            let inner_ty = |t: &Ty| -> Ty {
                match t {
                    Ty::Qty { inner, .. } => (**inner).clone(),
                    other => other.clone(),
                }
            };
            let ia = inner_ty(&a0);
            let ib = inner_ty(&b0);
            let mut subst = Subst::new();
            if let Err(mut reject) = crate::unify::unify(&mut subst, &ia, &ib, &db.name_ctx()) {
                // Retype whichever operand's inner value the coercion bridges — a quantity operand's inner
                // value is its `Qty.of`'s first arg (`qty_of_value_arg`); a bare operand's inner value is
                // the operand node itself. Mirrors the additive arm's numeric-coercion repair.
                let inner_val = |db: &mut Db, arg: StructId, t: &Ty| -> Option<StructId> {
                    if matches!(t, Ty::Qty { .. }) {
                        qty_of_value_arg(db, arg)
                    } else {
                        Some(arg)
                    }
                };
                let na = inner_val(db, args[0], &a0);
                let nb = inner_val(db, args[1], &b0);
                // Prefer the WIDENING repair — coerce the INT operand UP to the FLOAT operand's type
                // (`1` → `1.0`), matching the bare `(* 5.0 1)` fix, regardless of operand order. Whichever
                // operand's inner is a Float is the `expected` type; the OTHER operand's inner value node
                // is the one to retype. `numeric_text_coercion_fix(expected=Float, actual=Int, node)`
                // fires the `node → node.0` widening. Falls back to the generic bridge if neither is Float.
                let fix = if matches!(ia, Ty::Float(_)) {
                    nb.and_then(|n| numeric_text_coercion_fix(db, &ia, &ib, n))
                } else if matches!(ib, Ty::Float(_)) {
                    na.and_then(|n| numeric_text_coercion_fix(db, &ib, &ia, n))
                } else {
                    na.and_then(|n| numeric_text_coercion_fix(db, &ib, &ia, n))
                        .or_else(|| nb.and_then(|n| numeric_text_coercion_fix(db, &ia, &ib, n)))
                };
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
                        a0.render_name(&db.name_ctx()),
                        b0.render_name(&db.name_ctx())
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
                // A CHAR other needs the two-step `(BigInt.of (Char.to-int …))` (via `total_conversion_wrap`)
                // — `numeric_text_coercion_fix` only retypes/wraps a fixed-INT other, so a char got no fix.
                // The char-with-BigInt twin of the char-with-Float arith repair.
                let fix = if matches!(other, Ty::Char) {
                    total_conversion_wrap(&Ty::BigInt, other)
                        .map(|(p, s, v)| Fix::wrap_heuristic(fix_arg, p, s, v))
                } else {
                    numeric_text_coercion_fix(db, &Ty::BigInt, other, fix_arg)
                };
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
                        a0.render_name(&db.name_ctx()),
                        b0.render_name(&db.name_ctx())
                    ),
                );
                // The Rational twin of the BigInt wrap above: offer `(Rational.of-int …)` on the fixed-int
                // operand where the other side is a Rational (`(+ r 1)` → wrap `1`).
                let (fix_arg, other) = if a_rat {
                    (args[1], &b0)
                } else {
                    (args[0], &a0)
                };
                // A CHAR other needs the two-step `(Rational.of-int (Char.to-int …))` (via
                // `total_conversion_wrap`) — the char-with-Rational twin of the BigInt/Float char repairs.
                let fix = if matches!(other, Ty::Char) {
                    total_conversion_wrap(&Ty::Rational, other)
                        .map(|(p, s, v)| Fix::wrap_heuristic(fix_arg, p, s, v))
                } else {
                    numeric_text_coercion_fix(db, &Ty::Rational, other, fix_arg)
                };
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
                        a0.render_name(&db.name_ctx()),
                        b0.render_name(&db.name_ctx())
                    )
                };
                let mut reject = Reject::coded(Code::NumericMismatch, msg).at(app);
                // A CHAR operand against the float — `(+ #\a 1.0)`. A Char is not a number, so the generic
                // mix message above stands, but the WORKING repair is the two-step `(Float{W}.of-int
                // (Char.to-int …))` (`total_conversion_wrap` on `(Float, Char)`); a bare `Char.to-int` would
                // re-fail (Int64 vs Float, no implicit promotion) — the arithmetic twin of the char-vs-float
                // COMPARISON fix. Offered on whichever operand is the Char, BEFORE the numeric-literal retype
                // (a Char has no int/float literal spelling, so that fallback finds nothing here).
                let char_fix = if matches!(a0, Ty::Char) {
                    total_conversion_wrap(&b0, &a0)
                        .map(|(p, s, v)| Fix::wrap_heuristic(args[0], p, s, v))
                } else if matches!(b0, Ty::Char) {
                    total_conversion_wrap(&a0, &b0)
                        .map(|(p, s, v)| Fix::wrap_heuristic(args[1], p, s, v))
                } else {
                    None
                };
                // Prefer conforming the SECOND operand to the first (the first establishes the intended
                // type). But when that operand has no clean one-shot — a NON-LITERAL float against an int
                // context (`(+ 5 y)`, `y : Float64`: `numeric_text_coercion_fix(Int64, Float64, y)` is
                // `None`, since a runtime float has no int spelling) — the SYMMETRIC repair often IS
                // available: conform the FIRST operand to the second's type (retype the LITERAL `5` → `5.0`).
                // Without this, `(+ 5 y)` offered NO fix while `(+ y 5)` did — an order asymmetry for the
                // identical slip. Try the second-operand coercion first, then fall back to the first, so a
                // literal-int-on-either-side/float-param mix always gets the retype. Deterministic (second
                // preferred, then first); a fix on neither leaves the bare CDZ0301.
                let fix = char_fix
                    .or_else(|| numeric_text_coercion_fix(db, &expected, actual, fix_arg))
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
                    a0.render_name(&db.name_ctx()),
                    b0.render_name(&db.name_ctx())
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
                            if let Err(mut reject) =
                                crate::unify::unify(&mut subst, ia, ib, &db.name_ctx())
                            {
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
                    //
                    // But ONLY when the non-quantity operand is actually a NUMBER: this CDZ0501 message
                    // ("a quantity and a plain number") and its `(Qty.of <n> <unit>)` repair are meaningful
                    // solely for a bare numeric operand. A quantity added to a NON-numeric value — e.g. an
                    // `(Option (Qty …))` from `List.at`, a tuple, a string — is not a dimension slip; the
                    // accurate report is the generic scheme-unify's CDZ0203 (which, for an `Option`, even
                    // guides "the value is optional; match it to handle the `None` case"). So when the
                    // non-quantity side is not numeric, fall THROUGH to the generic path rather than
                    // mislabel the operand "a plain number" and offer a nonsensical `(Qty.of …)` wrap.
                    _ if {
                        let is_num = |t: &Ty| {
                            matches!(t, Ty::Int(_) | Ty::Float(_) | Ty::BigInt | Ty::Rational)
                        };
                        // The operand that is NOT the quantity — the one this arm calls "a plain number".
                        let non_qty = if matches!(a, Ty::Qty { .. }) { &b } else { &a };
                        is_num(non_qty)
                    } =>
                    {
                        trace!(target: "rcdzc::infer", head = head.0, "fault: combining a quantity with a non-quantity (CDZ0501)");
                        let mut reject = Reject::coded(
                            Code::DimensionMismatch,
                            format!(
                                "{} a quantity and a plain number: {} and {} — a quantity has a \
                                 dimension a bare number lacks, and there is no implicit \
                                 dimensionless coercion",
                                additive_op_gerund(prim),
                                a.render_name(&db.name_ctx()),
                                b.render_name(&db.name_ctx()),
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
                    // A quantity combined with a NON-numeric non-quantity (an `Option`, tuple, string, …):
                    // NOT a dimension slip. Fall through (no `return`) to the generic scheme-unify path
                    // below, which gives the accurate CDZ0203 for the actual type clash.
                    _ => {}
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
            // Map.insert twin of the map-literal check (M75). When the two are same-kind compounds that
            // differ structurally (a record field-set / tuple arity / sum payload), append the
            // minimal-conflict delta the map-LITERAL arm already carries — the map's key type is the
            // EXPECTED side, the inserted key the ACTUAL, so this is the directional `structural_delta_hint`
            // (M184 audit: the Map.insert op arm missed the delta its peer-join twin carries).
            let delta = structural_delta_hint(&kt, &key_ty, &db.name_ctx()).unwrap_or_default();
            // Anchor at the inserted KEY (`args[1]`), the actionable locus, not the whole `(Map.insert …)`
            // application node — the squiggle points at the mismatching key (matches the file's "anchor the
            // specific offending element" pattern; the Map twin of the list-op anchoring, PR #399).
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a map associates keys of one type, but this key's type differs: the map's keys are \
                     {}, this key is {}{delta}",
                    kt.render_name(&db.name_ctx()),
                    key_ty.render_name(&db.name_ctx())
                ),
            )
            .at(args[1]);
            if let Some(fix) = float_literal_retype_fix(db, args[1], &key_ty, &kt) {
                reject = reject.with_fix(fix);
            }
            out.push(reject);
        }
        if !vt.agrees_with(&val_ty) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Map.insert value type disagrees with the map's value type (CDZ0201)");
            // Append the same directional structural-delta the key arm now carries — the map's value type
            // is EXPECTED, the inserted value ACTUAL — so a same-kind compound value mismatch names the
            // field/element/payload conflict rather than leaving the reader to diff two rendered types.
            let delta = structural_delta_hint(&vt, &val_ty, &db.name_ctx()).unwrap_or_default();
            // Anchor at the inserted VALUE (`args[2]`), not the whole application — same locus fix as the
            // key arm above.
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a map associates values of one type, but this value's type differs: the map's \
                     values are {}, this value is {}{delta}",
                    vt.render_name(&db.name_ctx()),
                    val_ty.render_name(&db.name_ctx())
                ),
            )
            .at(args[2]);
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
        // The key/value AGREE with the map's types — but a bare inserted literal whose width is fixed by the
        // map's key/value type (from a SIBLING insert in the chain, e.g. `(Map.insert (Map.insert m 1 (: 5
        // UInt8)) 2 300)` where the inner `(: 5 UInt8)` pins the value type UInt8) must still be RANGE-checked
        // against that width. `agrees_with` only tests kind/shape agreement (a deferred `Int64` agrees with
        // `UInt8`), not fit — so `300` slipped CDZ0302 → wasm wrapped, rust E0308 (breaker's Map face of the
        // sibling-width skip). Range-check the inserted key against `kt` and value against `vt` via the same
        // `width_fault_against_ty` the annotation path uses. (The operand map's own inserts are checked when
        // `collect` recurses into `args[0]` below.)
        // Range-check against the operand map's column (the sibling-inferred width, e.g. `300` over a UInt8
        // column pinned by a prior insert), AND against the inserted literal's OWN adopted type (`key_ty`/
        // `val_ty`). The latter matters when the operand column does NOT yet pin the width (the FIRST insert's
        // operand is `Map.empty` → `Ty::Map(Any, Any)`, so `kt`/`vt` are `Any` and impose no range), but the
        // literal ADOPTED a narrow width from a LATER sibling in the chain (seq-40 width-unification) —
        // `(Map.insert (Map.insert Map.empty 0 1.0e300) 1 (: 2.0 Float32))`: the inner `1.0e300` adopts Float32
        // (its `type_of`), and must be range-checked against THAT (it overflows binary32 → CDZ0302), matching
        // the explicit `(: 1.0e300 Float32)` reject. Without the own-type check the adopt path materialized an
        // out-of-range value (±inf / a wrapped int) that the annotation path rejects — an adopt-vs-annotation
        // inconsistency (v-rb routed: the adopt-site range-check is this width-unification lane, not an emit demote).
        if let Some(reject) = width_fault_against_ty(db, args[1], &kt)
            .or_else(|| width_fault_against_ty(db, args[2], &vt))
            .or_else(|| width_fault_against_ty(db, args[1], &key_ty))
            .or_else(|| width_fault_against_ty(db, args[2], &val_ty))
        {
            out.push(reject);
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
                if crate::unify::unify(&mut subst, first_ty, &et, &db.name_ctx()).is_err() {
                    clash = Some((e, et));
                    break;
                }
            }
        }
        if let (Some((first, first_ty)), Some((e, et))) = (&first_pair, &clash) {
            trace!(target: "rcdzc::infer", head = head.0, "fault: Set.of elements do not share one type (CDZ0201)");
            let delta = peer_type_delta_hint(first_ty, et, &db.name_ctx()).unwrap_or_default();
            // Anchor at the OUTLIER element `e` (the one that broke homogeneity against the first
            // element's type), not the whole `(Set.of …)` application — the squiggle lands on the off
            // element, not the entire set construction (the Set twin of the list/map-literal outlier
            // anchoring, PR #399).
            let mut reject = Reject::coded(
                Code::Malformed,
                format!(
                    "a set contains elements of one type, but the elements differ: {} and {}{delta}",
                    first_ty.render_name(&db.name_ctx()),
                    et.render_name(&db.name_ctx())
                ),
            )
            .at(*e);
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
        // HOMOGENEOUS set (no clash) — RANGE-CHECK each element against the SETTLED element type. A bare
        // out-of-range element whose width is fixed by an annotated SIBLING (`(Set.of (list (: 1 UInt64)
        // -41))`) must reject CDZ0302, exactly like the list-literal sibling-width fix — else wasm wraps +
        // rust E0308 (breaker's Set face of the sibling-unification skip). The `Set.of` path walks its
        // elements HERE, bypassing the list-literal fault arm, so the check must live at this seam too. The
        // settled type is the JOIN of the element types (takes the fixed width regardless of position).
        if let Some((_, first_ty)) = first_pair {
            let settled = elems
                .iter()
                .skip(1)
                .fold(first_ty, |acc, &e| acc.join(&type_of(db, e)));
            if let Some(reject) = elems
                .iter()
                .find_map(|&e| width_fault_against_ty(db, e, &settled))
            {
                out.push(reject);
            }
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
            let mut homogeneity_fault = false;
            for &e in args.iter().skip(1) {
                let et = type_of(db, e);
                if crate::unify::unify(&mut subst, &first_ty, &et, &db.name_ctx()).is_err() {
                    homogeneity_fault = true;
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
                    let delta =
                        peer_type_delta_hint(&first_ty, &et, &db.name_ctx()).unwrap_or_default();
                    // Anchor at the OUTLIER element `e` (the one that broke homogeneity against the
                    // established first-element type), not the whole `(list …)` — the squiggle lands on
                    // `"three"` in `(list 1 2 "three" 4 5)` rather than the entire list, so the reader sees
                    // exactly which element is off. (Without `.at`, `collect` stamps the coarse list node.)
                    let mut reject = Reject::coded(
                        code,
                        format!(
                            "list elements must share one type: {} and {}{delta}",
                            first_ty.render_name(&db.name_ctx()),
                            et.render_name(&db.name_ctx())
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
            // RANGE-CHECK each element literal against the SETTLED element type. A list whose element type
            // is fixed by a SIBLING (`(list (: 1 UInt64) -41)` — the annotated `1` pins `UInt64`, the bare
            // `-41` unifies in as a deferred int) never re-validated the bare literal against that inferred
            // width: the outer list carries no annotation, so `nested_literal_width_faults` (annotation-
            // driven) never ran, and `-41`'s own `type_of` is the `Int64` default — so `cdz check` ACCEPTED
            // it while wasm SILENTLY WRAPPED (-41 → a huge UInt64) and rust emitted an invalid `u64` init
            // (E0308) — a backend-divergent miscompile (fuzzer/corpus-bugfix differential). Once the element
            // type settles, run the same `width_fault_against_ty` the annotation path uses on EACH element
            // against it. Only when the list was HOMOGENEOUS (no unify fault above — else the element type is
            // not meaningfully settled and the homogeneity reject is the right one). The FIRST out-of-range
            // element rejects (CDZ0302), anchored at that element.
            if !homogeneity_fault {
                // The settled element type is the JOIN of all element types — it takes the FIXED width/sign
                // from whichever sibling supplies it, regardless of position (so a leading bare `-41` in
                // `(list -41 (: 1 UInt64))` is checked against `UInt64` too, not its own deferred `Int64`).
                let settled = args
                    .iter()
                    .skip(1)
                    .fold(first_ty.clone(), |acc, &e| acc.join(&type_of(db, e)));
                if let Some(reject) = args
                    .iter()
                    .find_map(|&e| width_fault_against_ty(db, e, &settled))
                {
                    out.push(reject);
                }
            }
        }
        for &e in args {
            collect(db, e, out);
        }
        return;
    }
    // `List.push`/`List.update`/`List.concat` — the HOMOGENEITY of a functional-construction list op:
    // the pushed/updated element (or the concatenated list's element) must share the operand list's
    // element type (collections-and-text.md §A List Is An Ordered Homogeneous Sequence). A disagreement
    // is a MALFORMED collection → CDZ0201 (uniform with the list-literal + map/set homogeneity checks,
    // §A Collection's Homogeneity Violation Is A Malformed Collection), NOT the CDZ0203 the generic
    // scheme-unify (the member-op arm below) would give. We check the ELEMENT disagreement specifically —
    // the operand IS a `Ty::List(elem)` and the pushed/updated element (or the other list's element)
    // does not `agrees_with` it — so a wrong LIST-OPERAND (`(List.push 5 true)`, first arg not a list)
    // falls through to the generic member-op arm's CDZ0203, which is the right code there. On the
    // homogeneity fault, keep the same operation-naming message the member-op arm produces (so the reader
    // still sees WHICH op wanted WHAT) but code it CDZ0201, descend into the operands, and stop.
    {
        // (expected element/list type, the given element/list type) on a genuine element mismatch.
        // The offending ARGUMENT occurrence is the actionable locus (the pushed/updated element, or the
        // second list in `concat`) — the reject anchors there, not the whole application node, so the
        // squiggle points at the culprit (matching the file's "anchor the specific offending element"
        // pattern; PR #399 review). The tuple carries `(expected, given, culprit_arg_occ)`.
        let list_op_mismatch: Option<(Ty, Ty, StructId)> =
            match crate::eval::meta_apply_of(db, head) {
                // push: args = [list, elem]; the elem must match the list's element type.
                Some(crate::resolved::Prim::ListPush) if args.len() == 2 => {
                    match type_of(db, args[0]) {
                        Ty::List(elem) => {
                            let given = type_of(db, args[1]);
                            (!elem.agrees_with(&given)).then(|| ((*elem).clone(), given, args[1]))
                        }
                        _ => None,
                    }
                }
                // prepend: args = [list, elem]; like push, the elem must match the list's element type
                // (the front-insertion companion — same homogeneity fault, same actionable locus).
                Some(crate::resolved::Prim::ListPrepend) if args.len() == 2 => {
                    match type_of(db, args[0]) {
                        Ty::List(elem) => {
                            let given = type_of(db, args[1]);
                            (!elem.agrees_with(&given)).then(|| ((*elem).clone(), given, args[1]))
                        }
                        _ => None,
                    }
                }
                // update: args = [list, index, elem]; the elem must match the list's element type.
                Some(crate::resolved::Prim::ListUpdate) if args.len() == 3 => {
                    match type_of(db, args[0]) {
                        Ty::List(elem) => {
                            let given = type_of(db, args[2]);
                            (!elem.agrees_with(&given)).then(|| ((*elem).clone(), given, args[2]))
                        }
                        _ => None,
                    }
                }
                // concat: args = [a, b]; the two lists' element types must agree — name the whole list types.
                // The SECOND list is the actionable locus (the first is the operand whose element type the
                // result takes; the mismatch is that the second doesn't match it).
                Some(crate::resolved::Prim::ListConcat) if args.len() == 2 => {
                    match (type_of(db, args[0]), type_of(db, args[1])) {
                        (Ty::List(ea), Ty::List(eb)) if !ea.agrees_with(&eb) => {
                            Some((Ty::List(ea), Ty::List(eb), args[1]))
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
        if let Some((expected, given, culprit)) = list_op_mismatch {
            trace!(target: "rcdzc::infer", head = head.0, "fault: a list op's element does not share the list's element type (CDZ0201)");
            // Name the operation the same way the generic member-op arm does (`List.push` expects …), but
            // code it CDZ0201 — the uniform collection-homogeneity code, not the member-op arm's CDZ0203.
            // Append the SAME structural-delta hint that arm carries, so a same-kind compound mismatch
            // (`(List.push xs (record (y 2)))` for a `List (Record (x Int64))`) names the minimal conflict
            // (`field `x` is missing (found `y`)`) instead of leaving the reader to diff two rendered types.
            let message = match member_op_head_name(db, head) {
                Some((module, member)) => {
                    let delta = structural_delta_hint(&expected, &given, &db.name_ctx()).unwrap_or_default();
                    format!(
                        "`{module}.{member}` expects an argument of type {}, but a value of type {} was given{delta}",
                        expected.render_name(&db.name_ctx()),
                        given.render_name(&db.name_ctx())
                    )
                }
                None => "list elements must share one type (the operation's element type differs from the list's)"
                    .to_string(),
            };
            out.push(Reject::coded(Code::Malformed, message).at(culprit));
            for &a in args {
                collect(db, a, out);
            }
            return;
        }
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
        // An entry is the native `(= k v)` FieldPair leaf (M2, what `#map`/`(map (= k v))` emit), the
        // transitional name-head `(= k v)`, or the legacy 2-element `(k v)` pair — mirror `resolve_map`.
        // Before reading the FieldPair, a native-FieldPair entry (3-element) failed the 2-element check → the
        // whole `(map (= k v)…)` name-alias literal collected ZERO entries, so the key/value HOMOGENEITY +
        // duplicate-const-key checks never ran → a mixed-type / duplicate-key map silently type-checked
        // (soundness miscompile; the native `#map` `Resolved::Map` arm already saw its entries and worked).
        let entries: Vec<(StructId, StructId)> = args
            .iter()
            .filter_map(|&e| {
                db.ast
                    .field_pair_parts(e)
                    .or_else(|| db.ast.field_pair(e))
                    .or_else(|| match db.ast.get(e) {
                        crate::ast::Struct::List(items) if items.len() == 2 => {
                            Some((items[0], items[1]))
                        }
                        _ => None,
                    })
            })
            .collect();
        let mut ksubst = Subst::new();
        let mut vsubst = Subst::new();
        if let Some(&(fk, fv)) = entries.first() {
            let (fkt, fvt) = (type_of(db, fk), type_of(db, fv));
            for &(k, v) in entries.iter().skip(1) {
                let kt = type_of(db, k);
                if crate::unify::unify(&mut ksubst, &fkt, &kt, &db.name_ctx()).is_err() {
                    // Name the two clashing key types (like the list-homogeneity message) and, for an
                    // int-literal-vs-float clash, offer the same `3.0` retype fix — the map-key twin of the
                    // list/if/match "same repair wherever the same mismatch surfaces" (M57). The
                    // structural-delta hint names the SPECIFIC differing sub-part when the two key types are
                    // same-kind compounds (a record field / tuple position / sum payload) — the peer-join
                    // hint the list/if/match/set sites carry.
                    let delta = peer_type_delta_hint(&fkt, &kt, &db.name_ctx()).unwrap_or_default();
                    // Anchor at the OUTLIER key `k` (the one that broke homogeneity against the first
                    // entry's key type), not the whole `(map …)` literal — the squiggle lands on the off
                    // entry's key, not the entire map (the map-literal twin of the list-outlier anchoring).
                    let mut reject = Reject::coded(
                        Code::Malformed,
                        format!(
                            "a map associates keys of one type, but the keys differ: {} and {}{delta}",
                            fkt.render_name(&db.name_ctx()),
                            kt.render_name(&db.name_ctx())
                        ),
                    )
                    .at(k);
                    if let Some(fix) = float_literal_retype_fix(db, fk, &fkt, &kt)
                        .or_else(|| float_literal_retype_fix(db, k, &kt, &fkt))
                    {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
                let vt = type_of(db, v);
                if crate::unify::unify(&mut vsubst, &fvt, &vt, &db.name_ctx()).is_err() {
                    let delta = peer_type_delta_hint(&fvt, &vt, &db.name_ctx()).unwrap_or_default();
                    // Anchor at the OUTLIER value `v`, not the whole `(map …)` literal — same locus fix as
                    // the key arm above.
                    let mut reject = Reject::coded(
                        Code::Malformed,
                        format!(
                            "a map associates values of one type, but the values differ: {} and {}{delta}",
                            fvt.render_name(&db.name_ctx()),
                            vt.render_name(&db.name_ctx())
                        ),
                    )
                    .at(v);
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
        // NAME the variant in the message (read from `app`'s source spelling — bare `None` or qualified
        // `Option.None`) and point at the errant payload argument, so the diagnostic is actionable: it says
        // WHICH constructor and HOW to fix it (drop the payload / use the unit form), the rustc-gold bar.
        let vname = ctor_app_name(db, app);
        for &arg in args {
            let at = type_of(db, arg);
            if !at.agrees_with(&Ty::Unit) {
                trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, at = %at.render_name(&db.name_ctx()), "fault: nullary variant applied to a non-unit payload (CDZ0201)");
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "the variant `{vname}` is nullary — it carries no payload, so it cannot be \
                             applied to a value of type {}; construct it as `{vname}` alone (or `({vname} \
                             unit)`)",
                            at.render_name(&db.name_ctx())
                        ),
                    )
                    .at(arg),
                );
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
        // Each buffered fault carries `always_flush`: `true` for a type-valued-param BOUNDARY-CHECK
        // violation (see `boundary_vars`), which β-reduction erases so step 2 can't re-detect it — it must
        // never be dropped as a "step-2 twin"; `false` for an ordinary annotated-param fault (droppable
        // when step 2 covers it).
        let mut arg_faults: Vec<(usize, Reject, bool)> = Vec::new();
        // Vars bound by a type-valued param (a `(: t Type)` checking boundary). A value-arg fault at a
        // param whose type mentions one of these is a BOUNDARY-CHECK violation β-reduction erases (step 2
        // can't re-detect it, since substituting the arg drops the `(: x t)` annotation), so it must ALWAYS
        // flush — never dropped as a "step-2 twin". (An ordinary annotated-param fault stays droppable.)
        let mut boundary_vars: crate::fxhash::FxHashSet<u32> = crate::fxhash::FxHashSet::default();
        if let Some(params) = crate::eval::lambda_params_of(db, head) {
            let mut subst = Subst::new();
            for (i, (&param_occ, &arg)) in params.iter().zip(args.iter()).enumerate() {
                let pt = type_of(db, param_occ);
                let at = type_of(db, arg);
                // BIDIRECTIONAL-CHECKING BOUNDARY (type-system.md #Generics Are Type-Valued Parameters,
                // line 60): a `(: t Type)` param is a checking boundary — the type it binds must be
                // CHECKED against a sibling annotation `(: x t)`, NOT solved by unification. A sibling
                // annotation reduces `t` to `Ty::Var(param_occ.0)` (`eval::typeval_of` →
                // `type_valued_param_binder` returns the SAME binder id as the var), but NOTHING bound that
                // var to the type VALUE the caller passed for `t` — so `f(Bool, 41)` for `(f (: t Type) (:
                // x t))` accepted (41's Int64 solved the var by unification, the passed `Bool` dead), an
                // over-accept the spec forbids. Bind it here: when this param is type-valued (its type is
                // `Ty::Type`) and the arg is a concrete type VALUE, unify `Ty::Var(param_occ.0)` :=
                // reflected(arg) into `subst` BEFORE the later sibling-value args unify against that var —
                // so `x`'s `41` (Int64) now conflicts with the bound `Bool` and rejects. Reflection via
                // `typeval_of`; a non-concrete type-value (a var / `Any`) binds nothing (stays generic,
                // solved by the value arg as before — no false reject on an undetermined type arg).
                //= spec/capabilities/type-system.md#inference-and-first-class-types-meet-at-a-bidirectional-boundary
                //# A position that binds a type-valued parameter MUST be a bidirectional-checking boundary, at which a type is either synthesized by monomorphization from the concrete type-value supplied or checked against an explicit annotation, rather than solved by unification, so that first-class computable types are reconciled with principal-type inference instead of contradicting it.
                if matches!(pt, Ty::Type)
                    && let Some(tv) = crate::eval::typeval_of(db, arg)
                    && !matches!(tv, Ty::Any)
                    && !tv.has_free_var()
                {
                    // A sibling annotation `(: x t)` reduces `t` to `Ty::Var(param_occ.0)` — the SAME
                    // binder id (`type_valued_param_binder`). Bind that var to the passed type value, and
                    // record it as a boundary var so a downstream sibling-value mismatch always flushes.
                    boundary_vars.insert(param_occ.0);
                    let _ =
                        crate::unify::unify(&mut subst, &Ty::Var(param_occ.0), &tv, &db.name_ctx());
                }
                if let Err(reject) = crate::unify::unify(&mut subst, &pt, &at, &db.name_ctx()) {
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
                    // Render the EXPECTED/actual types with the accumulated substitution APPLIED — an
                    // EARLIER argument may have already solved a shared type variable this parameter also
                    // mentions. `(def (pair (: t Type) (: x t) (: y t)) …)` called `(pair Int64 1 true)`:
                    // typing `x = 1` binds the shared `t`-var to `Int64` in `subst`, so `y`'s parameter type
                    // is Int64 — but the RAW `pt` is still the unsolved `Ty::Var`, which `render_name`
                    // prints as `_` ("a value of type `_` is expected here" — an unhelpful hole). Applying
                    // `subst` first renders the real "Int64", the type the sibling argument already pinned.
                    let spt = subst.apply(&pt);
                    let sat = subst.apply(&at);
                    let tail =
                        structural_delta_hint(&spt, &sat, &db.name_ctx()).unwrap_or_default();
                    let reject = Reject {
                        message: call_argument_mismatch_message(
                            callee.as_deref(),
                            param.as_deref(),
                            &spt,
                            &sat,
                            &tail,
                            &db.name_ctx(),
                        ),
                        ..reject
                    };
                    // A value of the parameter sum's PAYLOAD type where the SUM is expected — `(f 5)` to a
                    // `(: o (Option Int64))` parameter. Offer the rustc-flagship "wrap in `Some`" repair:
                    // WRAP the argument in the matching constructor `(Some 5)`. General over any sum (reads
                    // the expected sum's own variants), forced-choice only (ambiguous → no suggestion), and
                    // the wrap type-checks in one shot. Heuristic — the intent (which construction) is a guess.
                    // The fix heuristics read the SUBSTITUTED types too (a shared var an earlier arg solved
                    // is now concrete — e.g. a wrap/coercion suggestion keys off the real expected type, not
                    // an unsolved `Ty::Var` that matches nothing).
                    let tagged = if let Some(variant) = wrap_variant_for(db, &spt, &sat) {
                        reject.with_fix(Fix::wrap_heuristic(
                            arg,
                            format!("({variant} "),
                            ")",
                            format!("wrap the value in `{variant}`"),
                        ))
                    } else if let Some((prefix, suffix, verb)) = total_conversion_wrap(&spt, &sat) {
                        // A total prelude conversion bridges the mismatch at the CALL SITE — `String` where
                        // `Bytes` is expected → `(String.to-bytes …)`. The text-model twin of the numeric
                        // coercion wraps; heuristic, `--verify-fixes` upgrades it.
                        reject.with_fix(Fix::wrap_heuristic(arg, prefix, suffix, verb))
                    } else if let Some(fix) = record_field_typo_fix(db, &spt, &sat, arg)
                        .or_else(|| record_field_add_fix(db, &spt, &sat, arg))
                        .or_else(|| record_field_delete_fix(db, &spt, &sat, arg))
                        .or_else(|| tuple_element_add_fix(db, &spt, &sat, arg))
                        .or_else(|| tuple_element_delete_fix(db, &spt, &sat, arg))
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
                    // ALWAYS-FLUSH iff this param's type mentions a boundary var (a `(: t Type)` bound the
                    // var earlier this loop) — β-reduction erases the `(: x t)` annotation, so step 2 never
                    // re-detects the boundary-check violation; it is the SOLE report even for a referenced
                    // param. An ordinary annotated-param fault (no boundary var) stays droppable.
                    let mut pt_vars = Vec::new();
                    pt.collect_free_vars(&mut pt_vars);
                    let always_flush = pt_vars.iter().any(|v| boundary_vars.contains(v));
                    arg_faults.push((i, tagged, always_flush));
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
                // The baseline exists ONLY to avoid DUPLICATING a NAMED DEF callee's body faults — those
                // the callee's own `compile::collect_faults` reports at the definition (with its fix). It
                // must NOT fire for an INLINE lambda (`((fn (v0) (* (tuple) 0)) 0)`): an inline lambda's
                // body is never independently collected, so subtracting its unreduced-body faults DELETES
                // them entirely — `(* (tuple) 0)` (CDZ0201) slips past check and the backend emits invalid
                // wasm (the `v0`-unused case: reduction leaves the body unchanged, so its fault IS in the
                // baseline and gets filtered). Gate the baseline on the callee having a DEF INDEX (a named
                // def, top-level or a module member) — the only case whose body is separately collected.
                // An inline lambda (`callee_def_index_for_infer` = `None`) keeps an EMPTY baseline, so its
                // body faults surface here, exactly as the bare expression's do.
                // Is the callee a NAMED def (top-level `(def (f …) …)`, lambda-valued `(def f (fn …))`, or
                // a module member reached by name)? Only then is its body independently collected by
                // `compile::collect_faults`, making the baseline-subtraction a genuine DE-DUPLICATION. An
                // inline / anonymous `(fn …)` head has no name — its body is NOT separately collected, so
                // subtracting it would DELETE the fault. Keyed on the head resolving to a NAME (directly, or
                // through a `Ref`/`Member` chain to a named def), NOT on `callee_def_index_for_infer` — that
                // helper's `def_index_by_body` misses a lambda-valued def (its registered body is the `fn`
                // node, while the head resolves to the inner body), which would wrongly treat a named
                // lambda-valued def as inline and DOUBLE-report its body fault at every call site.
                let callee_is_named_def = named_callee_head(g, head);
                let baseline: std::collections::HashSet<(Option<crate::diag::Code>, String)> =
                    if body_faults.is_empty() || !callee_is_named_def {
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
        for (i, reject, always_flush) in arg_faults {
            let covered = reduced_ok && params.get(i).is_some_and(|&p| referenced.contains(&p));
            if always_flush || !covered {
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
                    // `(E 5)` for an effect `E`, `(Color 5)` for a type `Color`. Rendering `ht.render_name(&db.name_ctx())`
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
                    trace!(target: "rcdzc::infer", head = head.0, ty = %ht.render_name(&db.name_ctx()), "fault: applying a non-function value (CDZ0201)");
                    // A MISSING-COLON VALUE ANNOTATION — `(5 Int64)` written where `(: 5 Int64)` was
                    // meant. When a plain value head is applied to EXACTLY ONE argument that resolves as a
                    // TYPE (`typeval_of`), the author juxtaposed a value with its type instead of heading
                    // them with `:`, so it reads as an application of a non-function. This is the
                    // value-position twin of the parameter-position `(a Float64)` → `(: a Float64)` slice,
                    // and the argument-position counterpart of `(Int64 5)`'s "a type appears in an
                    // annotation `(: value Int64)`, not in call position". Name the real repair. When the
                    // head is a simple atom `atom_surface` can spell (a NAME or an INT literal — see its
                    // doc; a float/string/compound head yields no spelling) AND the type is a bare name,
                    // carry a HEURISTIC fix with the exact `(<value> <Type>)` → `(: <value> <Type>)`
                    // rewrite; otherwise the message alone routes the repair. The fix is heuristic, NOT
                    // verified — adding the `:` is the certain structural repair, but the resulting
                    // annotation may itself not hold (see the inner comment on the `(5 Bool)` case). Only
                    // in the `(None, None)` arm — a type/effect head or a nullary def keeps its own message.
                    let colon_annotation = if name_category.is_none()
                        && nullary_fn.is_none()
                        && args.len() == 1
                        && crate::eval::typeval_of(db, args[0]).is_some()
                    {
                        let value_text = atom_surface(db, head);
                        let type_text = db.ast.as_name(args[0]).map(str::to_string);
                        Some((value_text, type_text))
                    } else {
                        None
                    };
                    if let Some((value_text, type_text)) = colon_annotation {
                        let mut reject = Reject::coded(
                            Code::Malformed,
                            "a value is annotated `(: <value> <Type>)`, with a leading `:` — this value \
                             is juxtaposed with a type, so it reads as applying a non-function; add the \
                             `:` to annotate it",
                        )
                        .at(app);
                        if let (Some(v), Some(t)) = (&value_text, &type_text) {
                            // HEURISTIC, not Verified: adding the `:` is the certain STRUCTURAL repair,
                            // but the resulting annotation may itself not hold — `(5 Bool)` → `(: 5 Bool)`
                            // trades the CDZ0201 for a CDZ0203 (`Bool` does not match `Int64`). A Verified
                            // fix must CLEAR the diagnostic by construction; this one clears it only when
                            // the value's type satisfies the annotation, which the compiler has not proved
                            // here, so it stays a heuristic an agent confirms.
                            reject = reject
                                .with_fix(Fix::replace_heuristic(app, format!("(: {v} {t})")));
                        }
                        out.push(reject);
                        return;
                    }
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
                            ht.render_name(&db.name_ctx())
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
                let arg_ty = type_of(db, arg);
                let at = freshen_arg(db, &arg_ty, &mut fresh);
                if let Err(reject) = crate::unify::unify(&mut subst, &param, &at, &db.name_ctx()) {
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
                        let delta = structural_delta_hint(&sparam, &sat, &db.name_ctx())
                            .unwrap_or_default();
                        let mut reject = Reject::coded(
                            Code::Malformed,
                            format!(
                                "a variant constructor's payload has declared type {}, but a value of \
                                 type {} was applied{delta}",
                                sparam.render_name(&db.name_ctx()),
                                sat.render_name(&db.name_ctx())
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
                                sparam.render_name(&db.name_ctx()),
                                sat.render_name(&db.name_ctx())
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
                                    sat.render_with_article(&db.name_ctx()),
                                    sparam.render_name(&db.name_ctx())
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
                                    sat.render_with_article(&db.name_ctx()),
                                    sparam.render_name(&db.name_ctx())
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
                    } else if let Some(hint) =
                        option_payload_mismatch_hint(&db.name_ctx(), &sparam, &sat)
                    {
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
                        // When the two types are SAME-KIND compounds that differ structurally (a record
                        // field-set / field-type diff, a tuple arity diff, a sum-payload diff), append the
                        // minimal-conflict delta the annotation / operator-arg / peer-join sites carry —
                        // `(Log.put (record (y 2)))` for a `(Record (x Int64))` op then reads "… but field
                        // `x` is missing (found `y`)" instead of leaving the reader to compare two rendered
                        // record types. The M180/M181 structural-delta audit applied to the effect-op arm.
                        let delta = structural_delta_hint(&sparam, &sat, &db.name_ctx())
                            .unwrap_or_default();
                        let mut reject = Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "operation {op} expects an argument of type {}, but a value of type {} \
                                 was performed{delta}",
                                sparam.render_name(&db.name_ctx()),
                                sat.render_name(&db.name_ctx())
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
                        // Append the SAME structural-delta hint the effect-op / operator-arg / annotation
                        // sites carry, so a same-kind compound mismatch — `(List.push xs (record (y 2)))`
                        // for a `List (Record (x Int64))`, a tuple-arity diff — names the minimal conflict
                        // (`field `x` is missing (found `y`)`) instead of leaving the reader to diff two
                        // rendered compound types. The M180/M181 audit applied to the prelude-member-op arm.
                        let delta = structural_delta_hint(&sparam, &sat, &db.name_ctx())
                            .unwrap_or_default();
                        // Anchor at the offending ARGUMENT `arg`, not the whole `(Module.member …)`
                        // application node — the squiggle points at the wrong-typed argument, the actionable
                        // locus (`(List.at xs true)` lands on `true`, not the `List.at` head), matching the
                        // file's "anchor the specific offending element" pattern (PR #399 anchoring family).
                        let mut reject = Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "`{module}.{member}` expects an argument of type {}, but a value of \
                                 type {} was given{delta}",
                                sparam.render_name(&db.name_ctx()),
                                sat.render_name(&db.name_ctx())
                            ),
                        )
                        .at(arg);
                        if let Some(fix) = numeric_text_coercion_fix(db, &sparam, &sat, arg) {
                            reject = reject.with_fix(fix);
                        }
                        out.push(reject);
                    } else if let Some(hint) = fn_not_applied_hint(&sparam, &sat, &db.name_ctx()) {
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
                                sparam.render_name(&db.name_ctx())
                            ),
                            ..reject
                        });
                    } else if let Some(delta) = structural_delta_hint(&sparam, &sat, &db.name_ctx())
                    {
                        // Two SAME-KIND compounds that differ structurally — two records of different field
                        // sets (`(= (record (x 1)) (record (y 2)))`), two tuples of different arity, two
                        // collections of a differing element/key/value type, two same-sum-type payloads.
                        // These fall through every coercion/wrap branch (no total conversion bridges them)
                        // to the raw unify "type mismatch: (Record (x Int64)) and (Record (y Int64)) must be
                        // the same type here, but differ" — which BURIES the actual difference (field `x` vs
                        // `y`). REPLACE the raw lead with the readable arg-site phrasing and append the
                        // structural-delta hint (`field `y` … `, `element 1 …`) the annotation / peer-join
                        // sites already carry, so the operator-arg position names the minimal conflict too.
                        // No mechanical fix (retyping the structure is the author's choice); the reject's
                        // code is kept. Last structural branch before the bare fallthrough.
                        out.push(Reject {
                            message: format!(
                                "this argument is {}, but a value of type {} is expected here{delta}",
                                sat.render_with_article(&db.name_ctx()),
                                sparam.render_name(&db.name_ctx())
                            ),
                            ..reject
                        });
                    } else {
                        out.push(reject);
                    }
                } else if crate::eval::effect_op_of(db, head).is_some() {
                    // SUCCESSFUL unify against an effect-op parameter — but a DEFERRED integer LITERAL
                    // `(Send.put 999)` AGREES with any int width (the deferred width is compatible), so the
                    // unify does not fault, yet `999` does not FIT a `UInt8` op parameter. Run the same
                    // range-check (CDZ0302) every SIBLING narrow position enforces — a plain-fn param
                    // (`(f 999)` for `(: v UInt8)`, via the substitution/annotation path), an annotated
                    // literal (`(: 999 UInt8)`), a variant payload, the handle SEED. The effect-op perform
                    // is NOT inlined (the fold discharges it), so it never reaches those paths and the
                    // out-of-range literal silently inhabited the narrow binder → the arm OBSERVED the
                    // over-range value (`999` in a `UInt8` slot) and, for a HOST-delegated op, it crossed
                    // the COMPONENT boundary in the declared-width slot (breaker nw-class, operator-confirmed
                    // soundness). Fit-check the arg against the op's SOLVED parameter type here — the
                    // perform-site analogue of the parameter-substitution fit-check. `width_fault_against_ty`
                    // descends a compound (record/tuple/list payload) + handles the narrow-int / Float32
                    // cases, so a compound op argument's nested literal is checked too. Covers the ARGUMENT
                    // direction (all widths, both signs); the RESUME-VALUE-vs-op-RESULT direction (nw8) is
                    // checked at the resume-result site.
                    let sparam = subst.apply(&param);
                    if let Some(reject) = width_fault_against_ty(db, arg, &sparam) {
                        trace!(target: "rcdzc::infer", head = head.0, arg = arg.0, "fault: effect-op argument literal does not fit the declared narrow width (CDZ0302)");
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
                    trace!(target: "rcdzc::infer", head = head.0, ty = %other.render_name(&db.name_ctx()), "apply: applied a non-function (type fault)");
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
                                other.render_name(&db.name_ctx())
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
