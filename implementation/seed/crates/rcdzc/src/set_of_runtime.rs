//! SET.OF OVER A RUNTIME LIST — rewrite `(Set.of xs)` where `xs` is NOT a compile-time `(list …)` literal
//! into a call to a synthesized recursive index-fold that inserts each element into a fresh set.
//!
//! `lower_set_of` folds a CONSTANT `(list …)` literal to a canonical `Core::SetOf` at compile time, but a
//! set built from a RUNTIME list — a `Set.to-list` result, a `List.concat`, a param/recursively-built list
//! — has no visible element list to fold, so `lower` declined it ("Set.of over a runtime list is not yet
//! built"). This pre-pass supplies the runtime construction by SYNTHESIS: it appends ONE generic recursive
//! fold def
//!
//! ```text
//! (def (__set_of_rt xs i acc)
//!   (if (< i (List.len xs))
//!       (match (List.at xs i)
//!         ((Some v) (__set_of_rt xs (+ i 1) (Set.insert acc v)))
//!         ((None _u) acc))
//!       acc))
//! ```
//!
//! and rewrites each `(Set.of xs)` whose `xs` is not a `(list …)` literal into `(__set_of_rt xs 0 (Set.of
//! (list)))` — the seed being the EMPTY `(Set.of (list))`, itself a `(list)` literal so it folds to the
//! canonical empty set via the const path and never re-triggers this rewrite. `Set.of` semantically IS a
//! left fold of `Set.insert` from the empty set (add-or-collapse dedup), so the rewrite is value-exact.
//!
//! Runs at load, alongside `accum::introduce` / `binding_params::lower` (before the parent index /
//! `def_by_name`), so the synthesized def and the rewritten calls resolve + type-check like hand-written
//! source. The fold reuses only ops that already lower on every backend (`List.len`/`List.at`/`Set.insert`/
//! empty `Set.of`), so it is HASH-NEUTRAL — no new `Core`/runtime op, no rust-backend arm needed.
//!
//! ## Known limitation: ONE runtime-`Set.of` element type per program (a defined DECLINE, not a miscompile)
//! The synthesized `__set_of_rt` is a GENERIC recursive def. A program that builds a runtime set at TWO
//! DIFFERENT element types (a `Set Int64` AND a `Set Bool`) instantiates this one generic recursive def at
//! both, which the type checker currently REJECTS (CDZ0201) — the recursive-generic driver tie around
//! `Set.insert` / empty-seed element-var grounding (v-inference's parked territory; a plain generic
//! recursive def at two types works, so it is the Set-op grounding specifically). This is a clean DECLINE,
//! not a wrong answer: before this pass ALL runtime `Set.of` declined, so single-type support is a strict
//! gain and the two-type case merely still declines. When v-inference lands the recursive-generic Set-op
//! grounding fix, this SAME synthesis becomes fully general with no rework — remove this note + flip the
//! two-type negative corpus pin to positive.

