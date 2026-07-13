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
//! ## Scope (increment 1)
//! Exactly the LINEAR self-recursion `f p… = (if BASE ID (OP g (f REC…)))` where:
//!  - `OP` is an ASSOCIATIVE binary op with a known identity — `+`/`*`/`+%`/`*%` (identity `0`/`1`);
//!  - the base value equals `OP`'s identity;
//!  - exactly ONE self-recursive call, in one operand of `OP`; the OTHER operand `g` does not itself
//!    recurse (it is the per-step term folded into the accumulator);
//!  - a single recursion parameter (multi-parameter recursion is a later increment).
//!
//! ## Soundness (the operator's call: checked `+` too, accept a trap-point change)
//! Reassociating a CHECKED `+`/`*` can move WHERE an overflow traps (a partial sum overflows at a
//! different step under a left vs right fold). The transform preserves the FINAL value exactly; only the
//! overflow-trap TIMING for an already-overflowing input may differ. For the all-non-negative sums the
//! guide shows, the partials are monotonic and the trap point coincides. The `+%`/`*%` wrapping variants
//! never trap, so their reassociation is fully transparent.

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
    /// The single parameter's binder NAME (the recursion variable, e.g. `n`).
    param_name: String,
    /// The base-case condition occurrence `(= n 0)` — reused verbatim in the accumulator's `if`.
    base_cond: StructId,
    /// The associative op's spelling (`+`/`*`/`Int64.wrapping-add`/…) and its identity value.
    op_name: String,
    identity: i64,
    /// The per-step TERM occurrence `g` (the `+`'s non-recursive operand, e.g. `n`).
    term: StructId,
    /// The recursion ARGUMENT occurrence (`(- n 1)`) — the value the self-call passes for the parameter.
    rec_arg: StructId,
}

/// Match a def against the linear-accumulator shape. Returns `None` (leave the def alone) unless it is
/// EXACTLY `(def (f n) (if COND ID (OP g (f REC))))` with `OP` associative, `ID` = `OP`'s identity, and
/// the single self-call in one `OP` operand.
fn match_linear_recursion(ast: &Arenas, d: &Def) -> Option<Match> {
    // Exactly one parameter, a bare name (an annotated `(: n T)` is fine — take the inner name).
    let [param] = d.params.as_slice() else {
        return None;
    };
    let param_name = param_binder_name(ast, *param)?;
    let body = d.body?;
    // Locate the enclosing `(def sig body)` FORM (its body child is swapped to the seed call). The parent
    // index is not built yet, so find the module item whose `def` sig == this def's `sig_occ`.
    let def_form = find_def_form(ast, d.sig_occ)?;
    // Body must be `(if COND THEN ELSE)`.
    let if_tail = ast.as_form(body, "if")?;
    let [cond, then_, else_] = if_tail else {
        return None;
    };
    // One branch is the base value (the OP identity), the other the recursive combine. The base
    // condition `(= n 0)` selects the base branch when TRUE, so THEN is the base and ELSE the combine —
    // the guide's shape. (A flipped `(if (!= n 0) combine base)` is a later refinement.)
    let (base_val, combine, base_cond) = (*then_, *else_, *cond);
    // The combine must be `(OP g rec)` or `(OP rec g)` where exactly one operand is the self-call.
    let combine_tail = list_children(ast, combine)?;
    let [op_occ, a, b] = combine_tail.as_slice() else {
        return None;
    };
    let op_name = ast.as_name(*op_occ)?.to_string();
    let identity = associative_identity(&op_name)?;
    // The base value must be the op's identity literal.
    if int_literal(ast, base_val)? != identity {
        return None;
    }
    // Exactly one operand is the self-call `(f REC)` (a single-argument application of the def's own
    // name); the other is the per-step term. The term must NOT itself recurse (a nested `f …` in the
    // term would need multi-way accumulation — a later increment).
    let a_rec = self_call_arg(ast, *a, &d.name);
    let b_rec = self_call_arg(ast, *b, &d.name);
    let (term, rec_arg) = match (a_rec, b_rec) {
        (Some(rec), None) if !mentions_name(ast, *b, &d.name) => (*b, rec),
        (None, Some(rec)) if !mentions_name(ast, *a, &d.name) => (*a, rec),
        _ => return None, // zero or two self-calls, or the term also recurses — out of scope.
    };
    // The per-step term and the recursion argument must not smuggle in another self-call.
    if mentions_name(ast, term, &d.name) || mentions_name(ast, rec_arg, &d.name) {
        return None;
    }
    Some(Match {
        def_form,
        param_name,
        base_cond,
        op_name,
        identity,
        term,
        rec_arg,
    })
}

