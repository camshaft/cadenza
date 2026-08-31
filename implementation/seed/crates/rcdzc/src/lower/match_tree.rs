//! `lower::match_tree` — the match DECISION-TREE compiler + exhaustiveness checking, split out of
//! `lower.rs`. Lowers `match` over binary/sum/scalar scrutinees (`lower_match_bin`/`lower_match_sum`),
//! builds and refines the decision tree (`build_tree`/`build_tree_ft`/`refine_*`/`merge_rows`), tracks
//! per-path types + constant folding (`type_at_path`/`const_at_path`/`classify_probe`/`probe_matches_*`),
//! checks pattern linearity + binding patterns (`check_pattern_linear`/`check_binding_pattern`/
//! `classify_binding_ctor`/`collect_pattern_binders`), and builds the non-exhaustiveness diagnostics.
//! Behaviour-preserving move: items keep their original visibility (`pub use match_tree::*` in `lower`
//! re-exports each at its own vis, so `crate::lower::*` paths are unchanged); private items become
//! `pub(super)` and reach the rest of the tree via `use super::*`.

use super::*;

/// A constant scrutinee value for the guarded-match fold — an integer or a boolean.
pub(super) enum GuardFoldScrut {
    Int(IntValue),
    Bool(bool),
    Str(String),
    Char(char),
    Bytes(Vec<u8>),
}

/// Walk a constant-value path from `root` down `steps`, returning the leaf's core if EVERY step lands
/// in a compile-time-constant compound (`Core::SumNew` payloads / `Core::Tuple` elements). This folds a
/// nested payload binder over a constant scrutinee — `(match (Some (Some 5)) ((Some (Some y)) y))`
/// through `[Payload, Payload]` yields the constant `5`, no heap read. `None` if any step hits a runtime
/// value (then the binder emits a runtime `Core::SumPayload` walk).
/// Drop the `Payload` steps that fall over a NOMINAL NEWTYPE sub-value — each is a runtime no-op (the box
/// is erased, so the value already IS its underlying value; `core-semantics.md §156`). The remaining
/// steps are the REAL heap accesses (a boxed sum's `sum-payload`, a tuple's `arr-get`) the backend walks,
/// so the emit path needs no nominal awareness. Walks the scrutinee's type in lockstep with the steps —
/// exactly as `type_of(SumPayload)` does — using `heads` to instantiate a boxed-sum `Payload`. A
/// nominal `Payload` unwraps to `inner` and is DROPPED; every other step is KEPT and advances the type.
pub(super) fn erase_nominal_steps(
    db: &mut Db,
    scrutinee: StructId,
    steps: &[crate::core::PathStep],
    heads: &[StructId],
) -> Vec<crate::core::PathStep> {
    use crate::core::PathStep;
    let mut cur = crate::infer::type_of(db, scrutinee);
    let mut heads_it = heads.iter();
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        match step {
            PathStep::Payload => {
                if let crate::ty::Ty::Nominal { inner, .. } = &cur {
                    // Nominal unwrap — a no-op step. Advance the type to `inner`, DROP the step.
                    cur = (**inner).clone();
                } else {
                    // A real boxed-sum payload read — KEEP it, advance the type via the variant head.
                    let head = heads_it.next().copied();
                    out.push(*step);
                    cur = head
                        .and_then(|h| crate::infer::payload_ty_at_instantiation(db, h, &cur))
                        .unwrap_or(crate::ty::Ty::Any);
                    continue;
                }
            }
            PathStep::Elem(i) => {
                out.push(*step);
                cur = match &cur {
                    crate::ty::Ty::Tuple(elems) => {
                        elems.get(*i).cloned().unwrap_or(crate::ty::Ty::Any)
                    }
                    crate::ty::Ty::List(elem) => (**elem).clone(),
                    _ => crate::ty::Ty::Any,
                };
            }
            PathStep::RestFrom(_) => {
                // The rest sublist has the SAME type as the list scrutinee (`(List elem)`) — a tail of a
                // list is still a list of its element type.
                out.push(*step);
                // `cur` stays the list type (unchanged); a non-list here is a fault reported elsewhere.
            }
            PathStep::TupleRestFrom(k) => {
                // A tuple rest binder — advance the type to the trailing sub-tuple `(Tuple T_k …)`.
                out.push(*step);
                cur = match &cur {
                    crate::ty::Ty::Tuple(elems) => {
                        crate::ty::Ty::Tuple(elems.get(*k..).unwrap_or(&[]).to_vec().into())
                    }
                    _ => crate::ty::Ty::Any,
                };
            }
        }
    }
    out
}

pub(super) fn fold_sum_path(
    db: &mut Db,
    root: StructId,
    steps: &[crate::core::PathStep],
) -> Option<Core> {
    use crate::core::PathStep;
    let mut cur = root;
    // A TYPE cursor tracked ALONGSIDE `cur`, peeled one nominal layer per erased `Payload` step. Tracking
    // the peeled type — rather than re-reading `type_of(cur)` each step — is essential when a newtype WRAPS
    // A SUM (`(type W (V (Result …)))`): the newtype is erased, so `cur` stays the SAME node and its raw
    // type reads `Ty::Nominal` for EVERY step; re-reading it consumed the inner sum's `Payload` as a SECOND
    // nominal no-op and folded a payload binder to the WHOLE wrapper (a miscompile — `n` in `(W.V (Ok n))`
    // became the whole `Result`). The peeled cursor fires the nominal skip exactly once per layer, so the
    // inner sum's `Payload` then descends the sum (constant) or correctly declines the fold (runtime).
    let mut ty = crate::infer::type_of(db, root);
    for step in steps {
        // A `Payload` step over a NOMINAL NEWTYPE sub-value is a no-op: the box is erased, so the newtype
        // construction lowered its payload core DIRECTLY at `cur` (no `Core::SumNew` to descend). PEEL one
        // nominal layer off the type cursor and leave `cur` unchanged (a following `Payload` reads a wrapped
        // sum, a following `Elem` reads a multi-payload newtype's tuple).
        if matches!(step, PathStep::Payload)
            && let crate::ty::Ty::Nominal { inner, .. } = &ty
        {
            ty = (**inner).clone();
            continue;
        }
        // A `Payload` step over a MULTI-payload `SumNew` is a no-op landing on the payload TUPLE; the
        // following `Elem(i)` indexes `payloads[i]` (the `(Elem, SumNew)` arm below) — mirrors the runtime
        // `sum-payload` + `arr-get i`. Without it a constant multi-payload variant match (`(match (Mk 3 4)
        // ((Mk a b) …))`) never folded (fell to `None`, emitted a runtime build+disc-walk), and the wasm
        // `const_disc_at` twin lost the disc → wrong-payload-depth (Copilot PR#457). Single-payload is
        // `[Payload]` with no following `Elem`, so it still unwraps in the arm below.
        if matches!(step, PathStep::Payload)
            && let Core::SumNew { payloads, .. } = core_of(db, cur)
            && payloads.len() > 1
        {
            // Keep `cur` at the SumNew; re-sync the type cursor to the entered variant's payload tuple so a
            // following `Elem` step's type is correct.
            ty = crate::infer::type_of(db, cur);
            continue;
        }
        cur = match (step, core_of(db, cur)) {
            (PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                payloads[0]
            }
            (PathStep::Elem(i), Core::Tuple { elems }) => *elems.get(*i)?,
            // A list-pattern element binder reads position `i` of a CONSTANT list — the same `Elem` step a
            // tuple element uses, over a `Core::ListNew`. A runtime list has no `Core::ListNew` here.
            (PathStep::Elem(i), Core::ListNew { elems }) => *elems.get(*i)?,
            // A MULTI-payload variant's payloads: after the `Payload` no-op above, `cur` is the `SumNew` and
            // `Elem(i)` selects the i-th payload — the constant twin of `sum-payload` + `arr-get i`.
            (
                PathStep::Elem(i),
                Core::SumNew {
                    payloads: elems, ..
                },
            ) => *elems.get(*i)?,
            // A list-pattern REST binder over a CONSTANT list folds to a fresh `Core::ListNew` of the tail
            // elements (from index `k`) — a synthesized node so the tail sublist is itself constant.
            (PathStep::RestFrom(k), Core::ListNew { elems }) => {
                let tail: Vec<StructId> = elems.iter().skip(*k).copied().collect();
                return Some(Core::ListNew { elems: tail.into() });
            }
            // A tuple-pattern REST binder over a CONSTANT tuple folds to a fresh `Core::Tuple` of the
            // trailing elements (from index `k`) — a synthesized node so the sub-tuple is itself constant.
            (PathStep::TupleRestFrom(k), Core::Tuple { elems }) => {
                let tail: Vec<StructId> = elems.iter().skip(*k).copied().collect();
                return Some(Core::Tuple { elems: tail.into() });
            }
            _ => return None,
        };
        // Re-sync the type cursor to the descended node (its own type — a nested newtype's inner peels on
        // the next `Payload`, a tuple element's type drives a following step).
        ty = crate::infer::type_of(db, cur);
    }
    Some(core_of(db, cur))
}

/// Lower a match over a SUM scrutinee to a DECISION TREE (Maranget). Dispatch on the variant
/// DISCRIMINANT at each level; a NESTED pattern shares its outer probe and splits on the inner
/// discriminant, so `(Some (Some x))`, `(Some None)`, `None` test the outer `Some` tag ONCE and only
/// then the inner tag — two tag checks on the deep path, not a linear re-probe per arm
/// (`type-system.md §Patterns Compose`). Exhaustiveness (`type-system.md §A Match Is Exhaustive Against
/// The Sum Type's Variant Set`) is checked at EACH switch: every variant covered OR a default arm; else
/// CDZ0210. A constant sum FOLDS to the selected body (like a scalar match); a runtime sum emits a
/// `Core::MatchSum` tree. A payload binder resolves to a `SumPayload` on its own (resolve Case 6), so an
/// arm carries only its discriminant + continuation.
//= spec/capabilities/type-system.md#a-match-is-exhaustive-against-the-sum-type-s-variant-set
//# The exhaustiveness rule governing a match MUST be checked against the scrutinee sum type's variant set, so that a match covering fewer than all variants is a compile-time rejection determined by that variant set rather than a runtime outcome.
/// Lower a `match` over a BYTES scrutinee whose arms include `(bin …)` binary patterns (BN3, constant
/// scrutinee). Each arm is either a `(bin <seg>…)` pattern or a CATCH-ALL (a bare binder / `_`). A `bin`
/// arm MATCHES iff the segment automaton (`bin_match_decode`) consumes the whole scrutinee AND every
/// LITERAL-slot segment's decoded value equals the literal (a magic-number/tag probe); its binder slots
/// bind via `BinField` (resolve Case B) — so the arm body needs no per-binder threading here. A match
/// with NO catch-all and only `bin` arms is NON-EXHAUSTIVE (a `bin` pattern never covers every byte
/// sequence — empty input, wrong length, an unequal literal all fail) → CDZ0210, exactly like a sum
/// missing a variant. On a CONSTANT scrutinee, select the first matching arm and lower its body; a
/// runtime scrutinee declines (the BN4 cursor automaton).
pub(super) fn lower_match_bin(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Core {
    if let Core::Poison(r) = core_of(db, scrutinee) {
        return Core::Poison(r);
    }
    // Classify arms. A `(bin …)` arm carries its parsed segments + an OPTIONAL guard `(guard (bin …) cond)`
    // (§4b): the guard cond reads the arm's decoded segment binders (resolve Case 6bg gives them the same
    // `BinField` the body sees) and gates the arm — a false guard FALLS THROUGH to the next arm (not a
    // trap), the bin analogue of the scalar guarded-arm fall-through. A bare-name/`_` arm is a catch-all.
    enum BinArm {
        Bin(Vec<crate::resolved::Segment>, Option<StructId>, StructId), // segments, guard cond, body
        CatchAll(StructId),                                             // body (bare binder or `_`)
    }
    let mut classified: Vec<BinArm> = Vec::with_capacity(arms.len());
    for &(pat, body) in arms {
        // Peel a `(guard <inner-pat> <cond>)` wrapper: the inner pattern gives the bin segments, `<cond>` the
        // guard. A wrong-arity guard is a poison (mirrors the scalar path's guard-arity check).
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            Some(g) => {
                return Core::Poison(crate::resolve::fixed_arity_reject(
                    pat,
                    g,
                    2,
                    "a guarded pattern must be (guard <pattern> <cond>)",
                ));
            }
            None => (pat, None),
        };
        if db.ast.head_name(inner_pat) == Some("bin") {
            match crate::resolve::resolved_of(db, inner_pat) {
                crate::resolved::Resolved::Bin { segs } => {
                    classified.push(BinArm::Bin(segs.to_vec(), guard, body))
                }
                crate::resolved::Resolved::Poison(r) => return Core::Poison(r),
                _ => {
                    return Core::Poison(Reject::decline(
                        "a bin pattern did not resolve to segments",
                    ));
                }
            }
        } else if guard.is_none() && db.ast.as_name(inner_pat).is_some() {
            // A bare name (binder) or `_` — a catch-all binding the whole scrutinee. (A GUARDED bare-name
            // catch-all over Bytes is the scalar path's job, not the bin matcher's — decline to it.)
            classified.push(BinArm::CatchAll(body));
        } else {
            // A literal / other pattern against a Bytes scrutinee — not supported here; decline.
            return Core::Poison(Reject::decline(
                "a match over Bytes mixes a bin pattern with an unsupported pattern",
            ));
        }
    }
    // Exhaustiveness: a `bin` pattern never covers every byte sequence, so a match with no catch-all is
    // non-exhaustive (CDZ0210) — the same rule as a sum missing a variant.
    let has_catch_all = classified.iter().any(|a| matches!(a, BinArm::CatchAll(_)));
    if !has_catch_all {
        return Core::Poison(Reject::coded(
            Code::NonExhaustive,
            "a match over Bytes with only bin patterns and no catch-all is non-exhaustive",
        ));
    }
    // A CONSTANT scrutinee → select the first matching arm at compile time.
    let Some(raw) = bin_const_scrutinee(db, scrutinee) else {
        // RUNTIME scrutinee → build a runtime decision: an if-chain over per-arm predicates. Only for arms
        // whose `(bin …)` is ALL fixed-width int segments (a runtime bits/bytes/dependent segment is a
        // later slice); such an arm's predicate is `bytes-len == total_width & (each literal segment read
        // == its literal)`, and its binders read via `BinIntRead` (resolve Case B → decode_bin_field
        // runtime). The arms are processed in order into a nested `if`, tail = the catch-all body.
        //
        // Build from the LAST arm backward: `acc` starts at the catch-all body's occurrence, and each
        // preceding `(bin …)` arm wraps it as `(if <predicate> <arm-body> <acc>)`. A synthesized `if`
        // node's core is pre-filled so it lowers directly (no re-resolution).
        // MATERIALIZE the scrutinee ONCE: it is read many times (each arm's length probe + literal probes
        // + the matched arm's binder reads), so recomputing the `BinBuild` per read would both re-run the
        // construction AND clash scratch slots. Mark it a KEPT binding and read it through a `LocalRef`, so
        // it evaluates once into a slot and every read is a `local.get`. The whole match is wrapped in a
        // `Core::Let { (scrutinee, scrutinee), if-chain }` below.
        db.kept_bindings.insert(scrutinee);
        let scrut_ref = synth_core(
            db,
            Core::LocalRef { binder: scrutinee },
            crate::ty::Ty::Bytes,
        );
        let mut acc: Option<StructId> = None; // the else-tail so far (an occurrence)
        // Walk arms in REVERSE so the first arm ends up outermost (first-match order).
        for arm in classified.iter().rev() {
            match arm {
                BinArm::CatchAll(body) => {
                    // A catch-all resets the tail to its body (a later bin arm before it is unreachable in
                    // first-match order, but we keep the structure simple — the catch-all is normally last).
                    acc = Some(*body);
                }
                BinArm::Bin(segs, guard, body) => {
                    // Handled at runtime: fixed-width INT segments, bit-field runs, and DEPENDENT-SIZE
                    // `(bytes body n)` segments at ANY position (§4a: a non-final dependent size makes the
                    // following offset dynamic — `static_base + Σ preceding n` — which `bin_dynamic_offset`
                    // now threads). An UNSIZED `(bytes rest)` must still be FINAL: a non-final open-ended rest
                    // is the permanent CDZ0220 ill-formed shape (nothing can follow an unbounded remainder).
                    let ok = segs.iter().enumerate().all(|(i, s)| match &s.kind {
                        crate::resolved::SegKind::Int { .. } => true,
                        // A BIT-FIELD run is admitted iff each field decodes — `bin_bitfield_run` requires a
                        // byte-aligned run of ≤64 bits preceded only by fixed-int segments (a mid-stream run
                        // that makes a following int offset sub-byte, or a >64-bit run, still declines). A
                        // LITERAL bit-field segment (a probe) is admitted too (its predicate reads the run).
                        crate::resolved::SegKind::Bits { .. } => {
                            bin_bitfield_run(segs, i).is_some()
                        }
                        // A DEPENDENT-SIZE `(bytes body n)` is admitted at any position (§4a dynamic offset);
                        // an UNSIZED `(bytes rest)` only as the FINAL segment (non-final unsized = CDZ0220).
                        crate::resolved::SegKind::Bytes { size: Some(_) } => true,
                        crate::resolved::SegKind::Bytes { size: None } => i + 1 == segs.len(),
                        // A `(utf8 s SIZE)` segment — CONSTANT or DEPENDENT (name) size, at ANY position — is
                        // decoded at runtime like a `(bytes … SIZE)` (its byte width enters the same
                        // static-base / `off_plus` offset + length plumbing), then its range is validated as
                        // strict UTF-8 (ill-formed = NON-MATCH → fall through, mirroring the const path).
                        // Admitted broadly; a shape whose offset / size is not computable declines cleanly in
                        // `build_bin_arm_predicate` / `decode_bin_field_runtime` (a Poison, not a miscompile).
                        crate::resolved::SegKind::Utf8 { .. } => true,
                    });
                    if !ok {
                        return Core::Poison(Reject::unsupported(
                            "a runtime bin match with a bit-field or non-final unsized bytes segment is not lowered",
                        ));
                    }
                    let Some(else_body) = acc else {
                        // A bin arm with no following catch-all: exhaustiveness already required a
                        // catch-all, so this is unreachable — decline defensively.
                        return Core::Poison(Reject::decline(
                            "a runtime bin match arm has no fallthrough (unreachable)",
                        ));
                    };
                    // The predicate reads the scrutinee through the materialized `scrut_ref`.
                    let pred = match build_bin_arm_predicate(db, scrut_ref, segs) {
                        Ok(p) => p,
                        Err(r) => return Core::Poison(r),
                    };
                    // §4b: a GUARD nests INTO the predicate as `pred AND <guard>` (short-circuit — the guard
                    // reads the decoded segment binders, which are only in bounds once `pred`'s length probe
                    // holds). A false guard makes the whole arm predicate false → falls through to the next
                    // arm's predicate, NOT a trap. The guard cond's segment binders resolve to the same
                    // `BinField` (Case 6bg) the body reads off the materialized scrutinee.
                    let pred = match guard {
                        None => pred,
                        Some(cond) => synth_core(
                            db,
                            Core::And {
                                lhs: pred,
                                rhs: *cond,
                                is_and: true,
                            },
                            crate::ty::Ty::Bool,
                        ),
                    };
                    acc = Some(synth_if(db, pred, *body, else_body));
                }
            }
        }
        let Some(root) = acc else {
            return Core::Poison(Reject::decline("a runtime bin match has no arms"));
        };
        // Wrap in a `let` that materializes the scrutinee once (keyed by its own occurrence — the same
        // occurrence the `scrut_ref` + each arm body's `BinField` read resolve their `LocalRef` to).
        return Core::Let {
            bindings: vec![(scrutinee, scrutinee)].into(),
            body: root,
        };
    };
    for arm in &classified {
        match arm {
            BinArm::CatchAll(body) => return core_of(db, *body),
            BinArm::Bin(segs, guard, body) => {
                // A segment BN3 can't decide (a dependent-size `(bytes b n)`) → we cannot know whether
                // this arm matches, so we must NOT silently skip it to a later arm (that would MISCOMPILE
                // a case whose dependent arm should match). `bin_match_decode` handles dependent-size
                // `(bytes body n)` now (BN4), decoding `n` from an earlier segment; a genuine non-match
                // (overrun / leftover / dependent-size overrun) returns `None` → fall to the next arm.
                let Some(decoded) = bin_match_decode(db, &raw, segs) else {
                    continue;
                };
                // Every LITERAL-slot segment must equal its decoded value (a magic-number / tag probe).
                // A binder slot (a bare name) is bound, not tested.
                let mut all_literals_match = true;
                for (seg, dec) in segs.iter().zip(decoded.iter()) {
                    // A slot is a literal probe iff it is NOT a bare name. Read its constant value.
                    if db.ast.as_name(seg.slot).is_some() {
                        continue; // a binder — no probe
                    }
                    match (core_of(db, seg.slot), dec) {
                        (Core::ConstInt(lit), BinDecoded::Int(got)) => {
                            if !lit.eq_value(got) {
                                all_literals_match = false;
                                break;
                            }
                        }
                        // A non-constant / non-int literal slot can't be decided here — abort the fold.
                        _ => {
                            return Core::Poison(Reject::decline(
                                "a bin pattern literal segment is not a constant integer",
                            ));
                        }
                    }
                }
                if all_literals_match {
                    // §4b: a GUARD is evaluated after the literals match. The guard cond reads the decoded
                    // segment binders (Case 6bg → `BinField` off this const scrutinee, folding to constants),
                    // so `core_of(cond)` folds to `ConstBool`. TRUE → this arm is the match; FALSE → fall
                    // through to the next arm (mirrors `lower_match`'s scalar guard fold). A guard that does
                    // NOT fold to a constant over a CONST scrutinee is a decline (a runtime op leaked into a
                    // const fold — should not happen for a well-formed guard reading only decoded binders).
                    if let Some(cond) = guard {
                        match core_of(db, *cond) {
                            Core::ConstBool(true) => return core_of(db, *body),
                            Core::ConstBool(false) => continue,
                            Core::Poison(r) => return Core::Poison(r),
                            _ => {
                                return Core::Poison(Reject::decline(
                                    "a guarded bin-match arm over a constant scrutinee did not fold its guard to a constant",
                                ));
                            }
                        }
                    }
                    return core_of(db, *body);
                }
            }
        }
    }
    // A catch-all is guaranteed present (checked above), so some arm always matches — unreachable.
    Core::Poison(Reject::decline(
        "bin match: no arm matched (unreachable — catch-all present)",
    ))
}

/// Whether the scrutinee subtree reaches a HOST-DELEGATED perform, checked WITHOUT lowering — a purely
/// resolved/AST walk (no `core_of`). This is the gate for keeping the `MatchSum` scrutinee-materialization
/// wrapper on a single-arm `Leaf` fold (so a host-reaching scrutinee is evaluated ONCE, not re-emitted per
/// payload binder). CRITICAL that it does NOT call `core_of`: `subtree_reaches_host_call` (the memoized
/// lowering-based sibling) forces `core_of` on the scrutinee mid-`lower_match_sum`, locking in a lowering
/// decision in the wrong order and PERTURBING the emit of UNRELATED matches (a curried-ctor scrutinee
/// lowered to an invalid module; a newtype-erasure byte-identity broke) — the memoization-order hazard the
/// `should_keep_binding` lambda short-circuits warn about. A resolved walk sees a perform as an `Apply`
/// whose head is an `effect_op_of`; a host-delegated one is the concern (an in-program-handled perform folds
/// away in `reduce_handle` and never re-emits). CONSERVATIVE: reports ANY perform in the scrutinee (a
/// handled one that survives to a runtime match scrutinee is rare and the wrapper is still correct — it only
/// binds the scrutinee once); follows NON-RECURSIVE callee bodies, bounded depth, over-reports past the bound.
pub(super) fn scrutinee_reaches_host_perform(db: &mut Db, scrutinee: StructId) -> bool {
    fn walk(db: &mut Db, node: StructId, depth: u32) -> bool {
        if depth > 24 {
            return true; // too deep — assume it may perform (safe over-report; forces the wrapper)
        }
        // A `(host (E) body)` BLOCK reached in the scrutinee delegates its effects to the host boundary,
        // so a perform in `body` is an observable host call. Treat ANY `Resolved::Host` node as reaching a
        // host perform — a CONSERVATIVE OVER-APPROXIMATION, not a claim that every compiling host block
        // performs (it does not: an op-REFERENCE-only body like `(host (E) (E.get))` — the op named but
        // never applied — compiles WITHOUT a perform; see the regression `a_host_with_too_many_operands_
        // is_cdz0201`, which asserts `(host (E) (E.get))` compiles OK). Over-reporting is SAFE here:
        // it only keeps the `MatchSum` wrapper (materialize the scrutinee ONCE), which is never wrong — a
        // non-performing host-block scrutinee is merely materialized rather than folded through, same value,
        // no re-emit. Under-reporting would be the bug. This is the ROBUST detector for a host perform
        // inlined through a callee (adv-62): when a callee like `(def (mk) (host (io) (let ((v (io.get)))
        // (tuple …))))` β-inlines into the scrutinee, the copied `io.get` occurrence LOSES its `effect-op`
        // meta (a copy artifact), so the `effect_op_of` probe below returns None and a genuine perform is
        // MISSED — but the `Resolved::Host` wrapper survives the copy structurally, so keying on it catches
        // the perform the meta-probe can't. Without the wrapper the Leaf fold re-emits the whole host block
        // once per tuple binder → the host op fires per closure and traps (call 2 of 1 response). Purely
        // resolved (no `core_of`), so it keeps the same memoization-order safety the rest of this walk uses.
        if let Resolved::Host { .. } = resolved_of(db, node) {
            return true;
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node) {
            if crate::eval::effect_op_of(db, head).is_some() {
                return true;
            }
            if let Some(callee) = crate::eval::lambda_body(db, head)
                .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
            {
                if crate::eval::is_recursive(db, callee) {
                    return false; // a recursive callee is not inlined into the scrutinee — no re-emit dup
                }
                if walk(db, callee, depth + 1) {
                    return true;
                }
            }
            return args.iter().any(|&a| walk(db, a, depth + 1));
        }
        match db.ast.get(node).clone() {
            crate::ast::Struct::List(children) => children.iter().any(|&c| walk(db, c, depth + 1)),
            crate::ast::Struct::Atom(_) => false,
        }
    }
    walk(db, scrutinee, 0)
}

/// Whether the match scrutinee IS (or heads with) a RECURSIVE call — the twin of the S2 inline-tuple
/// exponential (`1d568117b`), but for pattern BINDERS. A single-arm `Leaf` fold drops the `MatchSum`
/// wrapper and lowers the bare body; each payload binder in that body resolves to a `Core::SumPayload`
/// EMBEDDING the scrutinee expression, so a binder used K times re-emits the scrutinee K times. When the
/// scrutinee is a RECURSIVE call, that is K self-calls per level → 2^depth runtime recompute (a `(match (f
/// …) ((Mk a _) (Mk a a)))` where `a` is used twice TRAPS on the step limit past ~n=30; use `a` once and it
/// is linear — verified). Keeping the `MatchSum` wrapper MATERIALIZES the scrutinee into ONE slot (the same
/// fix `scrutinee_reaches_host_perform` applies for the effectful case), so the recursive call runs once and
/// every binder reads the slot. This is the case that walk EXPLICITLY excludes (`is_recursive => false`) —
/// correct for the host-perform concern (a recursive callee is not INLINED, so no re-perform) but wrong for
/// the pure recompute concern (the recursive CALL itself is what re-emits). Resolved-only (no `core_of`,
/// same memoization-order hazard). A NON-recursive call scrutinee does not blow up (its emit is bounded —
/// verified byte-identical at 1 vs 2 uses), so it stays the byte-identical bare-body fold.
pub(super) fn scrutinee_reaches_recursive_call(db: &mut Db, scrutinee: StructId) -> bool {
    fn walk(db: &mut Db, node: StructId, depth: u32) -> bool {
        if depth > 24 {
            return false; // bounded; a too-deep scrutinee is not the shallow call-head this targets
        }
        if let Resolved::Apply { head, args } = resolved_of(db, node) {
            // The call HEAD is a recursive def/lambda — the scrutinee re-runs a recursive computation per
            // binder use. This is the exponential trigger.
            if let Some(callee) = crate::eval::lambda_body(db, head)
                .or_else(|| crate::eval::lambda_body_of_nullary(db, head))
                && crate::eval::is_recursive(db, callee)
            {
                return true;
            }
            // Otherwise descend the head + args (a recursive call nested in the scrutinee expression, e.g.
            // `(g (f n))` — `f` recursive under a non-recursive `g`).
            if walk(db, head, depth + 1) {
                return true;
            }
            return args.iter().any(|&a| walk(db, a, depth + 1));
        }
        match db.ast.get(node).clone() {
            crate::ast::Struct::List(children) => children.iter().any(|&c| walk(db, c, depth + 1)),
            crate::ast::Struct::Atom(_) => false,
        }
    }
    walk(db, scrutinee, 0)
}

