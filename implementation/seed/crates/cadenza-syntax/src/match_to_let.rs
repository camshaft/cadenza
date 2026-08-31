//! The `match`→`let` normalization codemod (an OPT-IN rewrite, NOT part of `cdz fmt`).
//!
//! Rewrites a SINGLE-CLAUSE `match` whose sole arm binds an IRREFUTABLE, UNGUARDED pattern into the
//! equivalent `let`: `match v with | (a, b) => body` → `let (a, b) = v in body`. This is the idiom
//! cleanup the operator asked for — a single-arm match that can never fail reads more directly as a
//! `let`-binding. It is delivered as an OPT-IN codemod (`cdz normalize --match-to-let`) rather than
//! folded into `cdz fmt`, because it is a CONTRACT-BREAKING AST canonicalization: the two forms have
//! DIFFERENT arenas (`(match v ((tuple a b) body))` vs `(let (((tuple a b) v)) body)`), so wiring it
//! into `fmt` would fail `corpus_roundtrip`'s STRUCTURAL round-trip on the ~37 existing single-clause
//! matches in the corpus. Keeping `fmt` structure-preserving preserves the "ML surface preserves your
//! AST exactly" guarantee the compiler/LSP/corpus-gate rest on; the normalization is a separate,
//! explicitly-invoked surface whose output re-reads as a DIFFERENT-but-equivalent tree — that is the
//! whole point of the codemod.
//!
//! ## Irrefutability (the correctness crux)
//!
//! A `let` binding cannot fail; an incomplete `match` TRAPS. So we may ONLY rewrite a clause whose
//! pattern is IRREFUTABLE — it matches every value of its type — else the rewrite would erase a trap
//! and change the program's meaning. This pass is PURELY SYNTACTIC (no type information), so it decides
//! only SHAPE-irrefutable patterns and is conservative on everything else:
//!
//! - `_` (wildcard) — irrefutable.
//! - a bare lowercase `Name` (a variable binder, e.g. `x`) — irrefutable.
//! - `(tuple p…)` — irrefutable iff EVERY element `p` is irrefutable (recurse).
//! - `(record (field p)…)` — irrefutable iff every field's sub-pattern `p` is irrefutable (recurse);
//!   a PARTIAL record (fewer fields than the type has) is still irrefutable — a missing field just
//!   is not bound.
//!
//! A `(Ctor p…)` sum-constructor pattern (CAPITALIZED head, `Some`/`Wrap`) is irrefutable ONLY when
//! its type has exactly ONE variant (there is no other variant to fall through to) AND its
//! sub-patterns are irrefutable. [`single_variant_ctors`] scans the program's own `(type …)` decls to
//! find such ctors; [`rewrite`] threads that set into the predicate. A MULTI-variant ctor, or one
//! whose type is not declared in this program (e.g. imported — its variant count is invisible), stays
//! conservatively REFUTABLE. [`is_irrefutable`] (no context) treats every ctor as refutable.
//!
//! Everything else is REFUTABLE and left untouched:
//! - a literal pattern (`0`, `"x"`, `true`, …) — matches only that value.
//! - a `(guard pat cond)` — the guard is a run-time condition that can fail.
//! - a nested REFUTABLE sub-pattern makes the whole pattern refutable: `(tuple a (Some x))` is
//!   shape-irrefutable at the outer `tuple` but its `(Some x)` element is refutable, so the whole
//!   thing is refutable (the recursion above enforces this).

use crate::ast::{CompoundCtor, Leaf};
use crate::query::Tree;
use std::collections::BTreeSet;

/// The set of constructor names that are the SOLE variant of their (same-program) sum type — so a
/// `(Ctor …)` pattern using one of them CANNOT fail (there is no other variant to miss), making it
/// irrefutable given its sub-patterns are. Built by [`single_variant_ctors`] from the program's own
/// `(type …)` declarations; a ctor from an IMPORT (whose decl this program can't see) is never in the
/// set, so it stays conservatively refutable.
pub type SingleVariantCtors = BTreeSet<String>;

