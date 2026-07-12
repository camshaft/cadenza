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

/// A compiled structural pattern.
#[derive(Clone, Debug)]
pub struct Pattern {
    pat: Pat,
}

/// The compiled pattern tree. A pattern is either a literal to match exactly, a list (which may
/// contain at most one splice among its direct children), a single-node metavariable (with optional
/// structural guards), or a splice metavariable.
#[derive(Clone, Debug)]
enum Pat {
    /// A literal atom that must match an equal leaf value.
    Lit(Leaf),
    /// A list; children matched positionally, honoring at most one splice.
    List(Vec<Pat>),
    /// `,x` / `,(x guard…)` — binds one node to `name` if every guard holds. `_` is the wildcard.
    Meta { name: String, guards: Vec<Guard> },
    /// `,@xs` — binds a run of sibling nodes. Only valid as a direct child of a [`Pat::List`].
    Splice { name: String },
}

/// A purely-STRUCTURAL constraint on the node a metavariable binds. Deliberately no scope/binding or
/// type predicates (`refs`/`defines`/`type-of`) — those need the compiler's resolver/checker and
/// live there, not in this syntax-only layer. Guards on a metavar are conjunctive (all must hold).
#[derive(Clone, Debug)]
enum Guard {
    /// The node is any literal atom (int/float/string/bool — NOT a name).
    IsLiteral,
    /// The node is a name atom.
    IsName,
    /// The node is an integer / float / string / bool literal.
    IsInt,
    IsFloat,
    IsStr,
    IsBool,
    /// The node is any atom (leaf), or any list.
    IsAtom,
    IsList,
    /// The node is a list whose head name is this string (`(head-is +)`).
    HeadIs(String),
    /// The node itself matches this sub-pattern (`(matches PAT)`). The sub-pattern's captures are a
    /// pure test — they do NOT leak into the outer bindings.
    Matches(Box<Pat>),
    /// Negation of a guard (`(not GUARD)`).
    Not(Box<Guard>),
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

/// The payload of a single-node metavariable `,X` (an `(unquote X)` form), where X is either a bare
/// name (`,x`) or a guarded form `(name guard…)` (`,(x is-literal)`). Returns `None` if `t` is not
/// an `unquote` form at all.
fn as_metavar_tree(t: &Tree) -> Option<&Tree> {
    match t {
        Tree::List(items, _) => match items.as_slice() {
            [h, payload] if h.as_name() == Some("unquote") => Some(payload),
            _ => None,
        },
        _ => None,
    }
}

/// A TEMPLATE metavariable `,x` — a bare-name `(unquote NAME)` payload. Templates take no guards
/// (guards are match-side only), so a guarded payload is not a template metavariable.
fn template_metavar(t: &Tree) -> Option<&str> {
    as_metavar_tree(t).and_then(|payload| payload.as_name())
}

/// If `t` is a splice metavariable `(unquote-splicing NAME)`, its name.
fn as_splice(t: &Tree) -> Option<&str> {
    match t {
        Tree::List(items, _) => match items.as_slice() {
            [h, name] if h.as_name() == Some("unquote-splicing") => name.as_name(),
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
    /// Compile a pattern from s-expression text (e.g. `"(+ ,x 0)"`, `"(f ,(x is-literal) ,@rest)"`).
    ///
    /// Rejects a list with more than one splice among its direct children (ambiguous run boundary),
    /// a splice used outside list-child position, and an unknown/ill-formed guard.
    pub fn compile(src: &str) -> Result<Pattern, PatternError> {
        let arena = sexpr::read(src).map_err(|e| PatternError(format!("pattern parse: {}", e.0)))?;
        let pat = compile_pat(&Tree::of(&arena))?;
        Ok(Pattern { pat })
    }

    /// Try to match this pattern against `subject`, filling `binds`. On a mismatch, `binds` may have
    /// been partially extended and must be discarded by the caller.
    fn matches(&self, subject: &Tree, binds: &mut Bindings) -> bool {
        match_pat(&self.pat, subject, binds)
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

/// Compile a pattern `Tree` into a [`Pat`]. A splice at the top level (not a list child) is an error.
fn compile_pat(t: &Tree) -> Result<Pat, PatternError> {
    if let Some(payload) = as_metavar_tree(t) {
        return compile_meta(payload);
    }
    if as_splice(t).is_some() {
        return Err(PatternError(
            "a `,@` splice is only valid as a direct child of a list".into(),
        ));
    }
    match t {
        Tree::Atom(l, _) => Ok(Pat::Lit(l.clone())),
        Tree::List(items, _) => {
            if items.iter().filter(|c| as_splice(c).is_some()).count() > 1 {
                return Err(PatternError(
                    "a pattern list may contain at most one `,@` splice".into(),
                ));
            }
            let mut kids = Vec::with_capacity(items.len());
            for c in items {
                if let Some(name) = as_splice(c) {
                    kids.push(Pat::Splice {
                        name: name.to_string(),
                    });
                } else {
                    kids.push(compile_pat(c)?);
                }
            }
            Ok(Pat::List(kids))
        }
    }
}

/// Compile the payload of an `,X` metavariable: a bare name (`,x`), or `(name guard…)` with guards.
fn compile_meta(payload: &Tree) -> Result<Pat, PatternError> {
    // Bare `,x` (or `,_`).
    if let Some(name) = payload.as_name() {
        return Ok(Pat::Meta {
            name: name.to_string(),
            guards: Vec::new(),
        });
    }
    // Guarded `,(name guard…)`.
    if let Tree::List(items, _) = payload {
        let name = items
            .first()
            .and_then(|t| t.as_name())
            .ok_or_else(|| PatternError("a guarded metavariable needs a name: `,(name guard…)`".into()))?;
        let guards = items[1..].iter().map(compile_guard).collect::<Result<_, _>>()?;
        return Ok(Pat::Meta {
            name: name.to_string(),
            guards,
        });
    }
    Err(PatternError(
        "a metavariable must be `,name` or `,(name guard…)`".into(),
    ))
}

/// Compile one structural guard. Unknown guard names are rejected at compile time.
fn compile_guard(t: &Tree) -> Result<Guard, PatternError> {
    // A bare-name guard.
    if let Some(name) = t.as_name() {
        return match name {
            "is-literal" => Ok(Guard::IsLiteral),
            "is-name" => Ok(Guard::IsName),
            "is-int" => Ok(Guard::IsInt),
            "is-float" => Ok(Guard::IsFloat),
            "is-str" => Ok(Guard::IsStr),
            "is-bool" => Ok(Guard::IsBool),
            "is-atom" => Ok(Guard::IsAtom),
            "is-list" => Ok(Guard::IsList),
            other => Err(PatternError(format!("unknown guard `{other}`"))),
        };
    }
    // A guard form: `(head-is NAME)`, `(matches PAT)`, `(not GUARD)`.
    if let Tree::List(items, _) = t {
        match items.first().and_then(|h| h.as_name()) {
            Some("head-is") => {
                let name = items
                    .get(1)
                    .and_then(|t| t.as_name())
                    .ok_or_else(|| PatternError("`head-is` needs a name: `(head-is +)`".into()))?;
                Ok(Guard::HeadIs(name.to_string()))
            }
            Some("matches") => {
                let sub = items
                    .get(1)
                    .ok_or_else(|| PatternError("`matches` needs a sub-pattern".into()))?;
                Ok(Guard::Matches(Box::new(compile_pat(sub)?)))
            }
            Some("not") => {
                let inner = items
                    .get(1)
                    .ok_or_else(|| PatternError("`not` needs a guard: `(not is-literal)`".into()))?;
                Ok(Guard::Not(Box::new(compile_guard(inner)?)))
            }
            _ => Err(PatternError(format!(
                "ill-formed guard `{}`",
                t.to_sexpr()
            ))),
        }
    } else {
        Err(PatternError(format!("ill-formed guard `{}`", t.to_sexpr())))
    }
}

// ============================================================================================
// Matching
// ============================================================================================

/// Match a compiled pattern `p` against subject node `s`, extending `binds`.
fn match_pat(p: &Pat, s: &Tree, binds: &mut Bindings) -> bool {
    match p {
        Pat::Meta { name, guards } => {
            guards.iter().all(|g| guard_holds(g, s)) && bind_single(binds, name, s)
        }
        Pat::Splice { .. } => unreachable!("a splice is only matched inside a list sequence"),
        Pat::Lit(pl) => matches!(s, Tree::Atom(sl, _) if pl == sl),
        Pat::List(pitems) => match s {
            Tree::List(sitems, _) => match_seq(pitems, sitems, binds),
            _ => false,
        },
    }
}

/// Does structural guard `g` hold for node `s`? Purely syntactic — no scope or type lookup.
fn guard_holds(g: &Guard, s: &Tree) -> bool {
    match g {
        Guard::IsLiteral => matches!(s, Tree::Atom(l, _) if !matches!(l, Leaf::Name(_))),
        Guard::IsName => matches!(s, Tree::Atom(Leaf::Name(_), _)),
        Guard::IsInt => matches!(s, Tree::Atom(Leaf::Int { .. }, _)),
        Guard::IsFloat => matches!(s, Tree::Atom(Leaf::Float(_), _)),
        Guard::IsStr => matches!(s, Tree::Atom(Leaf::Str(_), _)),
        Guard::IsBool => matches!(s, Tree::Atom(Leaf::Bool(_), _)),
        Guard::IsAtom => matches!(s, Tree::Atom(_, _)),
        Guard::IsList => matches!(s, Tree::List(_, _)),
        Guard::HeadIs(name) => head_name(s) == Some(name.as_str()),
        // A sub-pattern test: its captures are local (discarded), so `matches` is a pure predicate.
        Guard::Matches(sub) => {
            let mut scratch = Bindings::default();
            match_pat(sub, s, &mut scratch)
        }
        Guard::Not(inner) => !guard_holds(inner, s),
    }
}

/// The head name of a list node (its first child, if a name atom).
fn head_name(t: &Tree) -> Option<&str> {
    match t {
        Tree::List(items, _) => items.first().and_then(|h| h.as_name()),
        _ => None,
    }
}

/// Match a compiled pattern child-sequence against a subject child-sequence, honoring at most one
/// splice.
fn match_seq(pitems: &[Pat], sitems: &[Tree], binds: &mut Bindings) -> bool {
    let splice_at = pitems
        .iter()
        .position(|c| matches!(c, Pat::Splice { .. }));
    match splice_at {
        None => {
            pitems.len() == sitems.len()
                && pitems
                    .iter()
                    .zip(sitems)
                    .all(|(p, s)| match_pat(p, s, binds))
        }
        Some(k) => {
            let before = &pitems[..k];
            let after = &pitems[k + 1..];
            if sitems.len() < before.len() + after.len() {
                return false;
            }
            for (p, s) in before.iter().zip(&sitems[..before.len()]) {
                if !match_pat(p, s, binds) {
                    return false;
                }
            }
            let suffix_start = sitems.len() - after.len();
            for (p, s) in after.iter().zip(&sitems[suffix_start..]) {
                if !match_pat(p, s, binds) {
                    return false;
                }
            }
            let mid = &sitems[before.len()..suffix_start];
            let Pat::Splice { name } = &pitems[k] else {
                unreachable!("splice_at points at a splice")
            };
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
// Relational search — a pattern filtered by STRUCTURAL context (ancestors / descendants)
// ============================================================================================

/// A query is a main [`Pattern`] plus optional STRUCTURAL context constraints. A node matches the
/// query iff it matches `pattern` AND every constraint holds. All constraints are purely structural
/// (ancestry / containment) — no scope or type context, which belongs to the compiler.
///
/// - `inside` — some ANCESTOR of the node matches this pattern.
/// - `has` — some (strict) DESCENDANT of the node matches this pattern.
/// - `not_inside` / `not_has` — the negations.
#[derive(Clone, Debug, Default)]
pub struct Query {
    pub inside: Vec<Pattern>,
    pub has: Vec<Pattern>,
    pub not_inside: Vec<Pattern>,
    pub not_has: Vec<Pattern>,
}

impl Query {
    pub fn new() -> Query {
        Query::default()
    }
    pub fn inside(mut self, p: Pattern) -> Query {
        self.inside.push(p);
        self
    }
    pub fn has(mut self, p: Pattern) -> Query {
        self.has.push(p);
        self
    }
    pub fn not_inside(mut self, p: Pattern) -> Query {
        self.not_inside.push(p);
        self
    }
    pub fn not_has(mut self, p: Pattern) -> Query {
        self.not_has.push(p);
        self
    }

    /// Whether any constraints are set (an empty query is just the bare pattern).
    fn is_empty(&self) -> bool {
        self.inside.is_empty()
            && self.has.is_empty()
            && self.not_inside.is_empty()
            && self.not_has.is_empty()
    }
}

/// Does any node in the subtree rooted at `node` match `pattern`? (Reflexive — includes `node`.)
fn any_match(pattern: &Pattern, node: &Tree) -> bool {
    let mut binds = Bindings::default();
    if pattern.matches(node, &mut binds) {
        return true;
    }
    match node {
        Tree::List(items, _) => items.iter().any(|c| any_match(pattern, c)),
        Tree::Atom(_, _) => false,
    }
}

/// Does any STRICT descendant of `node` match `pattern`? (Non-reflexive — excludes `node` itself.)
fn any_descendant_match(pattern: &Pattern, node: &Tree) -> bool {
    match node {
        Tree::List(items, _) => items.iter().any(|c| any_match(pattern, c)),
        Tree::Atom(_, _) => false,
    }
}

/// Find every node matching `pattern` AND satisfying `query`'s structural constraints, pre-order.
/// The `ancestors` for `inside`/`not_inside` are the nodes strictly enclosing a candidate.
pub fn search_with(
    pattern: &Pattern,
    query: &Query,
    subject: &Tree,
    spans: Option<&SpanTable>,
) -> Vec<Match> {
    let mut out = Vec::new();
    let mut ancestors: Vec<&Tree> = Vec::new();
    search_with_at(pattern, query, subject, &mut ancestors, spans, &mut out);
    out
}

fn search_with_at<'t>(
    pattern: &Pattern,
    query: &Query,
    node: &'t Tree,
    ancestors: &mut Vec<&'t Tree>,
    spans: Option<&SpanTable>,
    out: &mut Vec<Match>,
) {
    let mut binds = Bindings::default();
    if pattern.matches(node, &mut binds) && constraints_hold(query, node, ancestors) {
        out.push(Match {
            node: node.clone(),
            span: node.origin().and_then(|id| spans.and_then(|s| s.get(id))),
            bindings: binds,
        });
    }
    if let Tree::List(items, _) = node {
        ancestors.push(node);
        for c in items {
            search_with_at(pattern, query, c, ancestors, spans, out);
        }
        ancestors.pop();
    }
}

/// Evaluate the structural constraints of `query` for a candidate `node` with the given `ancestors`.
fn constraints_hold(query: &Query, node: &Tree, ancestors: &[&Tree]) -> bool {
    let inside_ok = |p: &Pattern| {
        ancestors.iter().any(|a| {
            let mut b = Bindings::default();
            p.matches(a, &mut b)
        })
    };
    query.inside.iter().all(inside_ok)
        && !query.not_inside.iter().any(inside_ok)
        && query.has.iter().all(|p| any_descendant_match(p, node))
        && !query.not_has.iter().any(|p| any_descendant_match(p, node))
}

/// Count matches of a relational query.
pub fn count_with(pattern: &Pattern, query: &Query, subject: &Tree) -> usize {
    if query.is_empty() {
        return count(pattern, subject);
    }
    search_with(pattern, query, subject, None).len()
}

// ============================================================================================
// Rewrite — template-instantiated, single- or multi-rule, choice of traversal strategy
// ============================================================================================

/// One rewrite rule: match `pattern`, replace with `template` instantiated from the captures.
#[derive(Clone, Debug)]
pub struct Rule {
    pub pattern: Pattern,
    pub template: Template,
}

impl Rule {
    pub fn new(pattern: Pattern, template: Template) -> Rule {
        Rule { pattern, template }
    }

    /// Compile a rule from a `(rule PATTERN TEMPLATE)` s-expression form.
    pub fn compile_form(t: &Tree) -> Result<Rule, PatternError> {
        match t {
            Tree::List(items, _) if items.first().and_then(|h| h.as_name()) == Some("rule") => {
                match items.as_slice() {
                    [_, p, tmpl] => Ok(Rule {
                        pattern: Pattern { pat: compile_pat(p)? },
                        template: Template { tree: tmpl.clone() },
                    }),
                    _ => Err(PatternError(
                        "a rule is `(rule PATTERN TEMPLATE)`".into(),
                    )),
                }
            }
            _ => Err(PatternError("expected a `(rule …)` form".into())),
        }
    }

    /// Try this rule at `node`: on a match, the instantiated replacement; else `None`.
    fn fire(&self, node: &Tree) -> Option<Tree> {
        let mut binds = Bindings::default();
        if self.pattern.matches(node, &mut binds) {
            return instantiate(&self.template.tree, &binds);
        }
        None
    }
}

/// An ordered set of rules applied together: at each node, the FIRST rule that matches (and
/// instantiates) fires. This is the peephole-simplifier shape — many small `pattern→template`
/// identities in one traversal.
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    pub rules: Vec<Rule>,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> RuleSet {
        RuleSet { rules }
    }

    /// Compile a rule set from s-expression text: a sequence of `(rule PATTERN TEMPLATE)` forms
    /// (whitespace/`;`-comment separated, the same surface `sexpr::read_all` reads).
    pub fn compile(src: &str) -> Result<RuleSet, PatternError> {
        let arena = sexpr::read_all(src).map_err(|e| PatternError(format!("rules parse: {}", e.0)))?;
        // read_all wraps the forms in a synthetic `(do form…)`; the rules are its tail.
        let tree = Tree::of(&arena);
        let forms = match &tree {
            Tree::List(items, _) if items.first().and_then(|h| h.as_name()) == Some("do") => {
                &items[1..]
            }
            _ => std::slice::from_ref(&tree),
        };
        let rules = forms.iter().map(Rule::compile_form).collect::<Result<_, _>>()?;
        Ok(RuleSet::new(rules))
    }

    /// The first rule that fires at `node`, with its replacement.
    fn fire(&self, node: &Tree) -> Option<Tree> {
        self.rules.iter().find_map(|r| r.fire(node))
    }
}

/// The traversal order a rewrite uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Strategy {
    /// Rewrite children first, then match the node against its already-rewritten form. A rule that
    /// exposes a new match in its result is caught within the same pass at the parent. (The default —
    /// terminating, predictable, and what a constant-fold/peephole pass wants.)
    #[default]
    BottomUp,
    /// Match the node first; if it fires, DO NOT descend into the replacement in this pass (the
    /// replacement is taken as-is). Otherwise descend into children. Catches an outermost match once.
    TopDown,
}

/// Rewrite `subject` with a single `pattern → template` rule (bottom-up). Kept for the common case;
/// [`rewrite_rules`] generalizes to a rule set and a strategy.
///
/// A template metavariable with no binding, or used with the wrong arity (single vs splice), makes a
/// site fail to instantiate: the node is left unchanged and NOT counted (reject-don't-corrupt).
pub fn rewrite(pattern: &Pattern, template: &Template, subject: &Tree) -> Rewrite {
    let set = RuleSet::new(vec![Rule::new(pattern.clone(), template.clone())]);
    rewrite_rules(&set, subject, Strategy::BottomUp)
}

/// Rewrite `subject` with a rule set under a traversal `strategy`. At each visited node the first
/// matching rule fires; the count is the number of sites rewritten.
pub fn rewrite_rules(rules: &RuleSet, subject: &Tree, strategy: Strategy) -> Rewrite {
    let mut count = 0;
    let tree = rewrite_node(rules, subject, strategy, &mut count);
    Rewrite { tree, count }
}

fn rewrite_node(rules: &RuleSet, node: &Tree, strategy: Strategy, count: &mut usize) -> Tree {
    match strategy {
        Strategy::BottomUp => {
            // Rewrite children first, then fire at this node's rewritten form.
            let rewritten = match node {
                Tree::Atom(l, o) => Tree::Atom(l.clone(), *o),
                Tree::List(items, o) => Tree::List(
                    items
                        .iter()
                        .map(|c| rewrite_node(rules, c, strategy, count))
                        .collect(),
                    *o,
                ),
            };
            if let Some(new_tree) = rules.fire(&rewritten) {
                *count += 1;
                new_tree
            } else {
                rewritten
            }
        }
        Strategy::TopDown => {
            // Fire at this node first; if it fires, keep the replacement as-is (don't re-descend this
            // pass — run to a fixpoint for saturation). Otherwise descend into children.
            if let Some(new_tree) = rules.fire(node) {
                *count += 1;
                return new_tree;
            }
            match node {
                Tree::Atom(l, o) => Tree::Atom(l.clone(), *o),
                Tree::List(items, o) => Tree::List(
                    items
                        .iter()
                        .map(|c| rewrite_node(rules, c, strategy, count))
                        .collect(),
                    *o,
                ),
            }
        }
    }
}

/// Rewrite to a fixed point: repeat the single-rule [`rewrite`] until no site matches or `max_passes`
/// is reached. `count` is the total across all passes; `max_passes` bounds a rule whose output
/// re-matches its input (which would otherwise loop forever).
pub fn rewrite_fixpoint(
    pattern: &Pattern,
    template: &Template,
    subject: &Tree,
    max_passes: usize,
) -> Rewrite {
    let set = RuleSet::new(vec![Rule::new(pattern.clone(), template.clone())]);
    rewrite_rules_fixpoint(&set, subject, Strategy::BottomUp, max_passes)
}

/// Rewrite a rule set to a fixed point under `strategy`, bounded by `max_passes`.
pub fn rewrite_rules_fixpoint(
    rules: &RuleSet,
    subject: &Tree,
    strategy: Strategy,
    max_passes: usize,
) -> Rewrite {
    let mut current = subject.clone();
    let mut total = 0;
    for _ in 0..max_passes {
        let r = rewrite_rules(rules, &current, strategy);
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
    if let Some(name) = template_metavar(t) {
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

    /// Render every match of `pattern` (filtered by `query`'s structural constraints) in `target` as
    /// a report, one per line: `byte START-END: <matched s-expr>` when a span is known, else
    /// `<index>: <matched s-expr>`. The captured bindings are appended as `  $name = <sexpr>` lines.
    pub fn report_matches(pattern: &Pattern, query: &Query, target: &Target) -> String {
        let matches = search_with(pattern, query, &target.tree, target.spans.as_ref());
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

    /// Apply a `rules` set to `target` under `strategy` (optionally to a fixed point) and project the
    /// result in `to` format. VALIDATES the result as a transaction: the rewritten tree is re-printed
    /// to ML and re-parsed; if that fails, the rewrite is REJECTED (no output) with the parse error —
    /// never a half-applied edit. (Type-checking the result is the Rung-3 step, requiring the compiler
    /// crate; re-parse well-formedness is what this dependency-free layer can guarantee.)
    pub fn apply_rewrite(
        rules: &RuleSet,
        strategy: Strategy,
        target: &Target,
        to: Format,
        width: usize,
        fixpoint: bool,
    ) -> Result<RewriteOutcome, String> {
        let r = if fixpoint {
            rewrite_rules_fixpoint(rules, &target.tree, strategy, 64)
        } else {
            rewrite_rules(rules, &target.tree, strategy)
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

    /// Project the loaded target's ORIGINAL tree in `to` format — the "before" side of a diff. Diffing
    /// this against the rewrite's output shows ONLY the structural change, never reformatting noise
    /// (both sides go through the same printer at the same width).
    pub fn project_target(target: &Target, to: Format, width: usize) -> Result<String, String> {
        project(&target.tree.to_arena(), to, width)
    }

    /// A single query match rendered for machine consumption.
    pub fn matches_json(pattern: &Pattern, query: &Query, target: &Target, file: Option<&str>) -> String {
        let matches = search_with(pattern, query, &target.tree, target.spans.as_ref());
        let mut arr = json::Array::new();
        for m in &matches {
            let mut obj = json::Object::new();
            if let Some(f) = file {
                obj.string("file", f);
            }
            match m.span {
                Some(s) => obj.raw("span", &format!("{{\"start\":{},\"end\":{}}}", s.start, s.end)),
                None => obj.raw("span", "null"),
            }
            obj.string("matched", &m.node.to_sexpr());
            let mut binds = json::Object::new();
            for (name, nodes) in m.bindings.iter() {
                match nodes {
                    [one] => binds.string(name, &one.to_sexpr()),
                    many => {
                        let mut a = json::Array::new();
                        for n in many {
                            a.string(&n.to_sexpr());
                        }
                        binds.raw(name, &a.finish());
                    }
                }
            }
            obj.raw("bindings", &binds.finish());
            arr.raw(&obj.finish());
        }
        arr.finish()
    }

    /// A rewrite outcome rendered for machine consumption: `{file?, count, rewritten}`.
    ///
    /// The whole-file replacement (`rewritten`) IS the edit — our rewrite is whole-tree, so there is
    /// no non-overlapping per-span edit list to fake. `count` is the number of sites that fired.
    pub fn rewrite_json(file: Option<&str>, count: usize, rewritten: &str) -> String {
        let mut obj = json::Object::new();
        if let Some(f) = file {
            obj.string("file", f);
        }
        obj.raw("count", &count.to_string());
        obj.string("rewritten", rewritten);
        obj.finish()
    }

    /// Render a structural tree-diff (`a` → `b`) as human-readable lines, one change per line:
    /// `PATH: replace OLD => NEW` / `PATH: add NEW` / `PATH: remove OLD`. Empty when identical.
    pub fn changes_report(a: &Tree, b: &Tree) -> String {
        let changes = treediff::diff(a, b);
        let mut out = String::new();
        for c in &changes {
            let p = treediff::path_str(&c.path);
            let line = match &c.kind {
                treediff::ChangeKind::Replace { old, new } => format!("{p}: replace {old} => {new}"),
                treediff::ChangeKind::Add { new } => format!("{p}: add {new}"),
                treediff::ChangeKind::Remove { old } => format!("{p}: remove {old}"),
            };
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    /// Render a structural tree-diff as JSON: `[{path:[…], kind, old?, new?}]`.
    pub fn changes_json(a: &Tree, b: &Tree) -> String {
        let changes = treediff::diff(a, b);
        let mut arr = json::Array::new();
        for c in &changes {
            let mut obj = json::Object::new();
            let mut path = json::Array::new();
            for i in &c.path {
                path.raw(&i.to_string());
            }
            obj.raw("path", &path.finish());
            match &c.kind {
                treediff::ChangeKind::Replace { old, new } => {
                    obj.string("kind", "replace");
                    obj.string("old", old);
                    obj.string("new", new);
                }
                treediff::ChangeKind::Add { new } => {
                    obj.string("kind", "add");
                    obj.string("new", new);
                }
                treediff::ChangeKind::Remove { old } => {
                    obj.string("kind", "remove");
                    obj.string("old", old);
                }
            }
            arr.raw(&obj.finish());
        }
        arr.finish()
    }

    /// Convert a byte offset into `src` to a 1-based `(line, column)`. Column is counted in bytes from
    /// the line start (good enough for ASCII source and monotonic for UTF-8). A byte past the end
    /// clamps to the last position.
    pub fn line_col(src: &str, byte: usize) -> (usize, usize) {
        let byte = byte.min(src.len());
        let mut line = 1usize;
        let mut col = 1usize;
        for (i, ch) in src.char_indices() {
            if i >= byte {
                break;
            }
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// Run `lints` over `target`, rendering diagnostics one per line as
    /// `LABEL:line:col: SEVERITY: message` (LABEL is the file path or `(stdin)`; line:col from `src`
    /// when a span is known, else `?:?`). Returns `(report, had_error)` — `had_error` is the CI signal.
    pub fn lint_report(
        lints: &lint::LintSet,
        target: &Target,
        src: &str,
        label: &str,
    ) -> (String, bool) {
        let diags = lint::run(lints, &target.tree, target.spans.as_ref());
        let mut out = String::new();
        for d in &diags {
            let loc = match d.span {
                Some(s) => {
                    let (l, c) = line_col(src, s.start);
                    format!("{label}:{l}:{c}")
                }
                None => format!("{label}:?:?"),
            };
            out.push_str(&format!("{loc}: {}: {}\n", d.severity.as_str(), d.message));
        }
        (out, lint::has_error(&diags))
    }

    /// Run `lints` over `target`, rendering diagnostics as JSON
    /// `[{file?, line?, col?, severity, message, matched}]`. Returns `(json, had_error)`.
    pub fn lint_json(
        lints: &lint::LintSet,
        target: &Target,
        src: &str,
        file: Option<&str>,
    ) -> (String, bool) {
        let diags = lint::run(lints, &target.tree, target.spans.as_ref());
        let mut arr = json::Array::new();
        for d in &diags {
            let mut obj = json::Object::new();
            if let Some(f) = file {
                obj.string("file", f);
            }
            match d.span {
                Some(s) => {
                    let (l, c) = line_col(src, s.start);
                    obj.raw("line", &l.to_string());
                    obj.raw("col", &c.to_string());
                }
                None => {
                    obj.raw("line", "null");
                    obj.raw("col", "null");
                }
            }
            obj.string("severity", d.severity.as_str());
            obj.string("message", &d.message);
            obj.string("matched", &d.matched);
            arr.raw(&obj.finish());
        }
        (arr.finish(), lint::has_error(&diags))
    }
}

/// A minimal, dependency-free JSON string builder — just what the `--json` output needs (objects,
/// arrays, strings, and pre-rendered raw values like numbers/nested objects). No parser, no serde.
pub mod json {
    /// Escape a string as a JSON string literal (including the surrounding quotes).
    pub fn quote(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    /// Accumulates `"key": value` members into a JSON object.
    #[derive(Default)]
    pub struct Object {
        parts: Vec<String>,
    }
    impl Object {
        pub fn new() -> Object {
            Object::default()
        }
        /// A string-valued member.
        pub fn string(&mut self, key: &str, value: &str) {
            self.parts.push(format!("{}:{}", quote(key), quote(value)));
        }
        /// A member whose value is already-rendered JSON (a number, `null`, a nested object/array).
        pub fn raw(&mut self, key: &str, value: &str) {
            self.parts.push(format!("{}:{}", quote(key), value));
        }
        pub fn finish(&self) -> String {
            format!("{{{}}}", self.parts.join(","))
        }
    }

    /// Accumulates elements into a JSON array.
    #[derive(Default)]
    pub struct Array {
        parts: Vec<String>,
    }
    impl Array {
        pub fn new() -> Array {
            Array::default()
        }
        /// A string element.
        pub fn string(&mut self, value: &str) {
            self.parts.push(quote(value));
        }
        /// An element whose value is already-rendered JSON.
        pub fn raw(&mut self, value: &str) {
            self.parts.push(value.to_string());
        }
        pub fn finish(&self) -> String {
            format!("[{}]", self.parts.join(","))
        }
    }
}

/// A line-based unified diff, dependency-free — for previewing a rewrite before applying it. Diffs
/// the two texts by longest-common-subsequence over lines and emits `@@`-hunk unified-diff output
/// with 3 lines of context.
pub mod diff {
    /// One line-level edit, carrying the line text (borrowed from the inputs).
    enum Edit<'a> {
        Keep(&'a str),
        Del(&'a str),
        Ins(&'a str),
    }

    /// Unified diff of `old` → `new`, labeled `old_label`/`new_label`. Empty string when identical.
    pub fn unified(old: &str, new: &str, old_label: &str, new_label: &str) -> String {
        let a: Vec<&str> = old.lines().collect();
        let b: Vec<&str> = new.lines().collect();
        let script = edit_script(&a, &b);
        if script.iter().all(|e| matches!(e, Edit::Keep(_))) {
            return String::new();
        }
        let mut out = format!("--- {old_label}\n+++ {new_label}\n");
        for h in group_hunks(&script, 3) {
            out.push_str(&h);
        }
        out
    }

    /// LCS-based edit script over lines.
    fn edit_script<'a>(a: &[&'a str], b: &[&'a str]) -> Vec<Edit<'a>> {
        let (n, m) = (a.len(), b.len());
        // dp[i][j] = LCS length of a[i..], b[j..].
        let mut dp = vec![vec![0usize; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i][j] = if a[i] == b[j] {
                    dp[i + 1][j + 1] + 1
                } else {
                    dp[i + 1][j].max(dp[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        let mut script = Vec::new();
        while i < n && j < m {
            if a[i] == b[j] {
                script.push(Edit::Keep(a[i]));
                i += 1;
                j += 1;
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                script.push(Edit::Del(a[i]));
                i += 1;
            } else {
                script.push(Edit::Ins(b[j]));
                j += 1;
            }
        }
        while i < n {
            script.push(Edit::Del(a[i]));
            i += 1;
        }
        while j < m {
            script.push(Edit::Ins(b[j]));
            j += 1;
        }
        script
    }

    /// Group the edit script into unified-diff hunks with `context` lines around each change run.
    fn group_hunks(script: &[Edit], context: usize) -> Vec<String> {
        let changed: Vec<usize> = script
            .iter()
            .enumerate()
            .filter(|(_, e)| !matches!(e, Edit::Keep(_)))
            .map(|(i, _)| i)
            .collect();
        if changed.is_empty() {
            return Vec::new();
        }
        // Merge nearby changes (gap ≤ 2*context) into shared hunks.
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for &c in &changed {
            let lo = c.saturating_sub(context);
            let hi = (c + context).min(script.len() - 1);
            match ranges.last_mut() {
                Some(last) if lo <= last.1 + 1 => last.1 = last.1.max(hi),
                _ => ranges.push((lo, hi)),
            }
        }
        ranges
            .into_iter()
            .map(|(lo, hi)| render_hunk(script, lo, hi))
            .collect()
    }

    /// Render one hunk covering `script[lo..=hi]` with a 1-based `@@` line-range header.
    fn render_hunk(script: &[Edit], lo: usize, hi: usize) -> String {
        // Old/new start = count of old/new lines strictly before `lo`.
        let mut old_start = 0usize;
        let mut new_start = 0usize;
        for e in &script[..lo] {
            match e {
                Edit::Keep(_) => {
                    old_start += 1;
                    new_start += 1;
                }
                Edit::Del(_) => old_start += 1,
                Edit::Ins(_) => new_start += 1,
            }
        }
        let (mut old_count, mut new_count) = (0usize, 0usize);
        let mut body = String::new();
        for e in &script[lo..=hi] {
            match e {
                Edit::Keep(t) => {
                    body.push_str(&format!(" {t}\n"));
                    old_count += 1;
                    new_count += 1;
                }
                Edit::Del(t) => {
                    body.push_str(&format!("-{t}\n"));
                    old_count += 1;
                }
                Edit::Ins(t) => {
                    body.push_str(&format!("+{t}\n"));
                    new_count += 1;
                }
            }
        }
        format!(
            "@@ -{},{} +{},{} @@\n{body}",
            old_start + 1,
            old_count,
            new_start + 1,
            new_count
        )
    }
}

/// A STRUCTURAL tree diff — what SUBTREES changed between two programs, not what text lines moved.
/// The complement of `diff` (line-based): where the unified diff shows edited source lines, this
/// shows edited nodes, each addressed by a path (the child-index route from the root). It answers
/// "what did a rewrite/edit actually change to the tree?" independent of formatting.
pub mod treediff {
    use super::{tree_eq, Tree};

    /// One structural change between the old and new trees.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Change {
        /// The child-index route from the root to the changed node. `[]` is the root; `[2, 0]` is
        /// "child 2, then its child 0". For `Add`/`Remove` in a list, the last index is the position
        /// within that list.
        pub path: Vec<usize>,
        pub kind: ChangeKind,
    }

    /// The nature of a change. `old`/`new` are rendered s-expressions (one line each).
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum ChangeKind {
        /// A node was replaced by a different node (leaf value changed, a list became an atom or vice
        /// versa, or a list's HEAD changed so recursion can't align children meaningfully).
        Replace { old: String, new: String },
        /// A child node was inserted in a list (present in new, not aligned in old).
        Add { new: String },
        /// A child node was deleted from a list (present in old, not aligned in new).
        Remove { old: String },
    }

    /// Structurally diff `old` → `new`, returning the changes in pre-order. Empty when the trees are
    /// structurally equal (provenance ignored).
    ///
    /// Alignment rule (what makes the output read like code, not a raw insert/delete storm):
    /// - Two lists with the SAME head name and SAME arity recurse **positionally** — a single changed
    ///   operand is one `Replace` at that child, not "whole form replaced".
    /// - Two lists that differ in arity (same head or not) align children by LCS over structural
    ///   equality, yielding `Add`/`Remove` for the unmatched children and recursing into aligned pairs.
    /// - Anything else (atom vs list, differing leaf value, differing head) is one `Replace`.
    pub fn diff(old: &Tree, new: &Tree) -> Vec<Change> {
        let mut out = Vec::new();
        diff_at(old, new, &mut Vec::new(), &mut out);
        out
    }

    fn diff_at(old: &Tree, new: &Tree, path: &mut Vec<usize>, out: &mut Vec<Change>) {
        if tree_eq(old, new) {
            return;
        }
        match (old, new) {
            (Tree::List(a, _), Tree::List(b, _)) if same_head(a, b) => {
                diff_children(a, b, path, out);
            }
            // Different shape / leaf / head: a whole-node replace.
            _ => out.push(Change {
                path: path.clone(),
                kind: ChangeKind::Replace {
                    old: old.to_sexpr(),
                    new: new.to_sexpr(),
                },
            }),
        }
    }

    /// Do two child-lists share a head name? (Both empty, or both headed by the same name atom.) A
    /// shared head means recursing is meaningful; a changed head means the forms are different
    /// constructs and a whole-node replace reads better.
    fn same_head(a: &[Tree], b: &[Tree]) -> bool {
        match (a.first(), b.first()) {
            (None, None) => true,
            (Some(x), Some(y)) => match (x.as_name(), y.as_name()) {
                (Some(nx), Some(ny)) => nx == ny,
                // Non-name heads (rare): treat as alignable so we recurse rather than replace whole.
                _ => true,
            },
            _ => false,
        }
    }

    /// Diff the children of two same-head lists. Equal arity ⇒ positional; unequal ⇒ LCS alignment.
    fn diff_children(a: &[Tree], b: &[Tree], path: &mut Vec<usize>, out: &mut Vec<Change>) {
        if a.len() == b.len() {
            for (i, (x, y)) in a.iter().zip(b).enumerate() {
                path.push(i);
                diff_at(x, y, path, out);
                path.pop();
            }
            return;
        }
        // LCS over structural equality gives a stable alignment; unmatched a-children are Remove,
        // unmatched b-children are Add, matched pairs recurse.
        let ops = align(a, b);
        // `pos` tracks the index within the NEW list for reporting Add/Remove positions.
        let mut pos = 0usize;
        for op in ops {
            match op {
                Align::Keep(i, j) => {
                    path.push(j);
                    diff_at(&a[i], &b[j], path, out);
                    path.pop();
                    pos = j + 1;
                }
                Align::Del(i) => {
                    let mut p = path.clone();
                    p.push(pos);
                    out.push(Change {
                        path: p,
                        kind: ChangeKind::Remove { old: a[i].to_sexpr() },
                    });
                }
                Align::Ins(j) => {
                    let mut p = path.clone();
                    p.push(j);
                    out.push(Change {
                        path: p,
                        kind: ChangeKind::Add { new: b[j].to_sexpr() },
                    });
                    pos = j + 1;
                }
            }
        }
    }

    enum Align {
        Keep(usize, usize), // (index in a, index in b) — structurally equal, recurse for inner diffs
        Del(usize),         // index in a
        Ins(usize),         // index in b
    }

    /// LCS alignment of two child slices by structural equality (the same shape as the line diff, on
    /// trees). Prefers keeping structurally-equal children aligned so Add/Remove land on the genuinely
    /// new/old ones.
    fn align(a: &[Tree], b: &[Tree]) -> Vec<Align> {
        let (n, m) = (a.len(), b.len());
        let mut dp = vec![vec![0usize; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                dp[i][j] = if tree_eq(&a[i], &b[j]) {
                    dp[i + 1][j + 1] + 1
                } else {
                    dp[i + 1][j].max(dp[i][j + 1])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        let mut ops = Vec::new();
        while i < n && j < m {
            if tree_eq(&a[i], &b[j]) {
                ops.push(Align::Keep(i, j));
                i += 1;
                j += 1;
            } else if dp[i + 1][j] >= dp[i][j + 1] {
                ops.push(Align::Del(i));
                i += 1;
            } else {
                ops.push(Align::Ins(j));
                j += 1;
            }
        }
        while i < n {
            ops.push(Align::Del(i));
            i += 1;
        }
        while j < m {
            ops.push(Align::Ins(j));
            j += 1;
        }
        ops
    }

    /// Render a path `[2, 0]` as `2.0` (or `<root>` for the empty path) for human output.
    pub fn path_str(path: &[usize]) -> String {
        if path.is_empty() {
            "<root>".to_string()
        } else {
            path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(".")
        }
    }
}

/// Structural LINTING — flag anti-patterns by shape rather than fix them. A lint rule is a pattern
/// plus a message and a severity; every match becomes a diagnostic. Batched over a codebase, this is
/// a Semgrep-lite structural checker / CI gate: it exits non-zero when any `error`-severity rule
/// fires. Purely syntactic (no scope/type), like the rest of this layer.
pub mod lint {
    use super::{compile_pat, search_with, Pattern, PatternError, Query, Span, SpanTable, Tree};

    /// A diagnostic's severity. `error` is the only one that fails a run (non-zero exit); `warning`
    /// and `info` are reported but do not fail. `warning` is the default when a rule omits it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Severity {
        Error,
        Warning,
        Info,
    }

    impl Severity {
        /// Parse a severity name; `None` for an unknown one.
        pub fn parse(s: &str) -> Option<Severity> {
            match s {
                "error" => Some(Severity::Error),
                "warning" | "warn" => Some(Severity::Warning),
                "info" | "note" => Some(Severity::Info),
                _ => None,
            }
        }

        pub fn as_str(self) -> &'static str {
            match self {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "info",
            }
        }
    }

    /// One lint rule: match `pattern`, and every match reports `message` at `severity`.
    #[derive(Clone, Debug)]
    pub struct LintRule {
        pub pattern: Pattern,
        pub message: String,
        pub severity: Severity,
    }

    impl LintRule {
        /// Compile a rule from a `(lint PATTERN "message" [severity])` s-expression form. Severity
        /// defaults to `warning` when omitted; an unknown severity name is rejected.
        pub fn compile_form(t: &Tree) -> Result<LintRule, PatternError> {
            let items = match t {
                Tree::List(items, _) if head_is(items, "lint") => items,
                _ => return Err(PatternError("expected a `(lint …)` form".into())),
            };
            let (pat_tree, message, sev_tree) = match items.as_slice() {
                [_, p, msg] => (p, msg, None),
                [_, p, msg, sev] => (p, msg, Some(sev)),
                _ => {
                    return Err(PatternError(
                        "a lint rule is `(lint PATTERN \"message\" [severity])`".into(),
                    ))
                }
            };
            let message = as_str_leaf(message).ok_or_else(|| {
                PatternError("a lint rule's message must be a \"string\"".into())
            })?;
            let severity = match sev_tree {
                None => Severity::Warning,
                Some(s) => {
                    let name = s
                        .as_name()
                        .ok_or_else(|| PatternError("severity must be a bare name".into()))?;
                    Severity::parse(name)
                        .ok_or_else(|| PatternError(format!("unknown severity `{name}`")))?
                }
            };
            Ok(LintRule {
                pattern: Pattern { pat: compile_pat(pat_tree)? },
                message: message.to_string(),
                severity,
            })
        }
    }

    /// A set of lint rules, run together over a program.
    #[derive(Clone, Debug, Default)]
    pub struct LintSet {
        pub rules: Vec<LintRule>,
    }

    impl LintSet {
        pub fn new(rules: Vec<LintRule>) -> LintSet {
            LintSet { rules }
        }

        /// Compile a lint set from s-expression text: a sequence of `(lint …)` forms.
        pub fn compile(src: &str) -> Result<LintSet, PatternError> {
            let arena = super::sexpr::read_all(src)
                .map_err(|e| PatternError(format!("lint rules parse: {}", e.0)))?;
            let tree = Tree::of(&arena);
            let forms = match &tree {
                Tree::List(items, _) if head_is(items, "do") => &items[1..],
                _ => std::slice::from_ref(&tree),
            };
            let rules = forms
                .iter()
                .map(LintRule::compile_form)
                .collect::<Result<_, _>>()?;
            Ok(LintSet::new(rules))
        }
    }

    /// One reported diagnostic: the rule's message + severity, and the matched node's span (if the
    /// subject carried one). Rules are applied in order; within a rule, matches are pre-order.
    #[derive(Clone, Debug)]
    pub struct Diagnostic {
        pub message: String,
        pub severity: Severity,
        pub span: Option<Span>,
        /// The matched node rendered as s-expression (for context in output).
        pub matched: String,
    }

    /// Run every lint rule over `subject`, collecting diagnostics. Rules run in order; each rule's
    /// matches are reported in pre-order. `spans` (if any) attaches a source span to each diagnostic.
    pub fn run(set: &LintSet, subject: &Tree, spans: Option<&SpanTable>) -> Vec<Diagnostic> {
        let empty = Query::default();
        let mut out = Vec::new();
        for rule in &set.rules {
            for m in search_with(&rule.pattern, &empty, subject, spans) {
                out.push(Diagnostic {
                    message: rule.message.clone(),
                    severity: rule.severity,
                    span: m.span,
                    matched: m.node.to_sexpr(),
                });
            }
        }
        out
    }

    /// True if any diagnostic is `error`-severity — the signal a CI gate exits non-zero on.
    pub fn has_error(diags: &[Diagnostic]) -> bool {
        diags.iter().any(|d| d.severity == Severity::Error)
    }

    fn head_is(items: &[Tree], name: &str) -> bool {
        items.first().and_then(|h| h.as_name()) == Some(name)
    }

    /// If `t` is a string-literal atom, its contents.
    fn as_str_leaf(t: &Tree) -> Option<&str> {
        match t {
            Tree::Atom(super::Leaf::Str(s), _) => Some(s),
            _ => None,
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

    // ---- guards (structural predicates on a metavar) ----

    #[test]
    fn guard_is_literal_and_is_name_discriminate() {
        // `(+ ,(x is-literal) ,y)` — the first operand must be a literal.
        let lit = pat("(+ ,(x is-literal) ,y)");
        assert_eq!(count(&lit, &subj("(+ 1 a)")), 1);
        assert_eq!(count(&lit, &subj("(+ a 1)")), 0, "a is a name, not a literal");
        // is-name is the complement for atoms.
        let nm = pat("(+ ,(x is-name) ,y)");
        assert_eq!(count(&nm, &subj("(+ a 1)")), 1);
        assert_eq!(count(&nm, &subj("(+ 1 a)")), 0);
    }

    #[test]
    fn guard_typed_literals() {
        assert_eq!(count(&pat("(f ,(x is-int))"), &subj("(f 42)")), 1);
        assert_eq!(count(&pat("(f ,(x is-int))"), &subj("(f 4.2)")), 0);
        assert_eq!(count(&pat("(f ,(x is-str))"), &subj("(f \"hi\")")), 1);
        assert_eq!(count(&pat("(f ,(x is-bool))"), &subj("(f true)")), 1);
        assert_eq!(count(&pat("(f ,(x is-float))"), &subj("(f 4.2)")), 1);
    }

    #[test]
    fn guard_is_atom_vs_is_list() {
        assert_eq!(count(&pat("(f ,(x is-atom))"), &subj("(f a)")), 1);
        assert_eq!(count(&pat("(f ,(x is-atom))"), &subj("(f (g a))")), 0);
        assert_eq!(count(&pat("(f ,(x is-list))"), &subj("(f (g a))")), 1);
        assert_eq!(count(&pat("(f ,(x is-list))"), &subj("(f a)")), 0);
    }

    #[test]
    fn guard_head_is_constrains_a_list_operand() {
        // match a call whose single argument is itself a `*` application.
        let p = pat("(f ,(inner (head-is *)))");
        assert_eq!(count(&p, &subj("(f (* a b))")), 1);
        assert_eq!(count(&p, &subj("(f (+ a b))")), 0);
        assert_eq!(count(&p, &subj("(f a)")), 0, "a name has no head");
    }

    #[test]
    fn guard_matches_subpattern_and_still_binds_outer() {
        // `,(x (matches (lit ,_)))` — x must itself match `(lit ,_)`, and x is still bound.
        let p = pat("(node ,(x (matches (lit ,_))))");
        let s = subj("(node (lit 7))");
        let m = search(&p, &s, None);
        assert_eq!(m.len(), 1);
        // the sub-pattern's own capture does NOT leak; only the outer `x` is bound.
        assert_eq!(m[0].bindings.get("x").unwrap().to_sexpr(), "(lit 7)");
        assert!(m[0].bindings.get_run("_").is_none());
        assert_eq!(count(&p, &subj("(node (var 7))")), 0);
    }

    #[test]
    fn guard_not_negates() {
        // match `(f ,x)` where x is NOT a literal.
        let p = pat("(f ,(x (not is-literal)))");
        assert_eq!(count(&p, &subj("(f a)")), 1);
        assert_eq!(count(&p, &subj("(f 1)")), 0);
    }

    #[test]
    fn multiple_guards_are_conjunctive() {
        // an atom that is not a name ⇒ a literal atom; equivalently `is-atom` AND `(not is-name)`.
        let p = pat("(f ,(x is-atom (not is-name)))");
        assert_eq!(count(&p, &subj("(f 1)")), 1);
        assert_eq!(count(&p, &subj("(f a)")), 0);
        assert_eq!(count(&p, &subj("(f (g))")), 0);
    }

    #[test]
    fn guarded_metavar_still_enforces_consistency() {
        // `(+ ,(x is-name) ,(x is-name))` — both operands must be names AND equal.
        let p = pat("(+ ,(x is-name) ,(x is-name))");
        assert_eq!(count(&p, &subj("(+ a a)")), 1);
        assert_eq!(count(&p, &subj("(+ a b)")), 0);
        assert_eq!(count(&p, &subj("(+ 1 1)")), 0, "guard fails: not names");
    }

    #[test]
    fn unknown_guard_is_rejected_at_compile_time() {
        let e = Pattern::compile("(f ,(x is-frobnicated))").unwrap_err();
        assert!(e.0.contains("unknown guard"), "got {e}");
    }

    #[test]
    fn top_level_splice_is_rejected() {
        let e = Pattern::compile(",@xs").unwrap_err();
        assert!(e.0.contains("direct child of a list"), "got {e}");
    }

    // ---- relational context (inside / has) ----

    #[test]
    fn inside_restricts_to_a_matching_ancestor() {
        // Match `x` only where it occurs inside a `(danger …)`.
        let s = subj("(do (safe x) (danger (g x)))");
        let q = Query::new().inside(pat("(danger ,@_)"));
        let m = search_with(&pat("x"), &q, &s, None);
        assert_eq!(m.len(), 1, "only the x under (danger …): {m:?}");
        // Without the constraint, both `x` occurrences match.
        assert_eq!(count(&pat("x"), &s), 2);
    }

    #[test]
    fn not_inside_excludes_a_matching_ancestor() {
        let s = subj("(do (safe x) (danger (g x)))");
        let q = Query::new().not_inside(pat("(danger ,@_)"));
        let m = search_with(&pat("x"), &q, &s, None);
        assert_eq!(m.len(), 1, "only the x NOT under (danger …)");
    }

    #[test]
    fn has_requires_a_matching_descendant() {
        // Match a `(fn …)` that CONTAINS a `(raise ,_)` somewhere inside.
        let s = subj("(do (fn a (raise e)) (fn b (return c)))");
        let q = Query::new().has(pat("(raise ,_)"));
        let m = search_with(&pat("(fn ,@_)"), &q, &s, None);
        assert_eq!(m.len(), 1);
        assert!(m[0].node.to_sexpr().contains("raise"), "the fn with raise");
    }

    #[test]
    fn not_has_excludes_a_matching_descendant() {
        let s = subj("(do (fn a (raise e)) (fn b (return c)))");
        let q = Query::new().not_has(pat("(raise ,_)"));
        let m = search_with(&pat("(fn ,@_)"), &q, &s, None);
        assert_eq!(m.len(), 1);
        assert!(m[0].node.to_sexpr().contains("return"), "the fn without raise");
    }

    #[test]
    fn has_is_strict_not_reflexive() {
        // A `(raise ,_)` does not "have" itself as a descendant.
        let s = subj("(raise e)");
        let q = Query::new().has(pat("(raise ,_)"));
        assert_eq!(search_with(&pat("(raise ,_)"), &q, &s, None).len(), 0);
    }

    #[test]
    fn constraints_compose_conjunctively() {
        // Find a `(call …)` that is inside a `(module …)` AND does NOT contain a `(deprecated)` node.
        let s = subj("(module (call (deprecated)) (call ok))");
        let q = Query::new()
            .inside(pat("(module ,@_)"))
            .not_has(pat("(deprecated ,@_)"));
        let m = search_with(&pat("(call ,@_)"), &q, &s, None);
        let texts: Vec<_> = m.iter().map(|x| x.node.to_sexpr()).collect();
        // (call ok) qualifies; (call (deprecated)) is excluded by not_has; both are inside module.
        assert_eq!(texts, ["(call ok)"], "got {texts:?}");
    }

    #[test]
    fn empty_query_equals_bare_search() {
        let s = subj("(+ (+ x 0) 0)");
        assert_eq!(count_with(&pat("(+ ,e 0)"), &Query::new(), &s), 2);
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

    // ---- multi-rule sets + strategy ----

    fn rule(p: &str, t: &str) -> Rule {
        Rule::new(pat(p), tmpl(t))
    }

    #[test]
    fn rule_set_applies_several_identities_in_one_pass() {
        // A little arithmetic-identity peephole: +0, *1, *0.
        let rules = RuleSet::new(vec![
            rule("(+ ,x 0)", ",x"),
            rule("(* ,x 1)", ",x"),
            rule("(* ,_ 0)", "0"),
        ]);
        let s = subj("(f (+ a 0) (* b 1) (* c 0))");
        let r = rewrite_rules(&rules, &s, Strategy::BottomUp);
        assert_eq!(r.count, 3);
        assert_eq!(r.tree.to_sexpr(), "(f a b 0)");
    }

    #[test]
    fn rule_set_fires_first_matching_rule() {
        // Two rules both match `(x)`; the FIRST one wins.
        let rules = RuleSet::new(vec![rule("(x)", "first"), rule("(x)", "second")]);
        let r = rewrite_rules(&rules, &subj("(x)"), Strategy::BottomUp);
        assert_eq!(r.tree.to_sexpr(), "first");
    }

    #[test]
    fn rule_set_compiles_from_text() {
        let rules = RuleSet::compile("(rule (+ ,x 0) ,x)\n(rule (* ,x 1) ,x)").unwrap();
        assert_eq!(rules.rules.len(), 2);
        let r = rewrite_rules(&rules, &subj("(+ (* q 1) 0)"), Strategy::BottomUp);
        assert_eq!(r.tree.to_sexpr(), "q");
    }

    #[test]
    fn bad_rule_form_is_rejected() {
        assert!(RuleSet::compile("(notarule x y)").is_err());
        assert!(RuleSet::compile("(rule onlyone)").is_err());
    }

    #[test]
    fn strategy_topdown_vs_bottomup_differ_on_nested_matches() {
        // `(wrap ,x) -> ,x` on `(wrap (wrap a))`.
        let rules = RuleSet::new(vec![rule("(wrap ,x)", ",x")]);
        let s = subj("(wrap (wrap a))");
        // Bottom-up: inner unwraps to `(wrap a)`... then outer matches `(wrap a)` -> `a`. Fully peels.
        let bu = rewrite_rules(&rules, &s, Strategy::BottomUp);
        assert_eq!(bu.tree.to_sexpr(), "a");
        assert_eq!(bu.count, 2);
        // Top-down: outer fires first to `(wrap a)` and is NOT re-descended this pass -> one unwrap.
        let td = rewrite_rules(&rules, &s, Strategy::TopDown);
        assert_eq!(td.tree.to_sexpr(), "(wrap a)");
        assert_eq!(td.count, 1);
        // Top-down to a fixpoint peels fully.
        let td_fp = rewrite_rules_fixpoint(&rules, &s, Strategy::TopDown, 10);
        assert_eq!(td_fp.tree.to_sexpr(), "a");
    }

    #[test]
    fn rule_set_fixpoint_saturates() {
        // A cascading simplification that needs more than one interleaving.
        let rules = RuleSet::new(vec![rule("(+ ,x 0)", ",x"), rule("(neg (neg ,x))", ",x")]);
        let s = subj("(+ (neg (neg (+ y 0))) 0)");
        let r = rewrite_rules_fixpoint(&rules, &s, Strategy::BottomUp, 10);
        assert_eq!(r.tree.to_sexpr(), "y");
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
            let report = driver::report_matches(&pat("(g ,@xs)"), &Query::new(), &target);
            // g(c) -> (g c); the report line names its byte span and the matched form.
            assert!(report.contains("(g c)"), "report: {report}");
            assert!(report.contains("byte "), "has a span: {report}");
        }

        #[test]
        fn report_matches_honors_a_relational_query() {
            let (target, _) = driver::load(b"(do (safe x) (danger (g x)))", Format::Sexpr).unwrap();
            let q = Query::new().inside(pat("(danger ,@_)"));
            let report = driver::report_matches(&pat("x"), &q, &target);
            // only the x under (danger …) — one line.
            assert_eq!(report.lines().filter(|l| l.contains(": x")).count(), 1, "{report}");
        }

        #[test]
        fn load_ml_on_broken_input_still_yields_a_tree_and_reports_errors() {
            // The recovering parser gives a usable tree even here; the driver surfaces the errors.
            let (target, errors) = driver::load(b"f(@)", Format::Ml).unwrap();
            assert!(!errors.is_empty(), "recoverable errors surfaced");
            // still queryable: the call `f(...)` is present.
            assert!(driver::report_matches(&pat("(f ,@xs)"), &Query::new(), &target).contains("f"));
        }

        #[test]
        fn apply_rewrite_projects_ml_and_validates() {
            let (target, _) = driver::load(b"(+ x 0)", Format::Sexpr).unwrap();
            let rules = RuleSet::new(vec![Rule::new(pat("(+ ,e 0)"), tmpl(",e"))]);
            let out =
                driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Ml, 100, false)
                    .unwrap();
            assert_eq!(out.count, 1);
            assert_eq!(out.output.trim(), "x");
        }

        #[test]
        fn apply_rewrite_can_emit_sexpr() {
            let (target, _) = driver::load(b"(g (* a 1) (* b 1))", Format::Sexpr).unwrap();
            let rules = RuleSet::new(vec![Rule::new(pat("(* ,x 1)"), tmpl(",x"))]);
            let out =
                driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Sexpr, 100, false)
                    .unwrap();
            assert_eq!(out.count, 2);
            assert_eq!(out.output.trim(), "(g a b)");
        }

        #[test]
        fn apply_rewrite_runs_a_multi_rule_set() {
            let (target, _) = driver::load(b"(f (+ a 0) (* b 1))", Format::Sexpr).unwrap();
            let rules = RuleSet::compile("(rule (+ ,x 0) ,x) (rule (* ,x 1) ,x)").unwrap();
            let out =
                driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Sexpr, 100, false)
                    .unwrap();
            assert_eq!(out.count, 2);
            assert_eq!(out.output.trim(), "(f a b)");
        }

        #[test]
        fn output_only_format_is_rejected_as_input() {
            let e = driver::load(b"x", Format::Debug).unwrap_err();
            assert!(e.contains("output-only"), "got {e}");
        }

        #[test]
        fn matches_json_is_wellformed_with_span_and_bindings() {
            let (target, _) = driver::load(b"f(a, b)", Format::Ml).unwrap();
            let j = driver::matches_json(&pat("(f ,x ,y)"), &Query::new(), &target, Some("in.ml"));
            // one object, carrying file, a numeric span, the matched form, and both bindings.
            assert!(j.starts_with('[') && j.ends_with(']'), "array: {j}");
            assert!(j.contains("\"file\":\"in.ml\""), "{j}");
            assert!(j.contains("\"span\":{\"start\":"), "{j}");
            assert!(j.contains("\"matched\":\"(f a b)\""), "{j}");
            assert!(j.contains("\"x\":\"a\"") && j.contains("\"y\":\"b\""), "{j}");
        }

        #[test]
        fn matches_json_no_match_is_empty_array() {
            let (target, _) = driver::load(b"(g x)", Format::Sexpr).unwrap();
            assert_eq!(driver::matches_json(&pat("(f ,x)"), &Query::new(), &target, None), "[]");
        }

        #[test]
        fn rewrite_json_shape() {
            let j = driver::rewrite_json(Some("p.ml"), 2, "f(a, b)");
            assert!(j.contains("\"file\":\"p.ml\""), "{j}");
            assert!(j.contains("\"count\":2"), "{j}");
            assert!(j.contains("\"rewritten\":\"f(a, b)\""), "{j}");
        }

        #[test]
        fn project_target_is_the_before_side() {
            let (target, _) = driver::load(b"(+ x 0)", Format::Sexpr).unwrap();
            assert_eq!(driver::project_target(&target, Format::Sexpr, 100).unwrap(), "(+ x 0)");
        }

        #[test]
        fn line_col_maps_a_byte_offset() {
            let src = "abc\ndef\nghi";
            assert_eq!(driver::line_col(src, 0), (1, 1)); // 'a'
            assert_eq!(driver::line_col(src, 2), (1, 3)); // 'c'
            assert_eq!(driver::line_col(src, 4), (2, 1)); // 'd' (after first \n)
            assert_eq!(driver::line_col(src, 9), (3, 2)); // 'h'
            assert_eq!(driver::line_col(src, 9999), (3, 4)); // clamps past end
        }

        #[test]
        fn lint_report_renders_location_severity_message_and_flags_error() {
            let (target, _) = driver::load(b"g(deprecated())", Format::Ml).unwrap();
            let set =
                crate::query::lint::LintSet::compile("(lint (deprecated ,@_) \"no\" error)").unwrap();
            let (report, had_error) = driver::lint_report(&set, &target, "g(deprecated())", "in.ml");
            assert!(had_error, "an error-severity rule fired");
            assert!(report.contains("in.ml:1:"), "has location: {report}");
            assert!(report.contains("error: no"), "severity+message: {report}");
        }

        #[test]
        fn lint_json_is_wellformed_and_warning_is_not_an_error() {
            let (target, _) = driver::load(b"(deprecated x)", Format::Sexpr).unwrap();
            let set =
                crate::query::lint::LintSet::compile("(lint (deprecated ,@_) \"avoid\" warning)")
                    .unwrap();
            let (j, had_error) = driver::lint_json(&set, &target, "(deprecated x)", Some("a.sexp"));
            assert!(!had_error, "warning is not an error");
            assert!(j.contains("\"severity\":\"warning\""), "{j}");
            assert!(j.contains("\"message\":\"avoid\""), "{j}");
            assert!(j.contains("\"file\":\"a.sexp\""), "{j}");
        }
    }

    // ---- json + diff helpers ----

    mod json_tests {
        use crate::query::json;

        #[test]
        fn strings_are_escaped() {
            assert_eq!(json::quote("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        }

        #[test]
        fn object_and_array_compose() {
            let mut o = json::Object::new();
            o.string("k", "v");
            o.raw("n", "42");
            let mut a = json::Array::new();
            a.string("x");
            a.raw(&o.finish());
            assert_eq!(a.finish(), "[\"x\",{\"k\":\"v\",\"n\":42}]");
        }
    }

    mod diff_tests {
        use crate::query::diff;

        #[test]
        fn identical_inputs_produce_no_diff() {
            assert_eq!(diff::unified("a\nb\nc", "a\nb\nc", "old", "new"), "");
        }

        #[test]
        fn a_single_line_change_is_a_unified_hunk() {
            let d = diff::unified("a\nb\nc", "a\nB\nc", "old", "new");
            assert!(d.starts_with("--- old\n+++ new\n"), "header: {d}");
            assert!(d.contains("@@ -1,3 +1,3 @@"), "hunk header: {d}");
            assert!(d.contains("-b\n"), "deletion: {d}");
            assert!(d.contains("+B\n"), "insertion: {d}");
            assert!(d.contains(" a\n") && d.contains(" c\n"), "context: {d}");
        }

        #[test]
        fn pure_insertion_and_deletion() {
            let ins = diff::unified("a\nc", "a\nb\nc", "o", "n");
            assert!(ins.contains("+b\n"), "{ins}");
            let del = diff::unified("a\nb\nc", "a\nc", "o", "n");
            assert!(del.contains("-b\n"), "{del}");
        }
    }

    mod treediff_tests {
        use super::subj;
        use crate::query::treediff::{self, ChangeKind};

        #[test]
        fn identical_trees_have_no_changes() {
            assert!(treediff::diff(&subj("(+ a (* b c))"), &subj("(+ a (* b c))")).is_empty());
        }

        #[test]
        fn a_changed_leaf_is_one_replace_at_its_path() {
            // `(+ a b)` vs `(+ a c)`: one Replace at child 2, NOT a whole-form replace.
            let cs = treediff::diff(&subj("(+ a b)"), &subj("(+ a c)"));
            assert_eq!(cs.len(), 1, "{cs:?}");
            assert_eq!(cs[0].path, vec![2]);
            assert_eq!(
                cs[0].kind,
                ChangeKind::Replace { old: "b".into(), new: "c".into() }
            );
        }

        #[test]
        fn a_nested_change_reports_the_deep_path() {
            // `(f (g x))` vs `(f (g y))`: Replace at path 1.1 (child 1's child 1).
            let cs = treediff::diff(&subj("(f (g x))"), &subj("(f (g y))"));
            assert_eq!(cs.len(), 1, "{cs:?}");
            assert_eq!(cs[0].path, vec![1, 1]);
            assert_eq!(treediff::path_str(&cs[0].path), "1.1");
        }

        #[test]
        fn a_changed_head_replaces_the_whole_node() {
            // Different construct (`+` → `-`): a single whole-node Replace at the root, not per-child.
            let cs = treediff::diff(&subj("(+ a b)"), &subj("(- a b)"));
            assert_eq!(cs.len(), 1, "{cs:?}");
            assert_eq!(cs[0].path, Vec::<usize>::new());
            assert!(matches!(cs[0].kind, ChangeKind::Replace { .. }));
        }

        #[test]
        fn an_added_child_is_an_add() {
            // `(f a b)` vs `(f a b c)`: one Add of `c` at position 3.
            let cs = treediff::diff(&subj("(f a b)"), &subj("(f a b c)"));
            assert_eq!(cs.len(), 1, "{cs:?}");
            assert_eq!(cs[0].kind, ChangeKind::Add { new: "c".into() });
            assert_eq!(cs[0].path, vec![3]);
        }

        #[test]
        fn a_removed_child_is_a_remove() {
            // `(f a b c)` vs `(f a c)`: `b` removed. LCS keeps a and c aligned.
            let cs = treediff::diff(&subj("(f a b c)"), &subj("(f a c)"));
            assert_eq!(cs.len(), 1, "{cs:?}");
            assert_eq!(cs[0].kind, ChangeKind::Remove { old: "b".into() });
        }

        #[test]
        fn atom_vs_list_is_a_replace() {
            let cs = treediff::diff(&subj("x"), &subj("(f y)"));
            assert_eq!(cs.len(), 1);
            assert_eq!(
                cs[0].kind,
                ChangeKind::Replace { old: "x".into(), new: "(f y)".into() }
            );
        }

        #[test]
        fn several_independent_changes_are_all_reported() {
            // two operands change: `(f (+ a 0) (+ b 0))` vs `(f a b)`.
            let cs = treediff::diff(&subj("(f (+ a 0) (+ b 0))"), &subj("(f a b)"));
            assert_eq!(cs.len(), 2, "{cs:?}");
            assert_eq!(cs[0].path, vec![1]);
            assert_eq!(cs[1].path, vec![2]);
        }
    }

    mod lint_tests {
        use super::subj;
        use crate::query::lint::{self, LintSet, Severity};

        #[test]
        fn severity_parses_and_defaults_to_warning() {
            assert_eq!(Severity::parse("error"), Some(Severity::Error));
            assert_eq!(Severity::parse("warn"), Some(Severity::Warning));
            assert_eq!(Severity::parse("note"), Some(Severity::Info));
            assert_eq!(Severity::parse("bogus"), None);
            // a rule with no severity defaults to warning.
            let set = LintSet::compile("(lint (todo ,@_) \"has a todo\")").unwrap();
            assert_eq!(set.rules[0].severity, Severity::Warning);
        }

        #[test]
        fn compile_reads_pattern_message_and_severity() {
            let set =
                LintSet::compile("(lint (deprecated ,@_) \"do not use\" error)").unwrap();
            assert_eq!(set.rules.len(), 1);
            assert_eq!(set.rules[0].message, "do not use");
            assert_eq!(set.rules[0].severity, Severity::Error);
        }

        #[test]
        fn run_reports_a_diagnostic_per_match() {
            let set = LintSet::compile("(lint (deprecated ,@_) \"do not use\" error)").unwrap();
            let s = subj("(do (deprecated a) (ok b) (deprecated c))");
            let diags = lint::run(&set, &s, None);
            assert_eq!(diags.len(), 2, "{diags:?}");
            assert!(diags.iter().all(|d| d.severity == Severity::Error));
            assert_eq!(diags[0].message, "do not use");
            assert!(lint::has_error(&diags));
        }

        #[test]
        fn multiple_rules_run_in_order() {
            let set = LintSet::compile(
                "(lint (a ,@_) \"msg-a\" warning)\n(lint (b ,@_) \"msg-b\" info)",
            )
            .unwrap();
            let diags = lint::run(&set, &subj("(do (a 1) (b 2))"), None);
            assert_eq!(diags.len(), 2);
            assert_eq!(diags[0].message, "msg-a");
            assert_eq!(diags[1].message, "msg-b");
            assert!(!lint::has_error(&diags), "no error-severity rule fired");
        }

        #[test]
        fn a_non_string_message_is_rejected() {
            assert!(LintSet::compile("(lint (x) notastring)").is_err());
        }

        #[test]
        fn an_unknown_severity_is_rejected() {
            let e = LintSet::compile("(lint (x) \"m\" loud)").unwrap_err();
            assert!(e.0.contains("unknown severity"), "got {e}");
        }

        #[test]
        fn lint_rules_use_the_full_pattern_language() {
            // guards work in a lint pattern too: flag `(+ ,x 0)` only where 0 is literal.
            let set = LintSet::compile("(lint (+ ,x ,(z is-literal)) \"maybe redundant\")").unwrap();
            let diags = lint::run(&set, &subj("(do (+ a 0) (+ b c))"), None);
            assert_eq!(diags.len(), 1, "{diags:?}");
        }
    }
}