pub(super) fn lower_match_sum(
    db: &mut Db,
    scrutinee: StructId,
    arms: &[(StructId, StructId)],
) -> Core {
    // The scrutinee must be a COMPOUND the decision tree matches — a SUM (its type gives the root variant
    // set to switch on), a TUPLE (no discriminant; `Elem`-path binders/lit-tests), or a RECORD (no
    // discriminant and no destructure pattern — only a whole-value binder/wildcard arm). A poisoned
    // scrutinee propagates its poison; anything else is a decline (the caller routes only these here).
    let scrut_ty = crate::infer::type_of(db, scrutinee);
    if !matches!(
        scrut_ty,
        crate::ty::Ty::Sum { .. }
            | crate::ty::Ty::Nominal { .. }
            | crate::ty::Ty::Tuple(_)
            | crate::ty::Ty::Record(_)
    ) {
        if let Core::Poison(r) = core_of(db, scrutinee) {
            return Core::Poison(r);
        }
        return Core::Poison(Reject::decline(
            "compound match scrutinee is not a sum, tuple, or record",
        ));
    }
    // REJECT-DON'T-MISCOMPILE (v-effects finding #8): a record-LITERAL scrutinee whose fields REACH A HOST
    // PERFORM, destructured by a `(record (field binder) …)` arm, is a silent 3-backend miscompile — the
    // literal's performing fields fire once PER field-binder. A record-pattern field binder resolves to
    // `Resolved::Member { operand: scrutinee }` (resolve.rs Case 6rec), whose member-FOLD re-lowers the
    // source record literal's field init at EACH projection: `(match (record (a (E.get)) (b (E.get))) ((record
    // (a x) (b y)) …))` performs `E.get` once per `x`/`y` read (draws fire 2× the operations, wrong binder
    // values, and the re-eval advances are not all committed to the outer state — a DOUBLE defect). The
    // `MatchSum` wrapper materializes the scrutinee into ONE slot, but a record binder reads BY NAME through
    // the fold, bypassing that slot (unlike a tuple/sum binder, which reads the slot via `Elem`/`SumPayload`
    // — so a TUPLE-literal scrutinee with the same performing fields is CORRECT). The correct fix (fold the
    // record binder onto the materialized slot, the record twin of the tuple/sum `MatchSum` path) is a
    // deeper member-fold rewire — and is itself blocked behind a coupled effects-fold scope bug (a let-BOUND
    // record MATCHED under a handle declines CDZ0101 `unbound r`, even with no performing field; the
    // let-bound tuple/scalar equivalents compile). Until both land, DECLINE this exact shape rather than emit
    // wrong values + phantom host calls. TIGHTLY SCOPED: fires ONLY when the scrutinee's AST is a `record`
    // LITERAL form (a param/local/let-bound scrutinee — `Resolved` but not a literal `record` head — is
    // untouched: the working `let`+`(. r field)` projection readout stays green) AND that literal reaches a
    // host perform AND an arm is a record-destructure. The workaround the message names — bind the record
    // with `let`, then read its fields by `(. r field)` projection — evaluates each performing field exactly
    // once (verified: `rw2` = 56).
    if db
        .ast
        .compound_form_of(scrutinee, CompoundCtor::Record)
        .is_some()
        && scrutinee_reaches_host_perform(db, scrutinee)
        && arms.iter().any(|&(pat, _)| {
            let inner = match db.ast.as_form(pat, "guard") {
                Some(g) if g.len() == 2 => g[0],
                _ => pat,
            };
            db.ast
                .compound_form_of(inner, CompoundCtor::Record)
                .is_some()
        })
    {
        return Core::Poison(Reject::coded(
            Code::Malformed,
            "matching a record LITERAL whose fields perform an effect, destructured by a `(record …)` \
             pattern, is not supported — the record's performing fields would fire once per field \
             binder (a re-evaluation miscompile). Bind the record with `let` first, then read its fields \
             by `(. r field)` projection: `(let ((r (record …))) (+ (. r a) (. r b)))`",
        ));
    }
    // Build the initial pattern MATRIX: one row per arm, each a `(constraints, body)` where a constraint
    // is `(path, disc)` — "the sub-value at `path` must have discriminant `disc`". A row's constraints
    // start from its top-level pattern (path `[]`) and may nest. A malformed/unsupported pattern declines
    // the whole match (a heap walk / literal-in-sum is a later increment), never a silent match.
    let mut rows: Vec<MatchRow> = Vec::new();
    for &(pat, body) in arms {
        // Peel a `(guard <inner-pattern> <cond>)` wrapper: the arm's discriminant constraints come from
        // the inner pattern, and `<cond>` is carried as the row's guard (gated at the leaf in `build_tree`).
        let (inner_pat, guard) = match db.ast.as_form(pat, "guard") {
            Some(g) if g.len() == 2 => (g[0], Some(g[1])),
            _ => (pat, None),
        };
        // A GUARDED RECORD arm IS now lowered. The earlier decline (Inc-68) was NOT a `build_tree` gating
        // gap — an unguarded record arm produces no discriminant constraint but `build_tree` already gates
        // a record row's guard at the leaf like any other shape (it falls to the next arm on a guard-fail).
        // The real fault was a Perceus/borrow UAF: the guard cond's `Member`→`Core::Proj` read of the
        // record scrutinee reclaimed (dropped) the scrutinee handle after the cond, then the arm body's
        // field reads hit the freed handle → runtime `unreachable`. v-memory-safety's borrow fix (a
        // `Core::Proj` over a MATERIALIZED-SLOT operand BORROWS rather than reclaims — the enclosing match
        // owns the slot and drops it once after the arm) removed that, so the cond's Proj borrows and the
        // body's Proj reads a live handle. Verified end-to-end (guard-holds → value, guard-fails → next
        // arm, no drop between the cond and body field reads). A SCALAR-field record scrutinee is fully
        // sound; a COMPOUND-field record scrutinee is value-correct but leaks (the parked all-scalar
        // shell-reclaim floor, v-compiler-perf — not a new bug).
        // LINEARITY: a pattern is a BINDER POSITION and must bind each name at most once (core-semantics.md
        // §Patterns Compose: "A pattern MUST bind each name at most once … rather than silently shadowing").
        // `(tuple x x)` / `(Some (tuple x x))` binds `x` twice — CDZ0102, the same non-linear-binder error a
        // repeated `def` parameter gets — not a last-wins shadow that makes the first binder's payload
        // unreachable. Checked across the WHOLE arm pattern (nested sub-patterns included).
        if let Err(r) = check_pattern_linear(db, inner_pat) {
            return Core::Poison(r);
        }
        let mut lit_tests = Vec::new();
        match pattern_constraints(db, inner_pat, &scrut_ty, Vec::new(), &mut lit_tests) {
            Ok(constraints) => rows.push(MatchRow {
                constraints,
                lit_tests,
                body,
                guard,
            }),
            Err(r) => return Core::Poison(r),
        }
    }
    // Compile the matrix into a decision tree rooted at the scrutinee (path `[]`, type `scrut_ty`).
    let mut path_types: PathTypes = std::collections::HashMap::new();
    path_types.insert(Vec::new(), std::rc::Rc::new(scrut_ty));
    match build_tree(db, scrutinee, &rows, &mut path_types) {
        // The whole match reduces to one body (a top-level catch-all, a single-arm constructor
        // destructure, or a fully constant-folded tree). `build_tree` returns `Rc<SumCont>`; match the Leaf
        // shape through the Rc borrow. Normally a Leaf folds to `core_of(body)` — the scrutinee is only
        // re-read by the body's payload binders, cheap for a param/local. But when the scrutinee REACHES A
        // HOST perform, folding to the bare body is a MISCOMPILE: each `Core::SumPayload` binder in the body
        // RE-EMITS the scrutinee (select.rs `emit`), so `(match (mk (E.get)) ((T a b c) (+ (+ a b) c)))`
        // re-performs `E.get` once PER payload read (three host calls, want one) — the deterministic-host-
        // sequence violation the evaluate-once β-reduce fix exposed. Keep the `MatchSum` wrapper instead:
        // its emit MATERIALIZES a non-reusable scrutinee into ONE slot, so the host call runs once and every
        // payload binder reads the slot. A single-catch-all Leaf through `MatchSum` emits the same body (the
        // switch is degenerate), only with the scrutinee bound once first. GATED on `scrutinee_reaches_host_
        // perform` (a core_of-FREE resolved walk — NOT the memoized `subtree_reaches_host_call`, which
        // forces `core_of` mid-lower and perturbs unrelated matches): a reusable param/local scrutinee never
        // reaches a host call, so the common match still folds to the bare body, byte-identical to before.
        Ok(root)
            if matches!(&*root, crate::core::SumCont::Leaf(_))
                && (scrutinee_reaches_host_perform(db, scrutinee)
                    || scrutinee_reaches_recursive_call(db, scrutinee)) =>
        {
            // Keep the wrapper — MATERIALIZE the scrutinee into ONE slot — when it either reaches a host
            // perform (effectful re-perform, v-effects) OR reaches a recursive CALL (pure exponential
            // recompute: a payload binder used K times re-emits the recursive scrutinee K times → 2^depth,
            // the twin of the S2 inline-tuple fix `1d568117b`). Both are the same "non-reusable scrutinee
            // evaluated once" fix; a reusable param/local/constant scrutinee reaches neither and folds to the
            // bare body byte-identically.
            Core::MatchSum { scrutinee, root }
        }
        Ok(root) if matches!(&*root, crate::core::SumCont::Leaf(_)) => {
            let crate::core::SumCont::Leaf(body) = &*root else {
                unreachable!("just matched Leaf")
            };
            core_of(db, *body)
        }
        // Otherwise the root is a Switch (the usual case) — or a Guarded, when a disc-fold collapsed the
        // root switch to the selected variant's guarded arm. Either way the backend emits it through the
        // uniform `emit_sum_cont`, so carry the root continuation `Rc` directly (the DAG's entry point).
        Ok(root) => Core::MatchSum { scrutinee, root },
        Err(r) => Core::Poison(r),
    }
}

/// A match-decision PATH shared across nesting levels — an `Rc<[PathStep]>`, not a bare `Vec`:
/// `build_tree`'s partition loop re-clones every surviving row's constraint/lit-test paths at EACH nesting
/// level, and `build_tree` recurses once per level. With `Vec<PathStep>` paths a deeply-nested pattern
/// (`(Some (Some … x))`) deep-copied its O(depth)-long paths at every one of `depth` levels = O(depth³) (a
/// depth-800 nested match: ~2s, ~37% in `Vec::clone`). `Rc` makes each per-level path clone a pointer
/// bump, dropping the rebuild to O(depth²). (Same fix as `PathTypes`' `Rc<Ty>` values.)
pub(super) type MatchPath = std::rc::Rc<[crate::core::PathStep]>;
/// A discriminant CONSTRAINT: the sub-value at this `MatchPath` must have this variant discriminant.
pub(super) type PathConstraint = (MatchPath, u32);
/// A LITERAL test: the sub-value at this `MatchPath` must equal this literal probe.
pub(super) type PathLitTest = (MatchPath, crate::core::Probe);

/// One row of the pattern matrix: the discriminant CONSTRAINTS this arm imposes (each a `(path, disc)`),
/// and the arm's body. An empty constraint set is a catch-all (a bare binder / `_` top-level pattern) —
/// it matches regardless of any discriminant. Constraints are ordered outer-to-inner (a shorter path
/// first), which is the order the tree tests them.
#[derive(Clone)]
pub(super) struct MatchRow {
    constraints: Vec<PathConstraint>,
    /// LITERAL tests the arm imposes on payload sub-values: each `(path, probe)` requires the scalar at
    /// `path` to equal the literal. A `(Some 0)` pattern adds `([Payload], Int(0))`. Like a guard, a
    /// literal test does NOT count toward exhaustiveness (it may not match — it needs a same-variant
    /// binder/wildcard fall-through), and it is gated once the discriminant constraints are satisfied.
    lit_tests: Vec<PathLitTest>,
    body: StructId,
    /// A match-arm GUARD `(guard <pattern> <cond>)` — the boolean `<cond>` the arm additionally requires.
    /// `None` for an unguarded arm. Once every discriminant constraint is satisfied (the row reaches a
    /// leaf position in `build_tree`), a guarded row emits `if cond then body else <fall-through>` and
    /// does NOT count toward exhaustiveness; an unguarded row is an unconditional leaf.
    guard: Option<StructId>,
}

/// Reject a match-arm pattern that binds the same name more than once (CDZ0102) — a pattern is a BINDER
/// POSITION and must be LINEAR. Walks the whole pattern collecting BINDER names (a bare non-`_` name that
/// is NOT a variant constructor of a sum in scope, NOR a literal), and faults the second occurrence,
/// anchored there. A `_` binds nothing (may repeat); a variant name (`Some`, `E.Lit`) is a constructor,
/// not a binder; a literal is a value, not a binder. Recurses into tuple/variant sub-patterns and peels a
/// `(guard …)` wrapper — so linearity holds across the WHOLE composed pattern, a name in two sub-patterns
/// faulting exactly as one appearing twice in a flat pattern. (A non-deduping walk — unlike resolve's
/// binder lookups it must SEE every occurrence to catch the repeat.)
///
//= spec/capabilities/core-semantics.md#bindings-introduced-by-a-pattern-are-scoped-to-its-branch
//# A pattern MUST bind each name at most once; a pattern that binds the same name more than once MUST be a compile-time error (`CDZ0102`), so that a pattern is linear rather than silently shadowing an earlier binder or imposing a hidden equality constraint.
//= spec/capabilities/core-semantics.md#patterns-compose
//# A pattern MUST admit any pattern in each of its binder positions, so that a constructor pattern's binder and a tuple pattern's element MAY themselves be a wildcard, a name, a tuple pattern, or a constructor pattern, matched recursively to any depth.
//= spec/capabilities/core-semantics.md#patterns-compose
//# A composed pattern MUST bind the union of its sub-patterns' bindings, matched recursively, and MUST remain linear across the whole pattern, so that a name appearing in more than one sub-pattern is the same `CDZ0102` error as one appearing twice in a flat pattern.
pub(super) fn check_pattern_linear(db: &mut Db, pat: StructId) -> Result<(), Reject> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_pattern_binders(db, pat, &mut seen)
}

/// Validate a pattern in a BINDING position — a `let` binder, a `def`/`fn` parameter — where there is NO
/// alternative arm, so the pattern MUST be irrefutable. `value_ty` is the type of the value being bound (a
/// `let` initializer's type, or a parameter's solved type), used for the shape/arity check; pass `Ty::Any`
/// when it is not yet solved (the permissive treatment a projection of `Any` gets — no shape check, only
/// classification+linearity).
///
/// A binding pattern IS a single-arm match, so an ill-formed one gets the code the desugared match would.
/// A REFUTABLE pattern (a multi-variant constructor, a literal, a length-constrained list pattern) is
/// CDZ0210 (non-exhaustive — the other cases are uncovered and there is no fall-through arm). A
/// SHAPE-INCOMPATIBLE pattern (a wrong-arity tuple, a tuple pattern vs a non-tuple value) is CDZ0201. A
/// NON-LINEAR pattern (a binder repeated, flat or nested) is CDZ0102 (via `check_pattern_linear`).
///
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A binding position — a `let` binder, a function or `fn` parameter — MUST accept an irrefutable pattern in place of a bare name, binding the names the pattern introduces to the corresponding sub-values of the bound value, exactly as the same pattern would in a single match arm over that value. A bare name and a wildcard are the trivial irrefutable patterns; a tuple pattern whose every element is itself irrefutable is irrefutable, matched recursively to any depth in the sense of *Patterns Compose*. A destructuring parameter MUST NOT change the function's arity — the parameter occupies one argument position and names its parts, so `(def (f (tuple a b)) …)` remains a single-argument function.
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A binding position has no alternative arm, so its pattern MUST be irrefutable — it MUST match every value of the bound value's type.
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A refutable pattern in a binding position — a constructor pattern of a multi-variant sum, a literal, or a length-constrained list pattern, none of which matches every value of its type — MUST be a compile-time error (`CDZ0210`), the same non-exhaustiveness the equivalent single-arm match would raise under *Matching Is Exhaustive Or Rejected*.
//= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
//# A pattern whose shape cannot match the bound value's type at all — a tuple pattern of the wrong arity, or a tuple pattern against a non-tuple value — MUST be a compile-time error (`CDZ0201`), and a non-linear binding pattern MUST be the same `CDZ0102` error as in any other pattern position.
///
/// A pattern that is irrefutable in principle but not-yet-supported (a record pattern, a single-variant
/// user sum, any list pattern) DECLINES (reject-don't-miscompile — a later increment accepts it), NOT a
/// coded reject. The classifier consults the PRELUDE (a variant's owning sum + variant count), never a
/// head-string scan, so `None` is a constructor (not a binder) and a single-variant sum is told from a
/// multi-variant one.
///
/// A bare name / `_` is the trivial irrefutable pattern — Ok with no work (the common, hot binding).
pub(crate) fn check_binding_pattern(
    db: &mut Db,
    pat: StructId,
    value_ty: &crate::ty::Ty,
) -> Result<(), Reject> {
    // An ANNOTATED binding pattern `(: <pat> <Type>)` (type-system.md §Annotations Constrain, Never
    // Contradict): the annotation constrains the bound value's type and the inner `<pat>` is the real
    // binder. Peel it — check the annotation type AGREES with the value's type (a contradiction is
    // CDZ0203, `(: x Bool) = 5`), then recurse on `<pat>` so the inner pattern's own well-formedness
    // (irrefutable / linear / right shape) is still checked. A generic/deferred value type (`Any`, an
    // unsolved var) agrees with any annotation — the annotation grounds it, no contradiction.
    //= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
    //# A binding pattern MAY carry a type annotation `(: <pattern> <Type>)`, which constrains the bound value's type while the inner pattern binds its names, in accordance with *Annotations Constrain, Never Contradict* (`type-system.md`): the annotation participates in inference as an added constraint, and a value whose type cannot satisfy it MUST be a compile-time error (`CDZ0203`), exactly as a value annotation `(: <expression> <Type>)` is.
    if let Some(ann) = db.ast.as_form(pat, ":")
        && ann.len() == 2
    {
        let inner = ann[0];
        let ty_expr = ann[1];
        if let Some(annot_ty) = crate::eval::typeval_of(db, ty_expr)
            && !value_ty.agrees_with(&annot_ty)
        {
            // Append the structural DELTA (which field/element/axis differs) when the annotation and the
            // bound value are the same structured kind — two records of a different field set, two tuples
            // of a different arity, etc. Without it, `(: r (Record (x Int64)))` bound to `(record (y 2))`
            // rendered two whole record types the reader must diff; the delta names the minimal conflict
            // ("missing field `x`; no such field `y`"), the SAME hint the value-annotation / argument /
            // peer-join sites carry (`structural_delta_hint`, shared). The annotation is the expected type
            // (first), the value the actual (other).
            let delta = crate::infer::structural_delta_hint(&annot_ty, value_ty, &db.name_ctx())
                .unwrap_or_default();
            return Err(Reject::coded(
                Code::TypeMismatch,
                format!(
                    "a binder annotated {} is bound to a value of type {}{delta}",
                    annot_ty.render_name(&db.name_ctx()),
                    value_ty.render_name(&db.name_ctx())
                ),
            )
            .at(pat));
        }
        // The annotation may REFINE the value type (a deferred literal grounded to the annotated width),
        // so validate the inner pattern against the annotation type when it is more specific than the
        // value type, else the value type.
        let refined = crate::eval::typeval_of(db, ty_expr).unwrap_or_else(|| value_ty.clone());
        let inner_ty = if matches!(value_ty, crate::ty::Ty::Any) {
            refined
        } else {
            value_ty.clone()
        };
        return check_binding_pattern(db, inner, &inner_ty);
    }
    // A bare name (a binder) or `_` (wildcard) — trivially irrefutable, the common case.
    if let Some(name) = db.ast.as_name(pat) {
        // A bare name that resolves to a NULLARY constructor (`None`) is a refutable ctor, not a binder.
        if name != "_" && crate::eval::variant_disc_of(db, pat).is_some() {
            return classify_binding_ctor(db, pat, value_ty);
        }
        return Ok(());
    }
    // A literal `0` / `true` / `"s"` matches ONE value of its type — refutable, CDZ0210.
    if matches!(
        crate::resolve::resolved_of(db, pat),
        crate::resolved::Resolved::Int(_)
            | crate::resolved::Resolved::Bool(_)
            | crate::resolved::Resolved::Str(_)
            | crate::resolved::Resolved::Float(_)
            | crate::resolved::Resolved::Bytes(_)
    ) {
        return Err(Reject::coded(
            Code::NonExhaustive,
            "a literal pattern is refutable — it matches one value, not every value of its type, so it \
             cannot appear in a binding position",
        )
        .at(pat));
    }
    // A compound pattern `(head arg…)`. A `tuple` head is the one accepted destructuring shape in
    // Increment A; a constructor head is classified by variant count; a record/list head declines.
    //
    // This is where a tuple is DECONSTRUCTED by pattern matching: `(tuple a b)` in pattern position binds
    // its positional elements to `a`/`b` (each element sub-pattern recursed below), so a tuple's elements
    // are reachable by destructuring, not only by positional projection.
    //= spec/capabilities/core-semantics.md#a-tuple-is-a-fixed-size-positional-product
    //# A tuple MUST be deconstructible by pattern matching, so that `(tuple a b)` in pattern position binds the elements.
    if is_tuple_pattern(db, pat) {
        // Linearity across the WHOLE pattern (CDZ0102).
        check_pattern_linear(db, pat)?;
        let elems: Vec<StructId> = db
            .ast
            .compound_form_of(pat, CompoundCtor::Tuple)
            .unwrap_or(&[])
            .to_vec();
        // A binding position is IRREFUTABLE: each tuple ELEMENT sub-pattern must itself be irrefutable.
        // Recurse `check_binding_pattern` into each element with the element's own type, so a literal
        // element (int/bool/STRING/float) → CDZ0210, a multi-variant-ctor element → CDZ0210, a
        // single-variant/record/list element → DECLINE, and a bare-binder / nested-irrefutable-tuple
        // element → Ok — exactly the classification the TOP-LEVEL binder gets, at any nesting depth. The
        // BUG was that the tuple case called the MATCH-ARM collector `pattern_constraints` (where a
        // literal element is a runtime probe and a variant element a discriminant test — both legitimate
        // in a `match` arm) and then DISCARDED its result with a plain `Ok(())`, so a refutable
        // `(tuple 0 b)` / `(tuple (Some x) b)` binder slipped through and ran, silently dropping the
        // refutable sub-pattern. Recursing FIRST (before the arity check below) also gives a nested string/
        // float literal the same CDZ0210 the top-level binder emits, rather than the codeless
        // "malformed sum match pattern" decline `pattern_constraints`' atom fall-through produced.
        //
        // Split off a trailing `.. rest`: a tuple-rest binding pattern `#tuple(a .. rest)` is IRREFUTABLE —
        // a tuple has STATIC arity, so over any tuple of arity ≥ the leading count it ALWAYS matches, binding
        // the leading names + `rest` to the residual sub-tuple (v-spec-oracle ruling, core-semantics §A
        // Binding Position Accepts An Irrefutable Pattern lines 135-139; the binding-position resolver already
        // binds it via `find_binder_in_tuple`'s `TupleRestFrom`). Only the LEADING element sub-patterns are
        // recursed for irrefutability; the rest binder is a bare binder / `_` (itself irrefutable). Without
        // this split the `(.. rest)` node was recursed as an element → the head-not-recognized CDZ0201.
        let (lead_elems, rest): (&[StructId], Option<StructId>) = match db.ast.rest_marker(&elems) {
            Some((k, operand, trailing_start)) if trailing_start == elems.len() => {
                (&elems[..k], Some(operand))
            }
            _ => (&elems[..], None),
        };
        // A malformed `..` (not followed by exactly one binder) → the rest-shape CDZ0201 (the tuple twin of
        // the list/map/set rest-shape message), not the misleading head-not-recognized reject.
        if rest.is_none()
            && elems
                .iter()
                .any(|&e| db.ast.as_name(e) == Some("..") || db.ast.as_form(e, "..").is_some())
        {
            return Err(Reject::coded(
                Code::Malformed,
                "a tuple rest pattern is `#tuple(a… .. rest)` — exactly one binder after `..`",
            )
            .at(pat));
        }
        // Element types from the value type: leading positions map to the value tuple's leading types when it
        // is a tuple of arity ≥ the leading count (a rest absorbs the remainder — no exact-arity requirement);
        // else `Any` (permissive for an unsolved/`Any` or genuine-mismatch payload, faulted below).
        let elem_tys: Vec<crate::ty::Ty> = match value_ty {
            crate::ty::Ty::Tuple(ts) if ts.len() >= lead_elems.len() => {
                ts[..lead_elems.len()].to_vec()
            }
            _ => vec![crate::ty::Ty::Any; lead_elems.len()],
        };
        for (i, &elem) in lead_elems.iter().enumerate() {
            check_binding_pattern(db, elem, &elem_tys[i])?;
        }
        // Shape/arity against the value's type (CDZ0201) + nested-literal-TYPE agreement — reusing the
        // match-arm machinery verbatim. Runs AFTER the element refutability check so a refutable element's
        // CDZ0210 wins over this collector's shape decline; a well-shaped irrefutable pattern passes both.
        let mut lit_tests = Vec::new();
        pattern_constraints(db, pat, value_ty, Vec::new(), &mut lit_tests)?;
        return Ok(());
    }
    // A `(record (field p) …)` binding pattern — destructuring a record BY FIELD. Like a tuple, a record
    // is a fixed-shape product with an irrefutable destructure: `(record (x a) (y b))` names the `x`/`y`
    // fields' sub-values with NO discriminant test, so it is IRREFUTABLE iff each field's value sub-pattern
    // is. Its binders resolve field-by-field to a projection of the bound value (`a` ≡ `(. value x)`,
    // `resolve::last_binder_named`'s record arm → `Resolved::Member`, folding to a `Core::Proj` at the
    // field's SORTED slot) — the record analogue of the tuple's `SumPayload{Elem(i)}` binder, with the
    // name→index mapping handled by `runtime_member_index` where the record type is solved. This shape is
    // more flexible than a tuple: a PARTIAL record pattern `(record (x a))` over `(Record (x …)(y …))` binds
    // only `x` and is still irrefutable (a record pattern names the fields it wants; the rest are ignored),
    // whereas a tuple pattern must match the full arity. (Record-pattern irrefutability is governed by the
    // general §A Binding Position Accepts An Irrefutable Pattern sentence, cited above at the binding-position
    // entry — the spec has no record-pattern-specific sentence to //# here, so no dedicated citation.)
    if let Some(fields) = db
        .ast
        .compound_form_of(pat, CompoundCtor::Record)
        .map(<[_]>::to_vec)
    {
        // `compound_form_of` recognizes the native `#record(…)` ctor-leaf head too (not only the name/string
        // alias) — so a native `#record` destructuring PARAM binds like the classic `(record …)` (M3 canonical:
        // native works everywhere classic does; the def-param twin of the #5340/#5346 match-pattern hardening,
        // and the sibling of the tuple arm above which already reads native).
        // Linearity across the WHOLE pattern (CDZ0102) — two field values may not bind the same name.
        check_pattern_linear(db, pat)?;
        // The record's field types by name (when the value type is a solved record — else each field value
        // is checked against `Any`, the permissive treatment a wrong/unsolved value type gets: refutability
        // is a property of the pattern shape, not the value type, and a genuine field/type mismatch is
        // faulted by `pattern_constraints` below).
        let field_tys: Option<
            std::rc::Rc<std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty>>,
        > = match value_ty.strip_nominal() {
            crate::ty::Ty::Record(fs) => Some(fs.clone()),
            _ => None,
        };
        // Split off a trailing `.. rest`: a record open-row rest binding pattern `#record((= x a) .. rest)`
        // is IRREFUTABLE — a record has a STATIC field set, so over any record HAVING the named fields it
        // ALWAYS matches, binding the named field values + `rest` to a record of the UNNAMED fields
        // (core-semantics §A Binding Position Accepts An Irrefutable Pattern lines 135-139 + the tuple/record
        // rest MATCH clauses #6750; v-spec-oracle ruling). The rest binder resolves to a `Resolved::RecordRest`
        // in the binding position (mirroring the match-arm Case 6rec-rest); its type_of/const-fold are
        // origin-agnostic (v-inference). Only the NAMED field sub-patterns are recursed for irrefutability;
        // the rest binder is a bare binder / `_` (itself irrefutable). Without this split the `(.. rest)` node
        // was read as a field named `..` → the misclassified CDZ0203 "names field `..`".
        let (lead_fields, rest): (&[StructId], Option<StructId>) = match db.ast.rest_marker(&fields)
        {
            Some((k, operand, trailing_start)) if trailing_start == fields.len() => {
                (&fields[..k], Some(operand))
            }
            _ => (&fields[..], None),
        };
        // A malformed `..` (not followed by exactly one binder) → the rest-shape CDZ0201 (the record twin of
        // the list/map/set/tuple rest-shape message), not the misleading "names field `..`".
        if rest.is_none()
            && fields
                .iter()
                .any(|&f| db.ast.as_name(f) == Some("..") || db.ast.as_form(f, "..").is_some())
        {
            return Err(Reject::coded(
                Code::Malformed,
                "a record rest pattern is `#record((= f p)… .. rest)` — exactly one binder after `..`",
            )
            .at(pat));
        }
        let _ = rest;
        // Each `(key value)` field pair: the KEY is a field LABEL (never a binder), the VALUE sub-pattern is
        // a binder position — recurse `check_binding_pattern` into it with the field's own type, exactly as
        // the tuple arm recurses each element. A literal value → CDZ0210, a multi-variant-ctor value →
        // CDZ0210, a bare-binder / nested-irrefutable value → Ok, at any depth.
        for &pair in lead_fields {
            let crate::ast::Struct::List(kv) = db.ast.get(pair) else {
                continue; // a malformed field pair is faulted by `pattern_constraints` below
            };
            // A record-pattern field is the canonical `(= key sub-pattern)` triple (Phase B): key = child
            // 1, sub-pattern = child 2. A legacy `(key sub-pattern)` pair is tolerated (key = child 0).
            let (key_occ, value_pat) = if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") {
                (kv[1], kv[2])
            } else if kv.len() == 2 {
                (kv[0], kv[1])
            } else {
                continue; // a malformed field is faulted by `pattern_constraints` below
            };
            let key = crate::resolve::read_key(db, key_occ);
            // FIELD EXISTENCE (CDZ0201): when the value type is a SOLVED record, every named field the
            // pattern destructures must be a field of that record — a `(record (z a))` over `(Record (x
            // …)(y …))` names a field the value does not have. Anchor the fault at the field pair, the
            // minimal locus of the mistake. (An unsolved/`Any` value type grounds no such check here; a
            // non-record value type is faulted below.) This is the record analogue of the tuple arm's
            // arity check, and it fires EAGERLY at the pattern rather than only at the binder's projection
            // (where a missing field would otherwise surface as a `Member`-fold CDZ0201 pointing at the
            // body reference — the less actionable locus).
            if let (Some(fs), Some(sym)) = (field_tys.as_ref(), key.clone())
                && !fs.contains_key(&sym)
            {
                return Err(Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "a record binding pattern names field `{}`, which the bound value of type {} \
                         does not have",
                        sym.name,
                        value_ty.render_name(&db.name_ctx())
                    ),
                )
                .at(pair));
            }
            // FIELD-VALUE SCOPE (Increment B): a field's value sub-pattern is a bare BINDER `a` or a
            // WILDCARD `_`. A NESTED compound field value (`(record (p (tuple a b)))`) is irrefutable in
            // principle, but its binders cannot yet be WIRED — a record field projects by NAME→sorted-slot
            // (`Resolved::Member`, folding to `Core::Proj`), and `PathStep` has no name-keyed step to
            // COMPOSE a projection with a further sub-path, so `resolve::last_binder_named`'s record arm
            // wires only a bare-binder field. DECLINE a nested field value cleanly here (an honest coded
            // outcome), keeping LOWER and RESOLVE in lockstep — never a silent CDZ0101 on an unwired binder.
            // (A literal / multi-variant-ctor field value is REFUTABLE — routed to `check_binding_pattern`
            // for its CDZ0210, the same fault the tuple arm gives such an element.)
            let field_ty = key
                .and_then(|sym| field_tys.as_ref().and_then(|fs| fs.get(&sym).cloned()))
                .unwrap_or(crate::ty::Ty::Any);
            // Recurse first: a literal / multi-variant-ctor / refutable field value faults CDZ0210 (the same
            // fault the tuple arm gives such an element), winning over the scope decline below.
            check_binding_pattern(db, value_pat, &field_ty)?;
            // A field value that PASSED the irrefutability check but is NOT a bare binder / wildcard is a
            // NESTED compound. A POSITIONAL compound (a tuple/list whose binders are all reachable by `Elem`
            // steps — no variant `Payload`, no nested record) is now WIRED (§235, the binding twin of the
            // slice-1 match path): `resolve::find_record_binder_in_pattern` descends it into a `RecordField`
            // `sub_path`, and lowering appends those `Elem` steps below the field's `Elem(slot)`. ACCEPT it.
            // A field value with a nested RECORD (a deferred name-keyed slot) or a VARIANT (a `Payload` step
            // needing a `sub_heads` entry `RecordField` does not yet carry) is NOT yet wireable — DECLINE
            // cleanly, keeping LOWER and RESOLVE in lockstep (the resolve producer rejects the SAME shapes:
            // a record element via its false-branch, a variant via a non-empty `sub_heads`). Never a silent
            // CDZ0101 on an unwired binder.
            if !is_positional_field_value(db, value_pat) {
                // CODED decline tagged with the tracked `DeclineId` (v-deferral seq-286) — the check-side
                // twin of the resolve-side `last_binder_named` residual decline. Narrowed to the STILL-unwired
                // shapes (record/variant below a field); a positional tuple/list field value binds via the
                // `RecordField.sub_path` (§235).
                return Err(Reject::declined(
                    crate::diag::DeclineId::NestedRecordFieldPatternDescent,
                    "a nested compound sub-pattern inside a record binding pattern is not supported \
                     (a record binding binds a field to a bare name or a positional tuple/list pattern; \
                     destructure a nested record or variant field with a further `let`)",
                ));
            }
        }
        // The bound value must BE a record (or an unsolved/`Any` type the pattern grounds) — a `(record
        // …)` binding pattern over a non-record value (`((record (x a)) 5)`) is a type mismatch (CDZ0203).
        // Checked AFTER the per-field refutability + existence checks so a field-level fault (the more
        // specific mistake) wins. (`pattern_constraints` — the tuple arm's shape checker — DECLINES a
        // record pattern outright, so the record shape check is inline here rather than delegated.)
        if !matches!(
            value_ty.strip_nominal(),
            crate::ty::Ty::Record(_) | crate::ty::Ty::Any
        ) {
            return Err(Reject::coded(
                Code::TypeMismatch,
                format!(
                    "a record binding pattern destructures a record, but the bound value has type {}",
                    value_ty.render_name(&db.name_ctx())
                ),
            )
            .at(pat));
        }
        return Ok(());
    }
    // A `(list …)` binding pattern. A binding position is IRREFUTABLE, and a list pattern is irrefutable
    // ONLY in the REST form `(list p… .. rest)` — it matches ANY length ≥ the leading count, and the empty
    // prefix `(list .. rest)` matches every list. A FIXED-ARITY `(list a b)` matches only length-2 lists, so
    // it is REFUTABLE → CDZ0210, the same non-exhaustiveness the equivalent single-arm match raises
    // (`core-semantics.md §A Binding Position Accepts An Irrefutable Pattern`). Each leading element position
    // + the rest binder is itself a *Patterns Compose* binder position, so a nested element must ALSO be
    // irrefutable (recursed via `check_binding_pattern`), and the whole pattern must be LINEAR (CDZ0102).
    //= spec/capabilities/core-semantics.md#a-list-is-deconstructed-by-element-patterns-with-an-optional-rest
    //# Each element position and the rest binder MUST be a binder position in the sense of *Patterns Compose*, so an element MAY itself be any pattern (a wildcard, a name, a tuple pattern, a constructor pattern, or a nested element pattern) matched recursively, and the whole pattern MUST remain linear (`CDZ0102`).
    if let Some(elems) = db
        .ast
        .compound_form_of(pat, CompoundCtor::List)
        .map(<[_]>::to_vec)
    {
        // Linearity across the WHOLE list pattern (CDZ0102) — the same check the tuple case runs.
        check_pattern_linear(db, pat)?;
        let Some((dd, operand, trailing_start)) = db.ast.rest_marker(&elems) else {
            // No `..` — a FIXED-ARITY list pattern, refutable (matches only its exact length). CDZ0210.
            return Err(Reject::coded(
                Code::NonExhaustive,
                "a fixed-arity list pattern is refutable — it matches only lists of that exact length, \
                 not every list, so it cannot appear in a binding position (use the ZERO-LEADING \
                 `(list .. rest)`, the only rest form that matches EVERY list including the empty one — a \
                 leading-element `(list a .. rest)` is itself refutable here — or a `match`)",
            )
            .at(pat));
        };
        // A rest pattern needs EXACTLY one binder after `..`, and that binder must be a bare name / `_`
        // (it binds the tail SUBLIST — a nested rest pattern is a later increment).
        if trailing_start != elems.len() {
            return Err(Reject::coded(
                Code::Malformed,
                "a list rest pattern is `(list p… .. rest)` — exactly one binder after `..`",
            )
            .at(pat));
        }
        if db.ast.as_name(operand).is_none() {
            return Err(
                Reject::coded(Code::Malformed, crate::diag::LIST_REST_BINDER_NAME_ONLY).at(pat),
            );
        }
        // ONLY the ZERO-LEADING rest form `(list .. rest)` is IRREFUTABLE — it matches EVERY list (the
        // empty list included), binding `rest` to the whole list. A LEADING-element rest `(list a .. rest)`
        // (`dd > 0`) is REFUTABLE: it requires at least `dd` elements, so it does NOT match the EMPTY list
        // (core-semantics.md §147 — "a single-leading-element-plus-rest pattern MUST match any NON-empty
        // list"; only §"a-list-is-deconstructed…"'s zero-leading form matches every list). A refutable
        // pattern in a BINDING position is CDZ0210 (§139), the same rule the fixed-arity form above gets —
        // otherwise `(def (head (list x .. rest)) x)` would compile and then TRAP (`unreachable`) reading
        // element 0 of an empty list, a fault the type system MUST reject at compile time. A possibly-empty
        // leading-rest destructure belongs in a `match` (whose arms cover the empty case), not a binding.
        //= spec/capabilities/core-semantics.md#a-binding-position-accepts-an-irrefutable-pattern
        if dd > 0 {
            return Err(Reject::coded(
                Code::NonExhaustive,
                "a leading-element list rest pattern `(list a .. rest)` is refutable — it does not match \
                 the EMPTY list, so it cannot appear in a binding position (use the zero-leading \
                 `(list .. rest)`, which matches every list, or a `match`)",
            )
            .at(pat));
        }
        // The zero-leading `(list .. rest)` has no leading elements to recurse; it binds `rest` = the whole
        // list and is irrefutable. (The leading-element loop below is a no-op for `dd == 0`.)
        // The list's element type (for the leading sub-patterns' shape check); `Any` when unsolved.
        let elem_ty = match value_ty {
            crate::ty::Ty::List(e) => (**e).clone(),
            _ => crate::ty::Ty::Any,
        };
        // Each LEADING element sub-pattern must itself be IRREFUTABLE (composes to any depth). A refutable
        // leading element (a literal, a multi-variant ctor) is CDZ0210 exactly as a top-level binder.
        for &elem in &elems[..dd] {
            check_binding_pattern(db, elem, &elem_ty)?;
        }
        return Ok(());
    }
    // A `(map …)` binding pattern (native `#map(…)` OR the name/string alias — `compound_form_of` reads all
    // three). A map pattern tests KEY PRESENCE, so a keyed/empty map pattern is REFUTABLE (the named keys may
    // be absent), and the only irrefutable form — a bare `(map .. rest)` — binds the whole map, which a plain
    // name binder already does. So a map is never a useful IRREFUTABLE binding pattern → CDZ0210, with the
    // actionable repair, rather than the ctor-classifier's generic "not a tuple/record/constructor" (which
    // does not name maps). This is the map analogue of the fixed-arity/leading-rest list refutability above;
    // the MATCH-arm path (whose arms cover the missing-key case) is where a map is destructured.
    if db.ast.compound_form_of(pat, CompoundCtor::Map).is_some() {
        return Err(Reject::coded(
            Code::NonExhaustive,
            "a map binding pattern is refutable — it tests key presence, so it does not match every map and \
             cannot appear in a binding position; bind the whole map to a name and read keys with \
             `Map.lookup`, or destructure it in a `match` (whose arms cover the missing-key case)",
        )
        .at(pat));
    }
    // Otherwise a constructor-headed pattern `(Some x)` / `((. Sum V) x)` — classify by variant count.
    classify_binding_ctor(db, pat, value_ty)
}

