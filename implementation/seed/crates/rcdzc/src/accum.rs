//! ACCUMULATOR INTRODUCTION — turn a linear NON-tail recursion into a TAIL recursion (which the
//! `select` loop transform then compiles to a `loop`, so it runs in constant stack).
//!
//! The classic shape (the guide's `sm`):
//!
//! ```text
//! (def (f n) (if (= n 0) 0 (+ n (f (- n 1)))))
//! ```
//!
//! Here the recursive call `(f (- n 1))` sits in an OPERAND of `+`, not tail position, so it compiles
//! to a real `call` that grows the stack (`sm 100000` stack-overflows today). But `+` is ASSOCIATIVE
//! with identity `0`, so the sum can be reassociated to a left fold that IS tail-recursive:
//!
//! ```text
//! (def (f$acc n acc) (if (= n 0) acc (f$acc (- n 1) (+ acc n))))   ; tail — loops
//! (def (f n) (f$acc n 0))                                          ; f seeds the accumulator
//! ```
//!
//! The transform SYNTHESIZES the accumulator def as fresh AST (via `Arenas::list`/`name`/`atom_leaf`),
//! appends it to `defs`, and rewrites `f`'s body to the seed call. Fresh AST resolves through the
//! ordinary scope-walk (each synthesized name binds to the nearest enclosing synthesized binder, then
//! to the new def by name) — so it never hits the re-resolution scope-corruption trap that reusing
//! existing occurrences under `resolve_subtree` would.
//!
//! ## Scope
//! The LINEAR self-recursion `f p… = (if COND base-or-combine …)` where:
//!  - `OP` is an ASSOCIATIVE binary op with a known identity — the checked `+`/`*` OR the two's-complement
//!    wrapping ops spelled as a member access `(. T wrapping-add)`/`(. T wrapping-mul)` (identity `0`/`1`);
//!  - one `if` branch is the base value (`= OP`'s identity), the other the combine `(OP g (f REC…))`;
//!  - exactly ONE self-recursive call, in one operand of `OP`; the OTHER operand `g` does not itself
//!    recurse (it is the per-step term folded into the accumulator);
//!  - EITHER branch ordering: the guide's `(if (= n 0) base combine)` (base in THEN) OR a FLIPPED
//!    `(if (> n 0) combine base)` (base in ELSE). The condition is reused verbatim; the accumulator
//!    places its branches in the same order so the condition still selects the base case;
//!  - ANY number of parameters. Every self-call argument is threaded through the synthesized accumulator
//!    UNCHANGED (a recursion variable like `n` or a pass-through like a limit/config/closure `k` alike),
//!    and each original binder — annotations and all — carries over so a function-typed parameter still
//!    types.
//!
//! ## Known limitation: SELF-recursion only, not MUTUAL recursion (a defined trap, by design)
//! This transform matches a SELF-recursive def — the combine's single recursive call names the def's OWN
//! name. A MUTUALLY-recursive linear fold — `suma n = (+ n (sumb (- n 1)))` and `sumb n = (+ n (suma (- n
//! 1)))` — is NOT rewritten, so it compiles to plain recursive calls and STACK-EXHAUSTS at depth (~100k),
//! whereas the self-recursion twin loops in O(1) stack. This is a MISSED OPTIMIZATION, not a correctness
//! bug: stack exhaustion is a DEFINED trap and the value is exact until then. It is left un-rewritten
//! DELIBERATELY (ruled 2026-07-17): the mutual pattern is niche, and a group-wide accumulator transform
//! (detect the mutual-recursion SCC, match each member's combine against its SIBLING, synthesize the whole
//! accumulator group, re-seed every entry — all-or-nothing) is delicate to add to this pass. If real code
//! ever needs it, add a NARROW two-member-cycle version (the backend already handles the target shape:
//! separate per-member loops with mutual tail-calls run in O(1) stack).
//!
//! ## Soundness (the operator's call: checked `+` too, accept a trap-point change)
//! Reassociating a CHECKED `+`/`*` can move WHERE an overflow traps (a partial sum overflows at a
//! different step under a left vs right fold). The transform preserves the FINAL value exactly; only the
//! overflow-trap TIMING for an already-overflowing input may differ. For the all-non-negative sums the
//! guide shows, the partials are monotonic and the trap point coincides. The `(. T wrapping-add)`/
//! `wrapping-mul` variants never trap, so their reassociation is fully transparent — value-exact with no
//! trap-timing caveat at all.

