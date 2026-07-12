//! Structural query & rewrite over the AST — the codemod substrate.
//!
//! This is Rung 2 of `implementation/DESIGN-query-engine.md`: a *built-in* set of structural
//! transforms over the AST, run by a Rust driver, projecting output through the existing surfaces.
//! It stands in for the eventual self-hosted sidecar (Rung 3), whose combinator library
//! (`select`/`rewrite`/quasiquote patterns) this deliberately anticipates — a pattern here reads in
//! the *same shape* as the code it matches, exactly the "rewrite rule reads like the code" idiom the
//! structural-editing corpus pins (`spec/semantics/20-structural-editing.sexp`).
//!
//! # The pattern language is not a new language
//!
//! A pattern is an ordinary Cadenza s-expression with two metavariable sigils — and both are
//! *already* surface syntax the reader produces (`sexpr.rs`), so no grammar is invented here:
//!
//! - `,x`  — an `unquote`, binds ONE node to the name `x`. `,_` matches one node, binding nothing.
//! - `,@xs` — an `unquote-splicing`, binds a RUN of zero-or-more sibling nodes. `,@_` matches any run
//!   without binding. At most one splice may appear among a list's direct children (an unambiguous
//!   run boundary); it may be anchored by fixed nodes on either side: `(f ,head ,@mid ,last)`.
//!
//! Everything else is a literal that must match structurally: `(+ ,x 0)` matches an addition whose
//! second operand is exactly the integer `0`, binding the first operand to `x`.
//!
//! Repeated metavariables are CONSISTENT (non-linear): `(+ ,x ,x)` matches only `(+ e e)` where the
//! two `e` are structurally equal — the Semgrep / ast-grep / Comby convention. The metavariable
//! named `_` is the wildcard and is exempt: every `,_` / `,@_` is independent and binds nothing.
//!
//! # The value it operates on
//!
//! Everything works on an owned homoiconic [`Tree`] (`Atom` | `List`), NOT on arena indices — this
//! mirrors the built-in `Ast` sum a self-hosted sidecar destructures (`Ast.Int`/`Ast.Name`/
//! `Ast.List`), keeps the matcher/rewriter simple, and gives correct BOTTOM-UP rewriting (a parent is
//! matched against its already-rewritten children). Convert at the edges with [`Tree::from_arena`] /
//! [`Tree::to_arena`]. Each node keeps its source [`StructId`] as provenance, so a search match still
//! reports a span.
//!
//! # What it does
//!
//! - [`Pattern::compile`] / [`Template::compile`] turn pattern/template text into matchers.
//! - [`search`] walks the whole tree top-down and returns every match (matched sub-tree, span if
//!   known, captured bindings). [`count`] is its cardinality.
//! - [`rewrite`] rewrites bottom-up and returns the new tree + a count; [`rewrite_fixpoint`] re-runs
//!   until stable (bounded, to survive a rule whose output re-matches its input).
//!
//! A rewrite here only produces a new tree; VALIDATION (re-parse + type-check before accepting) is
//! the driver's job (§5 of the design doc), so this module stays dependency-free.

use crate::ast::{Arenas, Builder, Leaf, Struct, StructId};
use crate::sexpr;
use crate::span::Span;
use crate::spans::SpanTable;
use std::collections::BTreeMap;

/// An owned homoiconic syntax tree — the value a query/rewrite operates on. Mirrors the two-variant
/// arena `Struct`, plus the originating [`StructId`] (when built from a parsed arena) so a match can
/// report its source span. Provenance is ignored by all structural comparison.
#[derive(Clone, Debug)]
pub enum Tree {
    Atom(Leaf, Option<StructId>),
    List(Vec<Tree>, Option<StructId>),
}

impl Tree {
    /// Deep-copy the subtree at `id` out of `a` into an owned `Tree`, recording provenance.
    pub fn from_arena(a: &Arenas, id: StructId) -> Tree {
        match a.get(id) {
            Struct::Atom(l) => Tree::Atom(a.leaf(*l).clone(), Some(id)),
            Struct::List(items) => {
                Tree::List(items.iter().map(|&c| Tree::from_arena(a, c)).collect(), Some(id))
            }
        }
    }

    /// The whole program: `Tree` rooted at the arena's `root`.
    pub fn of(a: &Arenas) -> Tree {
        Tree::from_arena(a, a.root)
    }

    /// Materialize this tree into a fresh [`Arenas`] (dropping provenance).
    pub fn to_arena(&self) -> Arenas {
        let mut b = Builder::new();
        let root = self.build(&mut b);
        b.finish(root)
    }