/// Classify a CONSTRUCTOR-headed binding pattern (`(Some x)`, bare `None`, `((. Sum V) x)`): a
/// SINGLE-variant sum is irrefutable but a later increment (DECLINE); a MULTI-variant sum is refutable
/// (the other variants are uncovered) → CDZ0210. The head is resolved against the prelude
/// (`variant_owner_decl` → the owning sum's declaration → its variant count), never a head-string scan.
/// A head that is not a constructor at all is a shape error (CDZ0201).
pub(super) fn classify_binding_ctor(
    db: &mut Db,
    pat: StructId,
    value_ty: &crate::ty::Ty,
) -> Result<(), Reject> {
    // The constructor head: a bare name / member `(. Sum V)` used as a whole pattern, or a `(head arg…)`
    // application's head.
    let head = match db.ast.get(pat) {
        crate::ast::Struct::Atom(_) => pat,
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor is the pattern itself.
            Some(first) if db.ast.as_name(first) == Some(".") => pat,
            Some(first) => first,
            None => {
                return Err(Reject::coded(Code::Malformed, "an empty binding pattern").at(pat));
            }
        },
    };
    let Some(decl) = crate::eval::variant_owner_decl(db, head) else {
        // A MALFORMED `const` PARAMETER — `(const n Int64)` — that survived `strip_const_params` (which
        // only unwraps a well-formed single-operand `(const <binder>)`). A `const` param wraps ONE annotated
        // binder, so a bare `(const n Int64)` (two operands, unannotated) reaches here as a "binding pattern"
        // whose head is `const`; the generic "not a tuple/record/constructor" message is misleading (the
        // author reached for a real form, just mis-wrote it). Name the correct shape.
        if db.ast.as_name(head) == Some("const")
            && db.ast.as_form(pat, "const").is_some_and(|t| t.len() != 1)
        {
            return Err(Reject::coded(
                Code::Malformed,
                "a `const` parameter wraps exactly ONE annotated binder — write `(const (: <name> \
                 <Type>))`, e.g. `(const (: n Int64))`",
            )
            .at(pat));
        }
        // A TYPED PARAMETER MISSING ITS COLON — `(a Float64)` written where `(: a Float64)` was meant.
        // The author reached for the annotated-binder form (§Annotations Constrain) but juxtaposed the
        // binder and its type instead of heading them with `:`, so it reaches here as a two-element list
        // whose head `a` is not a constructor → the generic "not a tuple/record/constructor" message is
        // misleading (and, if the body uses `a`, spawns a consequent "unbound name `a`"). Recognize the
        // shape — a two-element list `(<name> <Type>)` whose SECOND child resolves as a type
        // (`typeval_of`) and whose FIRST is a plain binder name — and name the real repair. The rewrite
        // `(<name> <Type>)` → `(: <name> <Type>)` is a deterministic rule (not a guess), so when both
        // parts are simple name atoms the fix is VERIFIED and carries the exact replacement spelling.
        let two_children = match db.ast.get(pat) {
            crate::ast::Struct::List(items) => match items.as_slice() {
                [first, second] => Some((*first, *second)),
                _ => None,
            },
            _ => None,
        };
        if let Some((first, second)) = two_children
            && db.ast.as_name(first).is_some_and(|n| n != "_")
            && crate::eval::variant_owner_decl(db, first).is_none()
            && crate::eval::typeval_of(db, second).is_some()
        {
            let name = db.ast.as_name(first).unwrap().to_string();
            let base = Reject::coded(
                Code::Malformed,
                format!(
                    "a typed parameter is written `(: <name> <Type>)`, with a leading `:` — the binder \
                     `{name}` is juxtaposed with its type, so it reads as a constructor pattern, not an \
                     annotated binder; add the `:` to annotate it"
                ),
            )
            .at(pat);
            // A compound type (`(List Int64)`) has no single name atom to splice, so carry the fix only
            // when the type is a bare name — the common `(a Float64)` case — where the replacement is a
            // literal `(: <name> <Type>)`; otherwise the message alone routes the repair.
            return Err(match db.ast.as_name(second) {
                Some(ty_name) => base.with_fix(Fix::replace_verified(
                    pat,
                    format!("(: {name} {ty_name})"),
                    "add the leading `:`",
                )),
                None => base,
            });
        }
        // Not a constructor — a shape error (a head that is neither tuple/record/list nor a ctor). But a
        // BARE NAME head that is a plausible TYPO of a variant of the matched (element) SUM type — e.g.
        // `(list (Ad) .. r)` on `(List Op)` for `(type Op (Add) …)` — read as "not a constructor" here
        // (an unbound name is not a ctor), giving the opaque shape message. When the matched type is a sum,
        // ENRICH with the same "did you mean `Add`?" + rename fix a top-level match arm gets
        // (`enrich_pattern_head_suggestion` over the element sum's variants) — so a misspelled variant in a
        // LIST-ELEMENT pattern reads as well as one in a direct match arm. A non-sum element type, or a head
        // with no near variant, keeps the bare shape message.
        let base = Reject::coded(
            Code::Malformed,
            "a binding pattern head is not a tuple, record, or constructor",
        )
        .at(pat);
        return Err(enrich_pattern_head_suggestion(db, head, value_ty, base));
    };
    let variant_count = db
        .type_decl_by_occ(decl)
        .map(|d| d.variants.len())
        .unwrap_or(0);
    if variant_count == 1 {
        // A single-variant sum's sole constructor ALWAYS matches — the pattern is IRREFUTABLE, so it is a
        // valid binding position (`(let (((Id.Mk n) v)) …)`). Its payload sub-patterns must themselves be
        // irrefutable, exactly as a tuple pattern's elements are: recurse `check_binding_pattern` into each
        // payload arg at the payload's type (a literal payload → CDZ0210, a bare binder / nested tuple →
        // Ok). The payload TYPE is the variant's payload at this instantiation; `pattern_constraints` then
        // checks the shape/arity (CDZ0201) + linearity, reusing the match-arm machinery, so the binder
        // references (which resolve to a `SumPayload` reading the payload — resolve `last_binder_named`'s
        // ctor case) read a well-formed pattern. A nullary single-variant sum (`(type Marker (The))`) has
        // no payload arg to bind — nothing to recurse, trivially irrefutable.
        check_pattern_linear(db, pat)?;
        let args: Vec<StructId> = match db.ast.get(pat) {
            crate::ast::Struct::List(children) => match children.first().copied() {
                // A bare member `(. Sum V)` used whole — no payload args in the pattern.
                Some(first) if db.ast.as_name(first) == Some(".") => Vec::new(),
                _ => children[1..].to_vec(),
            },
            _ => Vec::new(),
        };
        // Each payload arg's type — the variant's payload types at the value's instantiation. A single
        // payload IS the underlying type; multiple payloads box as one tuple (matched positionally). Use
        // the value type's payload when resolvable, else `Any` (permissive — arity/shape faults below).
        for &arg in &args {
            // A payload arg is validated for irrefutability against `Any` (refutability is a property of
            // the pattern shape, not the value type — the tuple case does the same for its elements).
            check_binding_pattern(db, arg, &crate::ty::Ty::Any)?;
        }
        // Shape/arity + nested-literal-type agreement, reusing the match-arm collector (CDZ0201 on a
        // wrong-arity payload). Runs after the per-arg irrefutability check, exactly as the tuple case.
        let mut lit_tests = Vec::new();
        pattern_constraints(db, pat, value_ty, Vec::new(), &mut lit_tests)?;
        return Ok(());
    }
    // A multi-variant constructor is refutable — the other variants are uncovered, and there is no
    // alternative arm. CDZ0210, the non-exhaustive-single-arm-match code.
    Err(Reject::coded(
        Code::NonExhaustive,
        "a multi-variant constructor pattern is refutable — the other variants are uncovered, so it \
         cannot appear in a binding position (only in a `match` arm)",
    )
    .at(pat))
}

/// The recursive walk behind [`check_pattern_linear`]: insert each binder name into `seen`, faulting a
/// repeat. See that function for the binder-vs-ctor-vs-literal classification.
pub(super) fn collect_pattern_binders(
    db: &mut Db,
    pat: StructId,
    seen: &mut std::collections::HashSet<String>,
) -> Result<(), Reject> {
    // Peel a guard wrapper — the binder-carrying pattern is the inner one.
    if let Some(g) = db.ast.as_form(pat, "guard")
        && g.len() == 2
    {
        return collect_pattern_binders(db, g[0], seen);
    }
    // A bare atom: a literal binds nothing; a `_` binds nothing; any OTHER bare name is a binder UNLESS it
    // is a nullary variant constructor (`None`, `Sign.Neg`) — a ctor is not a binder. `variant_disc_of`
    // recognizes a ctor value; a name that is not one is a binder.
    if let crate::ast::Struct::Atom(_) = db.ast.get(pat) {
        // A pure TAG test — is this atom an Int/Bool literal? Dispatch through the BORROW companion
        // `resolved_ref`, not `resolved_of`: the latter CLONES the whole `Resolved` per call just to read
        // the variant tag, and `collect_pattern_binders` is on the hot match-lowering path (a match-heavy
        // program — a parser like `sread.cdz` — checks it per pattern atom; it was ~3% of that real check
        // workload via `Resolved::clone`). The fix-35/36/`prim_of` borrow pattern.
        if matches!(
            crate::resolve::resolved_ref(db, pat),
            crate::resolved::Resolved::Int(_) | crate::resolved::Resolved::Bool(_)
        ) {
            return Ok(()); // a literal is not a binder
        }
        if let Some(name) = db.ast.as_name(pat).map(|s| s.to_string()) {
            // `_` binds nothing; `..` is the list/map REST MARKER, a syntactic token — NOT a binder (the
            // rest BINDER is the name AFTER `..`). Without skipping `..`, two rest patterns in ONE arm — a
            // tuple of two rest-lists `(tuple (list a .. r1) (list b .. r2))` — falsely faulted CDZ0102
            // "binds `..` more than once" (the marker counted as a repeated binder). Both are non-binding.
            if name == "_" || name == ".." {
                return Ok(());
            }
            // A bare name that resolves to a variant constructor is a ctor, not a binder.
            if crate::eval::variant_disc_of(db, pat).is_some() {
                return Ok(());
            }
            if !seen.insert(name.clone()) {
                // RENAME the repeated binder to a fresh non-colliding name (`a` → `a2`), making the pattern
                // linear (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). Fresh
                // relative to the binders already seen in this pattern, so it collides with none. Heuristic:
                // the rename clears the hard error; the fresh binder is then unused until the author uses it
                // (two same-named binders were likely meant to be distinct values, or an equality the pattern
                // language does not express). Anchored at the repeated binder occurrence.
                let fresh = crate::diag::suggest::fresh_suffixed_name(&name, seen);
                return Err(Reject::coded(
                    Code::NonLinearBinder,
                    format!("pattern binds `{name}` more than once (a pattern must be linear)"),
                )
                .at(pat)
                .with_fix(Fix::replace_heuristic(pat, fresh)));
            }
        }
        return Ok(());
    }
    // A compound pattern `(head arg…)` — a variant `(Some p)`, a tuple `(tuple p…)`, or a member `(. S V)`
    // (a nullary ctor, no binders). The head is a ctor/`tuple`/`.` — not a binder; recurse into the args.
    if let crate::ast::Struct::List(children) = db.ast.get(pat) {
        let children = children.clone();
        // A `(. Sum V)` member pattern is a nullary-ctor reference — no binder args.
        if children.first().and_then(|&h| db.ast.as_name(h)) == Some(".") {
            return Ok(());
        }
        // A record-pattern FIELD `(= field sub-pattern)` (path B): only the sub-pattern (child 2) binds;
        // the field NAME does not. Without this the generic recursion would collect the field name as a
        // spurious binder (and falsely fault CDZ0102 when two fields share a slot name).
        if children.len() == 3 && db.ast.as_name(children[0]) == Some("=") {
            return collect_pattern_binders(db, children[2], seen);
        }
        // Skip the head (a ctor / `tuple`/`record` alias); recurse each argument sub-pattern. A
        // `(record (= f p) …)` field is handled above; a legacy `(f p)` pair recurses here (head skipped).
        for &arg in children.iter().skip(1) {
            collect_pattern_binders(db, arg, seen)?;
        }
    }
    Ok(())
}