/// True if `pat` is SHAPE-irrefutable, decidable WITHOUT type information (empty single-variant-ctor
/// context). The shape-only entry point: `_`/var/tuple/record (recursive) → irrefutable; a `(Ctor …)`
/// sum-constructor pattern is REFUTABLE here (no type info to prove it single-variant). Use
/// [`is_irrefutable_with`] to also accept single-variant-sum ctors.
pub fn is_irrefutable(pat: &Tree) -> bool {
    is_irrefutable_with(pat, &SingleVariantCtors::new())
}

/// True if `pat` is irrefutable GIVEN `single_ctors` — the shape rules of [`is_irrefutable`] PLUS: a
/// `(Ctor sub…)` pattern is irrefutable iff `Ctor` is in `single_ctors` (its type has exactly one
/// variant, so the match can't fall through) AND every sub-pattern is irrefutable (recurse). See the
/// module docs for the full set; recursion makes a refutable sub-pattern poison the whole pattern.
pub fn is_irrefutable_with(pat: &Tree, single_ctors: &SingleVariantCtors) -> bool {
    match pat {
        // A bare atom: a variable binder or `_` is irrefutable; a literal is refutable.
        Tree::Atom(Leaf::Name(n), _) => is_var_name(n),
        Tree::Atom(_, _) => false, // any non-Name leaf is a literal → refutable
        Tree::List(items, _) => {
            let Some((head, rest)) = items.split_first() else {
                return false; // an empty list is not a pattern shape we rewrite
            };
            // A TUPLE pattern head — the NATIVE `#tuple(…)` (`Leaf::Ctor(Tuple)`, post-M3) or the legacy
            // shadowable NAME head `tuple`. Irrefutable iff every sub-pattern is.
            if is_compound_head(head, CompoundCtor::Tuple, "tuple") {
                return rest.iter().all(|p| is_irrefutable_with(p, single_ctors));
            }
            // A RECORD pattern head — the NATIVE `#record(…)` (`Leaf::Ctor(Record)`) or the legacy NAME head
            // `record`. Each field is the canonical `(= fieldname subpat)` triple (path B — same form as a
            // value-record field); the sub-pattern is the LAST element. The `=` head is the native
            // `Leaf::FieldPair` (M2 native-compound-data) or a legacy `Name("=")` (dual-read). A bare
            // `(fieldname subpat)` pair is also tolerated (sub-pattern = 2nd element).
            if is_compound_head(head, CompoundCtor::Record, "record") {
                return rest.iter().all(|field| match field {
                    Tree::List(fitems, _)
                        if fitems.len() == 3
                            && (matches!(&fitems[0], Tree::Atom(Leaf::FieldPair, _))
                                || matches!(&fitems[0], Tree::Atom(Leaf::Name(eq), _) if &**eq == "=")) =>
                    {
                        is_irrefutable_with(&fitems[2], single_ctors)
                    }
                    Tree::List(fitems, _) if fitems.len() == 2 => {
                        is_irrefutable_with(&fitems[1], single_ctors)
                    }
                    // A bare field name `(record x)` shorthand binds `x` irrefutably.
                    Tree::Atom(Leaf::Name(n), _) => is_var_name(n),
                    _ => false,
                });
            }
            match head {
                // A `(Ctor sub…)` sum-constructor pattern: irrefutable ONLY when `Ctor` is the sole
                // variant of its (same-program) type AND every sub-pattern is irrefutable. A `guard`
                // head (`(guard …)`) is never capitalized so it falls through to refutable here.
                Tree::Atom(Leaf::Name(h), _) if is_ctor_name(h) && single_ctors.contains(&**h) => {
                    rest.iter().all(|p| is_irrefutable_with(p, single_ctors))
                }
                // `(Ctor …)` not known-single-variant → refutable; `(guard …)` → refutable; else refutable.
                _ => false,
            }
        }
    }
}

/// Scan `program` (a whole-program tree) for its top-level `(type Name variant…)` declarations and
/// return the set of constructor names that are the SOLE variant of their type. A `(type …)` form's
/// tail is `Name` then the variants; a variant is either a bare `Name` atom (nullary) or a
/// `(Ctor payload…)` list. A type contributes its ctor iff it is CLOSED and has EXACTLY ONE variant. An
/// OPEN sum (a trailing `.. r` row marker) contributes NOTHING even with one listed variant — the row
/// var stands for unnamed variants, so no listed ctor is statically the sole one (its pattern stays
/// refutable). Only same-program decls are scanned; an imported type is invisible here, so its ctors
/// stay out of the set (conservatively refutable).
pub fn single_variant_ctors(program: &Tree) -> SingleVariantCtors {
    let mut out = SingleVariantCtors::new();
    collect_type_decls(program, &mut out);
    out
}