    fn build(&self, b: &mut Builder) -> StructId {
        match self {
            Tree::Atom(l, _) => b.atom_leaf(l.clone()),
            Tree::List(items, _) => {
                let kids: Vec<StructId> = items.iter().map(|t| t.build(b)).collect();
                b.list(kids)
            }
        }
    }

    /// The source id this node came from, if any.
    pub fn origin(&self) -> Option<StructId> {
        match self {
            Tree::Atom(_, o) | Tree::List(_, o) => *o,
        }
    }

    /// If this is a bare `Name` atom, that name.
    fn as_name(&self) -> Option<&str> {
        match self {
            Tree::Atom(Leaf::Name(n), _) => Some(n),
            _ => None,
        }
    }

    /// This node rendered as one-line s-expression text.
    pub fn to_sexpr(&self) -> String {
        sexpr::print(&self.to_arena())
    }
}

/// Structural equality of two trees, ignoring provenance.
fn tree_eq(a: &Tree, b: &Tree) -> bool {
    match (a, b) {
        (Tree::Atom(la, _), Tree::Atom(lb, _)) => la == lb,
        (Tree::List(xs, _), Tree::List(ys, _)) => {
            xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| tree_eq(x, y))
        }
        _ => false,
    }
}

/// A compiled structural pattern (an owned `Tree`; metavariables are recognized structurally).
#[derive(Clone, Debug)]
pub struct Pattern {
    tree: Tree,
}

/// A compiled replacement template (same `,x` / `,@xs` sigils, filled from bindings on instantiate).
#[derive(Clone, Debug)]
pub struct Template {
    tree: Tree,
}

/// An error compiling a pattern or template from text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatternError(pub String);

impl std::fmt::Display for PatternError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The bindings captured by a successful match: metavariable name → the sub-tree(s) it bound. A
/// single metavariable (`,x`) binds one tree; a splice (`,@xs`) binds a run. Ordered by name.
#[derive(Clone, Debug, Default)]
pub struct Bindings {
    map: BTreeMap<String, Vec<Tree>>,
}

impl Bindings {
    /// The single tree bound to `name`, if `name` bound exactly one node.
    pub fn get(&self, name: &str) -> Option<&Tree> {
        match self.map.get(name)?.as_slice() {
            [one] => Some(one),
            _ => None,
        }
    }

    /// The run of trees bound to `name` (a splice binding), if present.
    pub fn get_run(&self, name: &str) -> Option<&[Tree]> {
        self.map.get(name).map(|v| v.as_slice())
    }

    /// Every binding, name-ordered.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[Tree])> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_slice()))
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// One search hit: the matched sub-tree, its source span (if the subject carried provenance and a
/// [`SpanTable`] was supplied), and the captured bindings.
#[derive(Clone, Debug)]
pub struct Match {
    pub node: Tree,
    pub span: Option<Span>,
    pub bindings: Bindings,
}

/// The result of a [`rewrite`]: the new tree and how many sites were rewritten.
#[derive(Clone, Debug)]
pub struct Rewrite {
    pub tree: Tree,
    pub count: usize,
}

// ============================================================================================
// Metavariable recognition — the sole coupling to the `unquote` surface.
// ============================================================================================

/// If `t` is a single-node metavariable `(unquote NAME)`, its name. `,_` yields `"_"`.
fn as_metavar(t: &Tree) -> Option<&str> {
    metavar_of(t, "unquote")
}

/// If `t` is a splice metavariable `(unquote-splicing NAME)`, its name. `,@_` yields `"_"`.
fn as_splice(t: &Tree) -> Option<&str> {
    metavar_of(t, "unquote-splicing")
}

/// Shared shape check: a two-element list `(head NAME)` where NAME is a bare name atom.
fn metavar_of<'a>(t: &'a Tree, head: &str) -> Option<&'a str> {
    match t {
        Tree::List(items, _) => match items.as_slice() {
            [h, name] if h.as_name() == Some(head) => name.as_name(),
            _ => None,
        },
        _ => None,
    }
}

/// Is `name` the anonymous wildcard (exempt from consistency, binds nothing)?
fn is_wildcard(name: &str) -> bool {
    name == "_"
}

// ============================================================================================
// Pattern / Template compilation
// ============================================================================================

