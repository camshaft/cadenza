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
//! Everything else is REFUTABLE and left untouched:
//! - a `(Ctor p…)` whose head is CAPITALIZED (a sum constructor like `Some`/`Cons`) — a multi-variant
//!   sum ctor can fail. A single-variant sum's ctor IS irrefutable, but proving that needs a type-decl
//!   scan (no type info here), so it is deferred to a later type-aware slice.
//! - a literal pattern (`0`, `"x"`, `true`, …) — matches only that value.
//! - a `(guard pat cond)` — the guard is a run-time condition that can fail.
//! - a nested REFUTABLE sub-pattern makes the whole pattern refutable: `(tuple a (Some x))` is
//!   shape-irrefutable at the outer `tuple` but its `(Some x)` element is refutable, so the whole
//!   thing is refutable (the recursion above enforces this).

use crate::ast::Leaf;
use crate::query::Tree;

/// True if `pat` is SHAPE-irrefutable — it matches every value of its type, decidable without type
/// information. See the module docs for the exact set. Recurses into `tuple`/`record` sub-patterns so a
/// refutable sub-pattern (`(tuple a (Some x))`) makes the whole pattern refutable.
pub fn is_irrefutable(pat: &Tree) -> bool {
    match pat {
        // A bare atom: a variable binder or `_` is irrefutable; a literal is refutable.
        Tree::Atom(Leaf::Name(n), _) => is_var_name(n),
        Tree::Atom(_, _) => false, // any non-Name leaf is a literal → refutable
        Tree::List(items, _) => {
            let Some((head, rest)) = items.split_first() else {
                return false; // an empty list is not a pattern shape we rewrite
            };
            match head {
                Tree::Atom(Leaf::Name(h), _) if h == "tuple" => rest.iter().all(is_irrefutable),
                Tree::Atom(Leaf::Name(h), _) if h == "record" => {
                    // Each field is `(fieldname subpat)`; the sub-pattern is the 2nd element.
                    rest.iter().all(|field| match field {
                        Tree::List(fitems, _) if fitems.len() == 2 => is_irrefutable(&fitems[1]),
                        // A bare field name `(record x)` shorthand binds `x` irrefutably.
                        Tree::Atom(Leaf::Name(n), _) => is_var_name(n),
                        _ => false,
                    })
                }
                // `(Ctor …)` (capitalized head) = sum ctor → refutable; `(guard …)` → refutable;
                // anything else → refutable.
                _ => false,
            }
        }
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
    let mut count = 0;
    let out = rewrite_rec(tree, &mut count);
    (out, count)
}

fn rewrite_rec(tree: &Tree, count: &mut usize) -> Tree {
    // Bottom-up: rewrite children first, then attempt this node. Uses native recursion; arena depth is
    // bounded by the reader's `MAX_NESTING_DEPTH` for parsed input (a codemod runs on parsed trees).
    let rewritten = match tree {
        Tree::Atom(_, _) => tree.clone(),
        Tree::List(items, origin) => {
            let kids: Vec<Tree> = items.iter().map(|c| rewrite_rec(c, count)).collect();
            Tree::List(kids, *origin)
        }
    };
    if let Some(lowered) = try_match_to_let(&rewritten) {
        *count += 1;
        return lowered;
    }
    rewritten
}

/// If `tree` is `(match SCRUT (PAT BODY))` with exactly one clause whose PAT is irrefutable and
/// unguarded, return `(let ((PAT SCRUT)) BODY)`; else `None`.
fn try_match_to_let(tree: &Tree) -> Option<Tree> {
    let Tree::List(items, _) = tree else {
        return None;
    };
    // (match SCRUT CLAUSE) — head, scrutinee, exactly one clause.
    if items.len() != 3 {
        return None;
    }
    match &items[0] {
        Tree::Atom(Leaf::Name(h), _) if h == "match" => {}
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
    if !is_irrefutable(pat) {
        return None;
    }
    // Build `(let ((PAT SCRUT)) BODY)`: a let-head, a binding-list holding one `(PAT SCRUT)` pair, body.
    let binding = Tree::List(vec![pat.clone(), scrut.clone()], None);
    let bindings = Tree::List(vec![binding], None);
    let let_head = Tree::Atom(Leaf::Name("let".to_string()), None);
    Some(Tree::List(vec![let_head, bindings, body.clone()], None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use crate::printer;
    use crate::query::Tree;

    /// Parse ML, run the codemod, return the printed ML result.
    fn normalize_ml(src: &str) -> String {
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse {src:?}: {:?}", a.errors);
        let tree = Tree::of(&a.arenas);
        let (out, _) = rewrite(&tree);
        printer::print(&out.to_arena(), 100)
    }

    /// Parse ML → the codemod's s-expr output (structure, not layout).
    fn normalize_sexpr(src: &str) -> String {
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse {src:?}: {:?}", a.errors);
        let tree = Tree::of(&a.arenas);
        let (out, n) = rewrite(&tree);
        (crate::sexpr::print(&out.to_arena()), n).0
    }

    fn count_rewrites(src: &str) -> usize {
        let a = parser::read_ml(src);
        assert!(a.ok(), "parse {src:?}: {:?}", a.errors);
        rewrite(&Tree::of(&a.arenas)).1
    }

    #[test]
    fn irrefutable_single_clause_matches_become_lets() {
        // tuple / record / var / wildcard patterns are irrefutable → rewritten.
        assert!(
            normalize_sexpr("def f(p) = match p with | (a, b) => a + b")
                .contains("(let (((tuple a b) p)) (+ a b))")
        );
        assert!(normalize_sexpr("def f(p) = match p with | x => x").contains("(let ((x p)) x)"));
        assert!(normalize_sexpr("def f(p) = match p with | _ => 9").contains("(let ((_ p)) 9)"));
        assert!(
            normalize_sexpr("def f(p) = match p with | { x = a } => a")
                .contains("(let (((record (x a)) p)) a)")
        );
    }

    #[test]
    fn refutable_or_multi_clause_matches_are_left_unchanged() {
        // Every one of these must NOT rewrite (would erase a trap or is not single-clause).
        for src in [
            "def f(p) = match p with | Some(x) => x", // sum ctor (capitalized) — refutable
            "def f(p) = match p with | 0 => 1",       // literal — refutable
            "def f(p) = match p with | x if x > 0 => x", // guarded — refutable
            "def f(p) = match p with | (a, Some(x)) => a", // nested refutable sub-pattern
            "def f(p) = match p with | Some(x) => x | None => 0", // multi-clause
            "def f(p) = match p with | (a, b) => a | _ => b", // multi-clause (irrefutable first)
        ] {
            assert_eq!(count_rewrites(src), 0, "must NOT rewrite: {src}");
        }
    }

    #[test]
    fn rewrite_is_semantically_shaped_and_idempotent() {
        // The lowered `let` re-reads + re-prints stably (idempotent), and running the codemod on the
        // already-lowered form is a no-op (0 further rewrites).
        let once = normalize_ml("def f(p) = match p with | (a, b) => a + b");
        let twice = normalize_ml(&once);
        assert_eq!(once, twice, "codemod output must be idempotent: {once:?}");
        assert_eq!(
            count_rewrites(&once),
            0,
            "already-lowered form must not re-rewrite"
        );
    }

    #[test]
    fn nested_match_in_body_is_rewritten_bottom_up() {
        // A single-clause irrefutable match inside another's body is also lowered (2 rewrites).
        let src = "def f(p) = match p with | (a, b) => match b with | (c, d) => a + c + d";
        assert_eq!(
            count_rewrites(src),
            2,
            "both nested single-clause matches lower"
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
                && h == "match"
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