use crate::ast::{Arenas, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::Def;
use crate::prelude::{push_atom, push_list};

/// Append a bare `Name` atom occurrence — the synthesis workhorse (a synthesized reference/binder).
fn push_name(ast: &mut Arenas, name: &str) -> StructId {
    push_atom(ast, Leaf::Name(name.into()))
}

/// Run accumulator introduction over the module's `defs`, mutating `ast` (appending synthesized nodes)
/// and `defs` (rewriting a matched def's body to the seed call + appending its accumulator def). Called
/// at load, after `scan_top_level` and BEFORE the parent index / `def_by_name` are built, so the
/// synthesized def is indexed and resolvable like any other. A def that does not match is left untouched.
/// Rewrite each linear non-tail recursion into a tail-recursive accumulator def, returning the
/// `(source-def-index, accumulator-def-index)` link for every def transformed. The link lets a later
/// consumer (the `Instantiations` query's DISPOSITION report) map a source def to the synthesized copy
/// its recursion actually became — so `fac` reads `transformed→fac$acc` rather than the literal
/// `inlined` (its seed wrapper folds, but the loop is emitted under the copy's name). Empty when no def
/// matches (byte-identical to before — a non-matching def is untouched).
pub(crate) fn introduce(
    ast: &mut Arenas,
    defs: &mut Vec<Def>,
    effect_decls: &[crate::db::EffectDecl],
) -> Vec<(usize, usize)> {
    // Index each top-level `(def sig body)` FORM by its signature occurrence ONCE, up front — an O(items)
    // pass. `match_linear_recursion` needs the enclosing form (to swap its body child), and the parent
    // index is not built yet; a per-def LINEAR scan of the module items (the old `find_def_form`) made
    // this O(defs²) — a module of N defs spent ~50% of the whole compile re-scanning items, each an
    // `as_form(item, "def")` string compare. The map turns that into an O(1) lookup per def.
    let def_forms = index_def_forms(ast);
    // The DECLARED-EFFECT NAMES (built once). Used to reject a per-step term that PERFORMS a discharged
    // effect — reassociating an effectful term changes eval order (see `term_performs_effect`). Built from
    // `effect_decls` (populated by `scan_top_level`, available at the load-time call site) so the syntactic
    // `(. E op)` perform-detection is PRECISE — it fires on a member access whose base names a declared
    // effect, NOT on a pure record/field access `(. r x)` (which would over-decline and regress the stack
    // wins accum exists for). Effect performs at this pre-resolve stage are member accesses off the effect.
    let effect_names: crate::fxhash::FxHashSet<String> =
        effect_decls.iter().map(|e| e.name.clone()).collect();
    // Collect the rewrites first (an immutable scan of `defs`), then apply — so the synthesis (which
    // reads `defs` for name collisions) sees a stable view.
    let mut plans: Vec<(usize, Match)> = Vec::new();
    for (i, d) in defs.iter().enumerate() {
        if let Some(m) = match_linear_recursion(ast, d, &def_forms, &effect_names) {
            plans.push((i, m));
        }
    }
    let mut links = Vec::with_capacity(plans.len());
    for (def_ix, m) in plans {
        // `apply` appends the accumulator def at the current `defs.len()` — capture that index as the
        // source→copy link before the append.
        let acc_ix = defs.len();
        apply(ast, defs, def_ix, m);
        links.push((def_ix, acc_ix));
    }
    links
}

/// A recognized linear-accumulator recursion, with the occurrences the rewrite reuses/reads.
struct Match {
    /// The original `(def sig body)` FORM occurrence — its body child is swapped to the seed call.
    def_form: StructId,
    /// Every parameter's binder NAME, in order (e.g. `[n]` or `[n, k]`). The synthesized accumulator
    /// reuses these spellings so the reused dispatch/`term`/`rec_args` occurrences resolve to it.
    param_names: Vec<String>,
    /// How the original DISPATCHES between the base and combine branches — an `(if …)` on a numeric
    /// recursion, or a `(match xs …)` on a LIST fold. `apply` reconstructs the accumulator's body in the
    /// same shape (reusing the original condition / arm patterns + scrutinee verbatim, since the discarded
    /// original body's occurrences have no other live parent after the seed-call rewrite).
    dispatch: Dispatch,
    /// The associative op's OCCURRENCE — a bare `Name` (`+`/`*`) or a member access `(. T wrapping-add)`.
    /// `apply` CLONES it (via `copy_subtree`) into the accumulator, so either spelling reconstructs
    /// correctly (a bare name rebuild couldn't represent the dotted form).
    op_occ: StructId,
    identity: i64,
    /// The per-step TERM occurrence `g` (the `+`'s non-recursive operand, e.g. `n`, `(* n k)`, or the
    /// list head `x`).
    term: StructId,
    /// The self-call's ARGUMENT occurrences (`(- n 1)`, `k`, `rest`, …), one per parameter — threaded
    /// UNCHANGED into the accumulator's tail self-call. Reassociation preserves the final value for ANY
    /// number of parameters (recursion variables and pass-throughs alike), since `+`/`*` are associative.
    rec_args: Vec<StructId>,
}

/// The branch-selecting form of a recognized recursion — the ONE part that differs between the numeric
/// `if` shape and the list-`match` fold. Everything else (the associative op, term, and recursion args)
/// is shared, since a list fold `(match xs ((list) 0) ((list x .. rest) (+ x (f rest))))` is the SAME
/// linear accumulation as `(if (= n 0) 0 (+ n (f (- n 1))))` — only its base/recursive cases are chosen
/// by matching a list's shape instead of testing a scalar condition.
enum Dispatch {
    /// `(if COND base combine)` (base in THEN — the guide's shape) or `(if COND combine base)` (a FLIPPED
    /// condition, base in ELSE). `cond` is reused verbatim; `base_is_then` records which slot the base
    /// occupied so `apply` places the accumulator's branches in the matching order.
    IfCond { cond: StructId, base_is_then: bool },
    /// A LIST fold `(match SCRUT (empty-pat base) (cons-pat combine))` — SCRUT a bare parameter, one arm
    /// the empty-list pattern `(list)` (body = the op identity), the other a cons pattern `(list … .. rest)`
    /// (body = the combine). `apply` reuses the scrutinee + both arm PATTERNS verbatim, rebuilding only the
    /// arm BODIES (empty → the bare accumulator; cons → the tail self-call). `empty_is_first` records the
    /// original arm order so the reused patterns keep selecting the same cases.
    ListMatch {
        scrut: StructId,
        empty_pat: StructId,
        cons_pat: StructId,
        empty_is_first: bool,
    },
}

/// Match a def against the linear-accumulator shape. Returns `None` (leave the def alone) unless it is
/// either
///  - `(def (f p…) (if COND …))` — a NUMERIC recursion, or
///  - `(def (f p…) (match SCRUT (empty-pat 0) (cons-pat (OP head (f … rest …)))))` — a LIST fold,
///
/// where one branch is `OP`'s identity `ID` and the other is the combine `(OP g (f REC…))` — `OP`
/// associative, the single self-call in one `OP` operand. Either branch/arm ordering is accepted. Any
/// number of parameters is accepted — every self-call argument is threaded through the accumulator
/// unchanged (reassociation is sound whether a parameter is a recursion variable or a pass-through).
fn match_linear_recursion(
    ast: &Arenas,
    d: &Def,
    def_forms: &crate::fxhash::FxHashMap<u32, StructId>,
    effect_names: &crate::fxhash::FxHashSet<String>,
) -> Option<Match> {
    // At least one parameter, each a bare name (an annotated `(: n T)` is fine — take the inner name).
    // Extra parameters (pass-throughs like a limit/config, or a second recursion variable) are threaded
    // through the accumulator unchanged — reassociation is sound regardless of how many params vary.
    if d.params.is_empty() {
        return None;
    }
    let param_names: Vec<String> = d
        .params
        .iter()
        .map(|p| param_binder_name(ast, *p))
        .collect::<Option<_>>()?;
    let body = d.body?;
    // The body is either an `(if …)` (numeric) or a `(match …)` (list fold) — the cheap structural gate
    // FIRST, so a non-matching def (the common case) rejects before any map lookup.
    let (dispatch, op_occ, identity, term, rec_args) =
        match_if_shape(ast, body, &d.name, &param_names)
            .or_else(|| match_list_fold_shape(ast, body, &d.name, &param_names))?;
    // EFFECTFUL-TERM SAFE-FLOOR (concierge/v-effects 14b:13175): decline when the per-step term `g` PERFORMS
    // a discharged effect. Accumulator introduction REASSOCIATES `g` into a left fold, which reorders WHEN
    // `g` runs relative to the recursion; for a PURE `g` that is value-exact (the pass's whole soundness
    // argument), but for an EFFECTFUL `g` — e.g. the abortive `(if (= k 2) (E.bail unit) k)` — the eval-order
    // change is observable, so the reassociation is unsound. Declining here (leaving the def a plain
    // non-tail recursion) removes that LATENT unsoundness and lets the effects pass see the term
    // un-reassociated, so v-effects' non-local-exit CC can fold it correctly. Sound-toward-decline: a false
    // positive only costs the stack optimization on an effectful recursion (a rare shape), never a
    // miscompile. This is a strict correctness improvement over the prior `abortive_perform_off_tail` guard,
    // which bandaged the post-reassociation form. See `term_performs_effect` for the PRECISE syntactic test.
    if term_performs_effect(ast, term, effect_names) {
        return None;
    }
    // Locate the enclosing `(def sig body)` FORM (its body child is swapped to the seed call). The parent
    // index is not built yet at load time, so this is an O(1) read of the prebuilt `sig_occ → form` index
    // (was a per-def linear scan of the module items → O(defs²)). Only reached once every cheaper check
    // has passed, so a non-matching def never even looks it up.
    let def_form = *def_forms.get(&d.sig_occ.0)?;
    Some(Match {
        def_form,
        param_names,
        dispatch,
        op_occ,
        identity,
        term,
        rec_args,
    })
}

/// Match the NUMERIC `(if COND THEN ELSE)` body shape: one branch is the OP identity literal, the other
/// the combine `(OP g (f REC…))`. Either ordering (base in THEN, the guide's shape; or base in ELSE, a
/// flipped condition). Returns the `Dispatch::IfCond` plus the shared combine decomposition, or `None`.
fn match_if_shape(
    ast: &Arenas,
    body: StructId,
    name: &str,
    param_names: &[String],
) -> Option<(Dispatch, StructId, i64, StructId, Vec<StructId>)> {
    let [cond, then_, else_] = ast.as_form(body, "if")? else {
        return None;
    };
    let (cond, then_, else_) = (*cond, *then_, *else_);
    // One branch is the base value (the OP identity), the other the recursive combine `(OP g (f REC…))`.
    // Try ELSE as the combine first (guide shape, base in THEN), then THEN (flipped, base in ELSE).
    let (base_is_then, base_val, op_occ, identity, term, rec_args) =
        if let Some((op, id, t, r)) = match_combine(ast, else_, name, param_names.len()) {
            (true, then_, op, id, t, r)
        } else if let Some((op, id, t, r)) = match_combine(ast, then_, name, param_names.len()) {
            (false, else_, op, id, t, r)
        } else {
            return None;
        };
    // The base value must be the op's identity literal.
    if int_literal(ast, base_val)? != identity {
        return None;
    }
    Some((
        Dispatch::IfCond { cond, base_is_then },
        op_occ,
        identity,
        term,
        rec_args,
    ))
}

/// Match the LIST-FOLD `(match SCRUT (empty-pat BASE) (cons-pat COMBINE))` body shape — the user's `sum`:
///
/// ```text
/// (def (sum xs) (match xs ((list) 0) ((list x .. rest) (+ x (sum rest)))))
/// ```
///
/// SCRUT must be a bare PARAMETER; exactly two arms, one an empty-list pattern `(list)` whose body is the
/// OP identity, the other a cons pattern `(list … .. rest)` whose body is the combine `(OP g (f REC…))`.
/// The self-call must thread the REST binder back through the SCRUTINEE's parameter position and every
/// OTHER argument unchanged — so the accumulator reproduces the original's element sequence exactly. The
/// per-step term `g` is the list HEAD binder (bound by a LEADING element position). Returns the
/// `Dispatch::ListMatch` plus the shared combine decomposition, or `None`.
fn match_list_fold_shape(
    ast: &Arenas,
    body: StructId,
    name: &str,
    param_names: &[String],
) -> Option<(Dispatch, StructId, i64, StructId, Vec<StructId>)> {
    let mtail = ast.as_form(body, "match")?;
    let (scrut, arms) = mtail.split_first()?;
    let scrut = *scrut;
    // Exactly two arms, each a `(pattern body)` pair.
    let [arm0, arm1] = arms else {
        return None;
    };
    let (pat0, body0) = arm_parts(ast, *arm0)?;
    let (pat1, body1) = arm_parts(ast, *arm1)?;
    // The scrutinee must be a bare PARAMETER — its position among the params drives how `rest` is threaded
    // back through the tail self-call.
    let scrut_name = ast.as_name(scrut)?;
    let scrut_ix = param_names.iter().position(|p| p == scrut_name)?;
    // One arm is the empty-list pattern `(list)` (BASE = the op identity); the other is a cons pattern
    // `(list … .. rest)` (COMBINE). Try (empty=arm0, cons=arm1) then flipped — `empty_is_first` records it.
    let ((empty_pat, base_val), (cons_pat, combine), empty_is_first) =
        if list_pattern_is_empty(ast, pat0) {
            ((pat0, body0), (pat1, body1), true)
        } else if list_pattern_is_empty(ast, pat1) {
            ((pat1, body1), (pat0, body0), false)
        } else {
            return None;
        };
    let (op_occ, identity, term, rec_args) = match_combine(ast, combine, name, param_names.len())?;
    // BASE must be the op's identity literal.
    if int_literal(ast, base_val)? != identity {
        return None;
    }
    // SOUNDNESS: the cons pattern must bind a REST sublist `(list … .. rest)`, and the self-call must
    // thread that rest binder back through the SCRUTINEE's position, so the accumulator peels elements in
    // the EXACT SAME order as the original recursion — binding head/rest (and every other param) identically
    // at each depth, and combining the identical per-step terms. That order-preservation is what makes the
    // reassociation to a left fold value-exact; the OTHER self-call arguments may vary freely (threaded
    // unchanged, exactly like the numeric shape's `(- n 1)`), each evaluated at its step's bindings.
    let rest_name = cons_pattern_rest_binder(ast, cons_pat)?;
    if ast.as_name(*rec_args.get(scrut_ix)?)? != rest_name {
        return None;
    }
    Some((
        Dispatch::ListMatch {
            scrut,
            empty_pat,
            cons_pat,
            empty_is_first,
        },
        op_occ,
        identity,
        term,
        rec_args,
    ))
}

/// If `combine` is `(OP g (f REC…))` or `(OP (f REC…) g)` — an ASSOCIATIVE op (with known identity) whose
/// operands are exactly one self-call `(f REC…)` and one non-recursive per-step term `g` — decompose it
/// into `(op_name, identity, term, rec_args)`. Returns `None` for any other shape (not associative, zero
/// or two self-calls, or a term that itself recurses — the last would need multi-way accumulation).
fn match_combine(
    ast: &Arenas,
    combine: StructId,
    name: &str,
    param_count: usize,
) -> Option<(StructId, i64, StructId, Vec<StructId>)> {
    let combine_tail = list_children(ast, combine)?;
    let [op_occ, a, b] = combine_tail.as_slice() else {
        return None;
    };
    let identity = associative_op_identity(ast, *op_occ)?;
    // Exactly one operand is the self-call `(f REC…)` (an application of the def's own name with one
    // argument PER PARAMETER); the other is the per-step term.
    let a_rec = self_call_args(ast, *a, name, param_count);
    let b_rec = self_call_args(ast, *b, name, param_count);
    let (term, rec_args) = match (a_rec, b_rec) {
        (Some(rec), None) if !mentions_name(ast, *b, name) => (*b, rec),
        (None, Some(rec)) if !mentions_name(ast, *a, name) => (*a, rec),
        _ => return None, // zero or two self-calls, or the term also recurses — out of scope.
    };
    // The per-step term and every recursion argument must not smuggle in another self-call.
    if mentions_name(ast, term, name) || rec_args.iter().any(|&r| mentions_name(ast, r, name)) {
        return None;
    }
    Some((*op_occ, identity, term, rec_args))
}

/// Synthesize the accumulator def and rewrite the original def to seed it.
fn apply(ast: &mut Arenas, defs: &mut Vec<Def>, def_ix: usize, m: Match) {
    let orig_name = defs[def_ix].name.clone();
    let acc_name = fresh_acc_name(defs, &orig_name);
    let acc_ix = defs.len();

    // ── Synthesize the FULL `(def (acc_name p… acc) (if BASE-COND acc (acc_name REC… (OP acc TERM))))`
    // FORM node in the arena. Resolution binds a def-body name by walking parents to a `(def sig body)`
    // form (resolve Case 4), so the accumulator must be a real form the reused param-references ascend to.
    // Its binders are the original params (same spellings, so the reused `base_cond`/`term`/`rec_args`
    // occurrences resolve to them) followed by a fresh `acc`.
    let acc_var = "acc$";
    // Clone each ORIGINAL parameter binder — preserving annotations like `(: g (-> Int64 Int64))` — as the
    // accumulator's leading signature binders. A recursive def cannot re-infer a function-typed parameter
    // from its body, so a bare-name rebuild would fail; the annotation must carry over. (The fresh `acc$`
    // stays unannotated — it is grounded from the arithmetic body like any accumulator.)
    let orig_params = defs[def_ix].params.clone();
    let param_binders: Vec<StructId> = orig_params.iter().map(|&p| copy_subtree(ast, p)).collect();
    let acc_binder = push_name(ast, acc_var);
    let mut sig_children = vec![push_name(ast, &acc_name)];
    sig_children.extend(param_binders.iter().copied());
    sig_children.push(acc_binder);

    // The base branch: the bare accumulator (base case returns what we've folded so far).
    let base_ref = push_name(ast, acc_var);

    // The recursive branch: `(acc_name rec_arg… (OP acc term))` — the tail self-call, passing every
    // original recursion argument unchanged plus the folded accumulator.
    // The combine `(OP acc term)`: LEFT-fold order — acc first. Reuse the ORIGINAL `term` occurrence
    // (it references the params / a list-arm binder, which bind to this def's params / the reused arm
    // pattern). CLONE the op occurrence so a member access like `(. Int64 wrapping-add)` reconstructs
    // correctly (not just a bare-name `+`/`*`).
    let op_ref = copy_subtree(ast, m.op_occ);
    let acc_ref_in_op = push_name(ast, acc_var);
    let combined = push_list(ast, vec![op_ref, acc_ref_in_op, m.term]);
    let mut rec_call_children = vec![push_name(ast, &acc_name)];
    rec_call_children.extend(m.rec_args.iter().copied());
    rec_call_children.push(combined);
    let rec_call = push_list(ast, rec_call_children);
    // The accumulator's body reconstructs the ORIGINAL dispatch shape, reusing its condition / arm patterns
    // + scrutinee verbatim, so the same case selection still holds. The base branch returns the bare
    // accumulator; the recursive branch is the tail self-call.
    let acc_body = match m.dispatch {
        // `(if base_cond BASE-OR-REC …)` — reuse the condition, keeping the base/recursive branches in the
        // SAME order the original used (whether `(if (= n 0) base rec)` or `(if (> n 0) rec base)`).
        Dispatch::IfCond { cond, base_is_then } => {
            let if_head = push_name(ast, "if");
            let (then_branch, else_branch) = if base_is_then {
                (base_ref, rec_call)
            } else {
                (rec_call, base_ref)
            };
            push_list(ast, vec![if_head, cond, then_branch, else_branch])
        }
        // `(match scrut (empty-pat acc) (cons-pat (acc_name rest… (OP acc head))))` — reuse the scrutinee
        // and BOTH arm patterns verbatim, replacing only the arm BODIES. The empty arm returns the bare
        // accumulator (the fold's running total); the cons arm is the tail self-call. Arm order matches the
        // original so the reused patterns select the same cases (the head/rest binders the reused `term`
        // and `rec_args` reference resolve to the reused cons pattern, re-parented by the rebuilt index).
        Dispatch::ListMatch {
            scrut,
            empty_pat,
            cons_pat,
            empty_is_first,
        } => {
            let match_head = push_name(ast, "match");
            let empty_arm = push_list(ast, vec![empty_pat, base_ref]);
            let cons_arm = push_list(ast, vec![cons_pat, rec_call]);
            let (arm_first, arm_second) = if empty_is_first {
                (empty_arm, cons_arm)
            } else {
                (cons_arm, empty_arm)
            };
            push_list(ast, vec![match_head, scrut, arm_first, arm_second])
        }
    };
    // Signature `(acc_name p… acc)` and the whole `(def sig acc_body)` form.
    let sig = push_list(ast, sig_children);
    let def_head = push_name(ast, "def");
    let _def_form = push_list(ast, vec![def_head, sig, acc_body]);

    let mut acc_params = param_binders;
    acc_params.push(acc_binder);
    defs.push(Def {
        name: acc_name.clone(),
        sig_occ: sig,
        params: acc_params,
        body: Some(acc_body),
        internal: false,
    });
    debug_assert_eq!(defs.len(), acc_ix + 1);

    // ── Rewrite the original def's body to `(acc_name <param>… <identity>)` ──
    // Reference each original parameter by its NAME (a fresh occurrence that binds to the original def's
    // param via the scope walk), seeding the accumulator with the op's identity.
    let mut seed_children = vec![push_name(ast, &acc_name)];
    for p in &m.param_names {
        seed_children.push(push_name(ast, p));
    }
    let seed_id = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(m.identity),
            radix: Radix::Dec,
        },
    );
    seed_children.push(seed_id);
    let seed_call = push_list(ast, seed_children);
    defs[def_ix].body = Some(seed_call);
    // The seed call's `<param>` reference resolves by walking parents to the original `(def sig body)`
    // form (resolve Case 4 checks the FORM's body child), so the form's body child must BE `seed_call`.
    // Swap it in place — the original `(def sig <old-body>)` becomes `(def sig seed_call)`. `m.def_form`
    // is the original def form occurrence located during matching.
    if let Struct::List(children) = &mut ast.structure[m.def_form.0 as usize]
        && children.len() == 3
    {
        children[2] = seed_call;
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────────────────────────

/// Index every top-level `(def sig body)` FORM occurrence by its SIGNATURE occurrence (`sig_occ.0`), in
/// ONE pass over the module's items — the O(items) prebuild the per-def rewrite reads O(1). The transform
/// runs before the parent index (a parent lookup is unavailable), so this is how a matched def finds its
/// enclosing form (to swap the body child). A def not directly under `(module …)`/`(do …)` is simply
/// absent from the map — its rewrite is then skipped, leaving it untouched (unchanged behavior vs the old
/// per-def `find` returning `None`). Keyed by the raw `u32` so the map needs no `StructId` hashing impl.
fn index_def_forms(ast: &Arenas) -> crate::fxhash::FxHashMap<u32, StructId> {
    let items: &[StructId] = match ast.get(ast.root) {
        Struct::List(top) => {
            // `(module NAME item…)` → items after NAME; `(do item…)` → all tail; else the root itself.
            match top.first().and_then(|&h| ast.as_name(h)) {
                Some("module") => top.get(2..).unwrap_or(&[]),
                Some("do") => &top[1..],
                _ => std::slice::from_ref(&ast.root),
            }
        }
        _ => return crate::fxhash::FxHashMap::default(),
    };
    let mut map = crate::fxhash::FxHashMap::default();
    for &item in items {
        if let Some(t) = ast.as_form(item, "def")
            && let Some(&sig) = t.first()
        {
            // First writer wins (a duplicate sig_occ cannot occur — each form has a distinct signature
            // node — but a defensive `or_insert` keeps the first, matching the old `find`'s first-hit).
            map.entry(sig.0).or_insert(item);
        }
    }
    map
}

/// The identity element of an associative binary op the transform reassociates, recognizing BOTH
/// spellings of an operand occurrence:
///  - a bare `Name` — the checked `+` (identity `0`) / `*` (identity `1`);
///  - a member access `(. T wrapping-add)` / `(. T wrapping-mul)` for any numeric type `T` — the
///    two's-complement WRAPPING ops (identity `0` / `1`). These NEVER trap, so their reassociation is
///    fully transparent (not even a trap-timing shift — the transform is unconditionally value-exact).
///
/// Returns `None` for any other op (subtraction, division, a non-associative or unknown member op).
fn associative_op_identity(ast: &Arenas, op_occ: StructId) -> Option<i64> {
    if let Some(name) = ast.as_name(op_occ) {
        return match name {
            "+" => Some(0),
            "*" => Some(1),
            _ => None,
        };
    }
    // A member access `(. T method)` — read the method name (the third child) and match the wrapping ops.
    // The receiver type `T` is irrelevant to the identity (every numeric width shares `0`/`1`).
    let dot = ast.as_form(op_occ, ".")?;
    let [_ty, method] = dot else {
        return None;
    };
    match ast.as_name(*method)? {
        "wrapping-add" => Some(0),
        "wrapping-mul" => Some(1),
        _ => None,
    }
}

/// The `(pattern body)` parts of a match ARM — a two-element list. `None` for a guard arm or any other
/// shape (a guard `((guard …) body)` is out of scope: the accumulator transform only handles the plain
/// two-arm empty/cons fold).
fn arm_parts(ast: &Arenas, arm: StructId) -> Option<(StructId, StructId)> {
    let c = list_children(ast, arm)?;
    let [pat, body] = c.as_slice() else {
        return None;
    };
    Some((*pat, *body))
}

/// The element occurrences of a `(list …)` match PATTERN, recognizing BOTH the string-headed ctor spelling
/// (`"list"`, what the reader desugars to) and a bare `(list …)` form — mirrors the resolver's
/// `as_ctor_form(…, "list").or_else(as_form(…, "list"))`.
fn list_pattern_elems(ast: &Arenas, pat: StructId) -> Option<&[StructId]> {
    ast.compound_form_of(pat, CompoundCtor::List)
}

/// Whether `pat` is the EMPTY-list pattern `(list)` — a `(list …)` pattern with no elements (the base arm).
fn list_pattern_is_empty(ast: &Arenas, pat: StructId) -> bool {
    list_pattern_elems(ast, pat).is_some_and(|e| e.is_empty())
}

/// The REST binder name of a cons pattern `(list … .. rest)` — the single element immediately after the
/// `..` marker. `None` if there is no `..` or its binder is missing / `_` (a wildcard rest can't be
/// threaded as the accumulator's scrutinee argument). Mirrors resolve's `list_pattern_rest_binds`.
fn cons_pattern_rest_binder(ast: &Arenas, pat: StructId) -> Option<String> {
    let elems = list_pattern_elems(ast, pat)?;
    let (_, rest_occ, _) = ast.rest_marker(elems)?;
    let name = ast.as_name(rest_occ)?;
    // `_` and `..` are not threadable rest binders (`..` is the helper's marker-atom fallback for a
    // malformed flat rest with no operand sibling — byte-identical to the old `elems.get(dd + 1)?` skip).
    if name == "_" || name == ".." {
        return None;
    }
    Some(name.to_string())
}

/// The binder name of a parameter occurrence — a bare `name`, or the inner name of `(: name T)`.
fn param_binder_name(ast: &Arenas, param: StructId) -> Option<String> {
    if let Some(n) = ast.as_name(param) {
        return Some(n.to_string());
    }
    let annot = ast.as_form(param, ":")?;
    ast.as_name(*annot.first()?).map(str::to_string)
}

/// Deep-copy a subtree, minting fresh occurrences for names (so a reused parameter binder can be given a
/// second home without acquiring a second parent). Non-name atoms (literals) are shared verbatim. Mirrors
/// `sums::copy_subtree`/`effects::copy_subtree`; used to clone the original param binders — annotations
/// and all — into the synthesized accumulator's signature.
fn copy_subtree(ast: &mut Arenas, node: StructId) -> StructId {
    match ast.get(node).clone() {
        Struct::Atom(lid) => match ast.leaf(lid).clone() {
            Leaf::Name(_) => {
                let leaf = ast.leaf(lid).clone();
                push_atom(ast, leaf)
            }
            _ => node,
        },
        Struct::List(children) => {
            let copied: Vec<StructId> = children.iter().map(|&c| copy_subtree(ast, c)).collect();
            push_list(ast, copied)
        }
    }
}

/// The children of a list occurrence (head + args), or `None` for an atom.
fn list_children(ast: &Arenas, id: StructId) -> Option<Vec<StructId>> {
    match ast.get(id) {
        Struct::List(c) => Some(c.clone()),
        Struct::Atom(_) => None,
    }
}

/// If `id` is `(name arg…)` — an application of `name` with exactly `arity` arguments — return the
/// arguments. Used to spot the self-recursive call `(f (- n 1))` or `(f (- n 1) k)`.
fn self_call_args(ast: &Arenas, id: StructId, name: &str, arity: usize) -> Option<Vec<StructId>> {
    let c = list_children(ast, id)?;
    let (head, args) = c.split_first()?;
    if ast.as_name(*head)? != name || args.len() != arity {
        return None;
    }
    Some(args.to_vec())
}

/// The integer value of a literal occurrence, if it is one.
fn int_literal(ast: &Arenas, id: StructId) -> Option<i64> {
    match ast.get(id) {
        Struct::Atom(lid) => match ast.leaf(*lid) {
            Leaf::Int { value, .. } => value.to_i64(),
            _ => None,
        },
        _ => None,
    }
}

/// Whether the subtree at `id` mentions `name` anywhere (a guard against a hidden second self-call in
/// the term or recursion argument, which would take the transform out of the linear-recursion class).
fn mentions_name(ast: &Arenas, id: StructId, name: &str) -> bool {
    match ast.get(id) {
        Struct::Atom(_) => ast.as_name(id) == Some(name),
        Struct::List(c) => c.clone().iter().any(|&ch| mentions_name(ast, ch, name)),
    }
}

/// Whether the per-step term `g` PERFORMS a discharged effect — the safe-floor test that keeps accumulator
/// introduction from reassociating an EFFECTFUL term (which would reorder its evaluation, an observable
/// change — unlike a pure term, whose reassociation is value-exact). PRECISE by design: an effect
/// performance at this pre-resolve load stage is a MEMBER ACCESS `(. E op)` whose base `E` names a DECLARED
/// EFFECT (`effect_names`, from `scan_top_level`'s `effect_decls`). Matching the base against the declared
/// effects is what separates a genuine perform (`(E.bail unit)` → `((. E bail) unit)`, base `E` a declared
/// effect) from a pure record/field access (`(. r x)`, base `r` not an effect) — so a pure `(+ (. r x) 1)`
/// term still accumulates, and only a real perform declines. We flag the member-access OCCURRENCE itself
/// (applied or not), which subsumes both a nullary `(E.done)` and an arg-bearing `(E.bail unit)` and is
/// sound-toward-decline: reading an effect op at all in the reassociated term is the eval-order-sensitive
/// signal. A structural walk over the whole term (the perform may be nested in a branch, as in the witness
/// `(if (= k 2) (E.bail unit) k)`); a member access on a non-effect base recurses into its children.
fn term_performs_effect(
    ast: &Arenas,
    term: StructId,
    effect_names: &crate::fxhash::FxHashSet<String>,
) -> bool {
    // A member access `(. E op)` (head ".", two children `[base, method]`) whose base names a declared
    // effect IS a perform site — the eval-order-sensitive node we decline on.
    if let Some([base, _method]) = ast.as_form(term, ".")
        && let Some(base_name) = ast.as_name(*base)
        && effect_names.contains(base_name)
    {
        return true;
    }
    // Otherwise recurse into children (an atom has none; a `(. r x)` on a non-effect base recurses too, so
    // a nested effect perform inside a pure member access is still found).
    match ast.get(term) {
        Struct::Atom(_) => false,
        Struct::List(c) => c
            .clone()
            .iter()
            .any(|&ch| term_performs_effect(ast, ch, effect_names)),
    }
}

/// A fresh accumulator-def name not colliding with any existing def (`f$acc`, then `f$acc$` …).
fn fresh_acc_name(defs: &[Def], base: &str) -> String {
    let mut name = format!("{base}$acc");
    while defs.iter().any(|d| d.name == name) {
        name.push('$');
    }
    name
}

#[cfg(test)]
mod tests {
    use crate::db::Db;

    /// A matching linear-recursion def gains a synthesized accumulator sibling and is re-seeded to a
    /// non-recursive call; a non-matching def (fib, two self-calls) is left as-is.
    #[test]
    fn introduce_adds_an_accumulator_def_for_a_linear_recursion() {
        let ast = crate::testkit::parse(
            "(module m (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1))))) (export sm))",
        );
        let db = Db::load(ast);
        // The original `sm` plus a synthesized `sm$acc` (the accumulator).
        assert!(db.def_by_name("sm").is_some(), "sm remains");
        assert!(
            db.def_by_name("sm$acc").is_some(),
            "an accumulator def sm$acc was synthesized"
        );
        // The accumulator takes two params (n, acc); sm still takes one.
        let acc = db.def_by_name("sm$acc").unwrap();
        assert_eq!(db.defs[acc].params.len(), 2, "accumulator has (n, acc)");
        let sm = db.def_by_name("sm").unwrap();
        assert_eq!(db.defs[sm].params.len(), 1, "sm still has (n)");
    }

    /// The def-form index (`index_def_forms`) must locate the enclosing `(def …)` FORM of a matching
    /// recursion NO MATTER its position among many defs — the property that lets the per-def form lookup
    /// be O(1) instead of a per-def linear item scan (the old `find_def_form`, which made an N-def module
    /// O(N²) — profiled at ~50% of the whole compile). A transformable `sm` placed AFTER a run of
    /// unrelated defs still gains its `sm$acc`, and the unrelated defs are left untouched.
    #[test]
    fn introduce_finds_a_matching_def_among_many_others() {
        let mut src = String::from("(module m");
        for i in 0..50 {
            src.push_str(&format!(" (def (g{i} x) (+ x {i}))"));
        }
        // The one transformable linear recursion, buried in the middle-to-end of the module.
        src.push_str(" (def (sm (: n Int64)) (if (= n 0) 0 (+ n (sm (- n 1)))))");
        for i in 50..100 {
            src.push_str(&format!(" (def (g{i} x) (+ x {i}))"));
        }
        src.push_str(" (export sm))");
        let db = Db::load(crate::testkit::parse(&src));
        assert!(
            db.def_by_name("sm$acc").is_some(),
            "the buried linear recursion is still found + transformed by the sig→form index"
        );
        // A non-matching neighbour never gains an accumulator (its form is indexed but never rewritten).
        assert!(
            db.def_by_name("g0$acc").is_none() && db.def_by_name("g99$acc").is_none(),
            "non-matching defs are left untouched"
        );
    }

    #[test]
    fn introduce_leaves_a_non_matching_recursion_alone() {
        let ast = crate::testkit::parse(
            "(module m (def (fib (: n Int64)) (if (< n 2) n (+ (fib (- n 1)) (fib (- n 2))))) (export fib))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("fib$acc").is_none(),
            "fib (two self-calls) must NOT be accumulator-transformed"
        );
    }

    /// A MULTI-parameter linear recursion (a pass-through `k` alongside the recursion variable `n`) also
    /// gets an accumulator — every self-call argument is threaded through unchanged, plus the folded acc.
    /// The synthesized accumulator carries ALL original params (2) followed by `acc` (3 total).
    #[test]
    fn introduce_handles_a_multi_parameter_recursion() {
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64) (: k Int64)) \
               (if (= n 0) 0 (+ (* n k) (f (- n 1) k)))) (export f))",
        );
        let db = Db::load(ast);
        let acc = db
            .def_by_name("f$acc")
            .expect("a multi-parameter recursion is accumulator-transformed");
        assert_eq!(
            db.defs[acc].params.len(),
            3,
            "accumulator carries both original params (n, k) plus acc"
        );
        let f = db.def_by_name("f").unwrap();
        assert_eq!(db.defs[f].params.len(), 2, "f still has (n, k)");
    }

    /// A FLIPPED base condition — `(if (> n 0) combine base)`, recursive branch in THEN and the base
    /// value in ELSE — is also transformed. The matcher recognizes the combine in either branch; the
    /// accumulator keeps the same branch order so the reused condition still selects the base case.
    #[test]
    fn introduce_handles_a_flipped_base_condition() {
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (if (> n 0) (+ n (f (- n 1))) 0)) (export f))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("f$acc").is_some(),
            "a flipped-base linear recursion is accumulator-transformed"
        );
    }

    /// A flipped shape whose base value is NOT the op's identity (`5`, not `0`) must NOT transform —
    /// reassociating it would change the result.
    #[test]
    fn introduce_declines_a_flipped_base_that_is_not_the_identity() {
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) (if (> n 0) (+ n (f (- n 1))) 5)) (export f))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("f$acc").is_none(),
            "a non-identity base value must NOT be reassociated"
        );
    }

    /// A WRAPPING op spelled as a member access `(. Int64 wrapping-add)` — a list form, not a `Name` atom
    /// — is also transformed (identity `0`, and wrapping never traps so reassociation is value-exact). The
    /// op occurrence is cloned into the accumulator, so the dotted spelling reconstructs correctly.
    #[test]
    fn introduce_handles_a_wrapping_op_member_access() {
        let ast = crate::testkit::parse(
            "(module m (def (sm (: n Int64)) \
               (if (= n 0) 0 ((. Int64 wrapping-add) n (sm (- n 1))))) (export sm))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("sm$acc").is_some(),
            "a wrapping-add member-access recursion is accumulator-transformed"
        );
    }

    /// A member-access op that is NOT one of the recognized associative ops (`(. Int64 min)`) must NOT
    /// match as the combine operator — only `wrapping-add`/`wrapping-mul` (and bare `+`/`*`) reassociate.
    #[test]
    fn introduce_declines_a_non_associative_member_op() {
        let ast = crate::testkit::parse(
            "(module m (def (f (: n Int64)) \
               (if (= n 0) 0 ((. Int64 min) n (f (- n 1))))) (export f))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("f$acc").is_none(),
            "a non-associative member op must NOT be reassociated"
        );
    }

    /// EFFECTFUL-TERM SAFE FLOOR (14b:13175): a linear recursion whose per-step term PERFORMS a discharged
    /// effect must NOT be accumulator-transformed — reassociating the effectful term reorders WHEN it runs,
    /// an observable change. The witness: `(+ (loop (- k 1)) (if (= k 2) (E.bail unit) k))` — the term
    /// `(if … (E.bail unit) k)` performs `E.bail`. Declined so the effects pass sees it un-reassociated.
    #[test]
    fn introduce_declines_a_term_that_performs_an_effect() {
        let ast = crate::testkit::parse(
            "(module m (effect E (op bail (-> Unit Int64))) \
               (def (loop (: k Int64)) \
                 (if (> k 0) (+ (loop (- k 1)) (if (= k 2) (E.bail unit) k)) 0)) (export loop))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("loop$acc").is_none(),
            "a recursion whose per-step term performs a discharged effect must NOT be reassociated"
        );
    }

    /// PRECISION GUARD (the concierge's regression concern): a per-step term that is a PURE member/field
    /// access `(. r x)` — base `r` NOT a declared effect — must STILL transform even when an effect is
    /// declared in the module. The perform-detection keys on the declared-effect NAMES, so a pure record
    /// access does not over-decline (which would forfeit accum's stack win on record-folding recursions).
    #[test]
    fn introduce_transforms_a_pure_member_access_term_despite_a_declared_effect() {
        let ast = crate::testkit::parse(
            "(module m (effect E (op bail (-> Unit Int64))) \
               (def (f (: n Int64) r) \
                 (if (= n 0) 0 (+ (. r x) (f (- n 1) r)))) (export f))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("f$acc").is_some(),
            "a pure member-access term (base not an effect) still accumulator-transforms"
        );
    }

    // ── LIST-FOLD shape: the user's `sum` — `(match xs ((list) 0) ((list x .. rest) (+ x (sum rest))))`
    // is the SAME linear accumulation as the numeric `if`, dispatched by matching a list's shape instead
    // of testing a scalar. It transforms to a TAIL fold `(sum$acc xs acc)` the `select` loop transform
    // then compiles to a constant-stack loop. ─────────────────────────────────────────────────────────

    /// The user's EXACT non-tail list `sum` gains a synthesized accumulator and re-seeds to `(sum$acc xs 0)`.
    #[test]
    fn introduce_adds_an_accumulator_for_a_non_tail_list_fold() {
        let ast = crate::testkit::parse(
            "(module m (def (sum xs) \
               (match xs ((list) 0) ((list x .. rest) (+ x (sum rest))))) (export sum))",
        );
        let db = Db::load(ast);
        let acc = db
            .def_by_name("sum$acc")
            .expect("a non-tail list fold gains an accumulator def");
        assert_eq!(
            db.defs[acc].params.len(),
            2,
            "accumulator carries the original param (xs) plus acc"
        );
        assert_eq!(
            db.defs[db.def_by_name("sum").unwrap()].params.len(),
            1,
            "sum still has (xs)"
        );
    }

    /// A FLIPPED arm order (cons arm first, empty arm second) also transforms — the matcher recognizes the
    /// empty/cons arms in either position and keeps the same order so the reused patterns select the same
    /// cases.
    #[test]
    fn introduce_handles_a_flipped_list_arm_order() {
        let ast = crate::testkit::parse(
            "(module m (def (sum xs) \
               (match xs ((list x .. rest) (+ x (sum rest))) ((list) 0))) (export sum))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("sum$acc").is_some(),
            "a flipped-arm-order list fold is accumulator-transformed"
        );
    }

    /// A list fold whose base value is NOT the op's identity (`100`, not `0`) must NOT transform —
    /// reassociating it to a left fold seeded with the identity would change the result.
    #[test]
    fn introduce_declines_a_list_fold_with_a_non_identity_base() {
        let ast = crate::testkit::parse(
            "(module m (def (sum xs) \
               (match xs ((list) 100) ((list x .. rest) (+ x (sum rest))))) (export sum))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("sum$acc").is_none(),
            "a non-identity list-fold base must NOT be reassociated"
        );
    }

    /// A cons arm with a WILDCARD rest (`(list x .. _)`) can't be threaded as the accumulator's scrutinee
    /// argument, so the fold is left alone. (It also isn't a real recursion over the tail.)
    #[test]
    fn introduce_declines_a_list_fold_with_a_wildcard_rest() {
        let ast = crate::testkit::parse(
            "(module m (def (sum xs) \
               (match xs ((list) 0) ((list x .. _) (+ x (sum x))))) (export sum))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("sum$acc").is_none(),
            "a wildcard-rest fold is not transformed"
        );
    }

    /// A list fold whose self-call does NOT thread the rest binder through the scrutinee position (here it
    /// re-passes the WHOLE list `xs`, an infinite recursion shape) must NOT transform — the accumulator
    /// would fold a different element sequence.
    #[test]
    fn introduce_declines_when_the_self_call_does_not_recurse_on_rest() {
        let ast = crate::testkit::parse(
            "(module m (def (sum xs) \
               (match xs ((list) 0) ((list x .. rest) (+ x (sum xs))))) (export sum))",
        );
        let db = Db::load(ast);
        assert!(
            db.def_by_name("sum$acc").is_none(),
            "a self-call not recursing on the rest sublist must NOT be reassociated"
        );
    }
}