/// Collect the discriminant constraints a PATTERN imposes on the sub-value at `path` (of type `ty`),
/// appending `(deeper-path, disc)` per variant test. A bare NAME is a binder/wildcard — NO constraint
/// (it matches any value; its binding is resolved independently). A variant pattern `(V arg…)` / bare
/// nullary `V` adds `(path, disc(V))` and recurses into its single payload arg at `path + [Payload]`
/// (a multi-payload variant's payload is a tuple — the arg descends through `Elem` in a later increment).
/// A variant name is distinguished from a binder by RESOLVING it against `ty`'s variant set: `None`
/// against `Option` is the nullary variant (a constraint), `x` is a binder (none). Errs (declines) on a
/// pattern this increment does not compile — a tuple/record destructure, a literal, a wrong-arity ctor.
///
/// A nullary variant pattern (`None`) and a unary+ one (`(Some x)`) are handled by the SAME arm — each
/// adds its discriminant test and descends into one payload position — so the matcher never branches on
/// a constructor's arity: every constructor pattern is treated uniformly as a single-arity application.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# The pattern matcher MUST NOT special-case "nullary" vs "unary+" constructors by arity.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# The pattern matcher MUST handle all constructor patterns uniformly as single-arity applications.
//= spec/capabilities/core-semantics.md#a-sum-type-constructor-is-a-single-arity-function-producing-the-tagged-variant
//# A pattern matching a sum type constructor MUST have the form `(Ctor binder)` in all cases: `(Some x)`, `(None _)`, `(Sign.Zero _)`.
/// Enrich the propagated "record has no field `Q`" poison of a MATCH-PATTERN head `(. Sum Q)` — where `Q`
/// is not a variant of the scrutinee sum — with a "did you mean?" over the sum's VARIANT NAMES, plus a
/// replace fix on the mistyped key. The pattern-position twin of `infer::no_field_reject`'s value-position
/// suggestion: `core_of(head)` (the member fold) emits the bare coded message; here — where the scrutinee
/// sum type `ty` is in hand — we can name the nearest variant. `ty` is the scrutinee's type (a `Ty::Sum`
/// when this fires; a non-sum leaves the bare `reject` untouched). Returns the enriched (or original)
/// reject. Deterministic — `suggest::nearest` over the declaration-ordered variant set.
pub(super) fn enrich_pattern_head_suggestion(
    db: &mut Db,
    head: StructId,
    ty: &crate::ty::Ty,
    reject: Reject,
) -> Reject {
    // The scrutinee's declaration occurrence — the key of its (memoized) variant candidate set. A boxed
    // multi-variant `Ty::Sum` AND a single-variant `Ty::Nominal` newtype (a `(type T (Mk …))` erases to a
    // nominal, whose sole variant is still named) both carry a `decl` whose variants are the candidate set,
    // so a wrong-ctor pattern over EITHER gets the same "did you mean?" / closest-variants enrichment.
    let decl = match ty {
        crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => *decl,
        _ => return reject,
    };
    // The mistyped key + the occurrence a replace fix rewrites. Two head shapes reach here:
    //  - a QUALIFIED `(. Sum Q)` head — the key is its second child (`((C.Alph) …)`, a CDZ0201 "record
    //    has no field" poison from the member fold); rewrite the key child.
    //  - a BARE name head — `((Alph) …)`, a CDZ0101 "unbound name" poison (a bare pattern name resolves as
    //    an ordinary name → unbound when it is not a variant). The key IS the head node; rewrite it whole.
    // Either way the scrutinee's sum gives the candidate variant set, so a mistyped BARE variant name gets
    // the same "did you mean?" the qualified form does (`((Alph) …)` on `(type C (Alpha) …)` → `Alpha`).
    let key_occ = match db.ast.as_form(head, ".").and_then(|t| t.get(1).copied()) {
        Some(k) => k,
        None if db.ast.as_name(head).is_some() => head,
        None => return reject,
    };
    let Some(key) = db.ast.as_name(key_occ).map(str::to_string) else {
        return reject;
    };
    // The confident single (memoized per (decl, key) — a wide sum matched with the SAME stale variant from
    // N sites would otherwise re-run the O(variants) scan each → O(N²)). Drives the REPLACE fix. The
    // scrutinee sum's variant names (the closed candidate set) are cloned + scanned ONLY on a memo MISS —
    // deferred behind the cache so a repeated (decl, key) query does not even re-clone the O(variants) list
    // (the wrong-sum-ctor path hits this from N sites against a wide sum).
    let candidate = if let Some(hit) = db.variant_suggest_winner.get(&(decl, key.clone())) {
        hit.clone()
    } else {
        let names: Vec<String> = match db.type_decl_by_occ(decl) {
            Some(t) => t.variants.iter().map(|v| v.name.clone()).collect(),
            None => return reject,
        };
        let winner = crate::diag::suggest::nearest(&key, &names);
        db.variant_suggest_winner
            .insert((decl, key.clone()), winner.clone());
        winner
    };
    match candidate {
        // TIER 1 — a confident typo: name the variant + carry the replace fix on the key occurrence
        // (mirroring `infer::no_field_reject`'s value-position enrichment).
        Some(candidate) => {
            let message = format!("{} — did you mean `{candidate}`?", reject.message);
            Reject { message, ..reject }.with_fix(Fix::replace_heuristic(key_occ, candidate))
        }
        // TIER 2 — no confident typo: LIST the closest variants (`— closest matches: `A`, `B``) so a far
        // pattern-head typo tells the author what variants the sum actually has, instead of the dead-end
        // "record has no field". A sum is a CLOSED variant set, so listing is signal (the pattern-position
        // twin of the member-access two-tier). No fix (a list of options is not one mechanical edit).
        None => {
            // The closest-variants LIST, MEMOIZED per (decl, key) — its `closest_matches` SORTS all N
            // variants by edit distance (O(N log N)), and a WRONG-SUM ctor (always a far miss) matched from
            // N sites against a wide sum re-ran it each → O(N² log N). Build `names` + sort only on a miss.
            let close = if let Some(hit) = db.variant_closest_matches.get(&(decl, key.clone())) {
                hit.clone()
            } else {
                #[cfg(test)]
                crate::db::VARIANT_CLOSEST_MATCHES_MISSES.with(|c| c.set(c.get() + 1));
                let names: Vec<String> = match db.type_decl_by_occ(decl) {
                    Some(t) => t.variants.iter().map(|v| v.name.clone()).collect(),
                    None => return reject,
                };
                let close = crate::diag::suggest::closest_matches(
                    &key,
                    names.iter().map(String::as_str),
                    3,
                );
                db.variant_closest_matches
                    .insert((decl, key.clone()), close.clone());
                close
            };
            if close.is_empty() {
                return reject; // an empty sum — nothing to list, keep the bare message
            }
            let quoted: Vec<String> = close.iter().map(|n| format!("`{n}`")).collect();
            let message = format!(
                "{} — closest matches: {}",
                reject.message,
                quoted.join(", ")
            );
            Reject { message, ..reject }
        }
    }
}

pub(super) fn pattern_constraints(
    db: &mut Db,
    pat: StructId,
    ty: &crate::ty::Ty,
    path: Vec<crate::core::PathStep>,
    lit_tests: &mut Vec<PathLitTest>,
) -> Result<Vec<PathConstraint>, Reject> {
    // A GUARDED pattern `(guard <inner-pattern> <cond>)` contributes the INNER pattern's discriminant
    // constraints (the guard itself is not a discriminant test — it is carried on the `MatchRow` by
    // `lower_match_sum` and gated at the leaf in `build_tree`). Descend into the inner pattern so a
    // `(guard (Some x) …)` still constrains `[]` to the `Some` disc + binds `x` at `[Payload]`.
    if let Some(g) = db.ast.as_form(pat, "guard") {
        if g.len() != 2 {
            // A surplus element gets the shared delete fix; too few is message-only. Anchored at `pat`.
            return Err(crate::resolve::fixed_arity_reject(
                pat,
                g,
                2,
                "a guarded pattern must be (guard <pattern> <cond>)",
            ));
        }
        return pattern_constraints(db, g[0], ty, path, lit_tests);
    }
    // A LITERAL payload sub-pattern — an integer or boolean atom, NOT a name. `(Some 0)` matches `Some`
    // carrying exactly `0`: the literal refines the match (`core-semantics.md §Pattern Matching`, "nested
    // patterns can combine constructors and literals"). It imposes NO discriminant constraint (a scalar
    // has no variant tag); it adds a LITERAL TEST `(path, probe)` — the sub-value at `path` must EQUAL
    // the literal — gated (like a guard) once the enclosing discriminant is satisfied, with a same-variant
    // fall-through for the non-matching case. The literal's TYPE must AGREE with the sub-value's type at
    // this position: `(tuple true b)` against `(tuple 1 2)` puts a `Bool` literal where the element is
    // `Int64` — a literal-pattern-type mismatch (CDZ0201, core-semantics.md §Equality Is Structural),
    // checked HERE (nested), exactly as the top-level `(match 5 (true 1))` case is, so a nested wrong-type
    // literal does not slip past as a runtime non-match. (`ty` is `Any` for an unsolved position — no
    // check, the not-yet-constrained treatment a projection of `Any` gets.)
    let probe = match crate::resolve::resolved_of(db, pat) {
        crate::resolved::Resolved::Int(v) => {
            // A bare integer literal defaults to `Int64`, but its value is arbitrary-precision
            // (`Probe::Int` carries an `IntValue`) so it matches by value against EITHER a fixed-width or
            // a `BigInt` sub-value. When the sub-value at this position is `BigInt` — e.g. an `Ast.Int`
            // payload in a quote pattern `` `(+ ,x 0) `` — the literal grounds to `BigInt` so its type
            // AGREES; otherwise it stays the default `Int64`. Without this a `0` literal-pattern against a
            // `BigInt` payload would spuriously reject (CDZ0201) even though the value test is exact.
            let lit_ty = if matches!(ty, crate::ty::Ty::BigInt) {
                crate::ty::Ty::BigInt
            } else {
                crate::ty::Ty::int()
            };
            Some((crate::core::Probe::Int(v), lit_ty))
        }
        crate::resolved::Resolved::Bool(b) => {
            Some((crate::core::Probe::Bool(b), crate::ty::Ty::Bool))
        }
        // A STRING-literal payload sub-pattern — `(Ast.Name "+")` matches an `Ast.Name` carrying exactly
        // "+". Like the Int/Bool literal, it imposes no discriminant, adds a `Probe::Str` lit-test gated
        // at the leaf, and folds against a constant `Core::ConstStr` (a runtime String payload declines
        // at `build_lit_test`, like the scalar string match). Enables the quote-pattern literal head
        // (`` `(+ …) `` → `(Ast.Name "+")`), matched by string equality.
        crate::resolved::Resolved::Str(s) => {
            Some((crate::core::Probe::Str(s), crate::ty::Ty::String))
        }
        // A SYMBOL-literal payload sub-pattern — `(Mk #"add")` matches a `Mk` carrying the symbol `#"add"`.
        // A symbol shares the constant-string rep (`SymbolConst` → `Core::ConstStr`), so it reuses the
        // SAME `Str` probe/fold/emit as a string literal; its expected sub-value type is `Symbol` (the
        // symbol twin of the String-literal payload — the nested face of the top-level symbol-match support).
        crate::resolved::Resolved::SymbolConst(s) => {
            Some((crate::core::Probe::Str(s), crate::ty::Ty::Symbol))
        }
        // A CHAR-literal payload sub-pattern — `(Tok.Ch #\a)` matches a `Tok.Ch` carrying `#\a`. Like the
        // Int/Bool literal it imposes no discriminant, adds a `Probe::Char` lit-test, and folds against a
        // constant `Core::ConstChar` (a runtime char payload declines at emit — a `Char` has no runtime
        // rep). Its expected sub-value type is `Char` (the char twin of the String/Symbol-literal payload).
        crate::resolved::Resolved::Char(c) => {
            Some((crate::core::Probe::Char(c), crate::ty::Ty::Char))
        }
        // A BYTE-STRING-literal payload sub-pattern — `(Some b"AB")` matches a `Some` carrying exactly the
        // bytes `AB`. The Bytes twin of the String-literal payload: it imposes no discriminant, adds a
        // `Probe::Bytes` lit-test gated at the leaf, folds against a constant `Core::ConstBytes`, and a
        // RUNTIME Bytes payload emits the `value-eq` byte-leaf content compare (`build_lit_test`'s
        // non-refining arm, exactly as a runtime String payload). Its expected sub-value type is `Bytes`.
        crate::resolved::Resolved::Bytes(bs) => {
            Some((crate::core::Probe::Bytes(bs.into()), crate::ty::Ty::Bytes))
        }
        _ => None,
    };
    if let Some((probe, lit_ty)) = probe {
        if !matches!(ty, crate::ty::Ty::Any) && !lit_ty.agrees_with(ty) {
            return Err(Reject::coded(
                Code::Malformed,
                format!(
                    "{} literal pattern does not match the {} sub-value it is matched against",
                    lit_ty.render_with_article(&db.name_ctx()),
                    ty.render_name(&db.name_ctx())
                ),
            )
            .at(pat));
        }
        lit_tests.push((path.into(), probe));
        return Ok(Vec::new());
    }
    // A bare NAME: either a NULLARY VARIANT of this sum (`None`) or a binder/wildcard. Resolve it against
    // the sum's variant set — a name that IS a variant contributes that discriminant (no payload to
    // recurse into); any other bare name binds and contributes nothing.
    if let Some(name) = db.ast.as_name(pat) {
        let name = name.to_string();
        if name != "_"
            && let Some(disc) = variant_disc_by_name(db, ty, &name)
        {
            return Ok(vec![(path.into(), disc)]);
        }
        // A bare name that is NOT a variant of the scrutinee's sum but is a PLAUSIBLE TYPO of one — `Rd`
        // over a `(type Color Red Green)` scrutinee — is almost certainly a misspelled nullary-variant
        // pattern, NOT a catch-all binder the author intended. Treating it as a binder silently turns the
        // arm into a wildcard, masking the real variants (a later `Green` arm reads "unreachable") and
        // drawing a misleading CDZ0306 "unused binding `Rd`" — the exact confusion the DOTTED form
        // (`Color.Rd`) avoids with "the type `Color` has no variant `Rd` — did you mean `Red`?". Give the
        // bare form the SAME enrichment: reject CDZ0201 with a did-you-mean + a replace fix on the name,
        // when `name` is a confident near-miss (`suggest::nearest`) of a variant of the scrutinee sum.
        // GATED to a real near-miss so a genuine binder (`x`, or any name unlike every variant) still
        // binds — only a name close to an existing variant is judged a typo, never an arbitrary lowercase
        // binder. (The value-position twin of `enrich_pattern_head_suggestion`; here the pattern is a bare
        // atom, which never reaches that compound-head path.)
        if name != "_"
            && let Some(candidate) = nearest_variant_typo(db, ty, &name)
        {
            return Err(Reject::coded(
                Code::Malformed,
                format!(
                    "this match arm names `{name}`, which is not a variant of the matched type \
                     {} — did you mean `{candidate}`? (a bare name here is read as a catch-all \
                     binding, which is almost certainly not intended)",
                    ty.render_name(&db.name_ctx())
                ),
            )
            .at(pat)
            .with_fix(Fix::replace_heuristic(pat, candidate)));
        }
        return Ok(Vec::new()); // a binder / wildcard — no constraint
    }
    // A TUPLE pattern `(tuple p0 p1…)` at `path` — a variant's tuple PAYLOAD, destructured positionally
    // (core-semantics.md §Patterns Compose: a tagged value carrying a tuple is one nested pattern). A
    // tuple has no discriminant, so it imposes NO constraint of its own; each element sub-pattern
    // descends at `path + [Elem(i)]`, of the tuple element's type. (Reached only inside a variant
    // payload — the top-level scrutinee is a sum, so `pattern_constraints` is entered on a variant.)
    if is_tuple_pattern(db, pat) {
        let elems: Vec<StructId> = db
            .ast
            .compound_form_of(pat, CompoundCtor::Tuple)
            .unwrap_or(&[])
            .to_vec();
        // A trailing `.. rest` in a tuple pattern — `(tuple a b .. rest)` — binds the TRAILING SUB-TUPLE
        // to `rest` (a `TupleRestFrom(lead)` read in resolve). UNLIKE a list rest (variable length), a
        // tuple's arity is FIXED and statically known, so a rest pattern of `lead` LEADING elements matches
        // a tuple of arity `>= lead` — the leading positions bind at `Elem(i)`, and `rest` gathers the
        // remaining `arity - lead` elements as a new tuple (its own type read in infer; no constraint of
        // its own, exactly as the tuple pattern itself imposes none). A non-rest pattern matches EXACTLY its
        // arity, as before. `..` was previously REJECTED for a tuple; it is now this trailing-gather bind.
        let (leads, has_rest): (&[StructId], bool) = match db.ast.rest_marker(&elems) {
            Some((k, _operand, trailing_start)) if trailing_start == elems.len() => {
                (&elems[..k], true)
            }
            _ => (&elems[..], false),
        };
        let arity_ok = |n: usize| {
            if has_rest {
                n >= leads.len()
            } else {
                n == leads.len()
            }
        };
        // The payload MUST be a tuple, and its arity must satisfy the pattern — a tuple pattern against a
        // non-tuple payload, or naming the wrong number of elements (`(tuple a b c)` against a 2-tuple; a
        // rest pattern `(tuple a b .. r)` against a 1-tuple), is an ill-typed destructure REJECTED (CDZ0201).
        let elem_tys: &[crate::ty::Ty] = match ty {
            crate::ty::Ty::Tuple(ts) if arity_ok(ts.len()) => ts,
            // `Any` payload (an unsolved/unknown type) can't be arity-checked here — descend the LEADING
            // elements permissively (each `Any`), the same not-yet-constrained treatment a projection of an
            // `Any` gets. The rest binder (if any) adds no constraint.
            crate::ty::Ty::Any => {
                let mut out = Vec::new();
                for (i, &elem) in leads.iter().enumerate() {
                    let mut deeper = path.clone();
                    deeper.push(crate::core::PathStep::Elem(i));
                    out.extend(pattern_constraints(
                        db,
                        elem,
                        &crate::ty::Ty::Any,
                        deeper,
                        lit_tests,
                    )?);
                }
                return Ok(out);
            }
            other => {
                // Anchor at the offending PATTERN node (`pat`), not the enclosing match — the squiggle
                // then points at `(tuple a b c)`, the actual wrong construct, rather than the whole
                // `(match … )`. DISTINGUISH the two shapes this arm catches:
                //  • the value IS a tuple but of an incompatible arity — name both counts (a rest pattern
                //    reads "at least N"); OR
                //  • the value is NOT a tuple at all — a `(tuple …)` pattern cannot destructure it.
                let n = leads.len();
                let plural = |k: usize| if k == 1 { "" } else { "s" };
                let least = if has_rest { "at least " } else { "" };
                let message = if let crate::ty::Ty::Tuple(ts) = other {
                    format!(
                        "this tuple pattern binds {least}{n} element{}, but the value is a tuple with {} \
                         element{} ({}) — a tuple pattern must bind {}as many elements as the tuple has",
                        plural(n),
                        ts.len(),
                        plural(ts.len()),
                        other.render_name(&db.name_ctx()),
                        if has_rest { "at most " } else { "exactly " },
                    )
                } else {
                    format!(
                        "this tuple pattern cannot destructure a value of type {} — a `(tuple …)` pattern \
                         matches only a tuple value",
                        other.render_name(&db.name_ctx())
                    )
                };
                return Err(Reject::coded(Code::Malformed, message).at(pat));
            }
        };
        let mut out = Vec::new();
        for (i, &elem) in leads.iter().enumerate() {
            let mut deeper = path.clone();
            deeper.push(crate::core::PathStep::Elem(i));
            out.extend(pattern_constraints(
                db,
                elem,
                &elem_tys[i],
                deeper,
                lit_tests,
            )?);
        }
        return Ok(out);
    }
    // A LIST pattern `(list p0 p1…)` at `path` — a variant's LIST payload, destructured element-by-element
    // (`metaprogramming.md` quote patterns desugar `` `(+ ,a ,b) `` to `(Ast.List (list (Ast.Name "+") a
    // b))`, whose `(list …)` payload sub-pattern this handles; also a user `(W.Wrap (list a b))`). A list
    // has a RUNTIME length, so the pattern imposes a `ListLen` test (like a literal test — gated once the
    // discriminant constraints hold, folded against a constant list); each LEADING element sub-pattern
    // descends at `path + [Elem(i)]`, of the list's element type. A trailing `.. rest` makes the length
    // test AT-LEAST-`lead` and binds the tail — the rest binder resolves independently via `RestFrom(lead)`
    // (`resolve::find_binder_in_list`), so it needs no constraint here. SCOPE: the CONSTANT-scrutinee fold
    // only — a runtime list payload's `ListLen`/element reads decline (`build_lit_test`).
    if is_list_pattern(db, pat) {
        let raw: Vec<StructId> = db
            .ast
            .compound_form_of(pat, CompoundCtor::List)
            .unwrap_or(&[])
            .to_vec();
        // Split off a trailing `.. rest`: a `..` MARKER followed by exactly one binder as the final two
        // elements. `lead` = the fixed leading element patterns; `has_rest` = a tail-binding rest pattern.
        let (leads, has_rest): (&[StructId], bool) = match db.ast.rest_marker(&raw) {
            Some((k, _, trailing_start)) if trailing_start == raw.len() => (&raw[..k], true), // `(list p… .. rest)` — well-formed
            Some(_) => {
                // A `..` that is not the second-to-last element is malformed (a rest binds the whole tail,
                // so it must be final). CDZ0201 — the same shape rule a top-level list pattern enforces.
                // Anchored at the offending list PATTERN, not the enclosing match.
                return Err(Reject::coded(
                    Code::Malformed,
                    "a list rest pattern `.. rest` must be the final element",
                )
                .at(pat));
            }
            None => (&raw[..], false),
        };
        let elem_ty = match ty {
            crate::ty::Ty::List(e) => (**e).clone(),
            crate::ty::Ty::Any => crate::ty::Ty::Any,
            other => {
                // NAME the value's type + say a list pattern needs a LIST (not "does not match the payload
                // type T" — "payload" is an internal term, misleading for a top-level `let`/`match` on a
                // plain value; the list twin of the tuple/constructor shape messages).
                return Err(Reject::coded(
                    Code::Malformed,
                    format!(
                        "this list pattern cannot destructure a value of type {} — a `(list …)` pattern \
                         matches only a list value",
                        other.render_name(&db.name_ctx())
                    ),
                )
                .at(pat));
            }
        };
        // The LENGTH test — exactly `leads.len()` for a fixed pattern, AT LEAST `leads.len()` when a
        // `.. rest` binds the tail. Gated like a literal test (folded against a constant `Core::ListNew`);
        // a mismatch falls through.
        lit_tests.push((
            path.clone().into(),
            crate::core::Probe::ListLen {
                len: leads.len(),
                at_least: has_rest,
            },
        ));
        let mut out = Vec::new();
        for (i, &elem) in leads.iter().enumerate() {
            let mut deeper = path.clone();
            deeper.push(crate::core::PathStep::Elem(i));
            out.extend(pattern_constraints(db, elem, &elem_ty, deeper, lit_tests)?);
        }
        return Ok(out);
    }
    // A MAP pattern `(map (k v) … .. rest)` at `path` — a variant's / tuple's MAP sub-value, destructured
    // by key. A map has no discriminant; it imposes a KEY-PRESENCE test (`MapHasKeys` — every named key
    // must be present, gated + folded against a constant `Core::MapNew` like `ListLen`). Each VALUE binder
    // is read independently via `MapField` (resolve Case M/Mn) — the value's key-directed access has no
    // `PathStep`, so a value sub-pattern is a BARE binder / `_` (no descent here); a nested value
    // sub-pattern (`(map (1 (Some x)))`) is a later increment (declines cleanly). The REST binder likewise
    // reads via `MapField`. SCOPE: the CONSTANT-scrutinee fold only (a runtime map declines at
    // `build_lit_test`), the same limit the direct map matcher (`lower_match_map`) has.
    if is_map_pattern(db, pat) {
        let (entries, _rest) = match crate::resolve::map_pattern_of(db, pat) {
            Some(mp) => mp,
            None => {
                // A NESTED map pattern (inside a variant/tuple payload) with a MALFORMED `..` rest — give
                // the SAME specific rest-shape message the TOP-LEVEL map matcher does (Inc 42), not the
                // vague "a malformed map pattern". The nested twin of that fix; `map_form_is_malformed_rest`
                // distinguishes a bad rest from any other malformed map shape.
                if crate::resolve::map_form_is_malformed_rest(db, pat) {
                    return Err(Reject::coded(
                        Code::Malformed,
                        "a map rest pattern is `(map (k v) … .. rest)` — exactly one binder after `..`",
                    )
                    .at(pat));
                }
                return Err(Reject::coded(Code::Malformed, "a malformed map pattern").at(pat));
            }
        };
        if !matches!(ty, crate::ty::Ty::Map(_, _) | crate::ty::Ty::Any) {
            return Err(Reject::coded(
                Code::Malformed,
                format!(
                    "a map pattern does not match the sub-value type {}",
                    ty.render_name(&db.name_ctx())
                ),
            )
            .at(pat));
        }
        // Every value binder must be a BARE name / `_` (its value reads via `MapField`, no path step). A
        // NESTED value sub-pattern needs a map-value access step — decline (a later increment), never a
        // miscompile.
        for &(_, v) in &entries {
            if db.ast.as_name(v).is_none() {
                return Err(Reject::unsupported(
                    "a nested (non-binder) map-pattern value sub-pattern is not supported",
                ));
            }
        }
        // The key-presence test at `path`: all named keys must be in the (constant) map. Value binders +
        // any rest binder read via `MapField`, so they contribute no path constraint here.
        let keys: Vec<StructId> = entries.iter().map(|&(k, _)| k).collect();
        lit_tests.push((
            path.into(),
            crate::core::Probe::MapHasKeys { keys: keys.into() },
        ));
        return Ok(Vec::new());
    }
    // A `(record (field p) …)` pattern — destructuring a record BY FIELD — is not yet matched (the record
    // MATCH twin of the record BINDING pattern (Increment B, `check_binding_pattern` above). A record is a
    // fixed-shape product like a tuple, projected by NAME: a field at the record type's SORTED slot `i` is
    // read by `Elem(i)` — the SAME array-cell access a tuple element uses (a record value is a sorted-field
    // array; `runtime_member_index`/the `Member`→`Proj` fold read that slot). A record has NO discriminant,
    // so a record pattern imposes NO constraint of its own (like a tuple) — each named field's sub-pattern
    // descends at `path + [Elem(sorted_slot)]` with the field's type, recursing exactly as the tuple arm
    // does. A field the record type lacks is a CDZ0201 shape error; a non-record scrutinee is CDZ0201.
    //= spec/capabilities/core-semantics.md#a-record-has-a-fixed-set-of-named-fields
    //# A record MUST be deconstructible by pattern matching on its field names, binding each named field's sub-value.
    //= spec/capabilities/core-semantics.md#a-record-has-a-fixed-set-of-named-fields
    //# A record pattern MAY name a subset of the fields, ignoring the rest.
    if let Some(all_fields) = db
        .ast
        .compound_form_of(pat, CompoundCtor::Record)
        .map(<[_]>::to_vec)
    {
        // Split off a trailing `.. rest`: the rest binder binds the RESIDUAL RECORD (the fields NOT named,
        // resolved to a `Resolved::RecordRest`), imposing NO constraint of its own — a record pattern names
        // a SUBSET of fields and ignores the rest, so `rest` is exactly that ignored remainder made
        // bindable. Only the leading `(= field p)` fields constrain (their sub-patterns descend); the `..`
        // marker itself is NOT a field (without this split it was mis-read as a field name → CDZ0201).
        let fields: Vec<StructId> = match db.ast.rest_marker(&all_fields) {
            Some((k, _operand, trailing_start)) if trailing_start == all_fields.len() => {
                all_fields[..k].to_vec()
            }
            _ => all_fields,
        };
        // The scrutinee's record field types by name→sorted-slot. When the type is a solved record, each
        // named field must exist (CDZ0201) and descends at its sorted index; an `Any`/unsolved type descends
        // permissively (slot by written order — harmless, refutability is a pattern-shape property). A
        // non-record, non-`Any` scrutinee cannot be destructured by a record pattern (CDZ0201).
        let field_slots: Option<
            std::rc::Rc<std::collections::BTreeMap<crate::resolved::Symbol, crate::ty::Ty>>,
        > = match ty.strip_nominal() {
            crate::ty::Ty::Record(fs) => Some(fs.clone()),
            crate::ty::Ty::Any => None,
            other => {
                return Err(Reject::coded(
                    Code::Malformed,
                    format!(
                        "this record pattern cannot destructure a value of type {} — a `(record …)` \
                         pattern matches only a record value",
                        other.render_name(&db.name_ctx())
                    ),
                )
                .at(pat));
            }
        };
        let mut out = Vec::new();
        for (written_ix, &pair) in fields.iter().enumerate() {
            let crate::ast::Struct::List(kv) = db.ast.get(pair) else {
                return Err(Reject::coded(
                    Code::Malformed,
                    "a record pattern field must be a `(= field <pattern>)` triple",
                )
                .at(pair));
            };
            // A record-pattern field is the canonical `(= field <pattern>)` triple (path B — same form
            // as a value-record field): field = child 1, sub-pattern = child 2, `=` head dropped. A
            // legacy `(field <pattern>)` pair is tolerated (field = child 0).
            let (key_occ, value_pat) = if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") {
                (kv[1], kv[2])
            } else if kv.len() == 2 {
                (kv[0], kv[1])
            } else {
                return Err(Reject::coded(
                    Code::Malformed,
                    "a record pattern field must be a `(= field <pattern>)` triple",
                )
                .at(pair));
            };
            // The field's sorted slot + type. A solved record resolves the slot by name (CDZ0201 if the
            // field is absent); an `Any` scrutinee uses the written order and types the field `Any`.
            let (slot, field_ty) = match &field_slots {
                Some(fs) => {
                    let key = crate::resolve::read_key(db, key_occ);
                    match key
                        .as_ref()
                        .and_then(|k| fs.keys().position(|fk| fk == k).map(|i| (i, fs[k].clone())))
                    {
                        Some(hit) => hit,
                        None => {
                            // A near-miss field name is a TYPO — suggest the nearest actual field AND carry
                            // an APPLYABLE replace fix, the record-PATTERN twin of the member-access `(. r
                            // fooo)` did-you-mean (CDZ0212) and the variant-pattern typo fix above. Same
                            // typo class; without this the pattern path dead-ended at "does not have".
                            // `suggest::nearest` returns a confident single candidate (its cutoff) or None (a
                            // far miss keeps the plain message + no fix). The fix REPLACES the field-name node
                            // `key_occ` with the candidate, so applying it rewrites `(record (helpr y))` →
                            // `(record (helper y))` — heuristic (the nearest name is a guess at intent).
                            let field_name = key.map(|k| k.name).unwrap_or_default();
                            let candidate = crate::diag::suggest::nearest(
                                &field_name,
                                fs.keys().map(|k| &*k.name),
                            );
                            let suggestion = candidate
                                .as_ref()
                                .map(|near| format!(" — did you mean `{near}`?"))
                                .unwrap_or_default();
                            let reject = Reject::coded(
                                Code::Malformed,
                                format!(
                                    "a record pattern names field `{field_name}`, which the matched value \
                                     of type {} does not have{suggestion}",
                                    ty.render_name(&db.name_ctx())
                                ),
                            )
                            .at(pair);
                            return Err(match candidate {
                                Some(near) => {
                                    reject.with_fix(Fix::replace_heuristic(key_occ, near))
                                }
                                None => reject,
                            });
                        }
                    }
                }
                None => (written_ix, crate::ty::Ty::Any),
            };
            let mut deeper = path.clone();
            deeper.push(crate::core::PathStep::Elem(slot));
            out.extend(pattern_constraints(
                db, value_pat, &field_ty, deeper, lit_tests,
            )?);
        }
        return Ok(out);
    }
    // A compound pattern. Its head is the variant CONSTRUCTOR — a member `(. Sum V)` or a bare variant
    // name — and the remaining children are payload sub-patterns.
    let (head, args): (StructId, Vec<StructId>) = match db.ast.get(pat) {
        crate::ast::Struct::List(children) => match children.first().copied() {
            // A bare member `(. Sum V)` used as a whole pattern — the ctor, no payload args.
            Some(first) if db.ast.as_name(first) == Some(".") => (pat, Vec::new()),
            Some(first) => (first, children[1..].to_vec()),
            None => return Err(Reject::decline("an empty sum match pattern")),
        },
        crate::ast::Struct::Atom(_) => {
            return Err(Reject::decline("a malformed sum match pattern"));
        }
    };
    // A BARE variant-name head that COLLIDES with a prelude entry (`(Int n)` on `(type T (Int Int64))`,
    // `(Some n)` on a user `(type T (Some …))`) resolves — via scope→def→PRELUDE — to the prelude `Int`
    // type constructor / Option `Some`, NOT this sum's variant, so the ctor check below would reject a
    // well-formed pattern (CDZ0203). The SCRUTINEE's type is known here, so its variant set disambiguates:
    // if the bare head names a variant of THIS sum/nominal, resolve it to that variant's CACHED ctor
    // occurrence (the same node the qualified `T.Int` form uses, which already carries the right `(meta t)`
    // scheme + `(meta variant)` disc) and use THAT as the head. This gives the bare form the same
    // local-variant precedence the qualified form has — the residual of the variant-shadows-prelude fix
    // (`9f326a2d` repaired TYPE/MODULE positions; this repairs the CONSTRUCT/PATTERN head). A NON-colliding
    // bare name already resolves to its own variant, so `variant_disc_by_name` finding it and re-reading
    // the SAME cached ctor is a harmless no-op; a bare name that is NOT a variant (a typo) is left for the
    // existing ctor check to reject.
    let head = 'remap: {
        let Some(name) = db.ast.as_name(head).map(str::to_string) else {
            break 'remap head;
        };
        if name == "." {
            break 'remap head;
        }
        // The scrutinee's declaration — a boxed `Ty::Sum` OR a single-variant `Ty::Nominal` newtype (a
        // `(type T (Int Int64))` erases to a nominal, whose sole variant is still reached by name).
        let decl = match ty {
            crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => *decl,
            _ => break 'remap head,
        };
        // The cached ctor of the variant of THIS declaration named `name` (if any). Resolving to it gives
        // the bare form the local-variant precedence the qualified `T.<name>` already has.
        match db
            .type_decl_by_occ(decl)
            .and_then(|t| t.variants.iter().find(|v| v.name == name))
            .and_then(|v| v.ctor)
        {
            Some(ctor) => ctor,
            None => head,
        }
    };
    // WITHHELD-CONSTRUCTOR via the BARE pattern head: a bare `((A v))` pattern head names variant `A` of the
    // scrutinee's type, but `A`'s constructor was WITHHELD from export to this file (an abstract import). The
    // remap above just gave the bare head local-variant precedence (so `variant_owner_decl == scrut_decl`
    // below and the wrong-ctor check passes) — but visibility was never gated on the bare path. The QUALIFIED
    // `((T.A v))` head is a `(. T A)` member whose fold poisons CDZ0214 (`withheld_ctor_reject`), propagated
    // below; the bare head resolves inert, so WITHOUT this it reads the private ADT payload — a one-token
    // bypass of the smart-ctor discipline (encapsulation SOUNDNESS; the verification kernel's Thm/Term trust;
    // breaker/corpus-bugfix 2026-07-29, direct + eval-quasiquote faces, both reach this shared lowering).
    // Gate the bare head with the SAME visibility check as the qualified selector: a bare name that IS a
    // variant of the scrutinee decl AND is withheld here → CDZ0214. (A visible ctor / a non-variant name is
    // untouched — the ctor / wrong-ctor checks below handle those.)
    // The bare pattern-head NAME (the `A` in `(A v)`) — `None` for a qualified `(. T A)` head (already gated
    // by the member fold's withheld poison) or a non-list pattern.
    let bare_head_name = match db.ast.get(pat) {
        crate::ast::Struct::List(children) => children.first().and_then(|&h| db.ast.as_name(h)),
        _ => None,
    };
    if let crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } = ty
        && let Some(name) = bare_head_name
        && name != "."
        && db
            .type_decl_by_occ(*decl)
            .is_some_and(|t| t.variants.iter().any(|v| v.name == name))
        // Gate on the SCRUTINEE TYPE being genuinely ABSTRACT here (handle imported, ctors withheld) — the
        // SAME `is_abstract_type_at` the qualified selector's `withheld_ctor_reject` uses. Without this, a bare
        // `((Some v))`/`((None _))` over a PRELUDE `Option` scrutinee mis-fires: `Some`/`None` are prelude
        // ctors NOT in the file-scope `visible_ctors` map (prelude resolves separately), so `ctor_is_withheld_at`
        // alone wrongly reports them withheld → a false CDZ0214 on every cross-module `Map.lookup`/`Option`
        // match (the 7 module-boundary gate regressions this fix first produced). A prelude/own/concrete type
        // is NOT abstract, so this excludes it; only a genuinely handle-only import reaches the withheld check.
        && db.is_abstract_type_at(pat, *decl)
        && db.ctor_is_withheld_at(pat, name)
    {
        let ty_name = ty.render_name(&db.name_ctx());
        return Err(Reject::coded(
            Code::AbstractCtor,
            format!(
                "`{ty_name}`'s constructor `{name}` is not exported to this file: the handle is visible \
                 but `{name}` is withheld, so a value cannot be matched (or constructed) through `{name}` \
                 here — inspect it through the functions the module that declares the type exports (or \
                 export the type's constructors to make them public)"
            ),
        )
        .at(pat));
    }
    // A NOMINAL NEWTYPE scrutinee — the sole constructor `(Mk arg…)` imposes NO discriminant constraint
    // (a newtype has no runtime disc; its one variant always matches), but its payload binders DO
    // destructure. The ctor must belong to THIS newtype's declaration (a `(Other x)` pattern over a
    // `UserId` scrutinee is a type error, CDZ0203 — same as the boxed-sum check below). The payload
    // descends at `path + [Payload]`, which `erase_nominal_steps` later drops as a no-op; the payload
    // type is the nominal's `inner` (single payload) or its tuple elements (multi-payload struct).
    if let crate::ty::Ty::Nominal {
        decl: scrut_decl,
        inner,
        ..
    } = ty
    {
        if crate::eval::variant_owner_decl(db, head) != Some(*scrut_decl) {
            // A WITHHELD constructor (`C.A` where `C`'s handle is imported but its ctor `A` is not exported
            // to this file) has its member-fold poison carry the precise CDZ0214 "constructor is withheld"
            // code — the SAME diagnostic a MULTI-variant match (the `variant_disc_of`-miss path below) and a
            // CONSTRUCTION site emit. A single-variant sum NEWTYPE-ERASES to `Ty::Nominal`, so its withheld
            // match reaches HERE (not the `Ty::Sum` path), where the bare `variant_owner_decl != scrut_decl`
            // check reported the generic CDZ0203 "not a variant of the matched type" instead of the
            // actionable withheld-ctor CDZ0214 (v-verification: exactly the HOL-kernel `Thm`/`Term` newtype
            // shape). Propagate the head's coded poison FIRST (the newtype twin of the Sum branch's
            // `core_of(head)` propagation), so a withheld single-variant ctor match names the real cause;
            // fall to the generic CDZ0203 only when the head has no coded poison (a genuine other-type ctor).
            if let Core::Poison(reject) = core_of(db, head)
                && reject.code.is_some()
            {
                return Err(enrich_pattern_head_suggestion(db, head, ty, reject));
            }
            // The pattern names a variant of a DIFFERENT type than the newtype scrutinee. Enrich with the
            // same "did you mean?" / closest-variants suggestion the boxed-sum path gets (the scrutinee's
            // own — single — variant is the candidate the author likely reached for), carrying a replace
            // fix on the pattern head. The newtype twin of the `Ty::Sum` wrong-ctor enrichment below.
            return Err(enrich_pattern_head_suggestion(
                db,
                head,
                ty,
                Reject::coded(
                    Code::TypeMismatch,
                    format!(
                        "this variant pattern is not a variant of the matched type {}",
                        ty.render_name(&db.name_ctx())
                    ),
                )
                .at(pat),
            ));
        }
        let inner = (**inner).clone();
        return match args.len() {
            // A bare `(Mk)` / member `(. T Mk)` with no payload arg — nothing to bind (a unit newtype).
            0 => Ok(Vec::new()),
            // `(Mk n)` — bind the single payload at `[Payload]` (erased later), typed as `inner`.
            //
            // ARITY: a MULTI-FIELD variant `(Mk Int64 Int64)` is MULTI-arity (two DECLARED fields, boxed as
            // a payload tuple), so a ONE-binder pattern `(Mk a)` — a lone BARE NAME — under-binds: it would
            // silently bind `a` to the whole field-tuple, the surprising slip a too-MANY `(Mk a b c)` is
            // already rejected for. Reject it symmetrically (concierge/operator ruling 8922: a multi-field
            // variant is multi-arity; "single-arity" in core-semantics §195/207 is the internal curried-
            // application ABI, not the surface field arity).
            //
            // TWO shapes are NOT under-binding and pass through:
            //  (1) the single arg is an explicit `(tuple …)` PATTERN — `(Mk (tuple a _))` — the CANONICAL
            //      single-arity payload-tuple destructure (§207); it recurses below and `pattern_constraints`'
            //      tuple arm arity-checks it against the payload tuple `inner`. This is the form the emit /
            //      recursive-match tests use.
            //  (2) a genuinely SINGLE-FIELD variant whose one payload is a compound VALUE — `(Pt (Tuple T
            //      T))`, `(Pt (Record …))`, `(Mk (-> …))` — has DECLARED arity 1 (`variant_payload_arity`),
            //      so `(Pt a)` correctly binds that whole payload (the corpus-blessed `(Pt r)` form).
            // So reject ONLY a >1-DECLARED-FIELD variant matched with a single NON-tuple sub-pattern — the
            // too-FEW twin of the `_ =>` too-MANY check.
            1 => {
                let arg_is_tuple_pattern = is_tuple_pattern(db, args[0]);
                if !arg_is_tuple_pattern
                    && crate::eval::variant_payload_arity(db, head).is_some_and(|n| n > 1)
                    && let crate::ty::Ty::Tuple(ts) = &inner
                {
                    let ctor = ctor_pattern_name(db, pat);
                    let plural = |k: usize| if k == 1 { "" } else { "s" };
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this pattern binds 1 element for `{ctor}`, but `{ctor}` carries {} \
                             field{} — a constructor pattern must bind exactly as many as the \
                             constructor has",
                            ts.len(),
                            plural(ts.len()),
                        ),
                    )
                    .at(pat));
                }
                let mut deeper = path;
                deeper.push(crate::core::PathStep::Payload);
                pattern_constraints(db, args[0], &inner, deeper, lit_tests)
            }
            // `(Mk a b …)` over a multi-payload struct — the payload is `inner` = `Ty::Tuple`; each arg
            // destructures an element at `[Payload, Elem(i)]` (the `Payload` erases, the `Elem` reads the
            // tuple handle). Arity is checked against the tuple below via the shared descent.
            _ => {
                let elem_tys: Vec<crate::ty::Ty> = match &inner {
                    crate::ty::Ty::Tuple(ts) if ts.len() == args.len() => ts.to_vec(),
                    crate::ty::Ty::Tuple(ts) => {
                        // NAME the constructor + count ELEMENTS (not "payload(s)"/"newtype" — internal
                        // terms leaking to the author), the constructor twin of the tuple-pattern arity
                        // message: `(Mk a b c)` against a `Mk` carrying 2.
                        let ctor = ctor_pattern_name(db, pat);
                        let plural = |k: usize| if k == 1 { "" } else { "s" };
                        return Err(Reject::coded(
                            Code::Malformed,
                            format!(
                                "this pattern binds {} element{} for `{ctor}`, but `{ctor}` carries {} \
                                 field{} — a constructor pattern must bind exactly as many as the \
                                 constructor has",
                                args.len(),
                                plural(args.len()),
                                ts.len(),
                                plural(ts.len()),
                            ),
                        )
                        .at(pat));
                    }
                    _ => {
                        let ctor = ctor_pattern_name(db, pat);
                        return Err(Reject::coded(
                            Code::Malformed,
                            format!(
                                "this pattern binds {} fields for `{ctor}`, but `{ctor}` carries a single \
                                 value of type {} — bind it with one sub-pattern `({ctor} x)`",
                                args.len(),
                                inner.render_name(&db.name_ctx())
                            ),
                        )
                        .at(pat));
                    }
                };
                let mut payload_path = path;
                payload_path.push(crate::core::PathStep::Payload);
                let mut out = Vec::new();
                for (i, (&arg, elem_ty)) in args.iter().zip(elem_tys.iter()).enumerate() {
                    let mut deeper = payload_path.clone();
                    deeper.push(crate::core::PathStep::Elem(i));
                    out.extend(pattern_constraints(db, arg, elem_ty, deeper, lit_tests)?);
                }
                Ok(out)
            }
        };
    }
    let Some(disc) = crate::eval::variant_disc_of(db, head) else {
        // The head names no variant. A `(. Sum Q)` head where `Q` is not a variant of the sum
        // (`((V.Q) …)` on a `(type V (A …) (B))`) lowers as a MEMBER ACCESS that already carries the
        // precise coded fault — `CDZ0201: record has no field \`Q\`` (a sum record's variants ARE its
        // fields), the SAME code the value position `(V.Q)` gets. Propagate that coded poison rather than
        // the generic UNCODED "not a variant constructor" decline, so a mistyped variant in a match
        // pattern NAMES the offending variant and is graded a rejection (not a to-do).
        if let Core::Poison(reject) = core_of(db, head)
            && reject.code.is_some()
        {
            // ENRICH with a "did you mean?" over the SCRUTINEE sum's variant names — the pattern-position
            // twin of `infer::no_field_reject`'s value-position suggestion. `core_of(head)` (a member fold)
            // emits the BARE `record has no field \`Q\``; here we know the scrutinee's sum type, so we can
            // name the nearest variant (`((V.Alph) …)` on `(type V (Alpha) (Beta))` → "did you mean
            // `Alpha`?") + carry a replace fix on the mistyped key occurrence, exactly as the value site.
            return Err(enrich_pattern_head_suggestion(db, head, ty, reject));
        }
        return Err(Reject::decline(
            "a sum match pattern head is not a variant constructor",
        ));
    };
    // TYPE-CHECK the pattern's constructor against the SCRUTINEE's sum type: the variant must belong to
    // the sum being matched, not merely be SOME sum's variant with the right name. A `Some`/`U.A` pattern
    // over a `T` scrutinee resolves to a valid discriminant of Option/U, but that variant is not T's — a
    // type confusion that would bind the payload under the wrong type (a wrong VALUE, or an INVALID WASM
    // component when the payload widths differ). Sum identity is by DECLARATION OCCURRENCE (`ty.rs`
    // §Two sums are the SAME type iff their `decl` agree), so compare the pattern ctor's owning `decl`
    // against the scrutinee `ty`'s `decl` — a mismatch is CDZ0203, the same type error `(: 5 Bool)` gets.
    // (A bare nullary-variant name took the `variant_disc_by_name` path above, which is already scoped to
    // this sum's declaration, so only a COMPOUND ctor pattern reaches here needing the check.)
    if let crate::ty::Ty::Sum {
        decl: scrut_decl, ..
    } = ty
        && crate::eval::variant_owner_decl(db, head) != Some(*scrut_decl)
    {
        // The ctor is a VALID variant, but of a DIFFERENT sum (`Nn` from `B` matched against `A`). Enrich
        // with the same "did you mean?" over the SCRUTINEE sum's variants the typo path gets — the author
        // reached for one of the MATCHED type's variants — and carry a replace fix on the pattern head.
        // A far miss lists the matched type's variants (a CLOSED set — listing is signal), so the reader
        // learns what `A` actually offers instead of only that this ctor is not one of them.
        return Err(enrich_pattern_head_suggestion(
            db,
            head,
            ty,
            Reject::coded(
                Code::TypeMismatch,
                format!(
                    "this variant pattern is not a variant of the matched type {}",
                    ty.render_name(&db.name_ctx())
                ),
            ),
        ));
    }
    let mut out: Vec<PathConstraint> = vec![(path.clone().into(), disc)];
    // Recurse into the payload. A single-payload variant `(Some p)` descends into `p` at `path +
    // [Payload]`; the payload's TYPE is the variant's payload type at this instantiation, so a nested
    // variant name there resolves against the right sum. A NULLARY variant pattern `(None)`/bare `None`
    // has no payload arg — nothing to recurse.
    match args.len() {
        0 => {}
        1 => {
            let payload_ty = crate::infer::payload_ty_at_instantiation(db, head, ty)
                .unwrap_or(crate::ty::Ty::Any);
            let mut deeper = path;
            deeper.push(crate::core::PathStep::Payload);
            let sub = pattern_constraints(db, args[0], &payload_ty, deeper, lit_tests)?;
            out.extend(sub);
        }
        // A MULTI-PAYLOAD variant pattern `(Cons h t)` is sugar for the single-tuple-payload form `(Cons
        // (tuple h t))`: the payloads are boxed as ONE tuple handle (`lower_sum_new` / the `SumNew`
        // backend), so `payload_ty_at_instantiation` reports the payload as a `Ty::Tuple`, and each arg
        // destructures a tuple ELEMENT at `path + [Payload, Elem(i)]` — exactly the descent the explicit
        // `(tuple …)` payload pattern takes. So destructuring a tagged value carrying a tuple of sub-values
        // (the shape a tree-walking pass over a recursive sum takes) is ONE nested arm, not a bind-then-rematch.
        //= spec/capabilities/core-semantics.md#patterns-compose
        //# A destructuring of a tagged value carrying a tuple of sub-values in a single arm — the shape every tree-walking pass over a recursive sum takes — MUST therefore be expressible directly as one nested pattern rather than requiring a bind-then-rematch.
        _ => {
            let payload_ty = crate::infer::payload_ty_at_instantiation(db, head, ty)
                .unwrap_or(crate::ty::Ty::Any);
            // The pattern's payload ARITY must match the variant's declared payload count — `(Mk a b c)`
            // against a 2-payload `Mk` names a nonexistent third element (it would read past the payload
            // tuple and bind `c` under an `Any`/wrong type — a wrong value, or invalid wasm). REJECT it
            // (CDZ0201), the same arity check the explicit `(tuple …)` payload pattern enforces above. An
            // `Any` payload (unsolved) can't be arity-checked — descend permissively (each `Any`).
            let elem_tys: Vec<crate::ty::Ty> = match &payload_ty {
                crate::ty::Ty::Tuple(ts) if ts.len() == args.len() => ts.to_vec(),
                crate::ty::Ty::Tuple(ts) => {
                    // NAME the constructor + count ELEMENTS/fields (not "payload(s)" — the internal term),
                    // the boxed-sum twin of the newtype message above and the tuple-pattern message.
                    let ctor = ctor_pattern_name(db, pat);
                    let plural = |k: usize| if k == 1 { "" } else { "s" };
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this pattern binds {} element{} for `{ctor}`, but `{ctor}` carries {} \
                             field{} — a constructor pattern must bind exactly as many as the \
                             constructor has",
                            args.len(),
                            plural(args.len()),
                            ts.len(),
                            plural(ts.len()),
                        ),
                    )
                    .at(pat));
                }
                crate::ty::Ty::Any => vec![crate::ty::Ty::Any; args.len()],
                // A non-tuple payload type under a multi-arg pattern is an arity error too (a single-value
                // variant matched with several binders).
                _ => {
                    let ctor = ctor_pattern_name(db, pat);
                    return Err(Reject::coded(
                        Code::Malformed,
                        format!(
                            "this pattern binds {} fields for `{ctor}`, but `{ctor}` carries a single \
                             value of type {} — bind it with one sub-pattern `({ctor} x)`",
                            args.len(),
                            payload_ty.render_name(&db.name_ctx())
                        ),
                    )
                    .at(pat));
                }
            };
            let mut payload_path = path;
            payload_path.push(crate::core::PathStep::Payload);
            for (i, (&arg, elem_ty)) in args.iter().zip(elem_tys.iter()).enumerate() {
                let mut deeper = payload_path.clone();
                deeper.push(crate::core::PathStep::Elem(i));
                let sub = pattern_constraints(db, arg, elem_ty, deeper, lit_tests)?;
                out.extend(sub);
            }
        }
    }
    Ok(out)
}

