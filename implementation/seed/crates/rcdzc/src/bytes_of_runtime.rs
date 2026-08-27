//! BYTES.OF OVER A RUNTIME LIST — rewrite `(Bytes.of xs)` where `xs` is NOT a compile-time `(list …)`
//! literal into a call to a synthesized recursive fold that appends each byte onto a growing `Bytes`.
//!
//! `lower_bytes_of` folds a CONSTANT `(list …)` literal (its elements are compile-time-visible `UInt8`s)
//! to a `Core::BytesOf`, but a `Bytes` built from a RUNTIME list — a `List.concat` of `UInt8` lists, a
//! param/recursively-built `(List UInt8)` — has no visible element list to fold, so `lower` declined it
//! ("Bytes.of of a runtime list is not yet supported"). This pre-pass supplies the runtime construction by
//! SYNTHESIS, mirroring `set_of_runtime`: it appends ONE fold def
//!
//! ```text
//! (def (__bytes_of_rt xs i acc)
//!   (if (< i (List.len xs))
//!       (match (List.at xs i)
//!         ((Some v) (__bytes_of_rt xs (+ i 1) (Bytes.concat acc (Bytes.of (list v)))))
//!         ((None _u) acc))
//!       acc))
//! ```
//!
//! and rewrites each `(Bytes.of xs)` whose `xs` is not a `(list …)` literal into `(__bytes_of_rt xs 0
//! (Bytes.of (list)))`. The combine `(Bytes.concat acc (Bytes.of (list v)))` appends the single byte `v`:
//! `(Bytes.of (list v))` is a `(list v)` literal (runtime ELEMENT, literal STRUCTURE), so it folds to a
//! one-byte `Bytes` via the const path — it is NOT itself a rewrite site — and the empty `(Bytes.of
//! (list))` seed likewise folds to the empty `Bytes`. `Bytes.of` is a left fold of "append one byte" from
//! the empty `Bytes`, so the rewrite is value-exact.
//!
//! Runs at load beside `accum::introduce` / `set_of_runtime::introduce` (before the parent index /
//! `def_by_name`), so the synthesis resolves + type-checks like hand-written source. Reuses only ops that
//! already lower on every backend (`List.len`/`List.at`/`Bytes.concat`/`Bytes.of` of a literal), so it is
//! HASH-NEUTRAL — no new `Core`/runtime op, no rust-backend arm.
//!
//! Unlike `set_of_runtime`, `Bytes.of : (List UInt8) → Bytes` is MONOMORPHIC (the element is always
//! `UInt8`), so the synthesized fold has a single instantiation — there is NO multi-element-type
//! limitation.

use crate::ast::{Arenas, CompoundCtor, IntValue, Leaf, Radix, Struct, StructId};
use crate::db::Def;
use crate::prelude::{push_atom, push_list};

/// The synthesized fold def's name. The `$`-suffixed spelling cannot collide with a source name.
const FOLD_NAME: &str = "__bytes_of_rt$";

/// Append a bare `Name` atom occurrence — the synthesis workhorse (a reference or a binder).
fn push_name(ast: &mut Arenas, name: &str) -> StructId {
    push_atom(ast, Leaf::Name(name.into()))
}

/// Whether `id` is a `(. Bytes of)` member-access head — the head of a `Bytes.of` application.
fn is_bytes_of_head(ast: &Arenas, id: StructId) -> bool {
    match ast.get(id) {
        Struct::List(items) if items.len() == 3 => {
            ast.as_name(items[0]) == Some(".")
                && ast.as_name(items[1]) == Some("Bytes")
                && ast.as_name(items[2]) == Some("of")
        }
        _ => false,
    }
}

/// Whether `id` is a `list` literal FORM — the compile-time list `lower_bytes_of` already folds. A
/// `Bytes.of` applied to such a list stays on the constant-fold path and is NOT rewritten here. A list
/// literal has EITHER a NAME head `(list …)` OR a STRING head `("list" …)` — both denote the `list`
/// compound ctor (the ML `[…]` reader and the s-expr writer differ on which they emit), so BOTH must be
/// recognized: matching only the name form let a string-headed `("list" …)` slip through and be wrongly
/// rewritten into the runtime fold (which broke compiler-ml self-host — `Bytes.of([…])`'s list arrives
/// string-headed).
fn is_list_literal(ast: &Arenas, id: StructId) -> bool {
    ast.compound_ctor_either(id) == Some(CompoundCtor::List)
}