impl Pattern {
    /// Compile a pattern from s-expression text (e.g. `"(+ ,x 0)"`).
    ///
    /// Rejects a list with more than one splice among its direct children (ambiguous run boundary).
    pub fn compile(src: &str) -> Result<Pattern, PatternError> {
        let arena = sexpr::read(src).map_err(|e| PatternError(format!("pattern parse: {}", e.0)))?;
        let tree = Tree::of(&arena);
        check_at_most_one_splice(&tree)?;
        Ok(Pattern { tree })
    }

    /// Try to match this pattern against `subject`, filling `binds`. On a mismatch, `binds` may have
    /// been partially extended and must be discarded by the caller.
    fn matches(&self, subject: &Tree, binds: &mut Bindings) -> bool {
        match_node(&self.tree, subject, binds)
    }
}

impl Template {
    /// Compile a template from s-expression text (e.g. `",x"`).
    pub fn compile(src: &str) -> Result<Template, PatternError> {
        let arena =
            sexpr::read(src).map_err(|e| PatternError(format!("template parse: {}", e.0)))?;
        Ok(Template {
            tree: Tree::of(&arena),
        })
    }
}

/// Reject any list that contains two or more direct-child splices — the run boundary would be
/// ambiguous. One splice plus fixed nodes on either side is fine (matched uniquely).
fn check_at_most_one_splice(t: &Tree) -> Result<(), PatternError> {
    if let Tree::List(items, _) = t {
        if items.iter().filter(|c| as_splice(c).is_some()).count() > 1 {
            return Err(PatternError(
                "a pattern list may contain at most one `,@` splice".into(),
            ));
        }
        for c in items {
            check_at_most_one_splice(c)?;
        }
    }
    Ok(())
}

// ============================================================================================
// Matching
// ============================================================================================

/// Match pattern node `p` against subject node `s`, extending `binds`.
fn match_node(p: &Tree, s: &Tree, binds: &mut Bindings) -> bool {
    // A single-node metavariable binds `s` (subject to consistency).
    if let Some(name) = as_metavar(p) {
        return bind_single(binds, name, s);
    }
    match (p, s) {
        (Tree::Atom(pl, _), Tree::Atom(sl, _)) => pl == sl,
        (Tree::List(pitems, _), Tree::List(sitems, _)) => match_seq(pitems, sitems, binds),
        _ => false,
    }
}

/// Match a pattern child-sequence against a subject child-sequence, handling at most one splice.
fn match_seq(pitems: &[Tree], sitems: &[Tree], binds: &mut Bindings) -> bool {
    let splice_at = pitems.iter().position(|c| as_splice(c).is_some());
    match splice_at {
        None => {
            pitems.len() == sitems.len()
                && pitems
                    .iter()
                    .zip(sitems)
                    .all(|(p, s)| match_node(p, s, binds))
        }
        Some(k) => {
            let before = &pitems[..k];
            let after = &pitems[k + 1..];
            if sitems.len() < before.len() + after.len() {
                return false;
            }
            for (p, s) in before.iter().zip(&sitems[..before.len()]) {
                if !match_node(p, s, binds) {
                    return false;
                }
            }
            let suffix_start = sitems.len() - after.len();
            for (p, s) in after.iter().zip(&sitems[suffix_start..]) {
                if !match_node(p, s, binds) {
                    return false;
                }
            }
            let mid = &sitems[before.len()..suffix_start];
            let name = as_splice(&pitems[k]).expect("splice present");
            bind_run(binds, name, mid)
        }
    }
}

/// Bind a single metavariable, enforcing consistency (a repeated non-wildcard name must bind a
/// structurally-equal subtree). Returns false on an inconsistent re-bind.
fn bind_single(binds: &mut Bindings, name: &str, node: &Tree) -> bool {
    if is_wildcard(name) {
        return true;
    }
    if let Some(prev) = binds.map.get(name) {
        return matches!(prev.as_slice(), [only] if tree_eq(only, node));
    }
    binds.map.insert(name.to_string(), vec![node.clone()]);
    true
}

/// Bind a splice metavariable to a run, with the same consistency rule.
fn bind_run(binds: &mut Bindings, name: &str, run: &[Tree]) -> bool {
    if is_wildcard(name) {
        return true;
    }
    if let Some(prev) = binds.map.get(name) {
        return prev.len() == run.len() && prev.iter().zip(run).all(|(a, b)| tree_eq(a, b));
    }
    binds.map.insert(name.to_string(), run.to_vec());
    true
}

// ============================================================================================
// Search — every match, top-down
// ============================================================================================

