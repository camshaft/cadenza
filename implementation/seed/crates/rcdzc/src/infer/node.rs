//! `infer::node` — the per-node fault collector (`collect_node`) and its inline-lambda re-entrancy
//! guard, extracted verbatim from `infer.rs` to keep the parent under the source-size limit.
//! Behavior + API unchanged: `collect_node` stays crate-private and is re-imported into `infer`
//! via `use node::*`; its sole call site (`collect`) resolves through that glob.

use super::*;

thread_local! {
    /// Re-entrancy depth of the uncalled-inline-lambda body check in [`collect_node`]'s `Lambda` arm —
    /// bounds the collect/solve recursion so a synthesized lambda whose body re-enters (a map-match desugar
    /// over a self-recursive def) cannot overflow the stack. Reset to 0 between top-level checks by being a
    /// balanced increment/decrement around the recursive `collect`.
    static INLINE_LAMBDA_CHECK_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(crate) fn collect_node(db: &mut Db, id: StructId, out: &mut Vec<Reject>) {
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
                trace!(target: "rcdzc::infer", node = id.0, cond_ty = %cond_ty.render_name(&db.name_ctx()), "fault: if condition not Bool (CDZ0203)");
                // Anchor at the CONDITION expression, not the whole `(if …)` — the squiggle then lands on
                // the non-Bool value (`5` in `(if 5 …)`), the actual culprit. (Without `.at`, `collect`
                // stamps the coarse `if`-node default.) The branch-mismatch reject below stays at the `if`
                // node: it concerns the RELATIONSHIP between both branches, not one sub-node.
                out.push(
                    Reject::coded(
                        Code::TypeMismatch,
                        format!(
                            "if condition must be Bool, found {}",
                            cond_ty.render_name(&db.name_ctx())
                        ),
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
                trace!(target: "rcdzc::infer", node = id.0, then_ty = %then_ty.render_name(&db.name_ctx()), else_ty = %else_ty.render_name(&db.name_ctx()), ?code, "fault: if branches differ");
                let delta =
                    peer_type_delta_hint(&then_ty, &else_ty, &db.name_ctx()).unwrap_or_default();
                let mut reject = Reject::coded(
                    code,
                    format!(
                        "if branches differ: {} vs {}{delta}",
                        then_ty.render_name(&db.name_ctx()),
                        else_ty.render_name(&db.name_ctx())
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
                    trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(&db.name_ctx()), "fault: connective operand not Bool (CDZ0201)");
                    // Anchor at the offending OPERAND, not the whole `(and …)`/`(or …)`.
                    out.push(
                        Reject::coded(
                            Code::Malformed,
                            format!(
                                "`{op}` operand must be Bool, found {}",
                                t.render_name(&db.name_ctx())
                            ),
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
                trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(&db.name_ctx()), "fault: not operand not Bool (CDZ0201)");
                // Anchor at the offending OPERAND, not the whole `(not …)`.
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        format!(
                            "`not` operand must be Bool, found {}",
                            t.render_name(&db.name_ctx())
                        ),
                    )
                    .at(operand),
                );
            }
            collect(db, operand, out);
        }
        // `(try e)` — the fallible short-circuit operator (`DESIGN-try-operator-rcdzc.md`). Two checks,
        // each firing only on a DEFINITE type (an unsolved `Any`/`Var` is left alone — no over-rejection,
        // exactly as the `Not`/`And` operand checks do; its own fault surfaces via the descent):
        //   (a) OPERAND shape — `?` on a DEFINITE non-fallible type has nothing to unwrap → CDZ0203 (§5).
        //   (b) BOUNDARY (§4 v1 / §6) — `?` short-circuits the enclosing function's fallible result type.
        //       No enclosing function, or a DEFINITE non-fallible enclosing result type, → CDZ0230 (no
        //       boundary admits the `?`). When BOTH operand and boundary are definite fallible sums of
        //       DIFFERENT kinds (a `Result`-`?` under an `Option` boundary, or vice-versa) → CDZ0203 (§5:
        //       no coercion). When both are `Result` but their ERROR types disagree (definite, unequal) →
        //       CDZ0203 too: the failure arm passes `Err(oe)` out UNCHANGED as the boundary value, so `oe`
        //       must match the boundary's error type (else a `Bool` error escapes as a claimed `Int64`).
        Resolved::Try { operand } => {
            // Collect the OPERAND's own faults FIRST — if `e` is itself ill-typed (a numeric mismatch, an
            // unbound name, …), THAT is the primary diagnostic, and the `?`-shape/boundary checks below
            // would only pile a confusing "operand must be fallible, found <fallback-type>" cascade on top
            // (`(try (+ 1 2.0))` reported "not fallible, found Float64" ahead of the real CDZ0301). So gate
            // the `?`-specific checks on the operand being clean, the same "let the operand's own error be
            // primary" discipline the `Member` arm applies via `operand_is_poison`.
            let before = out.len();
            collect(db, operand, out);
            let operand_has_own_fault = out.len() > before;
            let t = type_of(db, operand);
            let operand_fallible = fallible_shape(db, &t);
            let operand_definite = !matches!(t, Ty::Any | Ty::Var(_));
            if operand_has_own_fault {
                // The operand's own fault is the primary "no"; add nothing `?`-specific on top.
            } else if operand_definite && operand_fallible.is_none() {
                trace!(target: "rcdzc::infer", node = id.0, ty = %t.render_name(&db.name_ctx()), "fault: try operand not Option/Result (CDZ0203)");
                out.push(
                    Reject::coded(
                        Code::TypeMismatch,
                        format!(
                            "`?` operand must be a fallible `Result`/`Option`, found {}",
                            t.render_name(&db.name_ctx())
                        ),
                    )
                    .at(operand),
                );
            } else {
                // The operand is (or may be) fallible — check the enclosing boundary admits it.
                match enclosing_boundary_ty(db, id) {
                    // The boundary walk fell off the tree WITHOUT reaching an enclosing function body.
                    // In the ordinary (non-inlined) `type_errors` pass the walk ALWAYS reaches the def
                    // body (its `def_index_by_body` is `Some`), so a genuinely-non-fallible boundary is
                    // caught via the `Some(bt)` arm below, not here. This `None` is reached only when a
                    // β-reduction/inline COPY re-parented the `?`'s ancestors so the chain no longer leads
                    // to a `def_by_body`-registered root (the copy machinery re-anchors a synthesized
                    // subtree, and the boundary body of the copy is not itself an indexed def body) — the
                    // same copy-reparenting hazard as `guarded-literal-list-false-cdz0101-copy-ordering`.
                    // Treating it as a fault raised a FALSE CDZ0230 on a well-formed `?` in a CALLED
                    // (inlined, non-exported) helper (`(def (f) (let ((x (try (Some 7)))) (Some (+ x 3))))`
                    // called from `main`) — the boundary IS `Option`, but the inlined copy's walk fell off.
                    // So `None` is INCONCLUSIVE (like an unsolved boundary): raise nothing. The genuine
                    // "no fallible boundary" reject still fires from the original body's walk (parents
                    // intact) via `Some(bt)`.
                    None => {
                        trace!(target: "rcdzc::infer", node = id.0, "`?` boundary walk inconclusive (inlined-copy reparent) — no fault");
                    }
                    Some(bt) => {
                        let boundary_fallible = fallible_shape(db, &bt);
                        let boundary_definite = !matches!(bt, Ty::Any | Ty::Var(_));
                        match boundary_fallible {
                            None if boundary_definite => {
                                trace!(target: "rcdzc::infer", node = id.0, ty = %bt.render_name(&db.name_ctx()), "fault: `?` boundary not fallible (CDZ0230)");
                                // Name the CONCRETE fallible result the operand's kind implies, so the
                                // "annotate it as …" hint gives the exact type to write, not a generic
                                // `_`. An `Option` operand → `(Option <payload>)`; a `Result` operand →
                                // `(Result <payload> <err>)`. Falls back to the generic form when the
                                // operand's fallible shape isn't (yet) definite.
                                // The suggested annotation, ALREADY BACKTICK-WRAPPED here (not by the
                                // message template) so each option is exactly one balanced code span. A
                                // DEFINITE operand kind → one concrete form (`(Option T)` / `(Result T e)`);
                                // the fallback (operand kind not yet definite) → BOTH generic forms, each
                                // its own span. (Copilot PR #453: the old fallback embedded its own
                                // backticks INTO a template-wrapped `{suggested}`, coupling its rendering to
                                // the template's exact quoting; wrapping here keeps every arm balanced and
                                // self-contained.)
                                let suggested = match &operand_fallible {
                                    Some((FallibleKind::Option, payload, _)) => {
                                        format!(
                                            "`(Option {})`",
                                            payload.render_name(&db.name_ctx())
                                        )
                                    }
                                    Some((FallibleKind::Result, payload, err)) => format!(
                                        "`(Result {} {})`",
                                        payload.render_name(&db.name_ctx()),
                                        err.as_ref().map_or("e".to_string(), |e| e
                                            .render_name(&db.name_ctx()))
                                    ),
                                    None => "`(Result _ e)` or `(Option _)`".to_string(),
                                };
                                out.push(
                                    Reject::coded(
                                        Code::TryNoBoundary,
                                        format!(
                                            "`?` has no fallible boundary — the enclosing function's \
                                             result type is `{}`, neither `Result` nor `Option`. \
                                             Annotate it as {suggested} (the kind the `?`'d value \
                                             requires), or wrap the expression in a `try {{ … }}` block.",
                                            bt.render_name(&db.name_ctx())
                                        ),
                                    )
                                    .at(id),
                                );
                            }
                            // Boundary unsolved — cannot yet judge; leave it (no over-rejection).
                            None => {}
                            Some((bkind, _, boundary_err)) => {
                                if let Some((okind, _, operand_err)) = operand_fallible {
                                    if okind != bkind {
                                        // Kinds disagree — a `Result`-`?` under an `Option` boundary (or
                                        // vice-versa) cannot short-circuit (§5, no coercion).
                                        trace!(target: "rcdzc::infer", node = id.0, "fault: `?` operand kind disagrees with the boundary (CDZ0203)");
                                        // The concrete conversion idiom, phrased in terms of forms that
                                        // EXIST (a `match` re-wrap) — NOT `Result.map-err`/`Option.ok-or`,
                                        // which are not yet in the prelude (a "did you mean X" must not name
                                        // an absent op; those helpers are the T3 increment).
                                        let (o, b, convert) = match okind {
                                            FallibleKind::Option => (
                                                "Option",
                                                "Result",
                                                "match the `Option` and supply an error \
                                                 (`(None) => (Err e)`, `(Some x) => (Ok x)`)",
                                            ),
                                            FallibleKind::Result => (
                                                "Result",
                                                "Option",
                                                "match the `Result` and drop the error \
                                                 (`(Err _) => (None unit)`, `(Ok x) => (Some x)`)",
                                            ),
                                        };
                                        out.push(
                                            Reject::coded(
                                                Code::TypeMismatch,
                                                format!(
                                                    "a `{o}`-valued `?` cannot short-circuit a `{b}` \
                                                     boundary — the enclosing function returns `{}`. \
                                                     Either change the boundary's result type, or \
                                                     {convert} before the `?`.",
                                                    bt.render_name(&db.name_ctx())
                                                ),
                                            )
                                            .at(operand),
                                        );
                                    } else if bkind == FallibleKind::Result
                                        && let (Some(oe), Some(be)) = (operand_err, boundary_err)
                                        && !matches!(oe, Ty::Any | Ty::Var(_))
                                        && !matches!(be, Ty::Any | Ty::Var(_))
                                        && !oe.agrees_with(&be)
                                    {
                                        // Kinds AGREE (both `Result`), but the ERROR types disagree. On the
                                        // failure arm the operand's `Err(oe)` flows out UNCHANGED as the
                                        // boundary value, so `oe` MUST match the boundary's error type `be`
                                        // (§5: "the error type `b` unifies with the boundary's"). Without
                                        // this a `(try (Err true))` short-circuits a `(Result _ Int64)`
                                        // boundary, presenting a `Bool` where the boundary claims `Int64` —
                                        // a soundness hole the ordinary `(: (Err true) (Result _ Int64))`
                                        // annotation path already rejects. No coercion (no `From`), so a
                                        // definite mismatch is CDZ0203.
                                        trace!(target: "rcdzc::infer", node = id.0, "fault: `?` operand error type disagrees with the boundary (CDZ0203)");
                                        out.push(
                                            Reject::coded(
                                                Code::TypeMismatch,
                                                format!(
                                                    "the `?`'d `Result`'s error type `{}` does not match \
                                                     the enclosing function's error type `{}` — a `?` \
                                                     short-circuits by passing its `Err` out unchanged, so \
                                                     the error types must agree (Cadenza has no automatic \
                                                     error conversion). `match` the `Result` and re-wrap \
                                                     its error to `{}`, or change the boundary's error type.",
                                                    oe.render_name(&db.name_ctx()),
                                                    be.render_name(&db.name_ctx()),
                                                    be.render_name(&db.name_ctx()),
                                                ),
                                            )
                                            .at(operand),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // (The operand's own faults were already collected at the top of this arm.)
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
            // A record-PATTERN field binder's arm-BODY reference resolves to this same `Member` shape:
            // `((record (nope a)) a)` — the body `a` resolves (Case 6rec) to `Member{operand: scrutinee,
            // key: nope}`, a PROJECTION of the matched value at the pattern's field. When `nope` is absent
            // from the scrutinee's record, the canonical, richer CDZ0201 "a record pattern names field
            // `nope`, which the matched value of type … does not have" ALREADY fires at the pattern
            // (`lower.rs`), so a generic "record has no field `nope`" (CDZ0212) here is a REDUNDANT second
            // diagnostic for one error. Distinguish this synthesized-Member body-ref from a genuine member
            // ACCESS syntactically: a real `(. operand key)` is a LIST form, whereas a pattern-binder ref is
            // a BARE NAME atom that merely RESOLVED to a Member. So for a bare name, SKIP this whole
            // Member-access fault arm — not just the no-field case, ALL of it (no-field AND not-a-record):
            // a bare name that resolves to a Member is ALWAYS a pattern-binder projection (never a genuine
            // access, which is always a `(. …)` list), so no member-access fault is ever legitimate here;
            // the pattern-lowering reject is the sole primary. (Node-syntactic + safe: a genuine `(. …)`
            // access on any record keeps its own reject, incl. a distinct absent field of the same name on
            // another record; only the pattern binder's bare-name ref is suppressed. v-patterns traced this
            // to the body-ref — the pattern-position binder is not the source, so a pattern-position or
            // dedup-node key would miss it; the bare-name test at this fault site does.)
            if db.ast.as_name(id).is_some() {
                return;
            }
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
                            let ty_name = other.render_name(&db.name_ctx());
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
                            type_of(db, operand).render_name(&db.name_ctx())
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
        // `(list 1 true)`) is a MALFORMED collection (CDZ0201), UNIFORMLY (collections-and-text.md §A
        // Collection's Homogeneity Violation Is A Malformed Collection), the same code the `list`
        // name-alias path, the map/set homogeneity checks, and `List.push`/`update`/`concat` use — NOT
        // the generic unify's CDZ0203 (reserved for a two-types-must-agree conflict). Then descend into
        // each element for its own faults.
        // A set shares the list's element homogeneity + range-check + element-descent (both are homogeneous
        // element sequences), so the fault-walk is identical; only the message says "list" (cosmetic).
        Resolved::List { elems } | Resolved::Set { elems } => {
            let mut subst = Subst::new();
            if let Some(&first) = elems.first() {
                let first_ty = type_of(db, first);
                let mut homogeneity_fault = false;
                for &e in elems.iter().skip(1) {
                    let et = type_of(db, e);
                    if crate::unify::unify(&mut subst, &first_ty, &et, &db.name_ctx()).is_err() {
                        homogeneity_fault = true;
                        let code = list_homogeneity_code(&first_ty, &et);
                        trace!(target: "rcdzc::infer", node = id.0, ?code, "fault: list elements differ in type");
                        // Two same-dimension DIFFERENT-scale quantities render to the same name (scale
                        // dropped), so the bare "must share one type: (Qty … meter) and (Qty … meter)" reads
                        // as a contradiction — route through the peer-join delta chain (which carries the
                        // qty-scale-mismatch tail) so this fault-walker site names the real cause, matching
                        // the `.at`-anchored list-literal join site.
                        let delta = peer_type_delta_hint(&first_ty, &et, &db.name_ctx())
                            .unwrap_or_default();
                        // Anchor at the OFFENDING element `e`, not the whole list node — the squiggle points
                        // at the specific element whose type breaks homogeneity, the minimal culprit (PR #399
                        // review; matches the file's "anchor the specific offending element" pattern). Without
                        // the explicit `.at(e)`, `collect`'s `set_origin_if_absent(id)` would stamp the whole
                        // `(list …)` node, highlighting the entire list rather than the one bad element.
                        // Attach the one-shot repair the peer-clash sites (if-branch, map axis) give, so a
                        // NATIVE `#list(…)`/`#set(…)` element clash proposes a fix exactly as the name-alias
                        // `(list …)` did pre-M3 (the M3 nativization routed these literals to this
                        // `Resolved::List | Resolved::Set` arm, which lacked the fix — a corpus fix-quality
                        // regression). An INT-LITERAL-vs-FLOAT clash retypes whichever element IS the int
                        // literal (`#list(1.0 2)` → `2.0`, `#list(1 2.0)` → `1.0`, sign preserved); a
                        // records-differing-by-a-misspelled-field clash renames the typo'd field on whichever
                        // element carries it. Mirrors the if-branch peer-clash fix chain above.
                        let mut reject = Reject::coded(
                            code,
                            format!(
                                "list elements must share one type: {} and {}{delta}",
                                first_ty.render_name(&db.name_ctx()),
                                et.render_name(&db.name_ctx())
                            ),
                        )
                        .at(e);
                        if let Some(fix) = float_literal_retype_fix(db, e, &et, &first_ty)
                            .or_else(|| float_literal_retype_fix(db, first, &first_ty, &et))
                            .or_else(|| record_field_typo_fix(db, &first_ty, &et, e))
                            .or_else(|| record_field_typo_fix(db, &et, &first_ty, first))
                        {
                            reject = reject.with_fix(fix);
                        }
                        out.push(reject);
                    }
                }
                // RANGE-CHECK each element against the SETTLED element type — the sibling-unification twin of
                // the `Apply(ListNew)` arm above (see its comment). A homogeneous list whose element width is
                // fixed by an ANNOTATED sibling (`(list (: 1 UInt64) -41)`) must re-validate each bare literal
                // against that width, or a `-41`/over-max element slips `cdz check` while wasm wraps + rust
                // E0308s (the backend-divergent miscompile). Only when homogeneous (else the settled type is
                // not meaningful and the homogeneity reject stands).
                if !homogeneity_fault {
                    // The JOIN of all element types — takes the FIXED width/sign from whichever sibling
                    // supplies it, position-independently (see the `Apply(ListNew)` arm's comment).
                    let settled = elems
                        .iter()
                        .skip(1)
                        .fold(first_ty.clone(), |acc, &e| acc.join(&type_of(db, e)));
                    if let Some(reject) = elems
                        .iter()
                        .find_map(|&e| width_fault_against_ty(db, e, &settled))
                    {
                        out.push(reject);
                    }
                }
            }
            // A native `#set` LITERAL keys/hashes its ELEMENTS (a `#list` does NOT), so a set element that is
            // a FUNCTION (CDZ0216, no canonical identity) or an ABSTRACT type (CDZ0202, representation not
            // observable here) is rejected at the literal — the SAME constraint `Set.of`/`Set.insert` enforce
            // as prim apps. Without this the native `#set` literal was a silent BYPASS (M2 soundness hole: the
            // s-expr `(Set.of (list (fn …)))` = a `Prim::SetOf` app correctly declined CDZ0216, but the M2
            // printer's `#(fn …)` native set-literal sailed through). `type_of` is `Ty::Set(k)` for a set,
            // `Ty::List(_)` for a list — so this fires ONLY for the set (a list of functions stays legal).
            if let Ty::Set(k) = type_of(db, id) {
                push_unhashable_key_fault(db, id, &k, out);
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
                    // NAME the two clashing types + carry the peer-delta hint + the int→float retype fix,
                    // anchored at the OUTLIER entry — the SAME diagnostic quality the `map` name-alias
                    // (`Apply(MapNew)`) path gives in `check_application`. The native `#map` literal
                    // (`Resolved::Map`) must not give a WORSE (type-name-less) message than the alias for the
                    // identical fault; the two forms were split apart by the infer.rs submodule extraction
                    // (#6039), which left this literal arm on the generic wording — restored here to match.
                    let kt = type_of(db, k);
                    if crate::unify::unify(&mut ksubst, &fkt, &kt, &db.name_ctx()).is_err() {
                        trace!(target: "rcdzc::infer", node = id.0, "fault: map keys differ in type (CDZ0201)");
                        let delta =
                            peer_type_delta_hint(&fkt, &kt, &db.name_ctx()).unwrap_or_default();
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
                        trace!(target: "rcdzc::infer", node = id.0, "fault: map values differ in type (CDZ0201)");
                        let delta =
                            peer_type_delta_hint(&fvt, &vt, &db.name_ctx()).unwrap_or_default();
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
            // A repeated COMPILE-TIME-CONSTANT key makes the association ambiguous → CDZ0201.
            if let Some(reject) = map_duplicate_const_key(db, &entries) {
                trace!(target: "rcdzc::infer", node = id.0, "fault: map has a duplicate constant key (CDZ0201)");
                out.push(reject);
            }
            // A native `#map` LITERAL keys/hashes its KEYS, so a FUNCTION key (CDZ0216) or an ABSTRACT-typed
            // key (CDZ0202) is rejected at the literal — the SAME constraint `Map.new`/`Map.insert`/lookup
            // enforce as prim apps (the `#map` sibling of the `#set` bypass above; M2 native-literal soundness).
            if let Ty::Map(k, _) = type_of(db, id) {
                push_unhashable_key_fault(db, id, &k, out);
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
                // A segment's VALUE must match its KIND *and*, for an integer segment, its WIDTH TYPE. A
                // `Utf8` segment takes a String, a `Bytes` segment a Bytes. A fixed-width int/bits segment
                // takes the segment's OWN width type: `(u8 v)` requires `v : UInt8`, `(bits v k)` requires
                // `v : (UInt k)`. A value that provably fits its type has no out-of-range case, so an int
                // segment needs no runtime range-check and cannot trap — a wider/negative value is a
                // COMPILE-TIME type error (CDZ0203), and narrowing (`UInt8.wrap`/`UInt8.of`) is the caller's
                // job. Flag ONLY a DEFINITE mismatch: a String/Bytes conflict for utf8/bytes, and for an int
                // segment a CONCRETELY-fixed integer type (both sign and width fixed) that differs from the
                // segment's — an `Any`/`Var` slot (a binder, an unreduced param, a bin PATTERN's binder,
                // which types as the decoded value) or a DEFERRED bare literal (grounds to the width + range-
                // checks separately, CDZ0304/CDZ0302) is never flagged, so a valid construction/pattern
                // stays clean.
                match &seg.kind {
                    crate::resolved::SegKind::Int { .. }
                    | crate::resolved::SegKind::Bits { .. } => {
                        let want =
                            seg_value_ty(&seg.kind).expect("int/bits segment has a value type");
                        let got = type_of(db, seg.slot);
                        if let (Ty::Int(want_it), Ty::Int(got_it)) = (&want, &got)
                            && got_it.width_is_fixed()
                            && matches!(got_it.sign, crate::ty::Sign::Fixed(_))
                            && (got_it.ground_signed() != want_it.ground_signed()
                                || got_it.ground_width() != want_it.ground_width())
                        {
                            // The narrowing conversion is a MEMBER of the width type's module, and BOTH the
                            // module spelling AND which conversions it offers depend on whether the width is
                            // ALIASED. An aliased width ({8,16,32,64}) has a BOUND module name (`UInt8`) that
                            // carries BOTH `wrap` (truncate) and `of` (checked) — so `UInt8.wrap`/`UInt8.of`.
                            // A NON-aliased width (`(UInt 4)`, a bit-field's own type) has NO bound name
                            // (`render_name` produces `UInt4`, an UNBOUND identifier); its on-demand module,
                            // reached via the type-constructor member form `(. (UInt 4) wrap)`, offers only
                            // `wrap` (no `of`). So suggest BOTH conversions only for an aliased width, and
                            // only `wrap` (in the member spelling) otherwise — never the unbound `UInt4.wrap`
                            // nor a `(. (UInt 4) of)` that does not resolve (PR #377 review).
                            let module = width_module_spelling(want_it);
                            let aliased =
                                crate::ty::ALIASED_INT_WIDTHS.contains(&want_it.ground_width());
                            let how = if aliased {
                                format!(
                                    "{} to truncate, or {} to check",
                                    width_conversion_spelling(&module, "wrap"),
                                    width_conversion_spelling(&module, "of"),
                                )
                            } else {
                                format!(
                                    "{} to truncate",
                                    width_conversion_spelling(&module, "wrap"),
                                )
                            };
                            out.push(
                                Reject::coded(
                                    Code::TypeMismatch,
                                    format!(
                                        "a bin {} segment takes a {} value, but this value is {} — \
                                         convert it explicitly (e.g. {how}) before placing it in the \
                                         segment",
                                        seg_kind_name(&seg.kind),
                                        want.render_name(&db.name_ctx()),
                                        got.render_name(&db.name_ctx()),
                                    ),
                                )
                                .at(seg.slot),
                            );
                        } else if definite_non_int(&got) {
                            // A non-integer value in an integer segment (`(bin (u8 "x"))`) — the wrong KIND.
                            out.push(
                                Reject::coded(
                                    Code::IllFormedBinary,
                                    format!(
                                        "a bin {} segment takes an integer value, but this value is {}",
                                        seg_kind_name(&seg.kind),
                                        got.render_name(&db.name_ctx())
                                    ),
                                )
                                .at(seg.slot),
                            );
                        }
                    }
                    crate::resolved::SegKind::Utf8 { .. } => {
                        if definite_conflicts_with(&type_of(db, seg.slot), &Ty::String) {
                            out.push(
                                Reject::coded(
                                    Code::IllFormedBinary,
                                    format!(
                                        "a bin utf8 segment takes a String value, but this value is {}",
                                        type_of(db, seg.slot).render_name(&db.name_ctx())
                                    ),
                                )
                                .at(seg.slot),
                            );
                        }
                    }
                    crate::resolved::SegKind::Bytes { .. } => {
                        if definite_conflicts_with(&type_of(db, seg.slot), &Ty::Bytes) {
                            out.push(
                                Reject::coded(
                                    Code::IllFormedBinary,
                                    format!(
                                        "a bin bytes segment takes a Bytes value, but this value is {}",
                                        type_of(db, seg.slot).render_name(&db.name_ctx())
                                    ),
                                )
                                .at(seg.slot),
                            );
                        }
                    }
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
                    validate_non_type_annotation(db, ann[1], "a binder's annotation", false, out);
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
                        let delta = peer_type_delta_hint(&first_ty, &bt, &db.name_ctx())
                            .unwrap_or_default();
                        let mut reject = Reject::coded(
                            Code::TypeMismatch,
                            format!(
                                "match arms differ: {} vs {}{delta}",
                                first_ty.render_name(&db.name_ctx()),
                                bt.render_name(&db.name_ctx())
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
                                st.render_name(&db.name_ctx())
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
                    // GUARDS MUST BE SIDE-EFFECT-FREE (operator directive, PR #2543): a guard condition that
                    // PERFORMS an effect is a compile error (CDZ0407). A guard is a boolean predicate the
                    // pattern engine may evaluate speculatively or repeatedly, so an effect performed in it has
                    // no well-defined evaluation count/order — the re-evaluation miscompile breaker finding #9
                    // exposed (an inline performing scrutinee + performing guard on a miss re-drew the
                    // scrutinee). Forbid ALL effects in guards (no non-mutating carve-out — operator's
                    // consistency call), rejecting at the offending `(E.op …)` node (perform-detection owned by
                    // v-effects, `effects::effect_op_in_guard_cond`). Checked BEFORE the Bool-type check so a
                    // performing guard names the real fault (the effect) rather than a downstream type message.
                    if let Some(op) = crate::effects::effect_op_in_guard_cond(db, cond) {
                        trace!(target: "rcdzc::infer", node = op.0, "fault: effect performed in guard (CDZ0407)");
                        out.push(
                            Reject::coded(
                                Code::EffectInGuard,
                                "a guard must be side-effect-free — an effect is performed in this guard, \
                                 which the pattern engine may evaluate speculatively or repeatedly; lift it \
                                 to a `let` evaluated once before the `match` and guard on the bound value",
                            )
                            .at(op),
                        );
                    }
                    let cond_ty = type_of(db, cond);
                    if !cond_ty.agrees_with(&Ty::Bool) {
                        trace!(target: "rcdzc::infer", node = cond.0, cond_ty = %cond_ty.render_name(&db.name_ctx()), "fault: guard condition not Bool (CDZ0203)");
                        out.push(
                            Reject::coded(
                                Code::TypeMismatch,
                                format!(
                                    "guard condition must be Bool, found {}",
                                    cond_ty.render_name(&db.name_ctx())
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
                        // The canonical `(= key value)` FieldPair (map entries unify with record fields).
                        crate::ast::Struct::List(_) if db.ast.field_pair(entry).is_some() => {}
                        // The legacy raw `(key value)` pair (accepted through the migration).
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
                                    ot.render_name(&db.name_ctx())
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
                        let field_names: Vec<&str> = fields.keys().map(|k| &*k.name).collect();
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
            ) && args.len() == 3
            {
                // `Record.extend r #z v` / `Record.with r #z v` (3-operand, DESIGN-record-update-syntax.md)
                // — the FIRST operand `r` is a record expression (descend it); the SECOND is a `#z` field
                // LABEL (`read_label`, static — NOT descended as an expression); the THIRD is the VALUE `v`,
                // an ordinary expression (descend it). `extend` REQUIRES `z` ABSENT (a present field →
                // CDZ0211, never a silent overwrite); `with` REQUIRES `z` PRESENT (an absent field →
                // CDZ0212, stays distinct from `extend`). A malformed label is CDZ0201.
                let is_extend = crate::eval::meta_apply_of(db, head)
                    == Some(crate::resolved::Prim::RecordExtend);
                collect(db, args[0], out);
                // The field-NAME-INTRODUCTION operand `args[1]` MUST be a static `#field` label. `read_label`
                // (shared with the READ/DROP ops, which legitimately take a bare label) also accepts a BARE
                // identifier — which PUNS an undeclared name into a new static field (`(Record.extend r fname
                // v)` silently adds `fname`). For the ADD/UPDATE ops that INTRODUCE a name, that pun is a
                // soundness-adjacent surprise (an undeclared name becomes a field): the name is a compile-time
                // LABEL, never a runtime value, and must be written `#z` (concierge ruling on breaker's pun,
                // type-system.md §A Record Row Is Reshaped Only Through An Explicit Operation). REJECT a
                // field-name operand that is NOT a `#`-sym / `(meta …)` label — a bare name, or any other
                // expression. Scoped to extend/with's name operand ONLY; the read/drop ops keep bare labels.
                let field_name_is_label =
                    db.ast.as_sym(args[1]).is_some() || db.ast.as_form(args[1], "meta").is_some();
                if !field_name_is_label {
                    collect(db, args[2], out); // still descend the value operand for its own faults
                    // A symbol literal is written `#"z"` (QUOTED) — a bare `#z` is NOT symbol syntax (the
                    // reader reads it as an identifier), so the example must be the quoted form or a user
                    // who copies it hits this same reject again. When the offending operand is a BARE
                    // IDENTIFIER (`Record.with r x 5`), the near-certain intent is the field label of that
                    // name, so carry a heuristic `x` → `#"x"` Replace fix (the actionable route). A non-name
                    // operand (a literal, a compound) has no single obvious label, so it gets the message
                    // alone.
                    let mut reject = Reject::coded(
                        Code::RecordFieldNameNotLabel,
                        "the field name introduced by `Record.extend`/`Record.with` must be a static \
                         `#\"field\"` symbol label (e.g. `#\"z\"`), not a bare identifier or a runtime \
                         value — a record field name is a compile-time label, not a value",
                    )
                    .at(args[1]);
                    if let Some(name) = db.ast.as_name(args[1]) {
                        reject = reject
                            .with_fix(Fix::replace_heuristic(args[1], format!("#\"{name}\"")));
                    }
                    out.push(reject);
                } else {
                    match (
                        type_of(db, args[0]),
                        crate::resolve::read_label(db, args[1]),
                    ) {
                        (Ty::Record(fields), Some(label)) => {
                            collect(db, args[2], out);
                            let present = fields.contains_key(&label);
                            // The operation-KEY occurrence of the `(. Record extend|with)` head — the node an
                            // OPERATOR-SWAP fix rewrites (`extend`→`with` or `with`→`extend`). The message
                            // already NAMES the sibling op to use; the fix makes that one-token swap applyable
                            // (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix — the
                            // row-op analogue of `float_sibling_operator`). `None` if the head is not `(. R op)`.
                            let op_key_occ =
                                db.ast.as_form(head, ".").and_then(|t| t.get(1).copied());
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
                                // precondition). The `#z` label operand is `args[1]` itself.
                                let label_node = Some(args[1]);
                                match (&near, label_node, op_key_occ) {
                                    (Some(near), Some(label_node), _) => {
                                        reject =
                                            reject.with_fix(crate::diag::Fix::replace_heuristic(
                                                label_node,
                                                near.clone(),
                                            ));
                                    }
                                    (None, _, Some(occ)) => {
                                        reject =
                                            reject.with_fix(crate::diag::Fix::replace_verified(
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
                            // The example must be the QUOTED symbol form `#"z"` — a bare `#z` is not symbol
                            // syntax (it reads as an identifier), matching the corrected CDZ0215 wording.
                            "the second operand is a `#\"field\"` symbol label, e.g. `#\"z\"`",
                        )),
                        _ => {}
                    }
                }
            } else if matches!(
                crate::eval::meta_apply_of(db, head),
                Some(crate::resolved::Prim::RecordExtend | crate::resolved::Prim::RecordWith)
            ) && args.len() == 2
                && crate::resolve::record_op_pair(db, args[1]).is_some()
            {
                // The OLD two-operand pair form `Record.with r (z v)` — MIGRATED to the 3-operand
                // `Record.with r #z v` (DESIGN-record-update-syntax.md). Rejected with a migration route so
                // the fix is mechanical: split the `(z v)` pair into a `#z` label and a bare value.
                let op = if crate::eval::meta_apply_of(db, head)
                    == Some(crate::resolved::Prim::RecordExtend)
                {
                    "Record.extend"
                } else {
                    "Record.with"
                };
                if let Some((label, _)) = crate::resolve::record_op_pair(db, args[1]) {
                    out.push(
                        Reject::coded(
                            Code::Malformed,
                            format!(
                                "`{op}` now takes three operands `r #\"{}\" v` — replace the `(name value)` pair with a `#\"field\"` symbol label and a value",
                                label.name
                            ),
                        )
                        .at(args[1]),
                    );
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
                    // A tuple row op over a DEFINITE NON-TUPLE operand — `(Tuple.remove n)` for `n : Int64` —
                    // is a kind error, the tuple twin of the record-row-op check. It was check-INVISIBLE
                    // and compiled to a MISLEADING "Tuple.<op> over a runtime tuple is not yet built" (the
                    // operand is not a tuple at all). `cat` checks BOTH operands; `split-at`/`pop` check
                    // `args[0]` (`split-at`'s `args[1]` is a position literal, not a tuple). Report
                    // "requires a tuple, found <T>" (the tuple-projection message already exists for `(. n
                    // N)`); `Any`/`Var` (an unconstrained param) is not flagged.
                    let operands: &[StructId] = if tup_op == "concat" {
                        &args
                    } else {
                        &args[..1]
                    };
                    operands
                        .iter()
                        .any(|&o| definite_non_tuple(&type_of(db, o)))
                }
            {
                let operands: &[StructId] = if tup_op == "concat" {
                    &args
                } else {
                    &args[..1]
                };
                for &o in operands {
                    let ot = type_of(db, o);
                    if definite_non_tuple(&ot) {
                        out.push(
                            Reject::coded(
                                Code::Malformed,
                                format!(
                                    "`Tuple.{tup_op}` requires a tuple, found {}",
                                    ot.render_name(&db.name_ctx())
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
                // `Tuple.concat a b` / `Tuple.remove t` — both operands (concat) / the sole operand (remove) are
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
                // (The sibling-width CDZ0302 range-check for `Set.of` elements runs in `check_application`'s
                // `Set.of` arm — the homogeneous-set path — which sees the settled element type; this
                // `collect` arm only descends into each element for its OWN nested faults.)
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
        // `(const e)` carries no type annotation, so it has no runtime-width / ctor-arity fault of its
        // own — descend the check into its inner expression (see-through).
        Resolved::ConstBlock { expr } => collect(db, expr, out),
        Resolved::Annot { expr, ty_expr } => {
            // A RUNTIME WIDTH is forbidden: `(: 5 (UInt n))` with `n` a runtime value (a parameter, a
            // call result) puts runtime data in a type-determining position, which the type system
            // forbids — an integer type's width MUST be a compile-time natural (`numeric-model.md §An
            // Integer Type Is Indexed By A Compile-Time Width`; `type-system.md §Generics Are
            // Type-Valued Parameters` — a type-valued parameter is resolved at compile time, never from
            // runtime data). This is checked on the ANNOTATION's un-inlined body, so `(def (mk n) (: 5
            // (UInt n)))` rejects (CDZ0302) even though a constant call site `(mk 8)` would fold `n` — a
            // width must be non-dependent regardless of how it happens to be called.
            // Descends nested positions (`(: xs (List (UInt n)))`), returning the offending `(Int n)`/
            // `(Float n)` position so the message names the right axis and the reject anchors there.
            let runtime_width_pos = nested_runtime_width_type(db, ty_expr);
            let runtime_width = runtime_width_pos.is_some();
            if let Some(pos) = runtime_width_pos {
                trace!(target: "rcdzc::infer", node = id.0, "fault: numeric width from runtime data (CDZ0302)");
                // `(Float n)` and `(Int n)`/`(UInt n)` both forbid a runtime width; name the axis the
                // written type actually uses so the message is not misleadingly integer-only for a float.
                let msg = if crate::eval::is_float_ctor_type(db, pos) {
                    "a floating-point width must be a compile-time admitted width (32 or 64), not runtime data"
                } else {
                    "an integer width must be a compile-time natural, not runtime data"
                };
                out.push(Reject::coded(Code::IntOutOfRange, msg).at(pos));
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
            // A BARE type-CONSTRUCTOR name used with NO argument in a VALUE annotation — `(: (Mk 1) Box)`,
            // `(: 5 List)`. Like the applied wrong-arity above (and the param/payload paths), this must be
            // caught BEFORE the type is used: a user generic's bare name REDUCES to a `Ty::Sum` with a fresh
            // var, so `typeval_of` succeeds and the value-vs-annotation UNIFY below fires the CONFUSING
            // "annotation type Box does not match value type Box — wrap the value in `Mk`" (both render `Box`,
            // reading as a contradiction) instead of the clear "needs a type argument". Emit the same CDZ0203
            // the parameter annotation gives (`bare_type_ctor_needs_argument` → the constructor message), so
            // an under-applied generic in a value annotation reads like one in a parameter annotation. Returns
            // `None` for a monomorphic type / a genuine value, so those are unaffected.
            if !runtime_width
                && let Some((ctor, placeholder)) = bare_type_ctor_needs_argument(db, ty_expr)
            {
                trace!(target: "rcdzc::infer", node = id.0, "fault: bare type constructor missing its argument in a value annotation (CDZ0203)");
                out.push(Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "`{ctor}` is a type constructor — it needs a type argument here, e.g. `({ctor} {placeholder})`"
                    ),
                ));
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
                // The bare `(Float 16)` reduces to the sentinel `Ty::Float(Fixed(0))`, but a bad width
                // NESTED in a compound annotation (`(List (Float 8))`, `(Option (Float 16))`) reduces to a
                // well-formed container of the sentinel element — so the top-level `annot_ty` looks valid and
                // the ill-formed width slips past `cdz check`, the float twin of the nested-integer-width
                // gap. Descend the annotation type expression and reject CDZ0302 at the offending position.
                if let Some(pos) = nested_ill_formed_float_width(db, ty_expr) {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: float width not in the admitted set (CDZ0302)");
                    let mut reject =
                        Reject::coded(Code::IntOutOfRange, FLOAT_WIDTH_MESSAGE).at(pos);
                    if let Some(fix) = ill_formed_float_width_fix(db, pos) {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
                // An OUT-OF-CEILING / zero INTEGER width `(: e (UInt 65))` is ill-formed at the annotation
                // regardless of what `e` is — the integer analogue of the float admitted-set check just
                // above. `reduce_ctor` clamps such a width to the sentinel 0 (so `annot_ty` is `Int0`), and
                // the literal-fit check below would only fire for a CONCRETE literal and would name the
                // misleading clamped `UInt0`; catch it HERE by reading the ORIGINAL width, so a NON-literal
                // value (`(: x (UInt 65))`) is rejected too and the message names the written width. Same
                // CDZ0302 + wording as the parameter-annotation path (`param_annotation_faults`).
                if let Some((pos, fault)) = nested_ill_formed_int_width(db, ty_expr) {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: ill-formed integer width in a value annotation (CDZ0302)");
                    let mut reject =
                        Reject::coded(Code::IntOutOfRange, ill_formed_int_width_message(&fault))
                            .at(pos);
                    if let Some(fix) = ill_formed_int_width_fix(&fault, pos) {
                        reject = reject.with_fix(fix);
                    }
                    out.push(reject);
                }
                // An UNBOUND NAME in a width position — `(: v (Int hello))` — the value-annotation twin of
                // the parameter-path check: a width is not a type, so the nested-type-var walk skips it, and
                // it reads as a non-constant width, so it slipped past `cdz check`. A bound width variable is
                // valid and does not match.
                if let Some((pos, example)) = nested_unbound_width(db, ty_expr) {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: unbound name in a width position (CDZ0101)");
                    let name = db.ast.as_name(pos).unwrap_or("?").to_string();
                    out.push(
                        Reject::coded(Code::Unbound, unbound_width_message(&name, example)).at(pos),
                    );
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
                        trace!(target: "rcdzc::infer", node = id.0, annot_ty = %annot_ty.render_name(&db.name_ctx()), "fault: literal does not fit annotated width (CDZ0302)");
                        out.push(int_out_of_range_reject(
                            &annot_ty,
                            it.ground_signed(),
                            w,
                            &v,
                            ty_expr,
                            &db.name_ctx(),
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
                    // Two quantity types whose units are not IDENTICAL. A quantity annotation checks the
                    // DIMENSION, not the scale — a unit is construction sugar for a magnitude at the
                    // dimension's reference (DESIGN-quantity-reference-normalized-unwrap.md §Interaction
                    // With Annotations: "accept any unit of the right dimension; scale is construction
                    // sugar"), so the two cases split:
                    if !au.same_dimension(eu) {
                        // DIFFERENT DIMENSION — the value derives one dimension but the annotation asserts
                        // another (`(: (* (Qty 2 meter) (Qty 3 meter)) (Qty Float64 meter))` — the product
                        // is meter², annotated meter; or meter vs second). A genuine dimensional conflict:
                        // CDZ0501 (units-of-measure.md §A Dimensional Mismatch Is An Error), not the CDZ0203
                        // the generic unify would give. The inner types are irrelevant when the DIMENSIONS
                        // disagree — the dimension is the conflict. The two units render distinguishably
                        // (different base names / exponents), so the message reads correctly.
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
                        // SAME DIMENSION, DIFFERENT SCALE (`(: (Qty.of 1 kilometer) (Qty Int64 meter))` —
                        // km and meter are both length). The annotation is SATISFIED dimensionally: it
                        // checks the dimension, and the annotated value KEEPS ITS OWN SCALE (1 km stays
                        // 1 km — the annotation does NOT normalize/coerce to its unit). Do NOT fall through
                        // to the generic unify (it unifies the FULL `Ty::Qty`, which differ in scale → a
                        // spurious CDZ0203 that ALSO misrenders, both units printing as the reference name).
                        // Still unify the INNER numeric types, so a genuine inner mismatch — an Int64 value
                        // annotated `(Qty Float64 meter)` — is caught (CDZ0203), exactly as a bare numeric
                        // annotation mismatch is: the dimension agreeing does not excuse a numeric-type clash.
                        let mut subst = Subst::new();
                        if crate::unify::unify(&mut subst, ai, ei, &db.name_ctx()).is_err() {
                            trace!(target: "rcdzc::infer", node = id.0, annot_inner = %ai.render_name(&db.name_ctx()), expr_inner = %ei.render_name(&db.name_ctx()), "fault: same-dimension quantity annotation with a mismatched INNER numeric type (CDZ0203)");
                            out.push(Reject::coded(
                                Code::TypeMismatch,
                                format!(
                                    "annotation type {} does not match value type {} — the units share a \
                                     dimension, but the underlying numeric types differ",
                                    annot_ty.render_name(&db.name_ctx()),
                                    type_of(db, expr).render_name(&db.name_ctx()),
                                ),
                            ));
                        } else {
                            trace!(target: "rcdzc::infer", node = id.0, "same-dimension quantity annotation at a different scale — accepted (dimension checked, value keeps its own scale)");
                        }
                        // Descend for the expression's own faults (the generic `else` also does this).
                        collect(db, expr, out);
                    }
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
                    // A FUNCTION VALUE'S type must be its BODY-SOLVED arrow, not the bottom-up `type_of`
                    // arrow. The `type_of` Lambda arm leaves an UNANNOTATED parameter `Any` (`h x = x + 1`
                    // types `(-> Any Int64)`), and `Any` UNIFIES WITH ANYTHING — so a CONTRADICTORY arrow
                    // annotation `(: h (-> Bool Int64))` / `(: h (-> String Int64))` unified against `(-> Any
                    // Int64)` and SUCCEEDED, silently accepting a mismatch the check exists to reject. The
                    // SAME leak reaches a fn stored INSIDE a compound/sum being annotated — `(: (tuple h 0)
                    // (Tuple (-> Bool Int64) Int64))` / `(: (Some h) (Option (-> Bool Int64)))` — because the
                    // compound's bottom-up `type_of` renders its fn element as `(-> Any …)`, so the domain
                    // contradiction is masked (a RESULT contradiction was caught, since the codomain is
                    // concrete). `reflected_ty` grounds a fn's domain from its body for a BARE fn AND
                    // recursively through tuple/list/record/map elements + sum-variant payloads (the same
                    // grounding `Type.of` uses), so the annotation check sees each fn's real domain wherever
                    // it sits. It falls back to the plain `type_of` for a non-function, non-compound value —
                    // including a `(let …)`/call whose result is not itself a fn, so the error-type reject of
                    // `(: (let ((y (try (Err true)))) (Ok y)) (Result …))` is unaffected (`reflected_ty`'s fn
                    // check needs the value to REDUCE to a lambda, which a `(Ok y)` body does not). A
                    // genuinely-unconstrained param stays `Any` (polymorphic `(fn (x) x)`) and still unifies —
                    // honest, no false reject. GATED by `reflection_may_ground`: only a fn value or a
                    // compound/variant LITERAL takes the grounded `reflected_ty`; a plain `let`/call/scalar
                    // keeps its exact `type_of`, so reflecting a `(: (let ((y (try (Err …)))) (Ok y)) (Result
                    // …))` — which would speculatively reduce and suppress the `?`-error-type soundness reject
                    // — is avoided (that expr is a `let`, not a fn/compound literal).
                    let expr_ty = if reflection_may_ground(db, expr) {
                        reflected_ty(db, expr)
                    } else {
                        type_of(db, expr)
                    };
                    let mut subst = Subst::new();
                    if crate::unify::unify(&mut subst, &annot_ty, &expr_ty, &db.name_ctx()).is_err()
                    {
                        trace!(target: "rcdzc::infer", node = id.0, annot_ty = %annot_ty.render_name(&db.name_ctx()), expr_ty = %expr_ty.render_name(&db.name_ctx()), "fault: annotation type mismatch (CDZ0203)");
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
                        // Pre-render the type names the lead needs (they do not depend on `verb_tail`), so the
                        // closure captures owned Strings rather than borrowing `db` — a `db.name_ctx()` borrow
                        // held across the closure would conflict with the later `wrap_variant_for(db, …)`.
                        let expr_article = expr_ty.render_with_article(&db.name_ctx());
                        let annot_name = annot_ty.render_name(&db.name_ctx());
                        let expr_name = expr_ty.render_name(&db.name_ctx());
                        let mismatch_lead = |verb_tail: &str| {
                            if is_call_arg {
                                format!(
                                    "this argument is {}, but a value of type {} is expected here{verb_tail}",
                                    expr_article, annot_name,
                                )
                            } else {
                                format!(
                                    "annotation type {} does not match value type {}{verb_tail}",
                                    annot_name, expr_name,
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
                                let f = annot_ty.render_name(&db.name_ctx());
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
                                int_coercion_wrap(&annot_ty, &expr_ty, &db.name_ctx())
                            {
                                let n = annot_ty.render_name(&db.name_ctx());
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
                                option_payload_mismatch_hint(&db.name_ctx(), &annot_ty, &expr_ty)
                            } else {
                                None
                            };
                            // Two RECORDS that differ only in their FIELD SET — the value is missing a
                            // field the type requires, or carries one the type has no place for. Naming
                            // both full record types (`(Record (x Int64) (y Int64))` vs `(Record (x
                            // Int64))`) buries the actual difference; name the specific missing/extra
                            // fields instead (rustc's "missing field `y`" / "no field `z`"). Tail only.
                            let record_tail = if wrap.is_none() && option_tail.is_none() {
                                record_field_diff_hint(&annot_ty, &expr_ty, &db.name_ctx()).or_else(
                                    || {
                                        tuple_arity_mismatch_hint(
                                            &annot_ty,
                                            &expr_ty,
                                            &db.name_ctx(),
                                        )
                                    },
                                )
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
                                    fn_not_applied_hint(&annot_ty, &expr_ty, &db.name_ctx())
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
                                collection_element_mismatch_hint(
                                    &annot_ty,
                                    &expr_ty,
                                    &db.name_ctx(),
                                )
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
                                sum_payload_mismatch_hint(&annot_ty, &expr_ty, &db.name_ctx())
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
                                fn_signature_delta_hint(&annot_ty, &expr_ty, &db.name_ctx())
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
                                same_name_distinct_type_hint(&annot_ty, &expr_ty, &db.name_ctx())
                            } else {
                                None
                            };
                            // A FLOAT value annotated an INTEGER type with NO clean literal retype — a
                            // NON-integer float literal (`(: 3.5 Int64)`) or a non-literal float expression
                            // (`(: (Float64.of-int n) Int64)`). The integer-valued literal case (`(: 3.0
                            // Int64)`) already took the `literal_retype` fix above (drop the `.0`); this is
                            // the fallthrough where truncation would LOSE the fractional part, so there is no
                            // one-shot fix — but the bare "Int64 does not match Float64" dead-ends. Name WHY
                            // (a float carries a fraction an integer cannot hold) + the two real paths:
                            // annotate a float type, or explicitly round/truncate to an integer. Tail only
                            // (no mechanical fix — rounding-vs-truncating is the author's choice), last in the
                            // chain after every structural hint (the int-coercion WRAP cases already returned
                            // a wrap tail above, so reaching here means no wrap applied).
                            let float_int_tail = if wrap.is_none()
                                && option_tail.is_none()
                                && record_tail.is_none()
                                && fn_tail.is_none()
                                && collection_tail.is_none()
                                && sum_tail.is_none()
                                && matches!(&annot_ty, Ty::Int(_))
                                && matches!(&expr_ty, Ty::Float(_))
                            {
                                Some(format!(
                                    " — a floating-point value has a fractional part {} cannot hold; \
                                     annotate a float type (e.g. `{}`), or round/truncate it to an integer first",
                                    annot_ty.render_name(&db.name_ctx()),
                                    expr_ty.render_name(&db.name_ctx()),
                                ))
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
                                .or(float_int_tail)
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
                    false,
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
            // The nested twin: a narrow-width literal in a COMPOUND payload/element — `(: (Some 999)
            // (Option Int8))`, `(: (tuple 999) (Tuple Int8))`, `(: (list 999) (List Int8))` — where the
            // annotation's width propagates into the payload but the literal itself stays a deferred
            // `Int64`, so the scalar check above never fires. Descend the annotation's expected type against
            // the value's payload/elements and range-check each nested literal (the emit path already
            // rejects these; this makes `cdz check` agree). Only when the scalar check found nothing (a
            // top-level literal is not a compound).
            else if let Some(reject) = nested_literal_width_faults(db, expr, ty_expr) {
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
            let mut seen_arm_ops: std::collections::HashSet<(u32, std::sync::Arc<str>)> =
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
                    // SHADOWED-OP: if the arm's op is declared on a LATER same-named effect (not the one a
                    // bare `E` resolves to), EXPLAIN the shadowing instead of a baffling "closest matches"
                    // — the handler-arm twin of `no_field_reject`'s perform-site shadow hint (the
                    // works-as-specified duplicate-effect diagnostic; two same-named effects are DISTINCT).
                    // Supersedes the did-you-mean + suppresses the typo-replace fix (the op is real, just on
                    // another declaration).
                    let shadow = crate::effects::arm_op_shadow_hint(db, arm.op);
                    // The fallback MECHANICAL repair when there is no confident "did you mean" replacement:
                    // DELETE the whole `(op (params…) state body)` arm. The arm names an operation the
                    // effect does not declare, so removing it is always a valid edit that clears the
                    // closed-set violation (the handler-arm twin of the duplicate-arm delete fix above and
                    // the over-application delete). Located like the duplicate case: the op-key occurrence's
                    // projection parent is the arm form. Heuristic (the author may instead have meant a
                    // different op — hence a replace fix, when confident, is preferred over this delete).
                    let arm_form = crate::effects::arm_op_key_occ(db, arm.op)
                        .and_then(|key_occ| db.parent_of(key_occ))
                        .and_then(|proj| db.parent_of(proj))
                        .filter(|&f| matches!(db.ast.get(f), crate::ast::Struct::List(_)));
                    let delete_arm_fix = |reject: Reject| match arm_form {
                        Some(form) => reject.with_fix(crate::diag::Fix::delete_heuristic(
                            form,
                            "remove this handler arm (its operation is not declared by the effect)",
                        )),
                        None => reject,
                    };
                    match crate::effects::declared_op_hint(db, arm.op) {
                        Some((key_occ, hint, single)) => {
                            let suffix = shadow.clone().unwrap_or(hint);
                            let mut reject = Reject::coded(
                                Code::HandlerUndeclaredOp,
                                format!(
                                    "this handler arm names an operation its effect does not declare{suffix}"
                                ),
                            )
                            .at(key_occ);
                            if shadow.is_none()
                                && let Some(candidate) = single
                            {
                                // A confident typo → offer the RETYPE to the real op name (better than a
                                // blunt delete when we know what they meant).
                                reject =
                                    reject.with_fix(Fix::replace_heuristic(key_occ, candidate));
                            } else {
                                // No confident replacement (far miss, or a shadowed op the author must
                                // rewire by hand) → the delete is the actionable fallback.
                                reject = delete_arm_fix(reject);
                            }
                            out.push(reject);
                        }
                        None => out.push(delete_arm_fix(
                            Reject::coded(
                                Code::HandlerUndeclaredOp,
                                "this handler arm names an operation its effect does not declare",
                            )
                            .at(anchor),
                        )),
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
                // Only a USER node's unbound name is a real source reference. A SYNTHESIZED node (a β-copy /
                // inlined callee body an application's type computation builds — id ≥ `user_node_count`) that
                // resolves a name unbound is an INFERENCE ARTIFACT, not a program reference: the copy lost the
                // enclosing binder scope (e.g. a callee's match-arm binder is bound in the original body but
                // not in the spliced copy). A genuine unbound name ALSO surfaces at its own USER occurrence,
                // so skipping the synthesized copy never hides a real fault — but reporting it emits a false
                // CDZ0101 whose origin `sanitize_origin` can only UN-anchor (it has no source span), and which
                // (mis-)maps to the enclosing user call site. This is exactly the `sread.cdz` false "unbound
                // name `tyname`" (a `(match … (tyname a2) …)` binder in `ann-with-value`, unbound in the
                // inlined copy at the call site) that reddened the whole compiler-ml suite. Gate on
                // `is_user_node` — the same boundary `sanitize_origin`/`db.rs` use to keep a synthesized id off
                // the mapped-diagnostic path.
                if db.is_user_node(id) {
                    trace!(target: "rcdzc::infer", node = id.0, "fault: unbound name reported (CDZ0101)");
                    out.push(enrich_unbound(db, id, r));
                }
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
                    .map(|it| Ty::Int(it).render_name(&db.name_ctx()))
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
        // A bare FLOAT literal whose width is fixed CONTEXTUALLY to `Float32` through an arith spine — the
        // float twin of the integer contextual check above. `(+ a 1.0e300)` over `(: a Float32)`: the `+`
        // unifies operand widths, grounding `1.0e300` to Float32 where it saturates to `±inf` (a value with
        // no written form, numeric-model.md §A Floating-Point Literal That Denotes No Representable Value Is
        // Malformed) — yet without this it COMPILED + materialized `inf` (the int analogue rejects CDZ0201).
        // Skip an annotated literal (its `Float32` annotation is checked by `literal_width_fault`'s Float32
        // arm); only a bare literal grounded solely by its arith-operand context is judged here. CDZ0201
        // (contextual — no annotation to blame), matching the integer arith-spine verdict.
        Resolved::Float(dec) => {
            let annotated = db
                .parent_of(id)
                .and_then(|p| db.ast.as_form(p, ":"))
                .and_then(|t| t.first().copied())
                == Some(id);
            if !annotated
                && !dec.fits_f32()
                && literal_binop_float32_context(db, id)
                && matches!(type_of(db, id), Ty::Float(ft) if !ft.width_is_fixed())
            {
                trace!(target: "rcdzc::infer", node = id.0, "fault: float literal grounded to Float32 through an arith spine overflows to inf (CDZ0201)");
                // CDZ0201 (`Code::Malformed`) — CONTEXTUAL: a bare literal fixed by an arith-operand context,
                // no annotation to blame — matching the integer arith-spine verdict (not the annotated
                // CDZ0302 the direct `(: 1.0e300 Float32)` path takes).
                out.push(
                    Reject::coded(
                        Code::Malformed,
                        "float literal does not fit Float32 (it is grounded to Float32 by an \
                         arithmetic-operand context, where it overflows the Float32 range to infinity — \
                         the largest finite Float32 is about 3.4e38)",
                    )
                    .at(id),
                );
            }
        }
        Resolved::Prim(_)
        | Resolved::Ref { .. }
        | Resolved::SumPayload { .. }
        | Resolved::BinField { .. }
        | Resolved::MapField { .. }
        | Resolved::RecordField { .. }
        | Resolved::RecordRest { .. }
        | Resolved::SetRest { .. }
        | Resolved::Param { .. }
        | Resolved::Bool(_)
        | Resolved::Str(_)
        | Resolved::SymbolConst(_)
        | Resolved::Bytes(_)
        | Resolved::Char(_)
        | Resolved::Rational { .. }
        | Resolved::Unit
        | Resolved::TypeVal(_) => {}
        // An anonymous LAMBDA `(fn (params…) body)`: check its parameter list is LINEAR, exactly as a
        // top-level def's is (`(fn (x x) …)` shadowed the first `x` and silently bound nothing).
        Resolved::Lambda { params, body } => {
            param_list_linearity_faults(db, &params, out);
            // A NAMED-DEF lambda body — the `(def name (fn (p…) body))` form, which registers a def whose
            // `body` occurrence IS this lambda node — must be type-checked STANDALONE, exactly as the
            // `(def (name p…) body)` form's body is (the def-body loop in `compile::collect_faults` runs
            // `type_errors` over every def body; for the lambda-valued form that body is this `fn` node).
            // Its parameters are bound (a `(: x Int64)` param types fine), so an unbound name or a type
            // fault in the body is real well-formedness — the same "check missed a fault on a param /
            // non-exported body" hole `match_pattern_fault` / `binop_arity_fault` / the do-block poison
            // close. Without this, an ill-typed lambda-valued def body (`(def f (fn ((: x Int64)) (+ x
            // 1.0)))`) PASSED `cdz check` (and `compile`, when the def is unreached) while the exact same
            // logic written `(def (f (: x Int64)) (+ x 1.0))` was rejected — a check/compile discrepancy on
            // a purely syntactic surface choice. An INLINE / let-bound lambda (`(let ((g (fn (x) …))) …)`,
            // an argument `(hof (fn (x) …))`) is NOT a registered def body, so `def_index_by_body` is
            // `None` and it is skipped here — its body is checked at the β-reduction call site as before,
            // avoiding a double report and a spurious fault over an uninstantiated generic body.
            // A destructuring PARAM is desugared (binding_params) into a body `let`, so a refutable param
            // shows up as a refutable `let` LHS in `body` — validate binding-pattern irrefutability here for
            // an inline lambda too (a def-body `let` gets this via the def-body walk; an inline lambda's body
            // is otherwise unwalked, so a refutable `let`/param escaped CDZ0210). Shape-only, so it's safe on
            // a generic/uninstantiated body (unlike a full `collect`, which is why the arms below gate on
            // named-def / applied-try). Runs for EVERY lambda (the def-body/applied-try `collect` below
            // subsumes it, and `dedup_faults` collapses any overlap).
            inline_lambda_binding_pattern_faults(db, body, out);
            if db.def_index_by_body(id).is_some() {
                collect(db, body, out);
            } else if lambda_heads_an_application(db, id) {
                // An IMMEDIATELY-APPLIED lambda `((fn (…) body) args)`. Its body is checked at the
                // β-reduction call site, so only the `?`-boundary case needs a check here — and ONLY the
                // try-bearing subset, because descending into EVERY applied-lambda body reintroduces the
                // O(2^depth) re-reduce of a deep capturing chain (`((fn (a) …((fn (b) …) 1)) 0)`) the gate
                // guards against. The inlined (β-copied) body's `?` node is a PARENTLESS synth copy, so
                // `enclosing_boundary_ty`'s parent-walk falls off → `collect`'s `?` arm is INCONCLUSIVE and
                // never fires CDZ0230; check the ORIGINAL parented body here so the `?` boundary reaches THIS
                // `(fn …)` and the genuine CDZ0230 fires. `dedup_faults` collapses any overlap.
                if subtree_contains_try_form(db, body) {
                    collect(db, body, out);
                }
            } else {
                // A NOT-immediately-applied INLINE lambda — STORED/uncalled (`(list (fn (v0) …))`),
                // let-bound, or a HOF argument. Its body was previously UNCHECKED (it relied on the
                // β-reduction call site, which never happens for a closure that is never called), so a param
                // used at TWO INCOMPATIBLE CONCRETE types inside it ESCAPED the checker and the backend then
                // emitted INVALID WASM (fuzzer class: `(fn (v0) (+ v0 (. v0 0)))` tuple, `(fn (v0) (if v0 v0
                // 174.81))` if-join, `(+ v0 (List.len v0))` int-vs-list, `(+ v0 (Bytes.len v0))`, …). SOLVE +
                // seed each param's body type (exactly as `lower_lambda_value` does, so the body walk's
                // `type_of` reads the concrete type), then `collect` the body — the SAME CDZ0201 the
                // top-level twin `(def (v0) (+ v0 (. v0 0)))` gets, catching EVERY conflict-kind at the check
                // (not one runtime lowering at a time). An UNPINNED param solves to `Any` and is NOT seeded,
                // so a genuinely-polymorphic body (`(fn (t) (. t 0))`, a closure passed to a generic HOF) is
                // tolerated exactly as before — only a CONCRETE conflict faults. Not immediately-applied ⇒ no
                // re-reduce, so the O(2^depth) blowup the applied gate guards cannot arise here.
                // DEPTH GUARD: bound the re-entrant collect/solve. A synthesized lambda whose body re-enters
                // this arm (a map-match desugar over a self-recursive def — `(match mp … ((Zorp x) (go mp)))`)
                // otherwise recurses to a STACK OVERFLOW. The flat fuzzer shapes are depth 1; a few nesting
                // levels are still checked; a deeper/cyclic body BAILS (a MISS — never a miscompile, and a
                // strict SUBSET of the unbounded check, so it adds no new fault). Restored on every exit.
                const MAX_INLINE_LAMBDA_CHECK_DEPTH: u32 = 4;
                let depth = INLINE_LAMBDA_CHECK_DEPTH.with(|c| c.get());
                if depth < MAX_INLINE_LAMBDA_CHECK_DEPTH {
                    INLINE_LAMBDA_CHECK_DEPTH.with(|c| c.set(depth + 1));
                    for &p in params.iter() {
                        let occ = crate::eval::param_name_occ(db, p);
                        if matches!(type_of(db, occ), Ty::Any) {
                            let solved = solve_lambda_param_ty(db, occ, body);
                            if !matches!(solved, Ty::Any) {
                                db.param_types.entry(occ).or_insert(solved);
                            }
                        }
                    }
                    collect(db, body, out);
                    INLINE_LAMBDA_CHECK_DEPTH.with(|c| c.set(depth));
                }
            }
        }
    }
}