fn collect_type_decls(tree: &Tree, out: &mut SingleVariantCtors) {
    if let Tree::List(items, _) = tree {
        if let Some(Tree::Atom(Leaf::Name(h), _)) = items.first()
            && &**h == "type"
            && items.len() >= 3
        {
            // items = [type, Name, variant…]. A trailing `.. r` open-sum marker (a `Name("..")` then a
            // lowercase row var) means the sum is OPEN: the row variable stands for variants NOT named
            // here, so NO listed ctor is ever the SOLE variant — a value may be an unnamed variant, so
            // even a single-listed-ctor open sum has a REFUTABLE ctor pattern (a match needs an open-tail
            // `_` arm; §206). This mirrors the compiler's `newtype_underlying`, which refuses to treat an
            // open single-variant sum as a newtype for exactly this reason. So: only harvest a sole ctor
            // as single-variant when the sum is CLOSED (no `..` marker). (Peeling the marker and then
            // inserting the sole ctor — the pre-fix behavior — wrongly marked an open sum's ctor
            // irrefutable, which would let the codemod erase a non-exhaustive match's refutability.)
            let variants = &items[2..];
            let is_open = variants.len() >= 2
                && matches!(&variants[variants.len() - 2], Tree::Atom(Leaf::Name(d), _) if &**d == "..");
            if !is_open
                && variants.len() == 1
                && let Some(name) = ctor_name_of(&variants[0])
            {
                out.insert(name);
            }
        }
        for c in items {
            collect_type_decls(c, out);
        }
    }
}

/// The constructor name of a single variant entry: a bare `Name` atom (`Wrap`) or a `(Ctor payload…)`
/// list's head (`Wrap` in `(Wrap Int64)`). `None` for a malformed variant. Only a Capitalized name is a
/// real ctor (a lowercase row-var/type-param is not), matching the surface convention.
fn ctor_name_of(variant: &Tree) -> Option<String> {
    let name = match variant {
        Tree::Atom(Leaf::Name(n), _) => n,
        Tree::List(items, _) => match items.first() {
            Some(Tree::Atom(Leaf::Name(n), _)) => n,
            _ => return None,
        },
        _ => return None,
    };
    is_ctor_name(name).then(|| name.to_string())
}

/// A Capitalized name is a constructor (`Wrap`, `Some`); a lowercase one is a var/type-param.
fn is_ctor_name(n: &str) -> bool {
    n.chars().next().is_some_and(char::is_uppercase)
}

/// True if `head` is the head of a `ctor` compound pattern — the NATIVE `Leaf::Ctor(ctor)` (the post-M3
/// `#tuple(…)`/`#record(…)` surface) OR the legacy shadowable NAME head (`tuple`/`record`). The two spell
/// the same compound; the codemod must accept the native head so `--match-to-let` works on CURRENT code,
/// not only legacy name-headed patterns (before this it no-oped on native `#tuple`/`#record` matches).
fn is_compound_head(head: &Tree, ctor: CompoundCtor, legacy: &str) -> bool {
    match head {
        Tree::Atom(Leaf::Ctor(c), _) => *c == ctor,
        Tree::Atom(Leaf::Name(h), _) => &**h == legacy,
        _ => false,
    }
}

/// A lowercase-led name is a variable/wildcard binder (irrefutable); a Capitalized name is a nullary
/// CONSTRUCTOR (refutable — it matches only that one variant). `_` is the wildcard (irrefutable). Empty
/// names never occur in a well-formed pattern but are treated as non-vars defensively.
fn is_var_name(n: &str) -> bool {
    if n == "_" {
        return true;
    }
    match n.chars().next() {
        Some(c) => !c.is_uppercase(),
        None => false,
    }
}