/// Find every node in `subject` that matches `pattern`, in pre-order (parents before children). If
/// `spans` is given, each match carries its source span (looked up via the node's provenance id).
pub fn search(pattern: &Pattern, subject: &Tree, spans: Option<&SpanTable>) -> Vec<Match> {
    let mut out = Vec::new();
    search_at(pattern, subject, spans, &mut out);
    out
}

fn search_at(pattern: &Pattern, node: &Tree, spans: Option<&SpanTable>, out: &mut Vec<Match>) {
    let mut binds = Bindings::default();
    if pattern.matches(node, &mut binds) {
        out.push(Match {
            node: node.clone(),
            span: node.origin().and_then(|id| spans.and_then(|s| s.get(id))),
            bindings: binds,
        });
    }
    if let Tree::List(items, _) = node {
        for c in items {
            search_at(pattern, c, spans, out);
        }
    }
}

/// Count matches without materializing them.
pub fn count(pattern: &Pattern, subject: &Tree) -> usize {
    search(pattern, subject, None).len()
}

// ============================================================================================
// Rewrite — bottom-up, template-instantiated
// ============================================================================================

/// Rewrite `subject` bottom-up: transform children first, then try `pattern` at each node (matched
/// against its ALREADY-REWRITTEN form) and, on a match, replace it with `template` instantiated from
/// the captures. Returns the new tree and the rewrite count.
///
/// Bottom-up + match-against-rewritten-children means a captured metavariable reflects prior child
/// rewrites (so `(+ (+ x 0) 0)` collapses fully under `(+ ,x 0) → ,x` in one pass). A template
/// metavariable with no binding, or used with the wrong arity (single vs splice), makes that site
/// fail to instantiate: the node is left as its rewritten-children form and NOT counted
/// (reject-don't-corrupt).
pub fn rewrite(pattern: &Pattern, template: &Template, subject: &Tree) -> Rewrite {
    let mut count = 0;
    let tree = rewrite_node(pattern, template, subject, &mut count);
    Rewrite { tree, count }
}

fn rewrite_node(pattern: &Pattern, template: &Template, node: &Tree, count: &mut usize) -> Tree {
    // Bottom-up: rewrite children first.
    let rewritten = match node {
        Tree::Atom(l, o) => Tree::Atom(l.clone(), *o),
        Tree::List(items, o) => Tree::List(
            items
                .iter()
                .map(|c| rewrite_node(pattern, template, c, count))
                .collect(),
            *o,
        ),
    };
    // Then match this node in its rewritten form; on a match, instantiate the template.
    let mut binds = Bindings::default();
    if pattern.matches(&rewritten, &mut binds)
        && let Some(new_tree) = instantiate(&template.tree, &binds)
    {
        *count += 1;
        return new_tree;
    }
    rewritten
}

/// Rewrite to a fixed point: repeat [`rewrite`] until no site matches or `max_passes` is reached.
/// `count` is the total across all passes. `max_passes` bounds a rule whose output re-matches its
/// input (which would otherwise loop forever).
pub fn rewrite_fixpoint(
    pattern: &Pattern,
    template: &Template,
    subject: &Tree,
    max_passes: usize,
) -> Rewrite {
    let mut current = subject.clone();
    let mut total = 0;
    for _ in 0..max_passes {
        let r = rewrite(pattern, template, &current);
        if r.count == 0 {
            break;
        }
        total += r.count;
        current = r.tree;
    }
    Rewrite {
        tree: current,
        count: total,
    }
}

/// Instantiate `t` (a template node), filling metavariables from `binds`. Returns `None` if a
/// metavariable is unbound or used with the wrong arity (single vs splice). A `,@` splice is only
/// valid as a direct child of a list (handled in the list arm); at the top level or in single
/// position it is a failure.
fn instantiate(t: &Tree, binds: &Bindings) -> Option<Tree> {
    if let Some(name) = as_metavar(t) {
        return binds.get(name).cloned();
    }
    if as_splice(t).is_some() {
        return None; // splice in non-child position
    }
    match t {
        Tree::Atom(l, _) => Some(Tree::Atom(l.clone(), None)),
        Tree::List(items, _) => {
            let mut kids = Vec::with_capacity(items.len());
            for c in items {
                if let Some(name) = as_splice(c) {
                    let run = binds.get_run(name)?;
                    kids.extend(run.iter().cloned());
                } else {
                    kids.push(instantiate(c, binds)?);
                }
            }
            Some(Tree::List(kids, None))
        }
    }
}