use crate::ast::{Arenas, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::Def;
use crate::prelude::{push_atom, push_list};

/// The synthesized fold def's name — a single shared generic def (see the module's known-limitation note
/// on why one generic def, not per-type). The `$`-suffixed spelling cannot collide with a source name.
const FOLD_NAME: &str = "__set_of_rt$";

/// Append a bare `Name` atom occurrence — the synthesis workhorse (a reference or a binder).
fn push_name(ast: &mut Arenas, name: &str) -> StructId {
    push_atom(ast, Leaf::Name(name.into()))
}

/// Whether `id` is a `(. Set of)` member-access head — the head of a `Set.of` application.
fn is_set_of_head(ast: &Arenas, id: StructId) -> bool {
    match ast.get(id) {
        Struct::List(items) if items.len() == 3 => {
            ast.as_name(items[0]) == Some(".")
                && ast.as_name(items[1]) == Some("Set")
                && ast.as_name(items[2]) == Some("of")
        }
        _ => false,
    }
}

/// Whether `id` is a `list` literal FORM — the compile-time list `lower_set_of` already folds. A `Set.of`
/// applied to such a list stays on the constant-fold path and is NOT rewritten here. A list literal has
/// EITHER a NAME head `(list …)` OR a STRING head `("list" …)` — both denote the `list` compound ctor
/// (`ast::compound_ctor_spelling`; the ML `[…]` reader and the s-expr writer differ on which they emit), so
/// BOTH must be recognized: matching only the name form let a string-headed `("list" …)` slip through and
/// be wrongly rewritten into the runtime fold (which broke compiler-ml self-host — `Bytes.of([…])`'s list
/// arrives string-headed).
fn is_list_literal(ast: &Arenas, id: StructId) -> bool {
    match ast.get(id) {
        Struct::List(items) => items
            .first()
            .is_some_and(|&h| ast.as_name(h) == Some("list") || ast.as_str(h) == Some("list")),
        _ => false,
    }
}

/// A `(Set.of ARG)` application whose ARG is not a `(list …)` literal — i.e. a runtime-list construction
/// this pass rewrites. Returns `Some(arg)` (the runtime list occurrence) or `None`.
fn runtime_set_of_arg(ast: &Arenas, id: StructId) -> Option<StructId> {
    match ast.get(id) {
        Struct::List(items) if items.len() == 2 && is_set_of_head(ast, items[0]) => {
            let arg = items[1];
            if is_list_literal(ast, arg) {
                None // a `(list …)` literal — the const-fold path handles it
            } else {
                Some(arg)
            }
        }
        _ => None,
    }
}

/// Rewrite every `(Set.of <runtime-list>)` application into a call to a synthesized recursive fold,
/// appending the one fold def when at least one such site exists. Mutates `ast` (rewriting call nodes +
/// appending the fold's nodes) and `defs` (appending the fold def). A program with no runtime-`Set.of`
/// site is left byte-identical (no def appended).
pub(crate) fn introduce(ast: &mut Arenas, defs: &mut Vec<Def>) {
    // Collect the call-node occurrences to rewrite in ONE immutable scan (every structure node is a
    // candidate — a `Set.of` may sit anywhere: a def body, a `let` init, an argument). Capture the ARG so
    // the rewrite reuses that exact occurrence (it already binds to its enclosing scope).
    let mut sites: Vec<(StructId, StructId)> = Vec::new();
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        if let Some(arg) = runtime_set_of_arg(ast, id) {
            sites.push((id, arg));
        }
    }
    if sites.is_empty() {
        return; // no runtime `Set.of` — leave the program untouched
    }

    // ── Synthesize the ONE generic fold def (see the module note: one generic def; two element types is a
    // defined decline). Build `(def (__set_of_rt$ xs i acc) (if (< i (List.len xs)) (match (List.at xs i)
    // ((Some v) (__set_of_rt$ xs (+ i 1) (Set.insert acc v))) ((None _u) acc)) acc))` as fresh AST that
    // resolves through the ordinary scope walk (each synthesized name binds to the nearest synthesized
    // binder, then to the fold def by name), exactly as `accum`'s accumulator def does.
    let fold_body = synth_fold_body(ast);
    let fold_name = push_name(ast, FOLD_NAME);
    let xs_binder = push_name(ast, "xs");
    let i_binder = push_name(ast, "i");
    let acc_binder = push_name(ast, "acc");
    let sig = push_list(ast, vec![fold_name, xs_binder, i_binder, acc_binder]);
    let def_head = push_name(ast, "def");
    // A real `(def sig body)` form node so a body reference ascends parents to it (resolve Case 4).
    let _def_form = push_list(ast, vec![def_head, sig, fold_body]);
    defs.push(Def {
        name: FOLD_NAME.to_string(),
        sig_occ: sig,
        params: vec![xs_binder, i_binder, acc_binder],
        body: Some(fold_body),
        internal: false,
    });

    // ── Rewrite each site in place: `(Set.of ARG)` → `(__set_of_rt$ ARG 0 (Set.of (list)))`. The ARG
    // occurrence is REUSED verbatim (it keeps its binding); the seed is a fresh empty `(Set.of (list))`
    // (a `(list)` literal, so it folds to the canonical empty set and is not itself a rewrite site).
    for (call_id, arg) in sites {
        let fold_ref = push_name(ast, FOLD_NAME);
        let zero = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(0),
                radix: Radix::Dec,
            },
        );
        let seed = synth_empty_set(ast);
        // Overwrite the `(Set.of ARG)` node IN PLACE with the fold call's children — `call_id` BECOMES
        // `(__set_of_rt$ ARG 0 (Set.of (list)))`, so every reference to this occurrence (an argument, a
        // let init, a def body) now sees the fold call. The `arg` occurrence is reused verbatim (it keeps
        // its binding); the seed is a fresh empty set literal.
        ast.structure[call_id.0 as usize] = Struct::List(vec![fold_ref, arg, zero, seed]);
    }
}