/// Whether `id` is a tuple PATTERN `(tuple …)` — a `tuple` NAME head (the alias the reader keeps in a
/// pattern) or the `"tuple"` string-literal primitive. Mirrors `resolve::is_tuple_pattern` (kept local
/// so lower does not depend on resolve's private helpers).
pub(super) fn is_tuple_pattern(db: &Db, id: StructId) -> bool {
    db.ast.compound_form_of(id, CompoundCtor::Tuple).is_some()
}

/// Whether `id` is a list PATTERN `(list p0 p1…)` — a `list` NAME head (the shadowable alias the reader
/// keeps) or the `"list"` string-literal primitive. Routes a variant's list payload into element-by-
/// element descent (`pattern_constraints`'s list arm), the list analogue of [`is_tuple_pattern`].
pub(super) fn is_list_pattern(db: &Db, id: StructId) -> bool {
    db.ast.compound_form_of(id, CompoundCtor::List).is_some()
}

/// Whether a record field's value sub-pattern binds POSITIONALLY — reachable by `Elem` steps ALONE (the
/// §235 `sub_path` the binding-position `RecordField` wires, slice 2). True for a bare binder / `_`, or a
/// tuple / fixed-arity list (no `.. rest`) ALL of whose elements are themselves positional (recursively).
/// FALSE for a nested RECORD (a name-keyed slot, deferred), a VARIANT (a `Payload` step needing a head
/// `RecordField` does not yet carry), a list WITH a rest, or a literal — the caller then declines cleanly.
/// This mirrors `resolve::find_record_binder_in_pattern`'s binding-path acceptance: `find_binder_in_tuple`/
/// `find_binder_in_list` build all-`Elem` paths for exactly these shapes, and the producer rejects a
/// non-empty `sub_heads` (variant) / a nested record — so LOWER and RESOLVE stay in lockstep.
fn is_positional_field_value(db: &Db, pat: StructId) -> bool {
    if db.ast.as_name(pat).is_some() {
        return true; // bare binder / wildcard
    }
    if is_tuple_pattern(db, pat) {
        return db
            .ast
            .compound_form_of(pat, CompoundCtor::Tuple)
            .is_some_and(|elems| elems.iter().all(|&e| is_positional_field_value(db, e)));
    }
    if is_list_pattern(db, pat) {
        return db
            .ast
            .compound_form_of(pat, CompoundCtor::List)
            .is_some_and(|elems| {
                db.ast.rest_marker(elems).is_none()
                    && elems.iter().all(|&e| is_positional_field_value(db, e))
            });
    }
    // A nested RECORD field value is IRREFUTABLE (a record has a static field set) and now WIRES in a
    // binding position (§235 full nested descent — `RecordSubStep::Field`), so accept it if every field's
    // value is itself irrefutably wireable. (Its `.. rest` open-row is left to the record-rest path — a
    // bare-binder-fields record is the wireable field-value shape here.)
    if let Some(fields) = db.ast.compound_form_of(pat, CompoundCtor::Record) {
        return db.ast.rest_marker(fields).is_none()
            && fields.iter().all(|&fp| {
                // Each field `(= k sub)` — recurse into the sub-pattern `sub` (a legacy `(k sub)` too).
                match db.ast.get(fp) {
                    crate::ast::Struct::List(kv)
                        if kv.len() == 3 && db.ast.as_name(kv[0]) == Some("=") =>
                    {
                        is_positional_field_value(db, kv[2])
                    }
                    crate::ast::Struct::List(kv) if kv.len() == 2 => {
                        is_positional_field_value(db, kv[1])
                    }
                    _ => false,
                }
            });
    }
    false // variant / literal → refutable / not wireable in a binding position (deferred)
}