/// Rewrite EVERY single-clause irrefutable-unguarded `match` in `tree` into the equivalent `let`,
/// bottom-up (so a match nested inside another match's body is rewritten too). Returns the new tree and
/// the number of rewrites performed. A `match` is rewritten iff it has the shape
/// `(match SCRUT (PAT BODY))` — exactly ONE clause, that clause a 2-element `(PAT BODY)` list, and PAT
/// [`is_irrefutable`] — producing `(let ((PAT SCRUT)) BODY)`. All other `match` forms are left as-is.
pub fn rewrite(tree: &Tree) -> (Tree, usize) {
    // Scan the WHOLE program first for single-variant sum types, so a `(Ctor …)` pattern on a
    // single-variant type is accepted as irrefutable (type-aware). The scan is over the same tree we
    // rewrite, so a type declared anywhere in the program (before or after the match) is seen.
    let single_ctors = single_variant_ctors(tree);
    let mut count = 0;
    let out = rewrite_rec(tree, &single_ctors, &mut count);
    (out, count)
}

fn rewrite_rec(tree: &Tree, single_ctors: &SingleVariantCtors, count: &mut usize) -> Tree {
    // Bottom-up: rewrite children first, then attempt this node. Uses native recursion; arena depth is
    // bounded by the reader's `MAX_NESTING_DEPTH` for parsed input (a codemod runs on parsed trees).
    let rewritten = match tree {
        Tree::Atom(_, _) => tree.clone(),
        Tree::List(items, origin) => {
            let kids: Vec<Tree> = items
                .iter()
                .map(|c| rewrite_rec(c, single_ctors, count))
                .collect();
            Tree::List(kids, *origin)
        }
    };
    if let Some(lowered) = try_match_to_let(&rewritten, single_ctors) {
        *count += 1;
        return lowered;
    }
    rewritten
}

