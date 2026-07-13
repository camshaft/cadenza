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
//! ## Soundness (the operator's call: checked `+` too, accept a trap-point change)
//! Reassociating a CHECKED `+`/`*` can move WHERE an overflow traps (a partial sum overflows at a
//! different step under a left vs right fold). The transform preserves the FINAL value exactly; only the
//! overflow-trap TIMING for an already-overflowing input may differ. For the all-non-negative sums the
//! guide shows, the partials are monotonic and the trap point coincides. The `(. T wrapping-add)`/
//! `wrapping-mul` variants never trap, so their reassociation is fully transparent — value-exact with no
//! trap-timing caveat at all.

use crate::ast::{Arenas, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::Def;
use crate::prelude::{push_atom, push_list};

/// Append a bare `Name` atom occurrence — the synthesis workhorse (a synthesized reference/binder).
fn push_name(ast: &mut Arenas, name: &str) -> StructId {
    push_atom(ast, Leaf::Name(name.to_string()))
}

/// Run accumulator introduction over the module's `defs`, mutating `ast` (appending synthesized nodes)
/// and `defs` (rewriting a matched def's body to the seed call + appending its accumulator def). Called
/// at load, after `scan_top_level` and BEFORE the parent index / `def_by_name` are built, so the
/// synthesized def is indexed and resolvable like any other. A def that does not match is left untouched.
pub(crate) fn introduce(ast: &mut Arenas, defs: &mut Vec<Def>) {
    // Collect the rewrites first (an immutable scan of `defs`), then apply — so the synthesis (which
    // reads `defs` for name collisions) sees a stable view.
    let mut plans: Vec<(usize, Match)> = Vec::new();
    for (i, d) in defs.iter().enumerate() {
        if let Some(m) = match_linear_recursion(ast, d) {
            plans.push((i, m));
        }
    }
    for (def_ix, m) in plans {
        apply(ast, defs, def_ix, m);
    }
}

/// A recognized linear-accumulator recursion, with the occurrences the rewrite reuses/reads.
struct Match {
    /// The original `(def sig body)` FORM occurrence — its body child is swapped to the seed call.
    def_form: StructId,
    /// Every parameter's binder NAME, in order (e.g. `[n]` or `[n, k]`). The synthesized accumulator
    /// reuses these spellings so the reused `base_cond`/`term`/`rec_args` occurrences resolve to it.
    param_names: Vec<String>,
    /// The base-case condition occurrence `(= n 0)` — reused verbatim in the accumulator's `if`.
    base_cond: StructId,
    /// Which `if` branch holds the BASE value in the original: `true` = THEN (`(if (= n 0) base combine)`,
    /// the guide's shape), `false` = ELSE (a FLIPPED `(if (> n 0) combine base)`). `apply` places the
    /// accumulator's branches in the SAME order so the reused condition still selects correctly.
    base_is_then: bool,
    /// The associative op's OCCURRENCE — a bare `Name` (`+`/`*`) or a member access `(. T wrapping-add)`.
    /// `apply` CLONES it (via `copy_subtree`) into the accumulator, so either spelling reconstructs
    /// correctly (a bare name rebuild couldn't represent the dotted form).
    op_occ: StructId,
    identity: i64,
    /// The per-step TERM occurrence `g` (the `+`'s non-recursive operand, e.g. `n` or `(* n k)`).
    term: StructId,
    /// The self-call's ARGUMENT occurrences (`(- n 1)`, `k`, …), one per parameter — threaded UNCHANGED
    /// into the accumulator's tail self-call. Reassociation preserves the final value for ANY number of
    /// parameters (recursion variables and pass-throughs alike), since `+`/`*` are associative.
    rec_args: Vec<StructId>,
}

/// Match a def against the linear-accumulator shape. Returns `None` (leave the def alone) unless it is
/// `(def (f p…) (if COND …))` where one `if` branch is `OP`'s identity `ID` and the other is the combine
/// `(OP g (f REC…))` — `OP` associative, the single self-call in one `OP` operand. Either branch ordering
/// is accepted (base in THEN, the guide's shape; or base in ELSE, a flipped condition). Any number of
/// parameters is accepted — every self-call argument is threaded through the accumulator unchanged
/// (reassociation is sound whether a parameter is a recursion variable or a pass-through).
fn match_linear_recursion(ast: &Arenas, d: &Def) -> Option<Match> {
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
    // Locate the enclosing `(def sig body)` FORM (its body child is swapped to the seed call). The parent
    // index is not built yet, so find the module item whose `def` sig == this def's `sig_occ`.
    let def_form = find_def_form(ast, d.sig_occ)?;
    // Body must be `(if COND THEN ELSE)`.
    let if_tail = ast.as_form(body, "if")?;
    let [cond, then_, else_] = if_tail else {
        return None;
    };
    // One branch is the base value (the OP identity), the other the recursive combine `(OP g (f REC…))`.
    // Either ordering is accepted: the base condition may select the BASE branch when true (`(if (= n 0)
    // base combine)` — the guide's shape, base in THEN) or the RECURSIVE branch when true (a FLIPPED
    // condition `(if (> n 0) combine base)` — base in ELSE). Try ELSE as the combine first (guide shape),
    // then THEN (flipped). `base_is_then` records which slot holds the base, so `apply` places the
    // accumulator's branches in the matching order (the condition is reused verbatim).
    let (base_is_then, base_val, op_occ, identity, term, rec_args) = if let Some((op, id, t, r)) =
        match_combine(ast, *else_, &d.name, param_names.len())
    {
        (true, *then_, op, id, t, r)
    } else if let Some((op, id, t, r)) = match_combine(ast, *then_, &d.name, param_names.len()) {
        (false, *else_, op, id, t, r)
    } else {
        return None;
    };
    // The base value must be the op's identity literal.
    if int_literal(ast, base_val)? != identity {
        return None;
    }
    Some(Match {
        def_form,
        param_names,
        base_cond: *cond,
        base_is_then,
        op_occ,
        identity,
        term,
        rec_args,
    })
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
    // (it references the params, which bind to this def's params). CLONE the op occurrence so a member
    // access like `(. Int64 wrapping-add)` reconstructs correctly (not just a bare-name `+`/`*`).
    let op_ref = copy_subtree(ast, m.op_occ);
    let acc_ref_in_op = push_name(ast, acc_var);
    let combined = push_list(ast, vec![op_ref, acc_ref_in_op, m.term]);
    let mut rec_call_children = vec![push_name(ast, &acc_name)];
    rec_call_children.extend(m.rec_args.iter().copied());
    rec_call_children.push(combined);
    let rec_call = push_list(ast, rec_call_children);
    // `(if base_cond THEN ELSE)` — reuse the original `base_cond` occurrence, placing the base and
    // recursive branches in the SAME order the original used (so the reused condition still selects the
    // base case correctly, whether it was written `(if (= n 0) base rec)` or `(if (> n 0) rec base)`).
    let if_head = push_name(ast, "if");
    let (then_branch, else_branch) = if m.base_is_then {
        (base_ref, rec_call)
    } else {
        (rec_call, base_ref)
    };
    let acc_body = push_list(ast, vec![if_head, m.base_cond, then_branch, else_branch]);
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

/// The top-level `(def sig …)` FORM occurrence whose signature is `sig_occ`. Scans the module's items
/// (the transform runs before the parent index, so a parent lookup is unavailable). `None` if not found
/// (a def not directly under `(module …)`/`(do …)` — then the rewrite is skipped, leaving it untouched).
fn find_def_form(ast: &Arenas, sig_occ: StructId) -> Option<StructId> {
    let items = match ast.get(ast.root) {
        Struct::List(top) => {
            // `(module NAME item…)` → items after NAME; `(do item…)` → all tail; else the root itself.
            match ast.as_name(*top.first()?) {
                Some("module") => top.get(2..).unwrap_or(&[]).to_vec(),
                Some("do") => top[1..].to_vec(),
                _ => vec![ast.root],
            }
        }
        _ => return None,
    };
    items
        .into_iter()
        .find(|&item| ast.as_form(item, "def").and_then(|t| t.first().copied()) == Some(sig_occ))
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
}