/// The DISPLAY name of the constructor a `(Ctor arg…)` pattern applies — read from the pattern's SOURCE
/// spelling (its first child), so it works whether the head was written bare (`(Mk a b)`) or qualified
/// (`(P.Mk a b)` → the member key `Mk`). The head occurrence itself may have been remapped to a
/// synthesized cached-ctor node (not a name atom), so this reads `pat`'s first child, not the resolved
/// head. `"this constructor"` when the spelling is unreadable — a safe fallback for a message subject.
pub(super) fn ctor_pattern_name(db: &Db, pat: StructId) -> String {
    let first = match db.ast.get(pat) {
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
        .unwrap_or("this constructor")
        .to_string()
}

/// Whether `id` is a map PATTERN `(map (k v) … .. rest)` — a `map` NAME head (the alias) or the `"map"`
/// string-literal primitive. Routes a NESTED map sub-pattern into `pattern_constraints`'s map arm (a
/// key-presence test + `MapField` value reads), the map analogue of [`is_tuple_pattern`]/[`is_list_pattern`].
pub(super) fn is_map_pattern(db: &Db, id: StructId) -> bool {
    db.ast.compound_form_of(id, CompoundCtor::Map).is_some()
}

/// The element occurrences of `id` when it is a tuple CONSTRUCTOR expression — the symbol-headed
/// `Resolved::Tuple { elems }` or the `tuple` NAME-alias application (`Prim::TupleNew`). `None` for a
/// non-tuple. Used by `type_at_path` to type a tuple-scrutinee's element from the constructor directly,
/// bypassing the aggregate `type_of` that reads a recursive-call element as `Any`.
pub(super) fn tuple_constructor_elems(db: &mut Db, id: StructId) -> Option<Vec<StructId>> {
    match resolved_of(db, id) {
        Resolved::Tuple { elems } => Some(elems.to_vec()),
        Resolved::Apply { head, args }
            if crate::eval::meta_apply_of(db, head) == Some(Prim::TupleNew) =>
        {
            Some(args.to_vec())
        }
        _ => None,
    }
}

/// The CDZ0210 non-exhaustive-sum-match rejection, enriched with the MISSING variants and a structural
/// "add the missing arms" fix (`spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A
/// Fix — the match analogue of rustc's `error[E0004]: … patterns not covered` + its "add arms"
/// suggestion). `decl` is the scrutinee sum's declaration occurrence; `tested` the discriminants the
/// arms already cover; `scrutinee` the match's scrutinee node (its parent IS the `(match …)` form the
/// insert targets). The fix is Heuristic — the arm SHAPES cover the gap (applying makes the match
/// exhaustive), but their BODIES are `(trap "TODO: …")` placeholders the author fills.
pub(super) fn non_exhaustive_sum_reject(
    db: &Db,
    decl: StructId,
    tested: &[u32],
    scrutinee: StructId,
) -> Reject {
    let generic = "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)";
    let Some(t) = db.type_decl_by_occ(decl) else {
        return Reject::coded(Code::NonExhaustive, generic);
    };
    // An OPEN sum (`(type T … .. r)`) is exhaustive ONLY WITH an open-tail `_` arm — its variant set is
    // not closed, so however many NAMED variants a match covers, the row variable stands for variants it
    // cannot enumerate (`type-system.md §206`). When every named variant IS covered but the match lacks a
    // `_` arm, name the open-tail requirement + carry the "add a `_` arm" fix (the open-sum analogue of
    // the missing-variant fix below). The `_` body is a diverging `(trap "TODO")` placeholder.
    //= spec/capabilities/type-system.md#a-sum-type-may-be-open-with-a-mandatory-open-tail-arm
    //# A match on an open sum MUST carry an open-tail arm covering the variants not named, and a match that omits it MUST be a compile-time rejection, so that exhaustiveness holds for an open sum exactly as it does for a closed one and an unknown variant is handled rather than unmatched.
    let is_open = t.open_tail.is_some();
    let missing_named = t
        .variants
        .iter()
        .enumerate()
        .any(|(i, _)| !tested.contains(&(i as u32)));
    if is_open && !missing_named {
        let message =
            "non-exhaustive match: an open sum requires an open-tail `_` arm covering its unnamed variants"
                .to_string();
        return match db.parent_of(scrutinee) {
            Some(match_form) => Reject::coded(Code::NonExhaustive, message).with_fix(
                Fix::insert_arms_heuristic(match_form, vec!["(_ (trap \"TODO\"))".to_string()]),
            ),
            None => Reject::coded(Code::NonExhaustive, message),
        };
    }
    // The variants whose discriminant no arm tested, in declaration order (a deterministic list).
    let missing: Vec<&crate::db::Variant> = t
        .variants
        .iter()
        .enumerate()
        .filter(|(i, _)| !tested.contains(&(*i as u32)))
        .map(|(_, v)| v)
        .collect();
    if missing.is_empty() {
        return Reject::coded(Code::NonExhaustive, generic);
    }
    // Name the missing variants in the message (rustc "patterns `X` and `Y` not covered").
    let names: Vec<String> = missing.iter().map(|v| format!("`{}`", v.name)).collect();
    let message = format!(
        "non-exhaustive match: pattern{} {} not covered",
        if missing.len() == 1 { "" } else { "s" },
        join_and(&names),
    );
    // One arm per missing variant. A nullary variant → `(Name <body>)`; a payload variant → bind each
    // payload with a fresh `_`-prefixed name so the arm is well-formed AND does not itself warn unused:
    // `((Some _p0) <body>)`. The body is `(trap "TODO: <variant>")` — a DIVERGING placeholder the author
    // replaces. `trap : ∀a. String → a`, so it type-checks in ANY arm whatever the sibling arms' result
    // type is; a bare `unit` body cascaded to a CDZ0203 "match arms differ: T vs Unit" the moment the
    // other arms were not Unit-typed (trading one fault for another — a fix must resolve in ONE shot,
    // `spec/capabilities/diagnostics.md` §A Diagnostic Carries A Route To A Fix). The message names the
    // variant so the author sees which case is stubbed.
    let arms: Vec<String> = missing
        .iter()
        .map(|v| {
            if v.payloads.is_empty() {
                format!("({} (trap \"TODO: {}\"))", v.name, v.name)
            } else {
                let binders: Vec<String> =
                    (0..v.payloads.len()).map(|i| format!("_p{i}")).collect();
                format!(
                    "(({} {}) (trap \"TODO: {}\"))",
                    v.name,
                    binders.join(" "),
                    v.name
                )
            }
        })
        .collect();
    // The `(match …)` form is the scrutinee's parent — the list the arms append into.
    match db.parent_of(scrutinee) {
        Some(match_form) => Reject::coded(Code::NonExhaustive, message)
            .with_fix(Fix::insert_arms_heuristic(match_form, arms)),
        None => Reject::coded(Code::NonExhaustive, message),
    }
}

/// The CDZ0210 NON-EXHAUSTIVE message for a NESTED sub-match (a gap inside a payload pattern) — names the
/// missing variant(s) of the sub-value's sum but carries NO fix. A nested gap's covering arms would have to
/// be shaped to the enclosing pattern's nesting (`((Some (B)) …)`, not a flat `(B …)`), which the top-level
/// flat-append fix cannot express — so a nested non-exhaustive keeps the actionable "pattern `B` not
/// covered" NAME (a big improvement over the generic "must cover every variant") without a misleading
/// fix. The message-only twin of [`non_exhaustive_sum_reject`], sharing its missing-variant computation.
/// (v-diagnostics note 2026-07-16: the nested path fell to the generic message; this surfaces the witness.)
pub(super) fn non_exhaustive_sum_message(db: &Db, decl: StructId, tested: &[u32]) -> Reject {
    let generic = "a sum match must cover every variant or end in a wildcard `_` (non-exhaustive)";
    let Some(t) = db.type_decl_by_occ(decl) else {
        return Reject::coded(Code::NonExhaustive, generic);
    };
    // An OPEN nested sub-sum with every NAMED variant covered needs an open-tail `_` — name that.
    let is_open = t.open_tail.is_some();
    let missing: Vec<&crate::db::Variant> = t
        .variants
        .iter()
        .enumerate()
        .filter(|(i, _)| !tested.contains(&(*i as u32)))
        .map(|(_, v)| v)
        .collect();
    if missing.is_empty() {
        if is_open {
            return Reject::coded(
                Code::NonExhaustive,
                "non-exhaustive match: this open sum requires an open-tail `_` arm covering its unnamed variants",
            );
        }
        return Reject::coded(Code::NonExhaustive, generic);
    }
    let names: Vec<String> = missing.iter().map(|v| format!("`{}`", v.name)).collect();
    Reject::coded(
        Code::NonExhaustive,
        format!(
            "non-exhaustive match: pattern{} {} not covered",
            if missing.len() == 1 { "" } else { "s" },
            join_and(&names),
        ),
    )
}

/// Join names as `a`, `a and b`, or `a, b, and c` — the English list a "not covered" message reads
/// naturally with (matching rustc's phrasing).
pub(super) fn join_and(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    }
}

/// The CDZ0210 non-exhaustive-SCALAR-match rejection, enriched with an "add the covering arm" fix (the
/// scalar analogue of `non_exhaustive_sum_reject` — `spec/capabilities/diagnostics.md` §A Diagnostic
/// Carries A Route To A Fix). A BOOL scrutinee missing a literal (`bool_true`/`bool_false` = whether
/// each is covered by an unguarded arm) is a FINITE gap: name + insert exactly the missing
/// `(true (trap …))` / `(false (trap …))` arm, like a missing sum variant. Any OTHER scalar (an open
/// Int/String, or a Bool with neither literal) is closed only by a wildcard: insert `(_ (trap …))`. The
/// arm bodies are `(trap "TODO: …")` — a DIVERGING placeholder (`trap : ∀a. String → a`) that type-checks
/// in ANY arm whatever the sibling arms return; a bare `unit` body cascaded to a CDZ0203 "match arms
/// differ: T vs Unit" the moment the other arms were not Unit-typed. Heuristic (the author fills the
/// body). Anchored at the `(match …)` form (parent of the scrutinee); falls back to the plain reject (no
/// fix) if that parent is absent.
pub(super) fn non_exhaustive_scalar_reject(
    db: &Db,
    scrutinee: StructId,
    scrut_ty: &crate::ty::Ty,
    bool_true: bool,
    bool_false: bool,
) -> Reject {
    // A Bool scrutinee with exactly one literal covered → the missing one is a KNOWN, finite gap.
    let is_bool = scrut_ty.agrees_with(&crate::ty::Ty::Bool);
    let (message, arms) = if is_bool && (bool_true ^ bool_false) {
        let missing = if bool_true { "false" } else { "true" };
        (
            format!("non-exhaustive match: `{missing}` is not covered"),
            vec![format!("({missing} (trap \"TODO: {missing}\"))")],
        )
    } else {
        // An open scalar (or a Bool with neither literal) — only a wildcard closes it.
        (
            "non-exhaustive match: add a wildcard `_` arm to cover the remaining values"
                .to_string(),
            vec!["(_ (trap \"TODO\"))".to_string()],
        )
    };
    match db.parent_of(scrutinee) {
        Some(match_form) => Reject::coded(Code::NonExhaustive, message)
            .with_fix(Fix::insert_arms_heuristic(match_form, arms)),
        None => Reject::coded(Code::NonExhaustive, message),
    }
}

/// The discriminant of the variant named `name` in the sum `ty`, or `None` if `ty` is not a sum or has
/// no such variant. This is what distinguishes a bare NULLARY-VARIANT pattern (`None` against `Option`)
/// from a binder (`x`) — the name is looked up in the scrutinee sum's own declaration (occurrence-keyed,
/// so a same-named variant in another sum does not leak in).
pub(super) fn variant_disc_by_name(db: &mut Db, ty: &crate::ty::Ty, name: &str) -> Option<u32> {
    let decl = match ty {
        crate::ty::Ty::Sum { decl, .. } => *decl,
        _ => return None,
    };
    let t = db.type_decl_by_occ(decl)?;
    t.variants
        .iter()
        .position(|v| v.name == name)
        .map(|i| i as u32)
}

/// A CONFIDENT near-miss variant of the scrutinee sum `ty` for a bare arm-pattern `name` that is NOT
/// itself a variant — the "did you mean `Red`?" candidate for a misspelled bare nullary-variant pattern
/// (`Rd` over `(type Color Red Green)`). `None` when `ty` is not a sum, `name` IS a variant (no typo), or
/// no variant is within `suggest::nearest`'s edit-distance cutoff (a genuine binder, not a typo). This is
/// what distinguishes a misspelled variant arm from an intentional catch-all binder: only a name close to
/// an EXISTING variant is judged a typo. Reads the sum's variant names off its `decl` (the same candidate
/// set `enrich_pattern_head_suggestion` uses for the compound-head form).
pub(super) fn nearest_variant_typo(db: &mut Db, ty: &crate::ty::Ty, name: &str) -> Option<String> {
    let decl = match ty {
        crate::ty::Ty::Sum { decl, .. } | crate::ty::Ty::Nominal { decl, .. } => *decl,
        _ => return None,
    };
    let t = db.type_decl_by_occ(decl)?;
    // Already a variant → not a typo (the caller's `variant_disc_by_name` handles the exact match).
    if t.variants.iter().any(|v| v.name == name) {
        return None;
    }
    let names: Vec<String> = t.variants.iter().map(|v| v.name.clone()).collect();
    crate::diag::suggest::nearest(name, &names)
}

/// A map from an access PATH to the solved TYPE of the sub-value there — populated as the tree descends
/// (the root `[]` maps to the scrutinee type; entering a variant arm at `switch_path` extends it with
/// that variant's payload type at `switch_path + [Payload]`). Keyed per-branch (not global), because the
/// SAME path under different parent variants has different types (`Result`'s `[Payload]` is `a` in the
/// `Ok` arm, `e` in the `Err` arm) — a global map would collide; a branch-local one is always consistent.
// The value at each path is an `Rc<Ty>`, NOT a bare `Ty`. `build_tree` threads ONE shared map with scoped
// insert/restore per arm (see its arm loop): entering a variant arm inserts the arm's payload-path types
// (recording each key's prior value), recurses, then restores — so sibling arms don't share a mutation
// without cloning the whole map. An `Rc<Ty>` value keeps a restored prior entry a pointer-bump rather than
// a deep `Ty` copy. (An earlier version CLONED the whole map per arm per level → O(depth³) on a nested
// pattern; the shared-map + `Rc`-path fixes dropped that whole factor.)
pub(super) type PathTypes =
    std::collections::HashMap<Vec<crate::core::PathStep>, std::rc::Rc<crate::ty::Ty>>;
/// A path-type ADDITION a variant arm makes: `(path, sub-value type)`. `path` is a plain `Vec` (a `Vec` key
/// the `PathTypes` map owns), the type an `Rc` (shared, so a restore is a pointer bump).
pub(super) type PathTypeEntry = (Vec<crate::core::PathStep>, std::rc::Rc<crate::ty::Ty>);
/// A saved-for-restore `PathTypes` slot: the key, and its value BEFORE the arm's insert (`None` = absent).
pub(super) type PathTypeRestore = (
    Vec<crate::core::PathStep>,
    Option<std::rc::Rc<crate::ty::Ty>>,
);

/// Compile a pattern MATRIX (`rows`) into a decision-tree CONTINUATION for the value at `scrutinee`. If
/// the FIRST row is a catch-all (no constraints), it matches unconditionally → its body is the leaf (later
/// rows unreachable). Otherwise switch on the discriminant at the SHALLOWEST path any row constrains:
/// gather the discs tested there in source order, and for each build a specialized sub-matrix — rows
/// constraining that path with this disc (constraint removed) PLUS rows not constraining it (they match
/// any disc, flowing into every arm) — then recurse. A default arm (`disc: None`) covers the rows that
/// don't constrain the switch path. Exhaustiveness is checked at EACH switch (every variant tested, or a
/// default). A constant sub-value FOLDS to the matching arm's continuation (no runtime switch).
pub(super) fn build_tree(
    db: &mut Db,
    scrutinee: StructId,
    rows: &[MatchRow],
    path_types: &mut PathTypes,
) -> Result<std::rc::Rc<crate::core::SumCont>, Reject> {
    build_tree_ft(db, scrutinee, rows, path_types, None)
}

/// `build_tree` with an explicit shared FALL-THROUGH continuation. `fallthrough` is the continuation for
/// this sub-matrix's EMPTY terminus — the tree reached once every row here has been ruled out. It exists to
/// KILL an O(2^arms) blow-up: a match arm testing ≥2 literal columns (`(tuple 0 0 a)`) lowers via
/// [`build_lit_test`] to `LitTest{then_, els}` where `then_` (the arm's SECOND-column test) itself falls
/// through to the SAME remaining-arms matrix that `els` compiles — so without sharing, that fall-through is
/// re-compiled in both branches at every column, T(N)=2·T(N-1) (a 20-arm 2-column match: ~5s to check, a
/// 64MB module). For a NON-REFINING probe (Int/Str — its else is the remaining arms VERBATIM, unlike a
/// Bool/ListLen probe whose else is a REFINED matrix), `build_lit_test` compiles that fall-through ONCE into
/// an `Rc<SumCont>` and threads it here as `fallthrough`, so the arm's own further column-tests reuse the
/// SAME `Rc` (a refcount bump) instead of re-compiling — build O(arms), tree an O(arms)-node DAG. When
/// `fallthrough` is `None` (every call except a shared-else arm chain) the empty terminus is the ordinary
/// CDZ0210 non-exhaustive reject — semantics unchanged.
pub(super) fn build_tree_ft(
    db: &mut Db,
    scrutinee: StructId,
    rows: &[MatchRow],
    path_types: &mut PathTypes,
    fallthrough: Option<&std::rc::Rc<crate::core::SumCont>>,
) -> Result<std::rc::Rc<crate::core::SumCont>, Reject> {
    #[cfg(test)]
    crate::db::BUILD_TREE_CALLS.with(|c| c.set(c.get() + 1));
    // The FIRST row whose discriminant constraints are all satisfied (empty) is at a LEAF position. If it
    // is UNGUARDED it matches unconditionally → its body is the leaf (later rows unreachable). If it is
    // GUARDED, it fires only when its guard holds; on a false guard control FALLS THROUGH to the rest of
    // this sub-matrix (`build_tree` of the remaining rows) — the per-variant fall-through a guarded arm
    // needs. A guarded leaf does NOT terminate the matrix, so the fall-through must independently be
    // exhaustive (an unguarded arm of the same variant, or the default, below it).
    match rows.first() {
        None => {
            // Empty matrix: if a shared fall-through was threaded (a non-refining lit-test arm's chain
            // bottoms out here), that Rc IS the continuation — reuse it (a refcount bump, the O(1) that
            // makes a multi-column arm's chain share one fall-through). Otherwise the matrix is genuinely
            // exhausted with no cover → CDZ0210 (the ordinary, semantics-unchanged path).
            match fallthrough {
                // Return the fallthrough Rc ITSELF (a refcount bump), NOT a deref-clone of its top node.
                // This PRESERVES the shared-tail Rc identity: the same `Rc<SumCont>` is reachable from both
                // this arm's terminus AND the `els` that threaded it, so `Rc::ptr_eq` holds — the decision
                // tree stays a DAG through emit. A deref-clone here (the old `(**f).clone()`) would flatten
                // the DAG to a tree before the backend sees it, defeating the emit-side shared-continuation
                // dedup (v-wasm-opt's `emit_sum_cont` ptr_eq memo) and keeping the O(2^arms) emit blow-up.
                Some(f) => return Ok(std::rc::Rc::clone(f)),
                None => {
                    return Err(Reject::coded(
                        Code::NonExhaustive,
                        "a sum match must cover every variant or end in a wildcard `_` \
                         (non-exhaustive)",
                    ));
                }
            }
        }
        // A row whose discriminant constraints are all satisfied but that still carries LITERAL TESTS is
        // at a leaf gated by those tests: `(Some 0)` reaches here (after the `Some` switch) with a pending
        // `([Payload], Int(0))`. Emit a `LitTest` — test the sub-value at `path` against the literal; on a
        // match, CONTINUE with that test dropped (further lit-tests / the guard / the body); on a MISMATCH,
        // FALL THROUGH to the remaining rows (the same-variant binding arm `(Some k)`), exactly as a guard
        // threads its `else`. A literal test does NOT count toward exhaustiveness — the fall-through must
        // cover the variant. FOLD when the tested sub-value is a compile-time constant (a constant
        // scrutinee): a matching literal drops the test, a non-matching one skips to the fall-through
        // WITHOUT emitting the body — the constant-match half of corpus "nested patterns with literals".
        Some(row) if row.constraints.is_empty() && !row.lit_tests.is_empty() => {
            let (lit_path, probe) = row.lit_tests[0].clone();
            // A VACUOUSLY-TRUE length test — `ListLen{0, at_least}` from a zero-leading rest `(list .. r)` —
            // matches EVERY length, so it is not refutable at all: drop it and re-consider the row (it may
            // now be an unconditional leaf, e.g. a lone `(Bx (list .. r))` covers the `Bx` payload fully).
            // Without this, a `{0, at_least}` test threaded a needless LitTest whose else was an empty
            // fall-through → a spurious CDZ0210. (Only `{0, at_least}` is vacuous; `{k>0, at_least}` and any
            // exact test are genuinely refutable.)
            if let crate::core::Probe::ListLen {
                len: 0,
                at_least: true,
            } = probe
            {
                let mut relaxed = row.clone();
                relaxed.lit_tests.remove(0);
                let mut relaxed_rows = vec![relaxed];
                relaxed_rows.extend_from_slice(&rows[1..]);
                return build_tree(db, scrutinee, &relaxed_rows, path_types);
            }
            // The row with this first literal test consumed (its other tests / guard / body remain).
            let mut matched_row = row.clone();
            matched_row.lit_tests.remove(0);
            // FOLD against a constant sub-value.
            if let Some(c) = const_at_path(db, scrutinee, &lit_path) {
                // The fold picks ONE branch (hit → the matched arm, miss → the fall-through), so there is
                // no duplication to share here — build the matched sub-matrix the ordinary way (this row
                // then the rest), skipping the tail only when the matched row is now an unconditional leaf
                // (control stops at it; the fix-64 wasted-clone guard).
                let matched_row_is_leaf =
                    matched_row.lit_tests.is_empty() && matched_row.guard.is_none();
                let mut matched_rows = vec![matched_row.clone()];
                if !matched_row_is_leaf {
                    matched_rows.extend_from_slice(&rows[1..]);
                }
                let hit = match (&probe, &c) {
                    (crate::core::Probe::Int(v), Core::ConstInt(cv)) => v.eq_value(cv),
                    (crate::core::Probe::Bool(b), Core::ConstBool(cb)) => b == cb,
                    // A string-literal payload test folds against a constant `Core::ConstStr` by value
                    // equality (both NFC-normalized by the reader) — `(Ast.Name "+")` matches an
                    // `Ast.Name` carrying "+". A runtime string payload has no `ConstStr` → declines below.
                    (crate::core::Probe::Str(s), Core::ConstStr(cs)) => s.as_str() == &cs[..],
                    // A char-literal payload test folds against a constant `Core::ConstChar` by codepoint
                    // equality — `(Tok.Ch #\a)` matches a `Tok.Ch` carrying `#\a`. A runtime char payload
                    // has no `ConstChar` → declines below.
                    (crate::core::Probe::Char(c), Core::ConstChar(cc)) => c == cc,
                    // A LIST length test folds against a CONSTANT list: an exact test needs `== len`, a
                    // rest (`at_least`) test needs `>= len` (the tail binds the surplus). (A runtime list
                    // has no `ListNew` here → the runtime-test arm below, which declines.)
                    (crate::core::Probe::ListLen { len, at_least }, Core::ListNew { elems }) => {
                        if *at_least {
                            elems.len() >= *len
                        } else {
                            elems.len() == *len
                        }
                    }
                    // A MAP key-presence test folds against a CONSTANT map: every named key must be present
                    // (some entry key `const_compound_eq` to it). A runtime map has no `MapNew` here → the
                    // runtime-test arm below, which declines. (The `keys`/`entries` are cloned out of `c`
                    // first so the `const_compound_eq` `&mut db` calls don't overlap the borrow of `c`.)
                    (crate::core::Probe::MapHasKeys { keys }, Core::MapNew { entries, .. }) => {
                        let keys: Vec<StructId> = keys.to_vec();
                        let entries = entries.clone();
                        keys.iter().all(|&k| {
                            entries
                                .iter()
                                .any(|&(ek, _)| const_compound_eq(db, ek, k) == Some(true))
                        })
                    }
                    // A non-constant / type-mismatched sub-value can't fold — emit the runtime test.
                    _ => {
                        return build_lit_test(
                            db,
                            scrutinee,
                            lit_path,
                            probe,
                            &matched_rows,
                            &rows[1..],
                            path_types,
                            fallthrough,
                        );
                    }
                };
                if hit {
                    return build_tree(db, scrutinee, &matched_rows, path_types);
                } else {
                    return build_tree(db, scrutinee, &rows[1..], path_types);
                }
            }
            // RUNTIME test (the sub-value is not a compile-time constant) — the O(2^arms) path for a
            // multi-column arm. The matched branch is this ONE row (`matched_row`, its first test consumed)
            // followed by the fall-through `rows[1..]`; `build_lit_test` shares that fall-through as ONE
            // `Rc<SumCont>` across the arm's further column-tests instead of appending + re-compiling it.
            return build_lit_test(
                db,
                scrutinee,
                lit_path,
                probe,
                std::slice::from_ref(&matched_row),
                &rows[1..],
                path_types,
                fallthrough,
            );
        }
        Some(row) if row.constraints.is_empty() && row.guard.is_none() => {
            return Ok(std::rc::Rc::new(crate::core::SumCont::Leaf(row.body)));
        }
        Some(row) if row.constraints.is_empty() => {
            // A GUARDED leaf: `if guard then body else <fall-through over the remaining rows>`.
            let cond = row.guard.expect("matched the guarded arm");
            let body = row.body;
            // FOLD the guard when it is a compile-time-constant bool (a constant scrutinee makes its
            // payload binders constant, so `(> x 0)` over `x = 0` folds to `false`). A true guard SELECTS
            // the body directly; a false guard SKIPS to the fall-through tree — WITHOUT lowering the body.
            // This shields a body that would TRAP when folded (`(/ 10 x)` at `x = 0` → CDZ0304) from being
            // evaluated when its guard is false: the guard short-circuits the body exactly as `and`/`or`
            // and `if` shield an untaken branch (core-semantics.md §Boolean Connectives Short-Circuit).
            // Without this fold, a false-guarded arm's trapping body raised a SPURIOUS CDZ0304 for an arm
            // that never runs. A guard reading a RUNTIME value does not fold → the runtime `Guarded` cont.
            match core_of(db, cond) {
                Core::ConstBool(true) => {
                    // The guard folds TRUE, so this arm fires and its body is the value. But a guarded arm
                    // does NOT count toward exhaustiveness (core-semantics.md §Matching Is Exhaustive Or
                    // Rejected: "a guarded arm may be false, so it covers no variant"), and the match must
                    // be well-formed AS WRITTEN — a non-exhaustive match is CDZ0210 regardless of whether a
                    // constant scrutinee happens to satisfy a guard. So verify the fall-through `rows[1..]`
                    // still forms an exhaustive cover BEFORE folding to the body: `build_tree` on it
                    // surfaces CDZ0210 if the variant is otherwise uncovered (a bare `((guard (Some x) …)
                    // (None -1))` — `Some` covered ONLY by the guarded arm — must reject, matching the
                    // standalone-emitted body). The check's RESULT is discarded (we still fold to `body`
                    // when the scrutinee satisfies the guard); only its error propagates. This keeps the
                    // fold consistent with the runtime `Guarded` path below, which builds `els` (and thus
                    // checks the fall-through) unconditionally.
                    let _ = build_tree(db, scrutinee, &rows[1..], path_types)?;
                    return Ok(std::rc::Rc::new(crate::core::SumCont::Leaf(body)));
                }
                Core::ConstBool(false) => return build_tree(db, scrutinee, &rows[1..], path_types),
                _ => {}
            }
            let els = build_tree(db, scrutinee, &rows[1..], path_types)?;
            return Ok(std::rc::Rc::new(crate::core::SumCont::Guarded {
                cond,
                body,
                els,
            }));
        }
        _ => {}
    }
    // Pick the SWITCH path — the shallowest path any row constrains (outer patterns first, so the outer
    // probe is shared). Its TYPE gives the variant set for exhaustiveness + recursion. Read from
    // `path_types` (populated as sum-variant arms descend), else COMPUTE it by walking the path from the
    // scrutinee's own type — a `Ty::Tuple` element indexes at `Elem(i)`, so a sum nested in a TUPLE element
    // (`(match (tuple a b) ((tuple (E.Lit x) …)…))`, switch path `[Elem(0)]`) resolves even though no
    // sum-payload descent seeded it. (`path_types` still wins where present — a variant payload's
    // instantiated type is more precise than a raw type-walk.)
    let switch_path = shallowest_path(rows);
    // The switch sub-value's type, as an `Rc` — the seeded case SHARES the map's `Rc` (a pointer bump,
    // not a deep clone of an O(depth)-nested `Ty`), so descending a deeply-nested pattern does not re-clone
    // the growing type at every level. The computed fallback wraps its fresh `Ty` once.
    let sub_ty: std::rc::Rc<crate::ty::Ty> = match path_types.get(&switch_path[..]) {
        Some(t) => t.clone(),
        // Not seeded exactly: try a raw type-walk from the scrutinee, then (for a path that descends
        // through a boxed-sum `Payload` a raw walk can't cross) walk the SUFFIX from the longest seeded
        // PREFIX in `path_types` — a list-element switch `[Payload, Elem(1)]` resolves from the seeded
        // `[Payload]` = `(List Ast)` even though the raw `Payload` walk over the boxed sum returns None.
        None => match type_at_path(db, scrutinee, &switch_path)
            .or_else(|| type_from_seeded_prefix(path_types, &switch_path))
        {
            Some(t) => std::rc::Rc::new(t),
            None => {
                return Err(Reject::decline(
                    "compound match switch path has no solved type",
                ));
            }
        },
    };
    let (decl, variant_count) = match &*sub_ty {
        crate::ty::Ty::Sum { decl, .. } => match db.type_decl_by_occ(*decl) {
            Some(t) => (*decl, t.variants.len()),
            None => return Err(Reject::decline("sum match sub-value has no declaration")),
        },
        _ => {
            return Err(Reject::decline(
                "sum match dispatches on a non-sum sub-value",
            ));
        }
    };
    // Partition the matrix by the disc each row tests at `switch_path` in ONE pass (was one O(N) scan per
    // arm via `specialize` → O(N²) over N arms; the `tested.contains` loop was O(N²) too). Each row either
    // tests `switch_path` with some disc `d` (it belongs ONLY to arm `d`, with that now-satisfied
    // constraint dropped) or does NOT test it (a DEFAULT row — it flows into EVERY arm AND the default
    // arm, unchanged). Rows keep their source index so an arm's sub-matrix preserves source order (arm
    // priority = first-matching-row) when disc rows and default rows interleave.
    let mut tested: Vec<u32> = Vec::new();
    let mut disc_rows: crate::fxhash::FxHashMap<u32, Vec<(usize, MatchRow)>> = Default::default();
    let mut default_rows: Vec<(usize, MatchRow)> = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        match row.constraints.iter().find(|(p, _)| *p == switch_path) {
            Some((_, d)) => {
                let d = *d;
                let bucket = disc_rows.entry(d).or_insert_with(|| {
                    tested.push(d);
                    Vec::new()
                });
                bucket.push((
                    i,
                    MatchRow {
                        // Drop the now-satisfied `switch_path` constraint (control is in this arm).
                        constraints: row
                            .constraints
                            .iter()
                            .filter(|(p, _)| *p != switch_path)
                            .cloned()
                            .collect(),
                        lit_tests: row.lit_tests.clone(),
                        body: row.body,
                        guard: row.guard,
                    },
                ));
            }
            None => default_rows.push((
                i,
                MatchRow {
                    constraints: row.constraints.clone(),
                    lit_tests: row.lit_tests.clone(),
                    body: row.body,
                    guard: row.guard,
                },
            )),
        }
    }
    // The switched sub-value's STATICALLY-KNOWN discriminant, if any — a `SumNew` core at `switch_path`
    // has a fixed disc EVEN when its payload is a runtime value (`(Some n)` is `SumNew{Some, [n]}`: the
    // `Some` tag is known, only `n` is runtime). It drives the FOLD below (pick the known arm, no runtime
    // switch). It does NOT relax exhaustiveness: `core-semantics.md §Matching Is Exhaustive Or Rejected`
    // (corpus 02 "a sum match missing a variant is non-exhaustive EVEN when the scrutinee is the covered
    // one") makes exhaustiveness a property of the ARM SET against the TYPE's variant set, never of which
    // variant the scrutinee holds — a value-driven shortcut that skips the check because the constant hit
    // a present arm is exactly what that case forbids.
    let known_disc = match const_at_path(db, scrutinee, &switch_path) {
        Some(Core::SumNew { disc, .. }) => Some(disc),
        _ => None,
    };
    // Whether the switched sum is declared OPEN (`(type T … .. r)`). An open sum's variant set is not
    // closed — the row variable stands for variants this match cannot enumerate — so it is exhaustive
    // ONLY WITH an open-tail (`_`/binder default) arm, regardless of how many NAMED variants are covered
    // (`type-system.md §206`). A match over an open sum WITHOUT a default is non-exhaustive even when every
    // named variant is covered (there may always be an unnamed one). A CLOSED sum keeps the classic rule
    // (every variant covered OR a default). Read off the declaration's `open_tail`.
    let is_open = db
        .type_decl_by_occ(decl)
        .is_some_and(|t| t.open_tail.is_some());
    // Exhaustiveness: every variant tested, or a default (wildcard/binder) present — else CDZ0210. Against
    // the TYPE's variant set, independent of `known_disc` (see above). An OPEN sum ALWAYS needs a default,
    // even when every named variant is covered.
    let has_default = !default_rows.is_empty();
    if !has_default && (is_open || tested.len() < variant_count) {
        // Name the missing variants + carry an "add the missing arms" fix — but ONLY at the ROOT switch
        // (`switch_path` empty): there the missing-variant arms append directly to the `(match …)` form
        // and are well-formed top-level patterns. A NESTED non-exhaustive (a gap inside a payload
        // pattern) would need arms shaped to the nesting, which the flat append cannot express, so it
        // keeps the enriched message but no fix (the `db.parent_of(scrutinee)` there is not the match).
        if switch_path.is_empty() {
            return Err(non_exhaustive_sum_reject(db, decl, &tested, scrutinee));
        }
        // A NESTED gap (inside a payload pattern) — name the missing variant(s) of the sub-value's sum
        // (message-only: a nested arm's shape can't be flat-appended, so no fix). Big improvement over the
        // generic message: `(match o ((Some (A)) 1) ((None) 0))` now says "pattern `B` not covered" (the
        // uncovered inner variant) rather than "must cover every variant".
        return Err(non_exhaustive_sum_message(db, decl, &tested));
    }
    // One arm per tested discriminant, then the default arm (if any). Each arm's sub-matrix merges its
    // disc rows with the default rows by source index (both already ascending), recursing under a
    // `path_types` extended with THIS variant's payload type at `switch_path+[Payload]`.
    let mut sum_arms: Vec<crate::core::SumArm> = Vec::new();
    for &d in &tested {
        let own = disc_rows.remove(&d).unwrap_or_default();
        let sub_rows = merge_rows(own, &default_rows);
        // This variant's payload-type additions to `path_types`. SCOPED insert/restore over the SHARED map
        // (rather than a whole-map clone per arm): insert the new keys — recording each key's PRIOR value —
        // recurse, then RESTORE so a sibling arm (which extends the SAME `switch_path+[Payload]` key with
        // ITS own payload type) sees the parent state, not this arm's. Sibling arms must not share a
        // mutation; the parent map is left exactly as found. This is what drops the O(depth³) map-clone.
        let additions = path_type_additions(db, &switch_path, &sub_ty, decl, d);
        let mut prev: Vec<PathTypeRestore> = Vec::with_capacity(additions.len());
        for (path, ty) in additions {
            let old = path_types.insert(path.clone(), ty);
            prev.push((path, old));
        }
        let cont = build_tree(db, scrutinee, &sub_rows, path_types);
        // Restore BEFORE `?`-propagating, so the map is clean on the error path too.
        for (path, old) in prev.into_iter().rev() {
            match old {
                Some(t) => {
                    path_types.insert(path, t);
                }
                None => {
                    path_types.remove(&path);
                }
            }
        }
        sum_arms.push(crate::core::SumArm {
            disc: Some(d),
            // `SumArm.cont` is a by-value `SumCont` (a switch arm is not a shared-tail site — the
            // exponential sharing is the LitTest fall-through, which stays `Rc`). Unwrap the Rc: reuse the
            // inner value if uniquely owned, else clone it (the common leaf/switch arm is uniquely owned).
            cont: std::rc::Rc::try_unwrap(cont?).unwrap_or_else(|rc| (*rc).clone()),
        });
    }
    if has_default {
        // The default arm switches on nothing new at `switch_path` — its rows only reach paths they
        // already constrain (all in `path_types`), so no extension is needed.
        let sub_rows: Vec<MatchRow> = default_rows.into_iter().map(|(_, r)| r).collect();
        let cont = build_tree(db, scrutinee, &sub_rows, path_types)?;
        sum_arms.push(crate::core::SumArm {
            disc: None,
            cont: std::rc::Rc::try_unwrap(cont).unwrap_or_else(|rc| (*rc).clone()),
        });
    }
    // FOLD when the switched sub-value's discriminant is STATICALLY KNOWN (a `SumNew` core — its tag is
    // fixed even if its payload is runtime): pick the matching arm's continuation directly, no runtime
    // disc switch. `(match (Some n) …)` folds to the `Some` arm (whose body may still test the runtime
    // payload `n` via a `LitTest`). A scrutinee whose disc is NOT known keeps the runtime `Switch`.
    if let Some(disc) = known_disc {
        for arm in &sum_arms {
            if arm.disc.is_none() || arm.disc == Some(disc) {
                trace!(target: "rcdzc::fold", "sum match folds to a selected arm (known discriminant)");
                return Ok(std::rc::Rc::new(arm.cont.clone()));
            }
        }
    }
    trace!(target: "rcdzc::lower", scrutinee = scrutinee.0, depth = switch_path.len(), arms = sum_arms.len(), "sum switch (decision-tree node)");
    Ok(std::rc::Rc::new(crate::core::SumCont::Switch {
        // The emitted `SumCont` carries the shared `Rc<[PathStep]>` (`MatchPath`) directly — the match
        // compiler already threads it as an `Rc`, so cloning the node is a refcount bump, not a path copy.
        path: switch_path,
        arms: sum_arms,
    }))
}