/// If `tree` is `(match SCRUT (PAT BODY))` with exactly one clause whose PAT is irrefutable (given
/// `single_ctors`) and unguarded, return `(let ((PAT SCRUT)) BODY)`; else `None`.
fn try_match_to_let(tree: &Tree, single_ctors: &SingleVariantCtors) -> Option<Tree> {
    let Tree::List(items, _) = tree else {
        return None;
    };
    // (match SCRUT CLAUSE) — head, scrutinee, exactly one clause.
    if items.len() != 3 {
        return None;
    }
    match &items[0] {
        Tree::Atom(Leaf::Name(h), _) if &**h == "match" => {}
        _ => return None,
    }
    let scrut = &items[1];
    // The single clause must be a 2-element `(PAT BODY)` list.
    let Tree::List(clause, _) = &items[2] else {
        return None;
    };
    if clause.len() != 2 {
        return None;
    }
    let (pat, body) = (&clause[0], &clause[1]);
    if !is_irrefutable_with(pat, single_ctors) {
        return None;
    }
    // Build `(let ((PAT SCRUT)) BODY)`: a let-head, a binding-list holding one `(PAT SCRUT)` pair, body.
    let binding = Tree::List(vec![pat.clone(), scrut.clone()], None);
    let bindings = Tree::List(vec![binding], None);
    let let_head = Tree::Atom(Leaf::Name("let".into()), None);
    Some(Tree::List(vec![let_head, bindings, body.clone()], None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use crate::query::Tree;

    // The `normalize_ml` / `normalize_sexpr` / `count_rewrites` round-trip helpers were deleted with the
    // last of their callers when the match_to_let behavioral tests migrated to the spec/syntax codemod
    // corpus (inc-6) — the codemod goldens (`normalize.match-to-let.<ext>`) now drive `cdz normalize
    // --match-to-let` through gate-syntax + the per-case nix check, so no in-crate normalize helper is
    // needed. Only the internal PREDICATE unit tests below (`rewrite`/`is_irrefutable`/`single_variant
    // _ctors`) still exercise this module directly.

    // `irrefutable_single_clause_matches_become_lets` MIGRATED to the spec/syntax codemod corpus
    // (v-syntax green-lit match_to_let; a codemod belongs in the corpus, not inc-6): ml/25-match-to-let-
    // tuple, ml/26-…-var, ml/27-…-wildcard, ml/28-…-record — each pins the `cdz normalize --match-to-let`
    // output (a `normalize.match-to-let.cdz` golden) + the input's parse tree, graded by the per-case nix
    // check + gate-syntax. (The is_irrefutable/single_variant_ctors PREDICATE unit tests stay Rust — internal.)

    // native_compound_head_single_arm (native #tuple/#record single-arm match → let, + a refutable
    // sub-pattern stays) and refutable_or_multi_clause_matches_are_left_unchanged (6 refutable/multi-
    // clause matches that must NOT rewrite — the trap-preservation safety invariant) MIGRATED to the
    // spec/syntax codemod corpus (inc-6/codemod): sexp/37-39 (native tuple/record convert + refutable
    // no-op) + ml/29-34 (refutable sum-ctor/literal/guarded/nested + 2 multi-clause) — each pins the
    // `cdz normalize --match-to-let` output (a rewrite, or unchanged for a left-alone match).

    // single_variant_sum / multi_variant_or_imported / open_sum_sole_ctor (type-aware irrefutability:
    // a CLOSED single-variant ctor lowers, a multi-variant / no-decl / open-sum / unrelated ctor stays
    // refutable), rewrite_is_idempotent, and nested_match_bottom_up MIGRATED to the spec/syntax codemod
    // corpus: ml/35-43 — each pins the `cdz normalize --match-to-let` output (rewrite / unchanged /
    // nested double-lower / already-lowered no-op). The is_irrefutable + single_variant_ctors PREDICATE
    // unit tests below stay Rust (internal). match_to_let behavioral tests fully delanguaged now.

    #[test]
    fn single_variant_ctors_scan_direct() {
        // The scan finds exactly the CLOSED sole-variant ctors, ignoring multi-variant types AND open
        // sums (a `.. r` row-var tail means no listed ctor is statically the sole variant).
        let a = parser::read_ml(
            "type A = | Only(Int64)\ntype B = | X | Y\ntype C = | Solo\ntype D = | Lone(Int64) .. r\ndef m() = 0",
        );
        assert!(a.ok());
        let set = single_variant_ctors(&Tree::of(&a.arenas));
        assert!(set.contains("Only"), "closed single-variant payload ctor");
        assert!(set.contains("Solo"), "closed single-variant nullary ctor");
        assert!(
            !set.contains("X") && !set.contains("Y"),
            "multi-variant ctors excluded"
        );
        assert!(
            !set.contains("Lone"),
            "an OPEN sum's sole ctor is excluded (row var → not statically sole)"
        );
    }

    #[test]
    fn is_irrefutable_predicate_direct() {
        // Direct unit coverage of the predicate on parsed pattern shapes (via a whole-match parse).
        let irref = |pat_src: &str| {
            let src = format!("def f(p) = match p with | {pat_src} => 0");
            let a = parser::read_ml(&src);
            assert!(a.ok(), "parse {src:?}");
            let tree = Tree::of(&a.arenas);
            // dig out the match clause's pattern: (do? (def (f p) (match p (PAT 0))))
            find_first_match_pattern(&tree).map(|p| is_irrefutable(&p))
        };
        assert_eq!(irref("_"), Some(true));
        assert_eq!(irref("x"), Some(true));
        assert_eq!(irref("(a, b)"), Some(true));
        assert_eq!(irref("{ x = a }"), Some(true));
        assert_eq!(irref("Some(x)"), Some(false));
        assert_eq!(irref("0"), Some(false));
        assert_eq!(irref("(a, Some(x))"), Some(false));
    }

    /// Walk `tree` to the first `(match SCRUT (PAT BODY))` and return a clone of its PAT.
    fn find_first_match_pattern(tree: &Tree) -> Option<Tree> {
        if let Tree::List(items, _) = tree {
            if let Some(Tree::Atom(Leaf::Name(h), _)) = items.first()
                && &**h == "match"
                && items.len() == 3
                && let Tree::List(clause, _) = &items[2]
                && clause.len() == 2
            {
                return Some(clause[0].clone());
            }
            for c in items {
                if let Some(p) = find_first_match_pattern(c) {
                    return Some(p);
                }
            }
        }
        None
    }
}