/// Build the empty `(Set.of (list))` seed — `((. Set of) (list))`. A `(list)` with no children folds to
/// the canonical empty set via the const path (so it does not re-trigger this rewrite).
fn synth_empty_set(ast: &mut Arenas) -> StructId {
    let dot = push_name(ast, ".");
    let set_mod = push_name(ast, "Set");
    let of_key = push_name(ast, "of");
    let set_of_head = push_list(ast, vec![dot, set_mod, of_key]);
    let list_head = push_name(ast, "list");
    let empty_list = push_list(ast, vec![list_head]);
    push_list(ast, vec![set_of_head, empty_list])
}

/// Build the fold body: `(if (< i (List.len xs)) (match (List.at xs i) ((Some v) (__set_of_rt$ xs (+ i 1)
/// (Set.insert acc v))) ((None _u) acc)) acc)`. All names resolve through the enclosing synthesized `(def
/// (__set_of_rt$ xs i acc) …)` sig via the scope walk.
fn synth_fold_body(ast: &mut Arenas) -> StructId {
    // `(< i (List.len xs))` — the in-bounds guard.
    let list_len = {
        let dot = push_name(ast, ".");
        let list_mod = push_name(ast, "List");
        let len_key = push_name(ast, "len");
        let head = push_list(ast, vec![dot, list_mod, len_key]);
        let xs = push_name(ast, "xs");
        push_list(ast, vec![head, xs])
    };
    let guard = {
        let lt = push_name(ast, "<");
        let i = push_name(ast, "i");
        push_list(ast, vec![lt, i, list_len])
    };

    // `(List.at xs i)` — the fallible read.
    let list_at = {
        let dot = push_name(ast, ".");
        let list_mod = push_name(ast, "List");
        let at_key = push_name(ast, "at");
        let head = push_list(ast, vec![dot, list_mod, at_key]);
        let xs = push_name(ast, "xs");
        let i = push_name(ast, "i");
        push_list(ast, vec![head, xs, i])
    };

    // `(Set.insert acc v)` — insert the read element into the running set.
    let set_insert = {
        let dot = push_name(ast, ".");
        let set_mod = push_name(ast, "Set");
        let insert_key = push_name(ast, "insert");
        let head = push_list(ast, vec![dot, set_mod, insert_key]);
        let acc = push_name(ast, "acc");
        let v = push_name(ast, "v");
        push_list(ast, vec![head, acc, v])
    };
    // `(+ i 1)` — advance the index.
    let next_i = {
        let plus = push_name(ast, "+");
        let i = push_name(ast, "i");
        let one = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(1),
                radix: Radix::Dec,
            },
        );
        push_list(ast, vec![plus, i, one])
    };
    // `(__set_of_rt$ xs (+ i 1) (Set.insert acc v))` — the tail self-call.
    let rec_call = {
        let fold_ref = push_name(ast, FOLD_NAME);
        let xs = push_name(ast, "xs");
        push_list(ast, vec![fold_ref, xs, next_i, set_insert])
    };

    // `((Some v) <rec_call>)` and `((None _u) acc)` — the match arms.
    let some_arm = {
        let some = push_name(ast, "Some");
        let v = push_name(ast, "v");
        let pat = push_list(ast, vec![some, v]);
        push_list(ast, vec![pat, rec_call])
    };
    let none_arm = {
        let none = push_name(ast, "None");
        let u = push_name(ast, "_u");
        let pat = push_list(ast, vec![none, u]);
        let acc = push_name(ast, "acc");
        push_list(ast, vec![pat, acc])
    };
    let match_expr = {
        let match_head = push_name(ast, "match");
        push_list(ast, vec![match_head, list_at, some_arm, none_arm])
    };

    // `(if <guard> <match> acc)` — recurse while in bounds, else return the accumulated set.
    let if_head = push_name(ast, "if");
    let acc_base = push_name(ast, "acc");
    push_list(ast, vec![if_head, guard, match_expr, acc_base])
}