/// A `(Bytes.of ARG)` application whose ARG is not a `(list …)` literal — a runtime-list construction this
/// pass rewrites. Returns `Some(arg)` (the runtime list occurrence) or `None`.
fn runtime_bytes_of_arg(ast: &Arenas, id: StructId) -> Option<StructId> {
    match ast.get(id) {
        Struct::List(items) if items.len() == 2 && is_bytes_of_head(ast, items[0]) => {
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

/// Rewrite every `(Bytes.of <runtime-list>)` application into a call to a synthesized recursive fold,
/// appending the one fold def when at least one such site exists. Mutates `ast` (rewriting call nodes +
/// appending the fold's nodes) and `defs` (appending the fold def). A program with no runtime-`Bytes.of`
/// site is left byte-identical (no def appended).
pub(crate) fn introduce(ast: &mut Arenas, defs: &mut Vec<Def>) {
    // Collect the call-node occurrences to rewrite in ONE immutable scan — a `Bytes.of` may sit anywhere.
    // Capture the ARG so the rewrite reuses that exact occurrence (it already binds to its enclosing scope).
    let mut sites: Vec<(StructId, StructId)> = Vec::new();
    for i in 0..ast.structure.len() {
        let id = StructId(i as u32);
        if let Some(arg) = runtime_bytes_of_arg(ast, id) {
            sites.push((id, arg));
        }
    }
    if sites.is_empty() {
        return; // no runtime `Bytes.of` — leave the program untouched
    }

    // ── Synthesize the ONE fold def: `(def (__bytes_of_rt$ xs i acc) (if (< i (List.len xs)) (match
    // (List.at xs i) ((Some v) (__bytes_of_rt$ xs (+ i 1) (Bytes.concat acc (Bytes.of (list v))))) ((None
    // _u) acc)) acc))` as fresh AST that resolves through the ordinary scope walk.
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

    // ── Rewrite each site in place: `(Bytes.of ARG)` → `(__bytes_of_rt$ ARG 0 (Bytes.of (list)))`.
    for (call_id, arg) in sites {
        let fold_ref = push_name(ast, FOLD_NAME);
        let zero = push_atom(
            ast,
            Leaf::Int {
                value: IntValue::from_i64(0),
                radix: Radix::Dec,
            },
        );
        let seed = synth_empty_bytes(ast);
        // Overwrite the `(Bytes.of ARG)` node IN PLACE with the fold call's children — `call_id` BECOMES
        // `(__bytes_of_rt$ ARG 0 (Bytes.of (list)))`. The `arg` occurrence is reused verbatim (it keeps
        // its binding); the seed is a fresh empty byte sequence literal.
        ast.structure[call_id.0 as usize] = Struct::List(vec![fold_ref, arg, zero, seed]);
    }
}

/// Build the empty `(Bytes.of (list))` seed — `((. Bytes of) (list))`. A `(list)` with no children folds
/// to the empty `Bytes` via the const path (so it does not re-trigger this rewrite).
fn synth_empty_bytes(ast: &mut Arenas) -> StructId {
    let bytes_of_head = synth_bytes_of_head(ast);
    let list_head = push_name(ast, "list");
    let empty_list = push_list(ast, vec![list_head]);
    push_list(ast, vec![bytes_of_head, empty_list])
}

/// Build a `(. Bytes of)` member-access head node.
fn synth_bytes_of_head(ast: &mut Arenas) -> StructId {
    let dot = push_name(ast, ".");
    let bytes_mod = push_name(ast, "Bytes");
    let of_key = push_name(ast, "of");
    push_list(ast, vec![dot, bytes_mod, of_key])
}

/// Build the fold body: `(if (< i (List.len xs)) (match (List.at xs i) ((Some v) (__bytes_of_rt$ xs (+ i
/// 1) (Bytes.concat acc (Bytes.of (list v))))) ((None _u) acc)) acc)`.
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

    // `(Bytes.of (list v))` — a single-byte Bytes (a `(list v)` literal, so it folds via the const path).
    let one_byte = {
        let bytes_of_head = synth_bytes_of_head(ast);
        let list_head = push_name(ast, "list");
        let v = push_name(ast, "v");
        let single = push_list(ast, vec![list_head, v]);
        push_list(ast, vec![bytes_of_head, single])
    };
    // `(Bytes.concat acc (Bytes.of (list v)))` — append the read byte onto the running Bytes.
    let bytes_concat = {
        let dot = push_name(ast, ".");
        let bytes_mod = push_name(ast, "Bytes");
        let concat_key = push_name(ast, "concat");
        let head = push_list(ast, vec![dot, bytes_mod, concat_key]);
        let acc = push_name(ast, "acc");
        push_list(ast, vec![head, acc, one_byte])
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
    // `(__bytes_of_rt$ xs (+ i 1) (Bytes.concat acc (Bytes.of (list v))))` — the tail self-call.
    let rec_call = {
        let fold_ref = push_name(ast, FOLD_NAME);
        let xs = push_name(ast, "xs");
        push_list(ast, vec![fold_ref, xs, next_i, bytes_concat])
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

    // `(if <guard> <match> acc)` — append while in bounds, else return the accumulated Bytes.
    let if_head = push_name(ast, "if");
    let acc_base = push_name(ast, "acc");
    push_list(ast, vec![if_head, guard, match_expr, acc_base])
}