/// Synthesize the accumulator def and rewrite the original def to seed it.
fn apply(ast: &mut Arenas, defs: &mut Vec<Def>, def_ix: usize, m: Match) {
    let orig_name = defs[def_ix].name.clone();
    let acc_name = fresh_acc_name(defs, &orig_name);
    let acc_ix = defs.len();

    // ── Synthesize the FULL `(def (acc_name n acc) (if BASE-COND acc (acc_name REC (OP acc TERM))))`
    // FORM node in the arena. Resolution binds a def-body name by walking parents to a `(def sig body)`
    // form (resolve Case 4), so the accumulator must be a real form the reused `n`-references ascend to.
    // Its binders are `n` (same spelling as the original param, so the reused `base_cond`/`term`/
    // `rec_arg` occurrences resolve to it) and a fresh `acc`.
    let acc_var = "acc$";
    let n_binder = push_name(ast, &m.param_name);
    let acc_binder = push_name(ast, acc_var);

    // then-branch: the bare accumulator (base case returns what we've folded so far).
    let then_ref = push_name(ast, acc_var);

    // else-branch: `(acc_name rec_arg (OP acc term))` — the tail self-call.
    // The combine `(OP acc term)`: LEFT-fold order `(OP acc term)` — acc first. Reuse the ORIGINAL
    // `term` occurrence (it references `n`, which binds to this def's `n` param).
    let op_ref = push_name(ast, &m.op_name);
    let acc_ref_in_op = push_name(ast, acc_var);
    let combined = push_list(ast, vec![op_ref, acc_ref_in_op, m.term]);
    // the recursive call: `(acc_name rec_arg combined)`.
    let acc_call_head = push_name(ast, &acc_name);
    let rec_call = push_list(ast, vec![acc_call_head, m.rec_arg, combined]);
    // `(if base_cond then_ref rec_call)` — reuse the original `base_cond` occurrence.
    let if_head = push_name(ast, "if");
    let acc_body = push_list(ast, vec![if_head, m.base_cond, then_ref, rec_call]);
    // Signature `(acc_name n acc)` and the whole `(def sig acc_body)` form.
    let sig_name = push_name(ast, &acc_name);
    let sig = push_list(ast, vec![sig_name, n_binder, acc_binder]);
    let def_head = push_name(ast, "def");
    let _def_form = push_list(ast, vec![def_head, sig, acc_body]);

    defs.push(Def {
        name: acc_name.clone(),
        sig_occ: sig,
        params: vec![n_binder, acc_binder],
        body: Some(acc_body),
    });
    debug_assert_eq!(defs.len(), acc_ix + 1);

    // ── Rewrite the original def's body to `(acc_name <param> <identity>)` ──
    // Reference the original parameter by its NAME (a fresh occurrence that binds to the original def's
    // param via the scope walk).
    let seed_head = push_name(ast, &acc_name);
    let seed_param = push_name(ast, &m.param_name);
    let seed_id = push_atom(
        ast,
        Leaf::Int {
            value: IntValue::from_i64(m.identity),
            radix: Radix::Dec,
        },
    );
    let seed_call = push_list(ast, vec![seed_head, seed_param, seed_id]);
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

/// The associative binary ops the transform reassociates, with their identity element. Bare-name
/// operators only — `+`/`*`. The WRAPPING ops (`Int64.wrapping-add`/`-mul`) are spelled as a member
/// access `(. Int64 wrapping-add)` (a list form, not a `Name`), so they do not reach here yet; matching
/// that dotted spelling is a follow-up increment (the transform is fully sound for them — wrapping never
/// traps — just not yet wired to the member-access syntax).
fn associative_identity(op: &str) -> Option<i64> {
    match op {
        "+" => Some(0),
        "*" => Some(1),
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

/// The children of a list occurrence (head + args), or `None` for an atom.
fn list_children(ast: &Arenas, id: StructId) -> Option<Vec<StructId>> {
    match ast.get(id) {
        Struct::List(c) => Some(c.clone()),
        Struct::Atom(_) => None,
    }
}

/// If `id` is `(name arg)` — a one-argument application of `name` — return `arg`. Used to spot the
/// self-recursive call `(f (- n 1))`.
fn self_call_arg(ast: &Arenas, id: StructId, name: &str) -> Option<StructId> {
    let c = list_children(ast, id)?;
    let [head, arg] = c.as_slice() else {
        return None;
    };
    (ast.as_name(*head)? == name).then_some(*arg)
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
}