/// Build a runtime `SumCont::LitTest` node: test the sub-value at `lit_path` against `probe`; on a match
/// continue with `matched_rows` (this arm with the test consumed, then the rest of the sub-matrix), on a
/// mismatch fall through to `else_rows`. Both sub-trees are compiled by `build_tree`. Split out of
/// `build_tree` so the constant-fold path (a matching/non-matching constant sub-value) and the runtime
/// path share one construction; the `then_`/`els` recursion is what lets several literal tests on one arm
/// nest and a fall-through reach the same-variant binding arm.
#[allow(clippy::too_many_arguments)] // scrutinee + path + probe + matched/else rows + path_types + shared els
pub(super) fn build_lit_test(
    db: &mut Db,
    scrutinee: StructId,
    lit_path: MatchPath,
    probe: crate::core::Probe,
    matched_rows: &[MatchRow],
    else_rows: &[MatchRow],
    path_types: &mut PathTypes,
    fallthrough: Option<&std::rc::Rc<crate::core::SumCont>>,
) -> Result<std::rc::Rc<crate::core::SumCont>, Reject> {
    // A `Str` probe that did NOT fold (the payload is a RUNTIME value, not a constant `Core::ConstStr`) is
    // emitted as a runtime STRING-EQUALITY test: the backend walks `lit_path` to the leaf String handle,
    // `bytes-compact`s it (rope→canonical flat, refcount-neutral — the borrowed payload is not consumed),
    // builds the literal as a fresh `ConstStr` leaf, and `value-eq`s them (the same `champ_eq` physical-byte
    // compare `Core::ValueEq` uses on two strings) — dispatching an `Ast.Name "+"` / a `(k "lit")` map value
    // over a runtime value by its content. Like the `ListLen` runtime test, it is NON-refining (a string is
    // an open type — its else is the rest of the matrix verbatim), so it shares the fall-through the same
    // way. The CONSTANT case still folds in `build_tree` and never reaches here.
    // BOOL is a FINITE 2-value type: testing `Bool(b)` at `lit_path` means the ELSE branch is exactly the
    // world where that sub-value is `!b`. So in `else_rows`, refine every row's lit-test at `lit_path`
    // AGAINST the known `!b`: a row testing `Bool(!b)` there has its test SATISFIED (drop it — the arm now
    // matches unconditionally), and a row testing `Bool(b)` there is DEAD (the value can't be `b`) and is
    // dropped. This makes `(match t ((tuple true b) …) ((tuple false b) …))` EXHAUSTIVE — the `false` arm
    // becomes an unconditional leaf in the `true`-test's else — where before a bool sub-pattern (a lit-test,
    // not a discriminant) never counted toward coverage and the innermost fall-through was a spurious
    // CDZ0210 (the top-level scalar-bool matcher already treats `true`+`false` as exhaustive; this brings the
    // NESTED/decision-tree path to parity). Only Bool gets this — an Int/Str lit-test is over an infinite
    // type (its else is genuinely open, needs a `_`).
    //
    // The `then_`/`els` structure depends on whether the probe REFINES its else:
    //  - Int/Str (NON-refining): `else_rows` is the remaining arms VERBATIM in BOTH the matched arm's
    //    fall-through and this test's `els`. Compile it ONCE into an `Rc<SumCont>` and SHARE it — thread it
    //    into `then_`'s recursion as the matched arm's `fallthrough`, and reuse the same `Rc` as `els`. THIS
    //    is what kills the O(2^arms) blow-up on a multi-column literal arm (`(tuple 0 0 a)`): without
    //    sharing, an arm testing K columns re-compiles the remaining matrix 2^K times (each column's `then_`
    //    and `els` both descend into it). Exhaustiveness is unaffected — the matched arm's own coverage is
    //    checked when the shared tail is built (the `build_tree_ft(else_rows)` call), and reusing that
    //    verdict is byte-identical to re-deriving it.
    //  - Bool/ListLen (REFINING): the `els` matrix is REFINED by the failed test, so it DIFFERS from the
    //    matched arm's fall-through (which sees the unrefined remaining arms). Here the matched arm's own
    //    exhaustiveness must be re-checked against the ACTUAL remaining rows, so `then_` is built the
    //    ordinary way — the matched row APPENDED with `else_rows` — and `els` is the refined tree. No
    //    sharing (a finite refining probe has only 2 / a few branches — no exponential fan-out to dedup).
    let (then_, els) = match probe {
        crate::core::Probe::ListLen { len, at_least } => {
            // A ListLen probe REFINES its else (the failed-length world), so `els` differs from the matched
            // arm's fall-through — but BOTH the matched arm's fall-through (the PASSED-length world) and the
            // `els` (the failed-length world) are FIXED matrices, compilable ONCE and shared. Without sharing,
            // a match refining a LIST payload by literal elements (`(Some (list 0 0))`) re-compiles the whole
            // remaining matrix at each element-test → O(2^arms) (fire #28). Compile the PASSED-world tail once
            // (`refine_listlen_to_passed` — arms inconsistent with the passed length dropped) + thread it as
            // the matched arm's `fallthrough` (S1's mechanism, refined tail), and compile the FAILED-world
            // `els` once. Exhaustiveness is preserved: the passed-world refinement never drops a reachable
            // arm (a dropped row is provably unmatchable at the passed length), and the matched arm still
            // bottoms out on the shared passed tail exactly as `[matched] ++ else_rows` would.
            let passed = refine_listlen_to_passed(else_rows, &lit_path, len, at_least);
            // Only compile+share the passed-world tail when it is NON-EMPTY. An empty `passed` (every else
            // arm is length-inconsistent with the passed length, so dropped) has NO fall-through — building
            // `build_tree(&[])` would raise a spurious CDZ0210 (empty matrix) that must not propagate, since
            // the matched arm alone covers the passed world (it is an unconditional leaf after its length
            // test). Thread the OUTER `fallthrough` unchanged there (identical to the pre-S3 append when the
            // tail contributes nothing).
            // `build_tree_ft` now returns `Rc<SumCont>`, so `then_`/`els` are ALREADY the shared Rc — no
            // extra `Rc::new` wrap (that would nest `Rc<Rc<…>>` and, worse, break the ptr-identity the
            // emit-side dedup keys on).
            let then_ = if passed.is_empty() {
                build_tree_ft(db, scrutinee, matched_rows, path_types, fallthrough)?
            } else {
                let passed_tail = build_tree_ft(db, scrutinee, &passed, path_types, fallthrough)?;
                build_tree_ft(db, scrutinee, matched_rows, path_types, Some(&passed_tail))?
            };
            let refined = refine_listlen_else_rows(else_rows, &lit_path, len, at_least);
            let els = build_tree_ft(db, scrutinee, &refined, path_types, fallthrough)?;
            (then_, els)
        }
        crate::core::Probe::Bool(b) => {
            // Refining: matched arm sees the real tail; els is refined. (No sharing — a Bool test has only
            // 2 values, so its fan-out is capped and the exponential is small; the passed-world share is
            // deferred, unlike the unbounded-length ListLen case above.)
            let mut matched = matched_rows.to_vec();
            matched.extend_from_slice(else_rows);
            let then_ = build_tree_ft(db, scrutinee, &matched, path_types, fallthrough)?;
            let refined = refine_bool_else_rows(db, else_rows, &lit_path, b);
            let els = build_tree_ft(db, scrutinee, &refined, path_types, fallthrough)?;
            (then_, els)
        }
        _ => {
            // Non-refining (Int/Str): compile the fall-through ONCE and SHARE it across `then_` and `els`.
            // `tail` is the shared `Rc<SumCont>`; threading it as the matched arm's `fallthrough` makes the
            // arm's terminus return this SAME Rc (see `build_tree_ft`'s empty-matrix arm) — so `els` and
            // `then_`'s deepest terminus are `Rc::ptr_eq`, the DAG the emit-side dedup relies on.
            let tail = build_tree_ft(db, scrutinee, else_rows, path_types, fallthrough)?;
            let then_ = build_tree_ft(db, scrutinee, matched_rows, path_types, Some(&tail))?;
            (then_, tail)
        }
    };
    Ok(std::rc::Rc::new(crate::core::SumCont::LitTest {
        // The emitted node carries the shared `Rc<[PathStep]>` (`MatchPath`) directly — a refcount bump.
        path: lit_path,
        probe,
        then_,
        els,
    }))
}

/// Refine `else_rows` for the ELSE branch of a `Bool(tested)` test at `lit_path`: in that branch the
/// sub-value at `lit_path` is known to be `!tested`. For each row, look at its lit-test (if any) at
/// `lit_path`: a `Bool(!tested)` test there is now SATISFIED — drop it (the row matches this path
/// unconditionally); a `Bool(tested)` test there is UNSATISFIABLE — the row is dead, drop it entirely; any
/// other row (no lit-test at `lit_path`, or a non-bool test) passes through unchanged. This is the finite-
/// type refinement that makes a `true`+`false` cover exhaustive without a `_`.
pub(super) fn refine_bool_else_rows(
    db: &Db,
    else_rows: &[MatchRow],
    lit_path: &[crate::core::PathStep],
    tested: bool,
) -> Vec<MatchRow> {
    let _ = db;
    let mut out = Vec::with_capacity(else_rows.len());
    'rows: for row in else_rows {
        let mut kept: Vec<(std::rc::Rc<[crate::core::PathStep]>, crate::core::Probe)> =
            Vec::with_capacity(row.lit_tests.len());
        for (p, probe) in &row.lit_tests {
            if p.as_ref() == lit_path
                && let crate::core::Probe::Bool(rb) = probe
            {
                if *rb == tested {
                    continue 'rows; // tests `Bool(tested)` at this path — impossible in the `!tested` else
                }
                // tests `Bool(!tested)` — satisfied in this else; drop the test.
                continue;
            }
            kept.push((p.clone(), probe.clone()));
        }
        out.push(MatchRow {
            constraints: row.constraints.clone(),
            lit_tests: kept,
            body: row.body,
            guard: row.guard,
        });
    }
    out
}

/// Refine `else_rows` for the ELSE branch of a `ListLen{tested_len, tested_at_least}` test at `lit_path`
/// that FAILED: in that branch the list at `lit_path` is known NOT to satisfy the tested length. Unlike a
/// bool (a single complementary value), a list length ranges over ℕ, so the failed test leaves a RESIDUAL
/// length set. For each else row's own `ListLen` test at `lit_path`, if the residual is entirely CONTAINED
/// in that test's matched set, the test is guaranteed to hold → drop it (the row matches this path
/// unconditionally). This is the `ListLen` analogue of `refine_bool_else_rows` — it makes a set of same-path
/// list-length arms that jointly PARTITION ℕ exhaustive without a `_`: e.g. `(list)` [len == 0] followed by
/// `(list x .. r)` [len ≥ 1] — the else of the `== 0` test is "len ≥ 1", exactly the second arm's matched
/// set, so its length test drops and it becomes an unconditional leaf (the nested-payload / decision-tree
/// twin of Inc-23's list-of-bools saturation). SOUND: fires ONLY when the residual is provably a SUBSET of
/// the row's matched set, so a genuinely-uncovered length still reaches the empty fall-through → CDZ0210.
///
/// The RESIDUAL after failing `ListLen{tlen, t_at_least}` (an exact-test failure excludes one point; a
/// rest-test failure leaves a finite prefix):
///
/// - exact test `{tlen, false}` failed → residual = all lengths EXCEPT `tlen`.
/// - rest test `{tlen, true}` failed (len ≥ tlen was false) → residual = lengths `0..tlen` (a finite set).
///
/// A row test `{rlen, r_at_least}` MATCHES `{ len : r_at_least ? len ≥ rlen : len == rlen }`. The residual ⊆
/// that matched set is decidable with small interval logic (below); when the residual is the finite prefix
/// `0..tlen`, membership is a bounded check over that prefix.
pub(super) fn refine_listlen_else_rows(
    else_rows: &[MatchRow],
    lit_path: &[crate::core::PathStep],
    tested_len: usize,
    tested_at_least: bool,
) -> Vec<MatchRow> {
    // Does the residual after failing the tested probe lie entirely within the row test's matched set?
    let row_test_covers_residual = |rlen: usize, r_at_least: bool| -> bool {
        if tested_at_least {
            // Residual = finite prefix { 0, 1, …, tested_len-1 } (len ≥ tested_len was ruled out).
            // Every such n must satisfy the row test. A rest test `{rlen, true}` matches n ≥ rlen; an exact
            // test `{rlen, false}` matches only n == rlen. The prefix is covered iff EVERY n in 0..tested_len
            // is matched — a bounded check (tested_len is a source-written arity, small).
            (0..tested_len).all(|n| if r_at_least { n >= rlen } else { n == rlen })
        } else {
            // Residual = all lengths EXCEPT the single point `tested_len` (an exact test failed) = ℕ \
            // {tested_len}. An EXACT row test `{rlen, false}` matches ONE point — it can't contain an
            // infinite residual → never. An AT-LEAST row test `{rlen, true}` matches { n : n ≥ rlen }; the
            // points it MISSES are `{0, …, rlen-1}`. The residual ⊆ matched iff every missed point is NOT in
            // the residual, i.e. `{0..rlen-1} ⊆ {tested_len}` (the sole excluded residual point). That holds
            // iff the missed prefix is EMPTY (`rlen == 0`, matches all) OR is exactly that single point
            // (`rlen == 1` AND `tested_len == 0`) — the canonical `(list) [==0]` else covered by
            // `(list x .. r) [≥1]`. A wider missed prefix leaves a genuinely-uncovered length → decline.
            r_at_least && (rlen == 0 || (rlen == 1 && tested_len == 0))
        }
    };
    let mut out = Vec::with_capacity(else_rows.len());
    for row in else_rows {
        let mut kept: Vec<(std::rc::Rc<[crate::core::PathStep]>, crate::core::Probe)> =
            Vec::with_capacity(row.lit_tests.len());
        for (p, probe) in &row.lit_tests {
            if p.as_ref() == lit_path
                && let crate::core::Probe::ListLen { len, at_least } = probe
                && row_test_covers_residual(*len, *at_least)
            {
                // The residual guarantees this length test holds → drop it (row unconditional at this path).
                continue;
            }
            kept.push((p.clone(), probe.clone()));
        }
        out.push(MatchRow {
            constraints: row.constraints.clone(),
            lit_tests: kept,
            body: row.body,
            guard: row.guard,
        });
    }
    out
}

/// Refine `else_rows` for the SUCCEEDING (`then_`) branch of a `ListLen{tested_len, tested_at_least}` test
/// at `lit_path` — the sub-value there is now known to satisfy the tested length. This is the DUAL of
/// [`refine_listlen_else_rows`] (which refines the FAILED else): a row whose own `ListLen` test at `lit_path`
/// is INCONSISTENT with the passed length can never match in this world and is DROPPED entirely; a row whose
/// test is GUARANTEED by the passed length has that test satisfied → drop the test (leaving the row's deeper
/// tests). It exists to KILL an O(2^arms) blow-up (fire #28): a match arm refining a LIST payload by literal
/// elements (`(Some (list 0 0))`) lowers via [`build_lit_test`] to a `ListLen` probe then per-element
/// `LitTest`s; the ListLen `then_` (this world) is the matched arm's further element-tests, whose fall-through
/// on a mismatch is exactly these remaining SAME-LENGTH arms — so compiling this refined tail ONCE and
/// threading it as the arm's `fallthrough` (S1's mechanism, refined tail) makes the element-test chain reuse
/// it instead of re-compiling the whole `else_rows` matrix per element = O(arms).
///
/// The PASSED SET after `ListLen{tlen, t_at_least}` succeeds: `{ len : t_at_least ? len ≥ tlen : len == tlen }`.
/// A row test `{rlen, r_at_least}` MATCHES `{ len : r_at_least ? len ≥ rlen : len == rlen }`. For each row test
/// at `lit_path` we decide, over the PASSED set: DROP the whole row (unreachable here) iff the passed set
/// and the row's matched set are DISJOINT; DROP the test (guaranteed here) iff the passed set is a SUBSET of
/// the row's matched set; else KEEP the test verbatim (it still discriminates within the passed set).
///
/// Conservative on the exact/at-least interval logic — a wrong verdict is only ever "keep the test" (never a
/// spurious drop/reachability change), so exhaustiveness is preserved (the emitted tree is behavior-identical
/// to the un-refined `[matched] ++ else_rows`, just without the exponential re-compile).
pub(super) fn refine_listlen_to_passed(
    else_rows: &[MatchRow],
    lit_path: &[crate::core::PathStep],
    tested_len: usize,
    tested_at_least: bool,
) -> Vec<MatchRow> {
    // The passed set is `P = { n : tested_at_least ? n ≥ tested_len : n == tested_len }`. Classify a row's
    // own `ListLen{rlen, r_at_least}` test (matched set `R`) against `P`.
    #[derive(PartialEq)]
    enum Verdict {
        DropRow,  // P ∩ R = ∅ → this row can't match in the passed world
        DropTest, // P ⊆ R → the row's test is guaranteed; drop it (keep the row's deeper tests)
        KeepTest, // otherwise → the test still discriminates; keep it verbatim
    }
    let classify = |rlen: usize, r_at_least: bool| -> Verdict {
        // Membership of a point `n` in R.
        let in_r = |n: usize| if r_at_least { n >= rlen } else { n == rlen };
        if tested_at_least {
            // P = { n ≥ tested_len } (infinite). P ⊆ R iff R also contains all n ≥ tested_len: an exact R
            // ({rlen} — one point) can't → not ⊆; a rest R {≥ rlen} ⊆ holds iff rlen ≤ tested_len.
            // P ∩ R = ∅ iff no n ≥ tested_len is in R: an exact R disjoint iff rlen < tested_len; a rest R
            // {≥ rlen} always overlaps P (both are up-rays) → never disjoint.
            if r_at_least {
                if rlen <= tested_len {
                    Verdict::DropTest
                } else {
                    Verdict::KeepTest // rlen > tested_len: R ⊊ P, the test still discriminates
                }
            } else if rlen < tested_len {
                Verdict::DropRow // exact rlen below the passed floor — unreachable
            } else {
                Verdict::KeepTest // exact rlen ≥ tested_len: a single point inside P, still discriminates
            }
        } else {
            // P = { tested_len } (one point). Membership decides everything.
            if in_r(tested_len) {
                Verdict::DropTest // the sole passed length satisfies R → test guaranteed
            } else {
                Verdict::DropRow // the sole passed length fails R → row unreachable here
            }
        }
    };
    let mut out = Vec::with_capacity(else_rows.len());
    'rows: for row in else_rows {
        let mut kept: Vec<(std::rc::Rc<[crate::core::PathStep]>, crate::core::Probe)> =
            Vec::with_capacity(row.lit_tests.len());
        for (p, probe) in &row.lit_tests {
            if p.as_ref() == lit_path
                && let crate::core::Probe::ListLen { len, at_least } = probe
            {
                match classify(*len, *at_least) {
                    Verdict::DropRow => continue 'rows, // row can't match in the passed world
                    Verdict::DropTest => continue, // test guaranteed → drop it, keep deeper tests
                    Verdict::KeepTest => {}        // fall through to push it verbatim
                }
            }
            kept.push((p.clone(), probe.clone()));
        }
        out.push(MatchRow {
            constraints: row.constraints.clone(),
            lit_tests: kept,
            body: row.body,
            guard: row.guard,
        });
    }
    out
}

/// The solved TYPE of the sub-value at `path` from `scrutinee`, computed by walking the scrutinee's own
/// type: an `Elem(i)` step indexes a `Ty::Tuple`'s i-th element; a `Payload` step descends a sum
/// variant's payload (via the head recorded... but a raw type-walk cannot know WHICH variant a `Payload`
/// step refers to, so `Payload` is only resolvable through `extend_path_types`' instantiation — this
/// fallback handles the `Elem`-only paths a TUPLE-scrutinee match produces, where `path_types` was not
/// seeded). Returns `None` for a `Payload` step (deferred to `path_types`) or an out-of-range/ill-typed
/// index. Used as the fallback when `path_types` has no entry — a sum nested in a tuple element.
pub(super) fn type_at_path(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
) -> Option<crate::ty::Ty> {
    // A LEADING `Elem(i)` over a scrutinee that is a TUPLE CONSTRUCTOR — `(match (tuple (fold a) (fold b))
    // …)` — types element `i` DIRECTLY from the constructor rather than from the tuple's aggregate
    // `type_of`. `type_of((tuple (fold a) (fold b)))` types each element in AGGREGATE, where a RECURSIVE
    // call `(fold a)` (during `fold`'s own lowering) reads `Any` (the recursion guard), giving `(Tuple Any
    // Any)` → a non-sum decline at the switch. Typing the element occurrence on its OWN reaches
    // `apply_type`'s recursive-callee `def_scheme` fallback (`fold : E → E`), so `Elem(0)` resolves to `E`.
    // Only the leading `Elem` steps are peeled structurally; the rest fall through to the type-walk below.
    let mut cur = if let Some(&crate::core::PathStep::Elem(i)) = path.first()
        && let Some(elems) = tuple_constructor_elems(db, scrutinee)
        && let Some(&elem_occ) = elems.get(i)
    {
        // Descend the remaining path from this element occurrence (recurse, so a NESTED tuple constructor
        // element resolves too), then RETURN — the leading `Elem(i)` is consumed.
        return type_at_path(db, elem_occ, &path[1..]);
    } else {
        crate::infer::type_of(db, scrutinee)
    };
    for step in path {
        cur = match step {
            crate::core::PathStep::Elem(i) => match &cur {
                crate::ty::Ty::Tuple(elems) => elems.get(*i)?.clone(),
                // A LIST element (a `(list …)` payload sub-pattern) — every element has the list's one
                // element type (homogeneous), so `Elem(i)` over a `Ty::List(e)` is `e` for any `i`.
                crate::ty::Ty::List(elem) => (**elem).clone(),
                _ => return None,
            },
            // A rest sublist is the same `List` type as its scrutinee.
            crate::core::PathStep::RestFrom(_) => match &cur {
                crate::ty::Ty::List(_) => cur.clone(),
                _ => return None,
            },
            // A tuple rest binder — the trailing sub-tuple `(Tuple T_k …)`.
            crate::core::PathStep::TupleRestFrom(k) => match &cur {
                crate::ty::Ty::Tuple(elems) => {
                    crate::ty::Ty::Tuple(elems.get(*k..)?.to_vec().into())
                }
                _ => return None,
            },
            crate::core::PathStep::Payload => match &cur {
                // A `Payload` step over a NOMINAL NEWTYPE UNWRAPS the tag to its underlying type (a
                // runtime no-op). A newtype imposes NO discriminant constraint, so its `Payload` step is
                // NOT seeded in `path_types` by a variant descent — a raw type-walk must resolve it here,
                // or a switch on a sub-value INSIDE an erased newtype's payload (`(Outer.Wrap (tuple
                // (Inner.A v) k))` — switch path `[Payload, Elem(0)]` on `Inner`) has no solved type.
                crate::ty::Ty::Nominal { inner, .. } => (**inner).clone(),
                // A BOXED-sum `Payload` step's target type needs the variant instantiation
                // (`extend_path_types` seeds it in `path_types`); a raw type-walk cannot supply it.
                _ => return None,
            },
        };
    }
    Some(cur)
}