// ============================================================================================
// Driver — load a target, run a query/rewrite, project output. The CLI is a thin shell over this.
// ============================================================================================

pub mod driver {
    //! The Rung-2 driver: wire `target + operation + output` end-to-end over [`Arenas`]. Kept in the
    //! library (not the bin) so every step is unit-testable; the bin does only file/stdin/stdout.

    use super::*;
    use crate::convert::Format;
    use crate::{parser, printer};

    /// A loaded target program: its tree plus the span table when the source carried one (ML only).
    #[derive(Debug)]
    pub struct Target {
        pub tree: Tree,
        pub spans: Option<SpanTable>,
    }

    /// Load a target from `input` bytes in `from` format. ML is parsed with the recovering parser
    /// (so a target with recoverable errors still yields a tree, reported via `errors`); s-expr and
    /// binary produce a tree with no span table. Output-only formats (`debug`/`flat`) are rejected.
    pub fn load(input: &[u8], from: Format) -> Result<(Target, Vec<String>), String> {
        match from {
            Format::Ml => {
                let text = std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                let parsed = parser::read_ml(text);
                let errors = parsed
                    .errors
                    .iter()
                    .map(|e| format!("byte {}: {}", e.span.start, e.message))
                    .collect();
                let tree = Tree::of(&parsed.arenas);
                Ok((
                    Target {
                        tree,
                        spans: Some(parsed.spans),
                    },
                    errors,
                ))
            }
            Format::Sexpr => {
                let text = std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                // Mirror the ML parser's root convention: a SINGLE top-level form stays bare, so it
                // round-trips through the ML printer (which renders a root single-element `(do X)` as
                // bare `X`). Only multiple forms wrap in `(do …)`. `read` succeeds iff there's exactly
                // one form (it errors on trailing input); fall back to `read_all` for several.
                let arena = match sexpr::read(text) {
                    Ok(a) => a,
                    Err(_) => sexpr::read_all(text).map_err(|e| format!("s-expr parse: {}", e.0))?,
                };
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: None,
                    },
                    Vec::new(),
                ))
            }
            Format::Binary => {
                let arena = crate::codec::decode(input)
                    .ok_or_else(|| "invalid binary encoding".to_string())?;
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: None,
                    },
                    Vec::new(),
                ))
            }
            Format::Debug | Format::Flat => {
                Err(format!("`{}` is an output-only format, not an input", from.name()))
            }
        }
    }

    /// Render every match of `pattern` in `target` as a report, one per line:
    /// `byte START-END: <matched s-expr>` when a span is known, else `<index>: <matched s-expr>`.
    /// The captured bindings are appended as `  $name = <sexpr>` lines.
    pub fn report_matches(pattern: &Pattern, target: &Target) -> String {
        let matches = search(pattern, &target.tree, target.spans.as_ref());
        let mut out = String::new();
        for (i, m) in matches.iter().enumerate() {
            let loc = match m.span {
                Some(s) => format!("byte {}-{}", s.start, s.end),
                None => format!("#{i}"),
            };
            out.push_str(&format!("{loc}: {}\n", m.node.to_sexpr()));
            for (name, nodes) in m.bindings.iter() {
                let rendered = match nodes {
                    [one] => one.to_sexpr(),
                    many => {
                        let parts: Vec<_> = many.iter().map(|t| t.to_sexpr()).collect();
                        format!("[{}]", parts.join(" "))
                    }
                };
                out.push_str(&format!("  ${name} = {rendered}\n"));
            }
        }
        out
    }

    /// The outcome of a validated rewrite: the projected output text, the site count, and whether the
    /// result re-parsed cleanly (the validated-transaction check).
    pub struct RewriteOutcome {
        pub output: String,
        pub count: usize,
    }

    /// Apply `pattern → template` to `target` (optionally to a fixed point) and project the result in
    /// `to` format. VALIDATES the result as a transaction: the rewritten tree is re-printed to ML and
    /// re-parsed; if that fails, the rewrite is REJECTED (no output) with the parse error — never a
    /// half-applied edit. (Type-checking the result is the Rung-3 step, requiring the compiler crate;
    /// re-parse well-formedness is what this dependency-free layer can guarantee.)
    pub fn apply_rewrite(
        pattern: &Pattern,
        template: &Template,
        target: &Target,
        to: Format,
        width: usize,
        fixpoint: bool,
    ) -> Result<RewriteOutcome, String> {
        let r = if fixpoint {
            rewrite_fixpoint(pattern, template, &target.tree, 64)
        } else {
            rewrite(pattern, template, &target.tree)
        };
        let arena = r.tree.to_arena();

        // Validated transaction: the result must re-parse to a structurally-equal tree.
        let ml = printer::print(&arena, width);
        let reparsed = parser::read_ml(&ml);
        if !reparsed.ok() {
            return Err(format!(
                "rewrite rejected: result does not re-parse cleanly ({} error(s)); first: {}",
                reparsed.errors.len(),
                reparsed
                    .errors
                    .first()
                    .map(|e| e.message.as_str())
                    .unwrap_or("?")
            ));
        }
        if !reparsed.arenas.structurally_eq(&arena) {
            return Err(
                "rewrite rejected: result does not round-trip through the ML surface".to_string(),
            );
        }

        let output = project(&arena, to, width)?;
        Ok(RewriteOutcome {
            output,
            count: r.count,
        })
    }

    /// Render an arena in `to` format (text formats only for the query path).
    pub fn project(arena: &Arenas, to: Format, width: usize) -> Result<String, String> {
        match to {
            Format::Ml => Ok(printer::print(arena, width)),
            Format::Sexpr => Ok(sexpr::print(arena)),
            Format::Debug => Ok(crate::debug::print(arena)),
            Format::Flat => Ok(crate::debug::print_flat(arena)),
            Format::Binary => Err("binary output is not supported for query results".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;

    /// Read subject text via the s-expr surface into a `Tree` (no spans).
    fn subj(src: &str) -> Tree {
        let a = sexpr::read(src).unwrap_or_else(|e| panic!("subject parse {src:?}: {}", e.0));
        Tree::of(&a)
    }

    fn pat(src: &str) -> Pattern {
        Pattern::compile(src).unwrap_or_else(|e| panic!("pattern {src:?}: {e}"))
    }

    fn tmpl(src: &str) -> Template {
        Template::compile(src).unwrap_or_else(|e| panic!("template {src:?}: {e}"))
    }

    // ---- matching ----

    #[test]
    fn literal_pattern_matches_only_exact() {
        let s = subj("(+ 1 2)");
        assert_eq!(count(&pat("(+ 1 2)"), &s), 1);
        assert_eq!(count(&pat("(+ 1 3)"), &s), 0);
        assert_eq!(count(&pat("(- 1 2)"), &s), 0);
    }

    #[test]
    fn single_metavar_binds_one_node() {
        let s = subj("(+ x 0)");
        let m = search(&pat("(+ ,e 0)"), &s, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].bindings.get("e").unwrap().to_sexpr(), "x");
    }

    #[test]
    fn metavar_binds_a_whole_subtree() {
        let s = subj("(+ (* a b) 0)");
        let m = search(&pat("(+ ,e 0)"), &s, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].bindings.get("e").unwrap().to_sexpr(), "(* a b)");
    }

    #[test]
    fn wildcard_binds_nothing_and_is_independent() {
        // `,_` matches any node; two `,_` need NOT be equal (unlike a named var).
        let s = subj("(pair a b)");
        let m = search(&pat("(pair ,_ ,_)"), &s, None);
        assert_eq!(m.len(), 1);
        assert!(m[0].bindings.is_empty(), "wildcards bind nothing");
    }

    #[test]
    fn repeated_metavar_is_consistent() {
        let p = pat("(+ ,x ,x)");
        assert_eq!(count(&p, &subj("(+ a a)")), 1);
        assert_eq!(count(&p, &subj("(+ a b)")), 0);
        // structural (not identity) equality: distinct occurrences of the same subtree match.
        assert_eq!(count(&p, &subj("(+ (f 1) (f 1))")), 1);
        assert_eq!(count(&p, &subj("(+ (f 1) (f 2))")), 0);
    }

    #[test]
    fn splice_binds_the_argument_run() {
        let s = subj("(f a b c)");
        let m = search(&pat("(f ,@args)"), &s, None);
        assert_eq!(m.len(), 1);
        let run = m[0].bindings.get_run("args").unwrap();
        let texts: Vec<_> = run.iter().map(|t| t.to_sexpr()).collect();
        assert_eq!(texts, ["a", "b", "c"]);
    }

    #[test]
    fn splice_matches_zero_nodes() {
        let s = subj("(f)");
        let m = search(&pat("(f ,@args)"), &s, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].bindings.get_run("args").unwrap().len(), 0);
    }

    #[test]
    fn splice_with_anchored_prefix_and_suffix() {
        let s = subj("(call a x y z b)");
        let m = search(&pat("(call ,head ,@mid ,last)"), &s, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].bindings.get("head").unwrap().to_sexpr(), "a");
        assert_eq!(m[0].bindings.get("last").unwrap().to_sexpr(), "b");
        let mid: Vec<_> = m[0]
            .bindings
            .get_run("mid")
            .unwrap()
            .iter()
            .map(|t| t.to_sexpr())
            .collect();
        assert_eq!(mid, ["x", "y", "z"]);
    }

    #[test]
    fn splice_needs_enough_children_for_the_fixed_parts() {
        let s = subj("(call a)");
        assert_eq!(count(&pat("(call ,head ,@mid ,last)"), &s), 0);
    }

    #[test]
    fn two_splices_in_one_list_is_rejected() {
        let e = Pattern::compile("(f ,@a ,@b)").unwrap_err();
        assert!(e.0.contains("at most one"), "got {e}");
    }

    #[test]
    fn search_is_recursive_and_finds_nested_matches() {
        let s = subj("(+ (+ x 0) 0)");
        assert_eq!(count(&pat("(+ ,e 0)"), &s), 2);
    }

    #[test]
    fn search_reports_spans_when_available() {
        // Parse via the ML surface so a span table exists; `f(a, b)` -> (f a b).
        let parsed = parser::read_ml("f(a, b)");
        assert!(parsed.ok());
        let tree = Tree::of(&parsed.arenas);
        let m = search(&pat("(f ,@xs)"), &tree, Some(&parsed.spans));
        assert_eq!(m.len(), 1);
        let span = m[0].span.expect("span present");
        assert_eq!(&"f(a, b)"[span.start..span.end], "f(a, b)");
    }

    // ---- rewriting ----

    #[test]
    fn rewrite_replaces_with_a_captured_metavar() {
        let s = subj("(+ y 0)");
        let r = rewrite(&pat("(+ ,x 0)"), &tmpl(",x"), &s);
        assert_eq!(r.count, 1);
        assert_eq!(r.tree.to_sexpr(), "y");
    }

    #[test]
    fn rewrite_is_bottom_up_and_hits_every_site() {
        let s = subj("(g (* a 1) (* b 1))");
        let r = rewrite(&pat("(* ,x 1)"), &tmpl(",x"), &s);
        assert_eq!(r.count, 2);
        assert_eq!(r.tree.to_sexpr(), "(g a b)");
    }

    #[test]
    fn rewrite_bottom_up_collapses_nested_in_one_pass() {
        // Children rewritten first, then the parent matches against the rewritten child: the inner
        // `(+ x 0)` becomes `x`, so the outer `(+ x 0)` fires too. One pass, count 2, result `x`.
        let s = subj("(+ (+ x 0) 0)");
        let r = rewrite(&pat("(+ ,x 0)"), &tmpl(",x"), &s);
        assert_eq!(r.count, 2);
        assert_eq!(r.tree.to_sexpr(), "x");
    }

    #[test]
    fn rewrite_wraps_with_a_splice_template() {
        // Wrap a risky call in logging: `(risky ,@args)` -> `(log (risky ,@args))`.
        let s = subj("(risky a b)");
        let r = rewrite(&pat("(risky ,@args)"), &tmpl("(log (risky ,@args))"), &s);
        assert_eq!(r.count, 1);
        assert_eq!(r.tree.to_sexpr(), "(log (risky a b))");
    }

    #[test]
    fn rewrite_renames_a_bare_name() {
        let s = subj("(+ old (f old))");
        let r = rewrite(&pat("old"), &tmpl("new"), &s);
        assert_eq!(r.count, 2);
        assert_eq!(r.tree.to_sexpr(), "(+ new (f new))");
    }

    #[test]
    fn rewrite_leaves_nonmatching_tree_untouched() {
        let s = subj("(- a 1)");
        let r = rewrite(&pat("(+ ,x 0)"), &tmpl(",x"), &s);
        assert_eq!(r.count, 0);
        assert_eq!(r.tree.to_sexpr(), "(- a 1)");
    }

    #[test]
    fn rewrite_with_unbound_template_var_is_a_no_op_at_that_site() {
        // Template references `,y`, never bound by the pattern — the site can't instantiate, so it's
        // left unchanged and not counted (reject-don't-corrupt).
        let s = subj("(+ a 0)");
        let r = rewrite(&pat("(+ ,x 0)"), &tmpl(",y"), &s);
        assert_eq!(r.count, 0);
        assert_eq!(r.tree.to_sexpr(), "(+ a 0)");
    }

    #[test]
    fn fixpoint_saturates_and_is_idempotent() {
        let s = subj("(+ 0 (+ 0 (+ 0 v)))");
        let p = pat("(+ 0 ,x)");
        let t = tmpl(",x");
        // A single bottom-up pass already collapses inner-first.
        assert_eq!(rewrite(&p, &t, &s).tree.to_sexpr(), "v");
        // Fixpoint reaches the same stable result.
        assert_eq!(rewrite_fixpoint(&p, &t, &s, 10).tree.to_sexpr(), "v");
    }

    #[test]
    fn fixpoint_is_bounded_on_a_self_rematching_rule() {
        // `,x` -> `(w ,x)` would loop forever (its output re-matches); max_passes caps it.
        let s = subj("a");
        let r = rewrite_fixpoint(&pat(",x"), &tmpl("(w ,x)"), &s, 3);
        assert!(r.count >= 1);
        let _ = r.tree.to_sexpr(); // must be renderable (well-formed)
    }

    #[test]
    fn rewrite_result_reparses_structurally_equal_via_ml() {
        // The validated-transaction property (driver-side, checked here on the tree): a rewrite
        // result prints to ML and re-parses to a structurally-equal arena.
        let s = subj("(f (+ a 0) (+ b 0))");
        let r = rewrite(&pat("(+ ,x 0)"), &tmpl(",x"), &s);
        let arena = r.tree.to_arena();
        let ml = crate::printer::print(&arena, 100);
        let reparsed = parser::read_ml(&ml);
        assert!(reparsed.ok(), "rewrite result re-parses: {:?}", reparsed.errors);
        assert!(
            reparsed.arenas.structurally_eq(&arena),
            "ML round-trip of the rewrite is structurally stable"
        );
    }

    #[test]
    fn tree_arena_roundtrip_is_structural_identity() {
        let a = sexpr::read("(let ((p (record (x 1) (y 2)))) (. p x))").unwrap();
        let back = Tree::of(&a).to_arena();
        assert!(back.structurally_eq(&a));
    }

    // ---- driver ----

    mod driver_tests {
        use super::*;
        use crate::convert::Format;
        use crate::query::driver;

        #[test]
        fn load_ml_reports_matches_with_real_spans() {
            let src = "f(a, b)\ng(c)";
            let (target, errors) = driver::load(src.as_bytes(), Format::Ml).unwrap();
            assert!(errors.is_empty());
            let report = driver::report_matches(&pat("(g ,@xs)"), &target);
            // g(c) -> (g c); the report line names its byte span and the matched form.
            assert!(report.contains("(g c)"), "report: {report}");
            assert!(report.contains("byte "), "has a span: {report}");
        }

        #[test]
        fn load_ml_on_broken_input_still_yields_a_tree_and_reports_errors() {
            // The recovering parser gives a usable tree even here; the driver surfaces the errors.
            let (target, errors) = driver::load(b"f(@)", Format::Ml).unwrap();
            assert!(!errors.is_empty(), "recoverable errors surfaced");
            // still queryable: the call `f(...)` is present.
            assert!(driver::report_matches(&pat("(f ,@xs)"), &target).contains("f"));
        }

        #[test]
        fn apply_rewrite_projects_ml_and_validates() {
            let (target, _) = driver::load(b"(+ x 0)", Format::Sexpr).unwrap();
            let out = driver::apply_rewrite(
                &pat("(+ ,e 0)"),
                &tmpl(",e"),
                &target,
                Format::Ml,
                100,
                false,
            )
            .unwrap();
            assert_eq!(out.count, 1);
            assert_eq!(out.output.trim(), "x");
        }

        #[test]
        fn apply_rewrite_can_emit_sexpr() {
            let (target, _) = driver::load(b"(g (* a 1) (* b 1))", Format::Sexpr).unwrap();
            let out = driver::apply_rewrite(
                &pat("(* ,x 1)"),
                &tmpl(",x"),
                &target,
                Format::Sexpr,
                100,
                false,
            )
            .unwrap();
            assert_eq!(out.count, 2);
            assert_eq!(out.output.trim(), "(g a b)");
        }

        #[test]
        fn output_only_format_is_rejected_as_input() {
            let e = driver::load(b"x", Format::Debug).unwrap_err();
            assert!(e.contains("output-only"), "got {e}");
        }
    }
}