/// Resolve `path`'s type by walking its SUFFIX from the longest PREFIX present in `path_types`. Used
/// when a raw scrutinee type-walk can't cross a boxed-sum `Payload` step but `path_types` seeded a
/// prefix (e.g. `[Payload]` = a variant's payload type): the remaining `Elem` steps then walk the seeded
/// type structurally. Only `Elem` suffix steps are walked (over a `Tuple`/`List`); a further `Payload`
/// in the suffix is a nested boxed sum a plain type-walk can't cross → `None` (declines, as before).
pub(super) fn type_from_seeded_prefix(
    path_types: &PathTypes,
    path: &[crate::core::PathStep],
) -> Option<crate::ty::Ty> {
    // Longest seeded prefix (try the full path down to the empty prefix).
    for cut in (0..path.len()).rev() {
        if let Some(base) = path_types.get(&path[..cut].to_vec()) {
            let mut cur: crate::ty::Ty = (**base).clone();
            for step in &path[cut..] {
                cur = match step {
                    crate::core::PathStep::Elem(i) => match &cur {
                        crate::ty::Ty::Tuple(elems) => elems.get(*i)?.clone(),
                        crate::ty::Ty::List(elem) => (**elem).clone(),
                        _ => return None,
                    },
                    // A rest sublist `.. rest` has the SAME `List` type as its scrutinee (the tail is a
                    // list of the same element type).
                    crate::core::PathStep::RestFrom(_) => match &cur {
                        crate::ty::Ty::List(_) => cur.clone(),
                        _ => return None,
                    },
                    // A tuple rest binder — the trailing sub-tuple `(Tuple T_k …)`.
                    crate::core::PathStep::TupleRestFrom(k) => match &cur {
                        crate::ty::Ty::Tuple(elems) => {
                            crate::ty::Ty::Tuple(elems.get(*k..)?.to_vec().into())
                        }
                        _ => return None,
                    },
                    // A `Payload` step over a NOMINAL NEWTYPE UNWRAPS the tag to its underlying type (a
                    // runtime no-op) — a newtype imposes no discriminant, so its `Payload` is NOT seeded in
                    // `path_types`; peel it here so a switch NESTED inside an erased newtype's payload
                    // resolves. `(Ty.TyInt (IntTy.IntTy (Sign.Signed) w))` switches on `Sign` at
                    // `[Payload, Payload, Elem(0)]` off the seeded `[Payload]` = `IntTy` (a `Ty::Nominal`
                    // single-variant newtype): peel the nominal to `(Tuple Sign Width)`, then `Elem(0)` =
                    // `Sign`. Mirrors `type_at_path`'s `Payload` arm. A `Payload` over a REAL boxed sum still
                    // declines (its instantiation isn't recoverable from a plain type-walk).
                    crate::core::PathStep::Payload => match &cur {
                        crate::ty::Ty::Nominal { inner, .. } => (**inner).clone(),
                        _ => return None,
                    },
                };
            }
            return Some(cur);
        }
    }
    None
}

/// The path-type ADDITIONS the arm switching on variant `disc` at `switch_path` introduces (a sum of type
/// `sub_ty`, declaration `decl`): the sub-value at `switch_path + [Payload]` has the type of THAT variant's
/// payload at `sub_ty`'s instantiation. Read via the variant's constructor record (its `(meta t)` scheme
/// unified against `sub_ty`), so a generic sum's payload is instantiated (`Ok`'s payload in `Result Int
/// Str` is `Int`). A nullary variant has no payload — no additions.
///
/// Returns just the NEW `(path, type)` entries rather than a whole extended map: `build_tree` INSERTS them
/// into the shared `path_types`, recurses, then RESTORES the prior state (see `build_tree`'s arm loop). The
/// old code cloned the entire map here per arm; since the map grows one deeper key per nesting level, a
/// deeply-nested pattern (`(Some (Some … x))`) re-cloned the O(depth)-entry map at every one of `depth`
/// levels = O(depth³). Scoped insert/restore over a shared map drops that whole factor.
pub(super) fn path_type_additions(
    db: &mut Db,
    switch_path: &[crate::core::PathStep],
    sub_ty: &crate::ty::Ty,
    decl: StructId,
    disc: u32,
) -> Vec<PathTypeEntry> {
    let mut out = Vec::new();
    // The variant's constructor occurrence — cached on the variant at synthesis time (O(1)), rather than
    // re-scanning the sum record's variant fields by name per arm (that was O(V) per arm → O(V²) overall).
    // It carries the `(meta t)` scheme `payload_ty_at_instantiation` reads. (The declaration name
    // occurrence does not resolve to a scheme; the synthesized ctor field does.)
    let ctor = db
        .type_decl_by_occ(decl)
        .and_then(|t| t.variants.get(disc as usize))
        .and_then(|v| v.ctor);
    if let Some(ctor) = ctor
        && let Some(payload_ty) = crate::infer::payload_ty_at_instantiation(db, ctor, sub_ty)
    {
        let mut child = switch_path.to_vec();
        child.push(crate::core::PathStep::Payload);
        // A MULTI-payload variant's payload is a `Ty::Tuple` (its payloads boxed as one tuple handle);
        // also register each tuple ELEMENT's type at `switch_path + [Payload, Elem(i)]` so a nested switch
        // (a variant pattern in a payload position — `(Cons h (Cons h2 rest))`) resolves its sub-value's
        // type. A single-payload variant's payload is registered at `[Payload]` alone, unchanged.
        if let crate::ty::Ty::Tuple(elems) = &payload_ty {
            for (i, elem_ty) in elems.iter().enumerate() {
                let mut elem_path = child.clone();
                elem_path.push(crate::core::PathStep::Elem(i));
                out.push((elem_path, std::rc::Rc::new(elem_ty.clone())));
            }
        }
        out.push((child, std::rc::Rc::new(payload_ty)));
    }
    out
}

/// The shallowest (shortest, then by `path_cmp`) path any row constrains — the switch site. Returns a
/// SHARED `Rc<[PathStep]>` (a pointer-bump clone of the winner), not a fresh `Vec`: the old code cloned
/// EVERY path just to `min_by` them, which — since `build_tree` recurses once per pattern level — re-cloned
/// the O(depth)-long constraint paths at every level = O(depth³). Selecting by reference + returning the
/// winner's `Rc` makes each call an O(constraints × path-len) COMPARE with no path deep-copy.
pub(super) fn shallowest_path(rows: &[MatchRow]) -> MatchPath {
    rows.iter()
        .flat_map(|r| r.constraints.iter().map(|(p, _)| p))
        .min_by(|a, b| a.len().cmp(&b.len()).then_with(|| path_cmp(a, b)))
        .cloned()
        .unwrap_or_else(|| std::rc::Rc::from(&[][..]))
}

/// A total order on paths for a deterministic switch choice (Payload < Elem < RestFrom < TupleRestFrom,
/// each by index). A rest step (`RestFrom`/`TupleRestFrom`) never appears in a SUM decision-tree switch
/// path (only a rest binder's own path, which does not go through `MatchRow`), but the ordering is total
/// so the comparator stays well-defined.
pub(super) fn path_cmp(
    a: &[crate::core::PathStep],
    b: &[crate::core::PathStep],
) -> std::cmp::Ordering {
    use crate::core::PathStep;
    // A rank + inner index gives a total order across all step kinds in one comparison.
    fn key(s: &PathStep) -> (u8, usize) {
        match s {
            PathStep::Payload => (0, 0),
            PathStep::Elem(i) => (1, *i),
            PathStep::RestFrom(k) => (2, *k),
            PathStep::TupleRestFrom(k) => (3, *k),
        }
    }
    for (x, y) in a.iter().zip(b.iter()) {
        let o = key(x).cmp(&key(y));
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    a.len().cmp(&b.len())
}

/// Merge an arm's OWN disc rows with the shared DEFAULT rows into one sub-matrix, preserving SOURCE order
/// (arm priority = first-matching-row). Both inputs are `(source_index, row)` already ascending by index
/// (the partition in `build_tree` pushed them in row order), so this is a linear two-way merge — no sort.
/// A default row is cloned into each arm it flows into; `own` rows are moved (each belongs to one arm).
pub(super) fn merge_rows(
    own: Vec<(usize, MatchRow)>,
    defaults: &[(usize, MatchRow)],
) -> Vec<MatchRow> {
    let mut out = Vec::with_capacity(own.len() + defaults.len());
    let mut oi = own.into_iter().peekable();
    let mut di = defaults.iter().peekable();
    loop {
        match (oi.peek(), di.peek()) {
            (Some((oidx, _)), Some((didx, _))) => {
                if oidx <= didx {
                    out.push(oi.next().unwrap().1);
                } else {
                    out.push(di.next().unwrap().1.clone());
                }
            }
            (Some(_), None) => out.push(oi.next().unwrap().1),
            (None, Some(_)) => out.push(di.next().unwrap().1.clone()),
            (None, None) => break,
        }
    }
    out
}

/// The compile-time-constant `Core` at `path` from `scrutinee`, if every step lands in a constant
/// compound (`SumNew` payload / `Tuple` element) — else `None` (a runtime step). Drives the constant fold
/// at each switch. Mirrors `fold_sum_path` but starts from an occurrence and returns the leaf core.
pub(super) fn const_at_path(
    db: &mut Db,
    scrutinee: StructId,
    path: &[crate::core::PathStep],
) -> Option<Core> {
    use crate::core::PathStep;
    let mut cur = scrutinee;
    for step in path {
        // A `Payload` step over a NOMINAL NEWTYPE is a no-op (the box is erased; the underlying value IS
        // `cur`) — see `fold_sum_path`. Leave `cur` unchanged and continue. `type_is_nominal` reads only the
        // type's KIND (no full `Ty` clone) — `const_at_path` runs once per match-tree level over a path that
        // grows with depth, so a per-step deep-`Ty` clone here compounded to O(depth³) on a nested pattern.
        if matches!(step, PathStep::Payload) && crate::infer::type_is_nominal(db, cur) {
            continue;
        }
        // A `Payload` over a MULTI-payload `SumNew` is a no-op landing on the payload tuple; the following
        // `Elem(i)` indexes `payloads[i]` (see `fold_sum_path`/`const_disc_at` — Copilot PR#457).
        if matches!(step, PathStep::Payload)
            && let Core::SumNew { payloads, .. } = core_of(db, cur)
            && payloads.len() > 1
        {
            continue;
        }
        cur = match (step, core_of(db, cur)) {
            (PathStep::Payload, Core::SumNew { payloads, .. }) if payloads.len() == 1 => {
                payloads[0]
            }
            (PathStep::Elem(i), Core::Tuple { elems }) => *elems.get(*i)?,
            // A list-pattern element binder reads position `i` of a CONSTANT list — the same `Elem` step a
            // tuple element uses, over a `Core::ListNew`. A runtime list has no `Core::ListNew` here.
            (PathStep::Elem(i), Core::ListNew { elems }) => *elems.get(*i)?,
            // A MULTI-payload variant's payloads: `Elem(i)` after the `Payload` no-op selects payload `i`.
            (
                PathStep::Elem(i),
                Core::SumNew {
                    payloads: elems, ..
                },
            ) => *elems.get(*i)?,
            // A list-pattern REST binder over a CONSTANT list folds to a fresh `Core::ListNew` of the tail
            // elements (from index `k`) — a synthesized node so the tail sublist is itself constant.
            (PathStep::RestFrom(k), Core::ListNew { elems }) => {
                let tail: Vec<StructId> = elems.iter().skip(*k).copied().collect();
                return Some(Core::ListNew { elems: tail.into() });
            }
            // A tuple-pattern REST binder over a CONSTANT tuple folds to a fresh `Core::Tuple` of the
            // trailing elements (from index `k`) — a synthesized node so the sub-tuple is itself constant.
            (PathStep::TupleRestFrom(k), Core::Tuple { elems }) => {
                let tail: Vec<StructId> = elems.iter().skip(*k).copied().collect();
                return Some(Core::Tuple { elems: tail.into() });
            }
            _ => return None,
        };
    }
    // Return the sub-value's core ONLY when it is an actual compile-time CONSTANT — the kinds the
    // lit-test fold consumes (`const_at_path`'s caller matches `ConstInt`/`ConstBool`/`ConstStr`/
    // `ConstChar`/`ListNew`/`MapNew`). A RUNTIME core here — a `Core::Param`/`LocalRef` reached by walking
    // an INLINE-constructed scrutinee like `(match (tuple a b c) …)` where `a`/`b`/`c` are runtime params,
    // or any other non-constant — must return `None` so the caller takes the RUNTIME lit-test path
    // (`build_lit_test`, which SHARES the fall-through as one `Rc<SumCont>`). Returning `Some(Param)` here
    // wrongly entered the constant-FOLD branch, whose per-arm `matched_rows` construction does NOT thread
    // the shared fall-through — re-folding per column → the O(2^cols) emit blow-up on an inline-tuple
    // multi-column match (the exponent v-wasm-opt's S2 emit-dedup could not touch, because the tree was
    // materialized fully distinct). A bound tuple PARAM scrutinee never hit this: it has no inline
    // `Core::Tuple` to walk into, so `const_at_path` already returned `None` and shared correctly.
    let c = core_of(db, cur);
    if is_foldable_const(&c) { Some(c) } else { None }
}

/// Is this lowered `Core` a compile-time CONSTANT the lit-test fold can decide against (the kinds
/// `const_at_path`'s caller matches)? A runtime `Core::Param`/`LocalRef`/computed value is NOT — it must
/// route to the runtime shared-fall-through lit-test path, not the fold path. (Guards the inline-tuple
/// multi-column exponential: an inline `(tuple <param> …)` scrutinee walks to a `Param` sub-value, which
/// must decline the fold so the runtime path shares the tail.)
pub(super) fn is_foldable_const(c: &Core) -> bool {
    matches!(
        c,
        Core::ConstInt(_)
            | Core::ConstBool(_)
            | Core::ConstStr(_)
            | Core::ConstBytes(_)
            | Core::ConstChar(_)
            | Core::ListNew { .. }
            | Core::MapNew { .. }
            | Core::Tuple { .. }
            | Core::SumNew { .. }
            | Core::BytesOf { .. }
    )
}

/// Classify a match PATTERN occurrence into a [`Probe`], or `None` if it is not a Stage-3 scalar
/// pattern. An integer/boolean literal is a literal probe; a bare NAME (the wildcard `_`, or a BINDER
/// like `k`) always matches — a `Wild` probe. A binder differs from `_` only in scope: a reference to
/// it in the arm body resolves to the scrutinee (handled by `resolve`'s Case 5), so the PROBE is
/// identical (always matches, exhaustive tail). (A constructor / tuple / record pattern is a later
/// increment — it returns `None` here; with no sums yet, every bare name in a scalar match is a binder.)
pub(super) fn classify_probe(db: &mut Db, pat: StructId) -> Option<crate::core::Probe> {
    // A bare name — the wildcard `_` OR a binder — always matches. Detected structurally (before
    // resolving, which would look the name up / poison it); the binding is a scope concern, not a probe.
    if db.ast.as_name(pat).is_some() {
        return Some(crate::core::Probe::Wild);
    }
    match resolved_of(db, pat) {
        Resolved::Int(v) => Some(crate::core::Probe::Int(v)),
        Resolved::Bool(b) => Some(crate::core::Probe::Bool(b)),
        // A STRING-literal pattern (`("hello" …)`). Classified as a `Str` probe — a constant scrutinee
        // folds by value equality, a runtime scrutinee emits a `value-eq` content compare (the Str-probe
        // LitTest).
        Resolved::Str(s) => Some(crate::core::Probe::Str(s)),
        // A SYMBOL-literal pattern (`(#"add" …)`). A symbol SHARES the constant-string representation —
        // its identity is its text, `Resolved::SymbolConst` lowers to the same `Core::ConstStr` a string
        // does (see the `SymbolConst` arm of `core_of`), and `=` on two symbols folds via the shared
        // constant-string equality. So a symbol-literal pattern reuses the SAME `Str` probe: the const-fold
        // compares by text and the runtime path emits the same `value-eq` content compare — a match on
        // `#"add"` dispatches exactly as a match on `"add"` does. This is the head-dispatch idiom over a
        // symbol (`(match tag (#"add" …) (#"sub" …))`), the symbol twin of String-literal patterns.
        Resolved::SymbolConst(s) => Some(crate::core::Probe::Str(s)),
        // A BYTE-STRING-literal pattern (`(b"AB" …)`). Classified as a `Bytes` probe — the Bytes twin of
        // `Str`: a constant scrutinee folds by content equality, and a runtime Bytes scrutinee desugars to a
        // `value-eq` content-compare chain (a Bytes is a heap value, dispatched exactly as a runtime String).
        Resolved::Bytes(bs) => Some(crate::core::Probe::Bytes(bs.into())),
        // A CHAR-literal pattern (`(#\a …)`). Classified as a `Char` probe — a constant scrutinee folds by
        // codepoint equality (`Char` is `Eq`), the last scalar-literal kind to gain a match arm (Int/Bool/
        // Str/Symbol already do). A runtime char has no machine rep yet, so it declines at `is_scalar`
        // before a `Core::Match` is built — the char probe realizes ONLY the constant fold this increment.
        Resolved::Char(c) => Some(crate::core::Probe::Char(c)),
        _ => None,
    }
}

/// Whether a probe matches a constant integer scrutinee (for the fold). A `Wild` matches anything. The
/// literal comparison is BY VALUE (`eq_value`) — a folded `0` (empty magnitude) and a literal `0`
/// (`[0]`) denote the same integer, so struct `==` would wrongly miss (the parity-dispatch bug).
pub(super) fn probe_matches_int(probe: &crate::core::Probe, v: &IntValue) -> bool {
    match probe {
        crate::core::Probe::Int(p) => p.eq_value(v),
        crate::core::Probe::Wild => true,
        crate::core::Probe::Bool(_)
        | crate::core::Probe::Str(_)
        | crate::core::Probe::Char(_)
        | crate::core::Probe::Bytes(_)
        | crate::core::Probe::ListLen { .. }
        | crate::core::Probe::MapHasKeys { .. } => false,
    }
}

/// Whether a probe matches a constant boolean scrutinee (for the fold). A `Wild` matches anything.
pub(super) fn probe_matches_bool(probe: &crate::core::Probe, b: bool) -> bool {
    match probe {
        crate::core::Probe::Bool(p) => *p == b,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_)
        | crate::core::Probe::Str(_)
        | crate::core::Probe::Char(_)
        | crate::core::Probe::Bytes(_)
        | crate::core::Probe::ListLen { .. }
        | crate::core::Probe::MapHasKeys { .. } => false,
    }
}

/// Whether a probe matches a constant string scrutinee (for the fold). A `Wild` matches anything; a
/// string literal matches by VALUE equality (the `ConstStr` scrutinee and pattern are both already NFC-
/// normalized by the reader, so `==` is exact — the same basis as the constant `String` equality fold).
pub(super) fn probe_matches_str(probe: &crate::core::Probe, s: &str) -> bool {
    match probe {
        crate::core::Probe::Str(p) => p == s,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_)
        | crate::core::Probe::Bool(_)
        | crate::core::Probe::Char(_)
        | crate::core::Probe::Bytes(_)
        | crate::core::Probe::ListLen { .. }
        | crate::core::Probe::MapHasKeys { .. } => false,
    }
}

/// Whether a probe matches a constant char scrutinee (for the fold). A `Wild` matches anything; a char
/// literal matches by CODEPOINT equality (`Char` is `Eq` — a scalar Unicode-scalar-value comparison,
/// the same basis as the char `=` fold). The char twin of `probe_matches_str`.
pub(super) fn probe_matches_char(probe: &crate::core::Probe, c: char) -> bool {
    match probe {
        crate::core::Probe::Char(p) => *p == c,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_)
        | crate::core::Probe::Bool(_)
        | crate::core::Probe::Str(_)
        | crate::core::Probe::Bytes(_)
        | crate::core::Probe::ListLen { .. }
        | crate::core::Probe::MapHasKeys { .. } => false,
    }
}

/// Whether a probe matches a constant byte-sequence scrutinee (for the fold). A `Wild` matches anything; a
/// byte-string literal matches by CONTENT equality (raw byte compare — a `Core::ConstBytes` scrutinee and
/// pattern are both flat canonical byte forms, so `==` is exact, the same basis as the constant `Bytes`
/// equality fold). The Bytes twin of `probe_matches_str`.
pub(super) fn probe_matches_bytes(probe: &crate::core::Probe, b: &[u8]) -> bool {
    match probe {
        crate::core::Probe::Bytes(p) => **p == *b,
        crate::core::Probe::Wild => true,
        crate::core::Probe::Int(_)
        | crate::core::Probe::Bool(_)
        | crate::core::Probe::Str(_)
        | crate::core::Probe::Char(_)
        | crate::core::Probe::ListLen { .. }
        | crate::core::Probe::MapHasKeys { .. } => false,
    }
}

/// Whether an application HEAD is a RUNTIME function-value source that must apply via `call_indirect`
/// (a `Core::CallClosure`), rather than β-reduce at compile time — a `Param`, or a PATTERN BINDER reading
/// a runtime value out of a compound (a sum-variant payload `(match t ((T.Mk f) (f x)))`, or a
/// tuple/record element `(match t ((tuple f _) (f x)))`, which resolve to `SumPayload`/`Proj`). A
/// `Param` is always runtime. A `SumPayload`/`Proj` is runtime ONLY when the fold cannot reach the stored
/// lambda: over a CONSTANT compound the projection β-reduces to the lambda (`lambda_body` sees it) and
/// must fold — the runtime path is taken solely when `lambda_body` is `None` (a genuinely heap-held
/// closure). So this is checked AFTER the lambda-reduction attempt would have fired for a foldable head.
pub(super) fn head_is_runtime_fn_value(db: &mut Db, id: StructId) -> bool {
    // A CAPTURED free variable that is a fn value — a lifted closure body applies a closure it CAPTURED
    // (`(fn (x) (f x))` where `f` is captured from an enclosing scope). Inside the lifted body `f` is a
    // runtime closure HANDLE read from the env cell (`Core::Captured`), NOT the compile-time lambda it was
    // defined from — so it must apply via `call_indirect`, not β-reduce. Checked FIRST: without this the
    // `Ref` arm below follows `f` through to its original `(fn …)` definition and reports NOT-runtime, so
    // `(f x)` mis-lowered — `f`'s handle was read as a scalar and ADDED to `x` instead of called (a
    // miscompile of a closure that captures another capturing closure).
    if db.captured_ref.contains_key(&id) {
        return true;
    }
    match resolved_of(db, id) {
        Resolved::Param { .. } => true,
        // A KEPT `let`-binding that holds a runtime closure — the adv-50 force-keep (`should_keep_binding`
        // keeps a CAPTURING lambda whose handle both escapes-whole and is direct-called). The binding was
        // materialized as ONE `Core::Closure` in a `Core::Let` slot; its reference lowers to a
        // `Core::LocalRef` reading that cell, so an application of it must `call_indirect` the cell, NOT
        // β-fold the lambda (folding is exactly the poison the force-keep avoids). Checked BEFORE the
        // `Ref`-follow below, which would otherwise chase `value` through to the original `(fn …)` and
        // report NOT-runtime (β-reduce). Only a KEPT binding diverts here — an ordinary copy-propagated
        // lambda binding still follows through and folds, unchanged.
        Resolved::Ref { value } if db.kept_bindings.contains(&value) => true,
        Resolved::Ref { value } => head_is_runtime_fn_value(db, value),
        // A CALL that RETURNS a closure — a head `(selfp n)` where `selfp` is a RECURSIVE def whose result
        // is a function value (`(def (selfp n) (if (= n 0) (fn (x) …) (selfp (- n 1))))`), applied
        // `((selfp n) 5)`. The call CANNOT β-reduce (a recursive callee has no compile-time-visible lambda
        // body — `lambda_body` is `None`, and its reduction hits the depth guard), so its result is a
        // runtime closure HANDLE that must apply via `call_indirect`. Without this the head fell through to
        // the "value is not applyable" decline (a false reject of the factory-selected-by-recursion idiom).
        // Gated on (1) the head being a genuine APPLICATION that does NOT reduce to a lambda (a
        // NON-recursive closure-returner `((pick b) 5)` DOES reduce — via case-of-if / β-reduction — and
        // keeps that path, never reaching here), and (2) its result type being `Ty::Fn` (an ordinary
        // data-returning call is not a runtime fn value). The emit materializes the call's returned handle
        // as the closure cell (`Core::CallClosure`'s `emit(closure)` handles a `Core::Call` operand).
        Resolved::Apply { .. }
            if crate::eval::lambda_body(db, id).is_none()
                && matches!(crate::infer::type_of(db, id), crate::ty::Ty::Fn(_, _)) =>
        {
            true
        }
        // A payload/element binder — runtime iff the fold can't reduce it to a lambda (a constant compound
        // folds through the projection; a runtime one does not, so its stored closure applies indirect).
        Resolved::SumPayload { .. } | Resolved::Proj { .. } | Resolved::RecordField { .. } => {
            crate::eval::lambda_body(db, id).is_none()
        }
        // A record-field projection `(. rec f)` whose field TYPE is a function — the record-field analogue
        // of `Proj`'s tuple-element. When `rec` is a RUNTIME record (e.g. bound as a sum-match payload,
        // `(match h ((H.M rec) ((. rec f) x)))`) the fn field cannot fold to its lambda, so it is a runtime
        // closure handle read from the value heap and applied via `call_indirect`. FOUR gates keep this
        // from diverting things that already work: (1) it carries NO `(meta apply)` — a prelude OPERATION
        // reached by member syntax (`(. Bytes at)`, `Map.insert`) is an operator/type-builder with its own
        // prim path, NOT a runtime closure (diverting them broke every `Bytes.at`/`List.at`/… op); (2) it
        // is NOT a variant constructor — `(. Shape Rect)` is a `Member` of arrow type reached by its
        // `Prim::SumNew` path; (3) the field type is `Ty::Fn` — an ordinary DATA field read (`rec.n`) stays
        // on its folding path; (4) it does NOT reduce to a lambda — a constant record's fn field folds and
        // β-reduces. Only a genuine fn-typed field of a RUNTIME record (no prim, no ctor, no fold) is a
        // runtime closure handle.
        Resolved::Member { .. } => {
            crate::eval::meta_apply_of(db, id).is_none()
                && crate::eval::variant_disc_of(db, id).is_none()
                && matches!(crate::infer::type_of(db, id), crate::ty::Ty::Fn(_, _))
                && crate::eval::lambda_body(db, id).is_none()
        }
        _ => false,
    }
}
