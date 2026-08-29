//! Structural query & rewrite over the AST — the codemod substrate.
//!
//! This is Rung 2 of `implementation/design/DESIGN-query-engine.md`: a *built-in* set of structural
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
//!   without binding. Several splices may appear among a list's direct children as long as no two are
//!   ADJACENT — a fixed element between them anchors each run boundary: `(f ,head ,@mid ,last)`,
//!   `(case ,@before (needs ,_) ,@after)` (delete a clause from anywhere in a variadic form).
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
    ///
    /// EXPLICIT stack, not native recursion: this is the FIRST thing every codemod op does to a subject,
    /// which can originate POST-DECODE — and `codec::decode` accepts arbitrarily-deep valid-tree arenas
    /// (no cap, unlike the reader's `MAX_NESTING_DEPTH`), so a recursive copy overflowed the native stack
    /// on a deep tree. A `Job{Visit|Emit}` work-stack + a `results` stack builds the owned tree post-order
    /// (children before parent), children pushed reversed so they land in source order.
    pub fn from_arena(a: &Arenas, root: StructId) -> Tree {
        enum Job {
            Visit(StructId),
            // Assemble a `List` node for `id` once its `n` children sit atop `results`.
            Emit(StructId, usize),
        }
        let mut jobs: Vec<Job> = vec![Job::Visit(root)];
        let mut results: Vec<Tree> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Visit(id) => match a.get(id) {
                    Struct::Atom(l) => results.push(Tree::Atom(a.leaf(*l).clone(), Some(id))),
                    Struct::List(items) => {
                        jobs.push(Job::Emit(id, items.len()));
                        for &c in items.iter().rev() {
                            jobs.push(Job::Visit(c));
                        }
                    }
                },
                Job::Emit(id, n) => {
                    // The n child trees sit on top in source order (reversed push → in-order pop).
                    let kids = results.split_off(results.len() - n);
                    results.push(Tree::List(kids, Some(id)));
                }
            }
        }
        results.pop().expect("from_arena leaves the root tree")
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
        // EXPLICIT stack (dual of `from_arena`): materialize this owned tree into the arena post-order —
        // native recursion overflowed on a deep tree (a codemod's output can be as deep as its input).
        // `Visit(&Tree)` emits an atom immediately or defers a list via `Emit(n)`; children are pushed
        // reversed so their new ids land on `results` in source order for the parent's `b.list`.
        enum Job<'t> {
            Visit(&'t Tree),
            Emit(usize),
        }
        let mut jobs: Vec<Job> = vec![Job::Visit(self)];
        let mut results: Vec<StructId> = Vec::new();
        while let Some(job) = jobs.pop() {
            match job {
                Job::Visit(t) => match t {
                    Tree::Atom(l, _) => results.push(b.atom_leaf(l.clone())),
                    Tree::List(items, _) => {
                        jobs.push(Job::Emit(items.len()));
                        for item in items.iter().rev() {
                            jobs.push(Job::Visit(item));
                        }
                    }
                },
                Job::Emit(n) => {
                    let kids = results.split_off(results.len() - n);
                    results.push(b.list(kids));
                }
            }
        }
        results.pop().expect("build leaves the root id")
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
/// contain several splices among its direct children, none adjacent), a single-node metavariable
/// (with optional structural guards), or a splice metavariable.
#[derive(Clone, Debug)]
enum Pat {
    /// A literal atom that must match an equal leaf value.
    Lit(Leaf),
    /// A list; children matched positionally, honoring zero-or-more (non-adjacent) splices.
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
    /// The node is an IRREFUTABLE match pattern — a var/`_`/tuple/record (recursively), never a
    /// literal or a sum-constructor pattern (`is-irrefutable`). Delegates to
    /// [`crate::match_to_let::is_irrefutable`] (the no-context, conservatively-refutable-ctor form),
    /// so a guarded metavar accepts only a clause head safe to lower to a `let`. Used by the
    /// `idiomatic/single-arm-match` lint so its fix (`(match ,s (,p ,b))` → `(let ((,p ,s)) ,b)`) is a
    /// pure template that fires only when the single arm can never fail.
    IsIrrefutable,
    /// The node is a `camelCase` name atom (`is-camel-case`): a lowercase-initial identifier that
    /// contains an interior uppercase letter with no `_` separator (`fooBar`, `myFunc` — but NOT
    /// `snake_case`, a leading-uppercase `Ctor`/type name, or an all-caps `CONST`). Purely syntactic
    /// (name shape only). Used by the `naming/camel-case` lint to flag a binding whose name breaks the
    /// `snake_case` convention.
    IsCamelCase,
    /// The node's LIST-nesting depth is STRICTLY GREATER than this threshold (`(deeper-than N)`). Depth
    /// is the longest chain of nested list nodes rooted at the matched node (an atom is depth 0; a flat
    /// call `(f a b)` is depth 1; `(f (g (h x)))` is depth 3). A purely STRUCTURAL metric — independent
    /// of how the form is printed. Used by the `idiomatic/deep-nesting` lint to flag over-nested
    /// call/argument structure (DESIGN-cadenza-lint §naming/deep-nesting; operator's `hm-collect.cdz`
    /// motivating case). The threshold is exclusive: `(deeper-than 5)` fires only at depth ≥ 6.
    DeeperThan(usize),
    /// The node's CALL-CHAIN depth is STRICTLY GREATER than this threshold (`(calls-deeper-than N)`).
    /// Unlike [`Guard::DeeperThan`] (whole-subtree list-nesting, which over-counts ordinary structural
    /// nesting — a `module` holding `def`s holding `let`s scores high regardless of any call nesting),
    /// this counts ONLY the longest chain of nested APPLICATION forms: a list whose head is a plain
    /// callee name (not a keyword/reserved word, not an infix-operator head, not a compound-value
    /// constructor `tuple`/`list`/`record`/`map`). So `(f (g (h x)))` is call-depth 3, but a
    /// `(module … (def … (let … (+ a b))))` spine is call-depth 0 (no application forms) and
    /// `(collect (map (filter xs p) f) init)` counts its nested `collect`/`map`/`filter` calls. This is
    /// the metric the `idiomatic/deep-nesting` lint needs (DESIGN-cadenza-lint §deep-nesting, operator's
    /// PR-#2790 `hm-collect.cdz`: deeply nested call ARGUMENTS, not structural depth). Exclusive:
    /// `(calls-deeper-than 4)` fires only at call-depth ≥ 5.
    CallsDeeperThan(usize),
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
    /// Rejects a list with two ADJACENT splices among its direct children (no anchor divides the
    /// run), a splice used outside list-child position, and an unknown/ill-formed guard.
    pub fn compile(src: &str) -> Result<Pattern, PatternError> {
        let arena =
            sexpr::read(src).map_err(|e| PatternError(format!("pattern parse: {}", e.0)))?;
        let pat = compile_pat(&Tree::of(&arena))?;
        Ok(Pattern { pat })
    }

    /// Try to match this pattern against `subject`, filling `binds`. On a mismatch, `binds` may have
    /// been partially extended and must be discarded by the caller.
    fn matches(&self, subject: &Tree, binds: &mut Bindings) -> bool {
        match_pat(&self.pat, subject, binds)
    }

    /// The metavariable names this pattern BINDS on a match (single `,x` + splice `,@xs`), excluding the
    /// anonymous wildcard `_` (which binds nothing). The set a template's metavars must draw from — a
    /// template `,z` that is not among these can never be filled, so a rewrite using it is dead.
    fn bound_metavars(&self) -> std::collections::BTreeSet<String> {
        fn walk(p: &Pat, out: &mut std::collections::BTreeSet<String>) {
            match p {
                Pat::Meta { name, .. } | Pat::Splice { name } => {
                    if !is_wildcard(name) {
                        out.insert(name.clone());
                    }
                }
                Pat::List(items) => items.iter().for_each(|c| walk(c, out)),
                Pat::Lit(_) => {}
            }
        }
        let mut out = std::collections::BTreeSet::new();
        walk(&self.pat, &mut out);
        out
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

    /// A template from an already-parsed sub-`Tree` — the inline template of a paired form (a
    /// `(rule PATTERN TEMPLATE)` or the named lint's `=> TEMPLATE`), where the template is not a
    /// separate source string but a child of the enclosing form. Mirrors what `Rule::compile_form`
    /// does inline; exposed so the `lint` submodule can build the fix template the same way.
    pub(crate) fn from_tree(t: Tree) -> Template {
        Template { tree: t }
    }

    /// Every metavariable name this template REFERENCES — a single `,x` or a splice `,@xs`. Each must be
    /// BOUND by the paired pattern, or the template can never be instantiated. The wildcard `,_` IS
    /// included: a pattern never binds `_`, so a template `,_` is likewise unfillable (it would silently
    /// rewrite 0 sites), not a meaningful hole — so it is reported too.
    fn referenced_metavars(&self) -> std::collections::BTreeSet<String> {
        fn walk(t: &Tree, out: &mut std::collections::BTreeSet<String>) {
            if let Some(name) = template_metavar(t).or_else(|| as_splice(t)) {
                out.insert(name.to_string());
                return; // the metavar payload is a name, not a sub-template to descend
            }
            if let Tree::List(items, _) = t {
                items.iter().for_each(|c| walk(c, out));
            }
        }
        let mut out = std::collections::BTreeSet::new();
        walk(&self.tree, &mut out);
        out
    }
}

/// A potential VARIABLE CAPTURE a rewrite template could introduce: a binder the template adds (a
/// `let`/`fn`/`def`/`match`-arm binding) whose name occurs FREE inside the subtree a matched
/// metavariable bound. Splicing that metavariable's tree under the new binder silently re-scopes those
/// free occurrences to the template's binder — changing the program's meaning even though the rewrite
/// is a faithful structural replace. This is a LINT signal, not an error: the documented rewrite
/// contract is structural replace + re-parse (no α-renaming — binding is the compiler's domain, not
/// this syntax layer's), exactly like ast-grep/comby; capture avoidance is the template author's job.
/// But because `cdz rewrite` is scripted by agents as a "validated" edit, surfacing the risk is cheap
/// insurance. Reported by [`Template::capture_risks`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureRisk {
    /// The name the template's binder introduces (e.g. `x` in a template `(let ((x …)) …)`).
    pub binder: String,
    /// The metavariable whose bound subtree contains `binder` as a free name (e.g. `e` for `,e`).
    pub metavar: String,
    /// Whether the template references `metavar` as a SPLICE (`,@metavar`) rather than a single `,metavar`
    /// — so a caller can print the correct sigil (`,@e` vs `,e`) when naming the metavar to the user.
    pub is_splice: bool,
}

/// The names a binder FORM introduces into the scope of its body — the recognized surface binders,
/// mirroring the canonical s-expression shapes (`core-semantics.md`): `(let ((n v)…) body…)` binds each
/// `n`; `(fn (p…) body…)` and `(def (f p…) body…)` bind the parameter (and `def`'s function) names; a
/// `(match scrut (pat body)…)` arm binds the names its `pat` introduces. Returns the LITERAL binder
/// names only — a metavariable in binder position (`(let ((,n …)) …)`) introduces no statically-known
/// name (it is filled from the match), so it is skipped. Used by the capture lint, not by evaluation
/// (this layer has no scope semantics); it is a deliberately CONSERVATIVE over-approximation — an
/// unrecognized binder form simply contributes no names (a missed warning, never a false rewrite).
fn binder_names(form: &Tree) -> Vec<String> {
    let Tree::List(items, _) = form else {
        return Vec::new();
    };
    let head = items.first().and_then(|h| h.as_name());
    let mut out = Vec::new();
    // Collect the literal binder names a binding-target tree introduces. For the forms handled here
    // (`let` binding-name, `fn` param list, `def` signature) every bare-name leaf is a binder — a `fn`
    // param `(n)` and a `def` signature `(f a)` name real bindings in HEAD position too, so (unlike a
    // match-arm constructor pattern, not handled in this lint) no head is skipped. Metavars in binder
    // position are `(unquote …)` lists — their `unquote` head is not a `Name` leaf, so they contribute
    // nothing (the name is filled from the match, not statically known); wildcards contribute nothing.
    fn pat_names(t: &Tree, out: &mut Vec<String>) {
        match t {
            Tree::Atom(Leaf::Name(n), _) if !is_wildcard(n) => out.push(n.to_string()),
            Tree::List(kids, _) => {
                // A metavar `(unquote name)` / splice `(unquote-splicing name)` in binder position names
                // no static binder — skip it rather than treat `unquote`/its payload as binder names.
                if as_metavar_tree(t).is_some() || as_splice(t).is_some() {
                    return;
                }
                for k in kids {
                    pat_names(k, out);
                }
            }
            _ => {}
        }
    }
    match head {
        Some("let") => {
            // (let ((n v) (n v)…) body…) — each binding pair's first element is a binder name.
            if let Some(Tree::List(bindings, _)) = items.get(1) {
                for bind in bindings {
                    if let Tree::List(pair, _) = bind
                        && let Some(name) = pair.first()
                    {
                        pat_names(name, &mut out);
                    }
                }
            }
        }
        Some("fn") => {
            // (fn (p…) body…) — the parameter list binds each param.
            if let Some(params) = items.get(1) {
                pat_names(params, &mut out);
            }
        }
        Some("def") => {
            // (def (f p…) body…) — the signature list binds the function name AND its params.
            if let Some(sig) = items.get(1) {
                pat_names(sig, &mut out);
            }
        }
        Some("match") => {
            // (match scrut (pat body)… ) — each arm is a `(pattern body)` list; the PATTERN binds names
            // for that arm's scope. Unlike let/fn/def binding-targets, a match pattern's LIST form is a
            // CONSTRUCTOR pattern (`(Some n)`, `(L.Cons h t)`) whose leading name is the constructor, not a
            // binder — so arm-pattern names are collected with `arm_pattern_names`, which skips that head.
            for arm in items.iter().skip(2) {
                if let Tree::List(pair, _) = arm
                    && let Some(pattern) = pair.first()
                {
                    arm_pattern_names(pattern, &mut out);
                }
            }
        }
        _ => {}
    }
    out
}

/// The names a MATCH-ARM pattern binds — like [`binder_names`]'s `pat_names`, but the leading name of a
/// LIST pattern is a CONSTRUCTOR (`(Some n)` → ctor `Some`, binds `n`; `(L.Cons h t)` → binds `h`,`t`;
/// `(C.R)` → nullary ctor, binds nothing), so it is skipped. A bare-name pattern (`m`) binds that name;
/// a literal (`0`) or wildcard binds nothing; a metavar/splice pattern binds no static name.
fn arm_pattern_names(pat: &Tree, out: &mut Vec<String>) {
    match pat {
        Tree::Atom(Leaf::Name(n), _) if !is_wildcard(n) => out.push(n.to_string()),
        Tree::List(kids, _) => {
            if as_metavar_tree(pat).is_some() || as_splice(pat).is_some() {
                return;
            }
            // Skip a leading constructor head; descend the rest as sub-patterns (nested destructuring).
            let start = usize::from(kids.first().and_then(|k| k.as_name()).is_some());
            for k in &kids[start.min(kids.len())..] {
                arm_pattern_names(k, out);
            }
        }
        _ => {}
    }
}

/// The bare names occurring FREE in `tree` — every `Name` atom not in HEAD position of a list (a head
/// name is an operator/keyword/constructor, not a variable reference) and not shadowed by a binder the
/// subtree itself introduces. A conservative syntactic free-variable set for the capture lint: it has
/// no type/scope information (that is the compiler's domain), so it treats every non-head name as a
/// potential variable and only discounts shadowing by the recognized binder forms ([`binder_names`]).
/// Over-approximation is safe for a lint — it can only warn about a name that truly appears.
fn free_names(tree: &Tree, out: &mut std::collections::BTreeSet<String>) {
    fn walk(t: &Tree, bound: &BoundStack, out: &mut std::collections::BTreeSet<String>) {
        match t {
            Tree::Atom(Leaf::Name(n), _) if !is_wildcard(n) && !bound.contains(n) => {
                out.insert(n.to_string());
            }
            Tree::List(items, _) => {
                let introduced = binder_names(t);
                let inner = BoundStack::with(bound, &introduced);
                // Skip the head name (operator/keyword/constructor — not a variable reference); walk the
                // rest under any names this form binds.
                for (i, c) in items.iter().enumerate() {
                    if i == 0 && c.as_name().is_some() {
                        continue;
                    }
                    walk(c, &inner, out);
                }
            }
            _ => {}
        }
    }
    walk(tree, &BoundStack::empty(), out);
}

/// A tiny persistent "set of bound names" for the free-var walk — a linked frame of the names each
/// enclosing binder introduced, so shadowing is scoped without cloning a set at every node.
enum BoundStack<'a> {
    Empty,
    Frame(&'a [String], &'a BoundStack<'a>),
}
impl<'a> BoundStack<'a> {
    fn empty() -> BoundStack<'static> {
        BoundStack::Empty
    }
    fn with(parent: &'a BoundStack<'a>, names: &'a [String]) -> BoundStack<'a> {
        BoundStack::Frame(names, parent)
    }
    fn contains(&self, name: &str) -> bool {
        match self {
            BoundStack::Empty => false,
            BoundStack::Frame(names, parent) => {
                names.iter().any(|n| n == name) || parent.contains(name)
            }
        }
    }
}

impl Template {
    /// The potential variable CAPTURES this template introduces against a match's `bindings`: for each
    /// binder the template adds (`let`/`fn`/`def`/`match`-arm), whether its name occurs FREE in the
    /// subtree any matched metavariable bound. When it does, splicing that metavar under the new binder
    /// silently re-scopes those occurrences (the breaker's `"(+ ,e 1)" -> "(let ((x 100)) (+ ,e x))"`
    /// over `,e = x`: the spliced `x` now resolves to the template's `x`, not the outer one). Empty when
    /// no template binder shadows a free name of any bound tree — the common, safe case.
    ///
    /// A LINT, not a correctness gate: the rewrite contract is a structural replace with NO α-renaming
    /// (binding is the compiler's concern), so this only WARNS. Conservative on both sides (see
    /// [`binder_names`]/[`free_names`]): it may miss an unrecognized binder form but never fabricates a
    /// risk for a name that does not actually occur free.
    pub fn capture_risks(&self, bindings: &Bindings) -> Vec<CaptureRisk> {
        // Every binder name the template introduces anywhere.
        let mut binders: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        fn collect_binders(t: &Tree, out: &mut std::collections::BTreeSet<String>) {
            if let Tree::List(items, _) = t {
                out.extend(binder_names(t));
                items.iter().for_each(|c| collect_binders(c, out));
            }
        }
        collect_binders(&self.tree, &mut binders);
        if binders.is_empty() {
            return Vec::new();
        }
        // The metavar names the template references as a SPLICE (`,@name`), so a risk can carry the right
        // sigil. A name is only ever one or the other in a well-formed template (single vs splice).
        let mut spliced: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        fn collect_splices(t: &Tree, out: &mut std::collections::BTreeSet<String>) {
            if let Some(name) = as_splice(t) {
                out.insert(name.to_string());
            }
            if let Tree::List(items, _) = t {
                items.iter().for_each(|c| collect_splices(c, out));
            }
        }
        collect_splices(&self.tree, &mut spliced);
        // For each metavariable, the free names of the tree(s) it bound; a binder ∩ those is a risk.
        let mut risks = Vec::new();
        for (metavar, trees) in bindings.iter() {
            let mut free = std::collections::BTreeSet::new();
            for t in trees {
                free_names(t, &mut free);
            }
            for binder in &binders {
                if free.contains(binder) {
                    risks.push(CaptureRisk {
                        binder: binder.clone(),
                        metavar: metavar.to_string(),
                        is_splice: spliced.contains(metavar),
                    });
                }
            }
        }
        risks
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
            // Several `,@` splices per list are allowed, provided no two are ADJACENT: an anchor
            // (a fixed pattern element) between them keeps the run boundary unambiguous, so
            // `(F ,@a X ,@b)` matches a bounded sub-sequence around `X` (the clause-delete idiom).
            // Two directly-adjacent splices (`,@a ,@b`) have nothing to divide the run on and are
            // rejected.
            let mut prev_splice = false;
            for c in items {
                let is_splice = as_splice(c).is_some();
                if is_splice && prev_splice {
                    return Err(PatternError(
                        "two `,@` splices cannot be adjacent (nothing anchors the run boundary); \
                         separate them with a fixed element, e.g. `(f ,@a X ,@b)`"
                            .into(),
                    ));
                }
                prev_splice = is_splice;
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
        let name = items.first().and_then(|t| t.as_name()).ok_or_else(|| {
            PatternError("a guarded metavariable needs a name: `,(name guard…)`".into())
        })?;
        let guards = items[1..]
            .iter()
            .map(compile_guard)
            .collect::<Result<_, _>>()?;
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
            "is-irrefutable" => Ok(Guard::IsIrrefutable),
            "is-camel-case" => Ok(Guard::IsCamelCase),
            // An unknown bare-name guard — the guard vocabulary is a small CLOSED set, so LIST it (like the
            // string-escape message lists the escape set). A near-typo (`is-litera` for `is-literal`) is
            // then obvious from the list, and the message doubles as documentation of what a guard can be.
            other => Err(PatternError(format!(
                "unknown guard `{other}` — a guard is one of `is-literal` `is-name` `is-int` `is-float` \
                 `is-str` `is-bool` `is-atom` `is-list` `is-irrefutable` `is-camel-case`, or a form \
                 `(head-is NAME)` / `(matches PAT)` / `(not GUARD)` / `(deeper-than N)` / \
                 `(calls-deeper-than N)`"
            ))),
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
                let inner = items.get(1).ok_or_else(|| {
                    PatternError("`not` needs a guard: `(not is-literal)`".into())
                })?;
                Ok(Guard::Not(Box::new(compile_guard(inner)?)))
            }
            Some("deeper-than") => {
                // `(deeper-than N)` — N is a non-negative integer-literal threshold.
                let n = items
                    .get(1)
                    .and_then(|t| match t {
                        Tree::Atom(Leaf::Int { value, .. }, _) => {
                            value.to_u128().and_then(|u| usize::try_from(u).ok())
                        }
                        _ => None,
                    })
                    .ok_or_else(|| {
                        PatternError(
                            "`deeper-than` needs a non-negative integer: `(deeper-than 5)`".into(),
                        )
                    })?;
                Ok(Guard::DeeperThan(n))
            }
            Some("calls-deeper-than") => {
                // `(calls-deeper-than N)` — N is a non-negative integer-literal threshold.
                let n = items
                    .get(1)
                    .and_then(|t| match t {
                        Tree::Atom(Leaf::Int { value, .. }, _) => value.to_u128().and_then(|u| usize::try_from(u).ok()),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        PatternError(
                            "`calls-deeper-than` needs a non-negative integer: `(calls-deeper-than 4)`"
                                .into(),
                        )
                    })?;
                Ok(Guard::CallsDeeperThan(n))
            }
            _ => Err(PatternError(format!("ill-formed guard `{}`", t.to_sexpr()))),
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
        // An irrefutable match-pattern shape (var/`_`/tuple/record, recursively; never a literal or a
        // sum-ctor pattern). Purely structural — the no-context form treats every ctor as refutable.
        Guard::IsIrrefutable => crate::match_to_let::is_irrefutable(s),
        // A `camelCase` name atom (name-shape only) — lowercase-initial with an interior uppercase and
        // no `_`. Not a `Ctor`/type (leading uppercase), not `snake_case`, not all-caps.
        Guard::IsCamelCase => matches!(s, Tree::Atom(Leaf::Name(n), _) if is_camel_case(n)),
        // The node's list-nesting depth exceeds the threshold (STRICTLY). Structural, print-independent.
        Guard::DeeperThan(n) => list_depth(s) > *n,
        // The node's CALL-CHAIN depth exceeds the threshold (STRICTLY) — nested application forms only,
        // skipping structural/keyword/operator/compound-ctor heads. Print-independent.
        Guard::CallsDeeperThan(n) => call_depth(s) > *n,
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

/// The LIST-nesting depth of `t` — the longest chain of nested list nodes rooted here. An atom is 0; a
/// flat list `(f a b)` is 1; `(f (g (h x)))` is 3 (each nested list adds one). The metric the
/// `(deeper-than N)` guard / `idiomatic/deep-nesting` lint uses: a purely structural measure of
/// call/argument nesting, independent of how the form is printed. Iterative over an explicit stack (a
/// deeply-nested subject must not overflow the native stack — the same discipline `codec`/`canon` use).
fn list_depth(t: &Tree) -> usize {
    let mut max = 0;
    // Stack of (node, depth-of-this-node). A node's depth is 1 + its parent list's depth (0 at an atom).
    let mut stack = vec![(t, 0usize)];
    while let Some((node, d)) = stack.pop() {
        if let Tree::List(items, _) = node {
            let child_depth = d + 1;
            if child_depth > max {
                max = child_depth;
            }
            for child in items {
                stack.push((child, child_depth));
            }
        }
    }
    max
}

/// The CALL-CHAIN depth of `t` — the longest chain of nested APPLICATION forms rooted here. Distinct
/// from [`list_depth`], which counts EVERY list level (so ordinary structural nesting — a `module`
/// holding `def`s holding `let`s — dominates the count regardless of any call nesting, the exact reason
/// whole-subtree depth over-fires as a `deep-nesting` metric). Here only an APPLICATION form (see
/// [`is_application_form`]) adds one to the depth; a structural/keyword/operator/compound-ctor list adds
/// zero but is still descended into (a call nested inside a `let` body still counts). So
/// `(f (g (h x)))` is 3, `(collect (map (filter xs p) f) init)` is 3 (collect→map→filter), and a
/// `(module m (def (main) (+ x 1)))` spine adds 0 for `module`/`def`/`+`.
///
/// One bounded imprecision: a def SIGNATURE `(main)` and a let BINDER `(x e)` are plain-name-headed
/// lists, shape-indistinguishable from a nullary/unary application, so each reads as a depth-1
/// application. This never COMPOUNDS into a deep chain (a signature/binder is always shallow — its
/// depth is 1 plus whatever the bound EXPRESSION nests, which is counted on its own merits), so it
/// shifts the metric by at most a constant and is immaterial above the threshold the `deep-nesting`
/// lint uses. Distinguishing them would need parent-position context the free-standing metric does not
/// carry; the empirical threshold study (compiler-ml source) confirms the signal is clean regardless.
///
/// Iterative over an explicit stack (a deep subject must not overflow the native stack — the same
/// discipline `list_depth`/`codec`/`canon` use).
fn call_depth(t: &Tree) -> usize {
    let mut max = 0;
    // Stack of (node, call-depth-accumulated-to-this-node's-parent). A node contributes +1 only when it
    // is itself an application form; every child inherits the (possibly incremented) depth.
    let mut stack = vec![(t, 0usize)];
    while let Some((node, d)) = stack.pop() {
        if let Tree::List(items, _) = node {
            let here = if is_application_form(node) { d + 1 } else { d };
            if here > max {
                max = here;
            }
            for child in items {
                stack.push((child, here));
            }
        }
    }
    max
}

/// Is `t` an APPLICATION form — a call `(callee arg…)` whose head is a plain callee name, as opposed to
/// a structural/keyword form, an infix-operator form, or a compound-value constructor? An application is
/// the thing `idiomatic/deep-nesting` counts: user function/constructor calls nested as arguments. A
/// head is NOT an application head when it is:
///  - not a bare name (a list head, e.g. `((f) x)` — rare; treated as non-application for depth),
///  - a KEYWORD or reserved word (`let`/`if`/`match`/`fn`/`def`/`module`/… ∪ `and`/`or`) — structural,
///  - an INFIX-operator head (`+`/`-`/`=`/`<`/… via [`token::infix_prec`]) — an operator form, and note
///    equality's arena head is `=` (shared with a Phase-B record field `(= name value)`, also excluded),
///  - a COMPOUND-VALUE constructor (`tuple`/`list`/`record`/`map`) — a data literal, not a call,
///  - a structural non-keyword head the sexpr layer uses (`do`/`@`/`pragma`/`.`/`:` handled via the
///    operator/keyword checks above where applicable; `do`/`@`/`pragma`/`.` are listed explicitly).
///
/// Everything else — an ordinary lowercase callee or an uppercase constructor applied to args — IS an
/// application. Purely syntactic (head-name shape only), no scope/type lookup.
fn is_application_form(t: &Tree) -> bool {
    let Some(head) = head_name(t) else {
        return false;
    };
    // Keyword or word-operator (`and`/`or`) — a structural/operator form, never a call.
    if crate::token::is_reserved(head) {
        return false;
    }
    // An infix-operator head (arena head, e.g. `=`/`+`/`<`/`->`/`|>`) — an operator form, not a call.
    if crate::token::infix_prec(head).is_some() {
        return false;
    }
    // A compound-value constructor or a structural sexpr head is a data literal / grouping, not a call;
    // everything else is an application.
    !matches!(
        head,
        "tuple" | "list" | "record" | "map" | "do" | "@" | "pragma" | "."
    )
}

/// Is `name` a `camelCase` identifier — the shape the `naming/camel-case` lint flags (Cadenza convention
/// is `snake_case`)? True iff: it starts with a lowercase letter, contains NO `_`, and has at least one
/// interior uppercase letter (`fooBar`, `myFunc`). This deliberately EXCLUDES: `snake_case` (has `_`), a
/// `Ctor`/type/`Module` name (leading uppercase — a different, correct convention), an all-lowercase name
/// (`foo`), and an all-caps `CONST` (leading uppercase). A qualified segment (`A.b`) is a single name
/// atom here only when unqualified; a dotted path is a list, not a name, so it never reaches this.
/// Purely syntactic Unicode-case classification — no scope or type lookup.
fn is_camel_case(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false; // empty is not a name shape we flag
    };
    if !first.is_lowercase() {
        return false; // must start lowercase (excludes Ctor/type/Module and CONST)
    }
    if name.contains('_') {
        return false; // snake_case (or a synth `__name`) is the convention, not flagged
    }
    // At least one interior uppercase letter distinguishes `fooBar` from plain `foo`.
    chars.any(|c| c.is_uppercase())
}

/// Match a compiled pattern child-sequence against a subject child-sequence, honoring zero-or-more
/// splices (splices are never adjacent — the compiler guarantees a fixed anchor between any two).
///
/// The no-splice case is a fast positional zip. With splices the sequence is a run of fixed
/// patterns interleaved with splices; each splice absorbs a variable-length run of subject nodes.
/// A single splice is matched greedily around fixed prefix/suffix anchors; two-or-more splices need
/// backtracking to place each variable-length run, done by [`match_splice_seq`].
fn match_seq(pitems: &[Pat], sitems: &[Tree], binds: &mut Bindings) -> bool {
    let splices = pitems
        .iter()
        .filter(|c| matches!(c, Pat::Splice { .. }))
        .count();
    match splices {
        0 => {
            pitems.len() == sitems.len()
                && pitems
                    .iter()
                    .zip(sitems)
                    .all(|(p, s)| match_pat(p, s, binds))
        }
        1 => {
            let k = pitems
                .iter()
                .position(|c| matches!(c, Pat::Splice { .. }))
                .expect("one splice present");
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
        _ => match_splice_seq(pitems, sitems, binds),
    }
}

/// Match a pattern sequence containing TWO OR MORE splices against a subject sequence, by recursive
/// backtracking. Non-adjacency (compiler-guaranteed) means each splice is bounded by fixed patterns;
/// the first splice's run length is the only free choice at each step, so we try every feasible
/// length, commit the bindings on a snapshot, and recurse on the remainders — restoring the snapshot
/// on a failed branch. Exponential in the worst case (many splices) but a pattern has a handful.
fn match_splice_seq(pitems: &[Pat], sitems: &[Tree], binds: &mut Bindings) -> bool {
    // Consume any leading fixed (non-splice) patterns positionally — they anchor the front.
    let first_splice = pitems.iter().position(|c| matches!(c, Pat::Splice { .. }));
    let Some(k) = first_splice else {
        // No splice left: a plain positional match of the remaining fixed patterns.
        return pitems.len() == sitems.len()
            && pitems
                .iter()
                .zip(sitems)
                .all(|(p, s)| match_pat(p, s, binds));
    };
    if sitems.len() < k {
        return false;
    }
    for (p, s) in pitems[..k].iter().zip(&sitems[..k]) {
        if !match_pat(p, s, binds) {
            return false;
        }
    }
    let Pat::Splice { name } = &pitems[k] else {
        unreachable!("k points at a splice")
    };
    // The minimum subject length the pattern TAIL (after this splice) needs, so the run can't eat
    // nodes the tail requires. A trailing splice needs 0; each further fixed pattern needs 1.
    let tail = &pitems[k + 1..];
    let tail_min = tail
        .iter()
        .filter(|c| !matches!(c, Pat::Splice { .. }))
        .count();
    let rest = &sitems[k..];
    if rest.len() < tail_min {
        return false;
    }
    // Try every feasible run length for this splice (greedy or not — all lengths are explored so a
    // later anchor can force a shorter/longer run). Snapshot/restore bindings across attempts so a
    // failed branch leaves no partial binding behind.
    let max_run = rest.len() - tail_min;
    for run_len in 0..=max_run {
        let snapshot = binds.clone();
        let run = &rest[..run_len];
        if bind_run(binds, name, run) && match_splice_seq(tail, &rest[run_len..], binds) {
            return true;
        }
        *binds = snapshot;
    }
    false
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

    /// The template metavariables this rule references that its PATTERN does not bind, in sorted order.
    /// Such a metavar can never be filled, so every site "fails to instantiate" (0 rewrites) with no
    /// hint — a static mistake (a typo'd or stray template metavar) the caller should surface up front
    /// rather than leaving the author to wonder why nothing rewrote. Empty when the template is
    /// well-formed against its pattern.
    pub fn unbound_template_metavars(&self) -> Vec<String> {
        let bound = self.pattern.bound_metavars();
        self.template
            .referenced_metavars()
            .into_iter()
            .filter(|m| !bound.contains(m))
            .collect()
    }

    /// Compile a rule from a `(rule PATTERN TEMPLATE)` s-expression form.
    pub fn compile_form(t: &Tree) -> Result<Rule, PatternError> {
        match t {
            Tree::List(items, _) if items.first().and_then(|h| h.as_name()) == Some("rule") => {
                match items.as_slice() {
                    [_, p, tmpl] => Ok(Rule {
                        pattern: Pattern {
                            pat: compile_pat(p)?,
                        },
                        template: Template { tree: tmpl.clone() },
                    }),
                    _ => Err(PatternError("a rule is `(rule PATTERN TEMPLATE)`".into())),
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
        let arena =
            sexpr::read_all(src).map_err(|e| PatternError(format!("rules parse: {}", e.0)))?;
        // read_all wraps the forms in a synthetic `(do form…)`; the rules are its tail.
        let tree = Tree::of(&arena);
        let forms = match &tree {
            Tree::List(items, _) if items.first().and_then(|h| h.as_name()) == Some("do") => {
                &items[1..]
            }
            _ => std::slice::from_ref(&tree),
        };
        let rules = forms
            .iter()
            .map(Rule::compile_form)
            .collect::<Result<_, _>>()?;
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

fn rewrite_node(rules: &RuleSet, root: &Tree, strategy: Strategy, count: &mut usize) -> Tree {
    // EXPLICIT stack, not native recursion: `rewrite_node` descends the ENTIRE subject, which can be a
    // decoded arbitrarily-deep arena (no cap, unlike the reader's `MAX_NESTING_DEPTH`), so a recursive
    // rewrite overflowed the native stack on a deep subject via `cdz rewrite`. A `Job{Visit|EmitList}`
    // work-stack + a `results` stack rebuilds the tree; children are pushed reversed so their rewritten
    // forms land on `results` in source order for the parent's re-assembly. The observable semantics are
    // preserved exactly: BottomUp fires at the REBUILT node (its replacement is taken as-is, not
    // re-descended); TopDown fires FIRST and, if it fires, keeps the replacement whole (never descends).
    enum Job<'t> {
        Visit(&'t Tree),
        // Re-assemble a `List` (origin `o`) once its `n` rewritten children sit atop `results`.
        EmitList(Option<StructId>, usize),
    }
    let mut jobs: Vec<Job> = vec![Job::Visit(root)];
    let mut results: Vec<Tree> = Vec::new();
    while let Some(job) = jobs.pop() {
        match job {
            Job::Visit(node) => match strategy {
                Strategy::BottomUp => match node {
                    // An atom has no children — its "rewritten form" is itself; fire on it directly.
                    Tree::Atom(l, o) => {
                        let atom = Tree::Atom(l.clone(), *o);
                        results.push(fire_counting(rules, atom, count));
                    }
                    Tree::List(items, o) => {
                        jobs.push(Job::EmitList(*o, items.len()));
                        for c in items.iter().rev() {
                            jobs.push(Job::Visit(c));
                        }
                    }
                },
                Strategy::TopDown => {
                    // Fire at this node FIRST; if it fires, keep the replacement as-is (don't descend).
                    if let Some(new_tree) = rules.fire(node) {
                        *count += 1;
                        results.push(new_tree);
                    } else {
                        match node {
                            Tree::Atom(l, o) => results.push(Tree::Atom(l.clone(), *o)),
                            Tree::List(items, o) => {
                                // No fire on re-assembly for TopDown — just rebuild from rewritten kids.
                                jobs.push(Job::EmitList(*o, items.len()));
                                for c in items.iter().rev() {
                                    jobs.push(Job::Visit(c));
                                }
                            }
                        }
                    }
                }
            },
            Job::EmitList(o, n) => {
                let kids = results.split_off(results.len() - n);
                let rebuilt = Tree::List(kids, o);
                match strategy {
                    // BottomUp: fire at the fully-rewritten node; replacement taken as-is.
                    Strategy::BottomUp => results.push(fire_counting(rules, rebuilt, count)),
                    // TopDown already fired (or didn't) on the way DOWN; the rebuilt node is final.
                    Strategy::TopDown => results.push(rebuilt),
                }
            }
        }
    }
    results.pop().expect("rewrite_node leaves the root")
}

/// Fire the rule set at `node`; if a rule matches, count it and return the replacement, else return
/// `node` unchanged. Factored out so both the atom and the re-assembled-list BottomUp paths share it.
fn fire_counting(rules: &RuleSet, node: Tree, count: &mut usize) -> Tree {
    if let Some(new_tree) = rules.fire(&node) {
        *count += 1;
        new_tree
    } else {
        node
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
                let text =
                    std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
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
                let text =
                    std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                // Mirror the ML parser's root convention: a SINGLE top-level form stays bare, so it
                // round-trips through the ML printer (which renders a root single-element `(do X)` as
                // bare `X`). Only multiple forms wrap in `(do …)`. `read` succeeds iff there's exactly
                // one form (it errors on trailing input); fall back to `read_all` for several.
                //
                // The SPANNED readers record a source span per node, so a Sexpr target carries a
                // span table just like an ML one — this is what enables the formatting-preserving
                // (span-splicing) rewrite over a hand-formatted `.sexp` corpus (`apply_rewrite_text`).
                let (arena, spans) = match sexpr::read_spanned(text) {
                    Ok(pair) => pair,
                    // Render the position as `line:col`, not a raw `at byte N` — the same mapping the JSON/
                    // TOML arms above (and the `convert`/`check`/`load` paths) apply, so `query`/`rewrite`/
                    // `clones`/`diff` over a malformed MULTI-LINE `.sexp` point at a navigable place. This
                    // arm was the last s-expr reader-error render still leaking the raw byte offset (`cdz
                    // clones F` → `s-expr parse: unexpected ')' at byte 18`, where `check F` already said
                    // `at 2:8`); route it through `locate_byte_in_message` like every other surface.
                    Err(_) => sexpr::read_all_spanned(text).map_err(|e| {
                        format!(
                            "s-expr parse: {}",
                            crate::convert::locate_byte_in_message(&e.0, text)
                        )
                    })?,
                };
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: Some(spans),
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
            Format::Markdown => {
                // A markdown document is a queryable/rewritable tree like any surface (its `(document
                // …)` nodes and the embedded `cdz` program subtrees are all matchable). CommonMark
                // parsing is total, so there are no errors to report.
                let text =
                    std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                let (arena, spans) = crate::markdown::read_spanned(text);
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: Some(spans),
                    },
                    Vec::new(),
                ))
            }
            Format::Json => {
                // A JSON document is a queryable/rewritable tree like any surface (its `(json-object
                // …)`/`(json-array …)`/scalar nodes are all matchable). Unlike CommonMark, JSON can
                // fail — a malformed document is surfaced as an error.
                let text =
                    std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                let (arena, spans) = crate::json::read_spanned(text).map_err(|e| {
                    // Render the position as `line:col`, not a raw `at byte N` — the same mapping the
                    // convert path applies, so `query`/`rewrite` over a malformed `.json` points at a
                    // navigable place.
                    format!(
                        "JSON parse: {}",
                        crate::convert::locate_byte_in_message(&e.0, text)
                    )
                })?;
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: Some(spans),
                    },
                    Vec::new(),
                ))
            }
            Format::Toml => {
                // A TOML document is a queryable/rewritable tree like any surface (its `(toml-document
                // …)` decor-in-arena nodes are all matchable). Fallible, mapped to `line:col`.
                let text =
                    std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                let (arena, spans) = crate::toml_surface::read_spanned(text).map_err(|e| {
                    format!(
                        "TOML parse: {}",
                        crate::convert::locate_byte_in_message(&e.0, text)
                    )
                })?;
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: Some(spans),
                    },
                    Vec::new(),
                ))
            }
            #[cfg(feature = "cedar")]
            Format::Cedar => {
                // A Cedar policy is a queryable/rewritable tree like any surface (its `(cedar-policyset
                // …)` nodes — effects, scope constraints, `when`/`unless` expressions — are all
                // matchable). This is the point: an agent restructures a policy with the same tools.
                let text =
                    std::str::from_utf8(input).map_err(|e| format!("input not UTF-8: {e}"))?;
                let (arena, spans) = crate::cedar::read_spanned(text)
                    .map_err(|e| format!("Cedar parse: {}", e.0))?;
                Ok((
                    Target {
                        tree: Tree::of(&arena),
                        spans: Some(spans),
                    },
                    Vec::new(),
                ))
            }
            // Lean build (no `cedar` feature): the Cedar surface isn't compiled — a clean error, not a panic.
            #[cfg(not(feature = "cedar"))]
            Format::Cedar => Err(
                "the `cedar` surface is not compiled in this build (enable the `cedar` feature)"
                    .to_string(),
            ),
            Format::Debug | Format::Flat => Err(format!(
                "`{}` is an output-only format, not an input",
                from.name()
            )),
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
    //
    // The re-parse + structural-equality gate below is exactly this: a structural edit either yields a
    // program that re-parses cleanly (well-formed), or returns an `Err` naming why (a machine-readable
    // rejection) — it never emits a malformed result silently.
    //
    //= spec/capabilities/agent-authoring.md#structural-edits-preserve-well-formedness-or-report
    //# A structural edit MUST either yield a well-formed program or report a machine-readable rejection.
    //
    //= spec/capabilities/agent-authoring.md#structural-edits-preserve-well-formedness-or-report
    //# A structural edit MUST NOT yield a program that is malformed without reporting why.
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

    /// Apply a `rules` set to `target` under `strategy` (optionally to a fixed point) as a
    /// FORMATTING-PRESERVING rewrite: instead of reprinting the whole tree (which reflows a
    /// hand-formatted file), splice each changed subtree into the ORIGINAL source `src` at its span,
    /// leaving every unmatched byte verbatim. Needs `target.spans` (both surfaces now carry a span
    /// table). The result is validated as a transaction just like [`apply_rewrite`] — re-parsed and
    /// checked structurally-equal to the rewritten tree — so a splice that produced ill-formed text
    /// is REJECTED, never written.
    ///
    /// `surface` is the source's surface (ML or s-expr), used both to re-parse for validation and to
    /// render the replacement text of changed/inserted nodes so a splice reads like its neighbours.
    //
    // Splicing only the changed subtrees at their spans — leaving every unmatched byte verbatim — is
    // how a structural edit operates WITHOUT re-parsing code unrelated to its target: untouched
    // regions are copied through as source bytes, never re-read or re-printed.
    //
    //= spec/capabilities/agent-authoring.md#a-structural-interface-exists
    //# A structural query or edit MUST operate without re-parsing code unrelated to its target.
    pub fn apply_rewrite_preserving(
        rules: &RuleSet,
        strategy: Strategy,
        target: &Target,
        src: &str,
        surface: Format,
        fixpoint: bool,
    ) -> Result<RewriteOutcome, String> {
        let spans = target.spans.as_ref().ok_or(
            "formatting-preserving rewrite needs source spans (unavailable for this input)",
        )?;
        let r = if fixpoint {
            rewrite_rules_fixpoint(rules, &target.tree, strategy, 64)
        } else {
            rewrite_rules(rules, &target.tree, strategy)
        };

        // Map an ORIGINAL node (drawn from `target.tree`, so it carries provenance) to its span.
        let span_of = |t: &Tree| -> Option<(usize, usize)> {
            t.origin()
                .and_then(|id| spans.get(id))
                .map(|s| (s.start, s.end))
        };
        let edited = textedit::rewrite_preserving(src, &target.tree, &r.tree, &span_of, surface);

        // Validated transaction: the spliced text must re-parse to the SAME tree the structural
        // rewrite produced (so the splice didn't corrupt or drift from the intended edit).
        let want = r.tree.to_arena();
        let reparsed = reparse(&edited.output, surface)?;
        if !reparsed.structurally_eq(&want) {
            return Err(
                "formatting-preserving rewrite rejected: edited text does not re-parse to the \
                 rewritten tree (falling back to a whole-file reprint would be needed)"
                    .to_string(),
            );
        }

        Ok(RewriteOutcome {
            output: edited.output,
            count: r.count,
        })
    }

    /// Re-parse `text` in `surface` to an arena for the validated-transaction check.
    fn reparse(text: &str, surface: Format) -> Result<Arenas, String> {
        match surface {
            Format::Ml => {
                let parsed = parser::read_ml(text);
                if !parsed.ok() {
                    return Err(format!(
                        "result does not re-parse cleanly ({} error(s)); first: {}",
                        parsed.errors.len(),
                        parsed
                            .errors
                            .first()
                            .map(|e| e.message.as_str())
                            .unwrap_or("?")
                    ));
                }
                Ok(parsed.arenas)
            }
            Format::Sexpr => match sexpr::read(text) {
                Ok(a) => Ok(a),
                Err(_) => sexpr::read_all(text).map_err(|e| format!("s-expr re-parse: {}", e.0)),
            },
            other => Err(format!(
                "formatting-preserving rewrite unsupported for `{}` input",
                other.name()
            )),
        }
    }

    /// Render an arena in `to` format (text formats only for the query path).
    pub fn project(arena: &Arenas, to: Format, width: usize) -> Result<String, String> {
        match to {
            Format::Ml => Ok(printer::print(arena, width)),
            Format::Sexpr => Ok(sexpr::print(arena)),
            Format::Markdown => Ok(crate::markdown::print(arena, width)),
            Format::Json => Ok(crate::json::print(arena, width, crate::printer::print)),
            Format::Toml => Ok(crate::toml_surface::print(
                arena,
                width,
                crate::printer::print,
            )),
            #[cfg(feature = "cedar")]
            Format::Cedar => Ok(crate::cedar::print(arena, width, crate::printer::print)),
            // Lean build (no `cedar` feature): the Cedar surface isn't compiled — a clean error, not a panic.
            #[cfg(not(feature = "cedar"))]
            Format::Cedar => Err(
                "the `cedar` surface is not compiled in this build (enable the `cedar` feature)"
                    .to_string(),
            ),
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
    pub fn matches_json(
        pattern: &Pattern,
        query: &Query,
        target: &Target,
        file: Option<&str>,
    ) -> String {
        let matches = search_with(pattern, query, &target.tree, target.spans.as_ref());
        let mut arr = json::Array::new();
        for m in &matches {
            let mut obj = json::Object::new();
            if let Some(f) = file {
                obj.string("file", f);
            }
            match m.span {
                Some(s) => obj.raw(
                    "span",
                    &format!("{{\"start\":{},\"end\":{}}}", s.start, s.end),
                ),
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
                treediff::ChangeKind::Replace { old, new } => {
                    format!("{p}: replace {old} => {new}")
                }
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

    /// A precomputed line-start index over one source — the byte offset of each line's first char, in
    /// ascending order (`starts[0] == 0`). Built ONCE per source in O(len); each `line_col` lookup is then
    /// a binary search (O(log lines)) plus a bounded byte-count of the found line's prefix, instead of
    /// [`line_col`]'s O(byte) scan from the start. This is what keeps a report that resolves MANY sites in
    /// one source LINEAR-ish rather than O(sites × source_len): `cdz clones` on a big program spent ~86%
    /// of its time in the per-site `line_col` newline re-scan (each site's offset averages O(len/2), so N
    /// sites over an O(N) source was O(N²)). The column stays BYTE-counted from the line start (identical
    /// to `line_col`), so the reported `(line, col)` is unchanged.
    pub struct LineIndex {
        /// Byte offset of the start of each line (line 1 is `starts[0] = 0`). Strictly ascending.
        starts: Vec<usize>,
        len: usize,
        /// Whether the source is entirely ASCII. When true, char count == byte count, so a column is
        /// `byte - line_start + 1` in O(1) — no per-call char scan of the line prefix.
        ascii: bool,
    }

    impl LineIndex {
        /// Build the index for `src` — one linear pass recording the byte AFTER each `\n`.
        pub fn new(src: &str) -> LineIndex {
            let mut starts = vec![0usize];
            for (i, b) in src.bytes().enumerate() {
                if b == b'\n' {
                    starts.push(i + 1);
                }
            }
            LineIndex {
                starts,
                len: src.len(),
                ascii: src.is_ascii(),
            }
        }

        /// The 1-based `(line, column)` of `byte` in `src` — byte-identical to [`line_col`], via a binary
        /// search over the line starts (O(log lines)) plus the column. `src` MUST be the source this index
        /// was built from. A byte past the end clamps (like `line_col`).
        pub fn line_col(&self, src: &str, byte: usize) -> (usize, usize) {
            let byte = byte.min(self.len);
            // The line is the last start `<= byte`. `partition_point` gives the count of starts `<= byte`;
            // since `starts[0] == 0 <= byte` always, that count is `>= 1`, and the 1-based line IS the count.
            let line = self.starts.partition_point(|&s| s <= byte);
            let line_start = self.starts[line - 1];
            // Column = CHARS from the line start to `byte`, + 1. For an ALL-ASCII source (the common case:
            // Cadenza source is ASCII) char count == byte count, so this is O(1) — critical because a
            // program on ONE long line (a corpus `(input …)` form, minified/generated code) makes the byte
            // span `[line_start, byte)` grow to O(source_len), so the char-count fallback below was O(N) per
            // call → O(N²) over N sites (a wide single-line `cdz exports`/`highlight`/`uses` = ~62% self in
            // `do_count_chars`). A multibyte source keeps the exact char count (bounded by the line length).
            let col = if self.ascii {
                byte - line_start + 1
            } else {
                src[line_start..byte].chars().count() + 1
            };
            (line, col)
        }
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
        lint_report_with_levels(lints, target, src, label, &lint::LintLevels::default())
    }

    /// [`lint_report`] with per-lint level overrides applied (allow suppresses, deny → error, warn →
    /// warning). The level-free [`lint_report`] delegates here with an empty [`lint::LintLevels`].
    pub fn lint_report_with_levels(
        lints: &lint::LintSet,
        target: &Target,
        src: &str,
        label: &str,
        levels: &lint::LintLevels,
    ) -> (String, bool) {
        let diags = lint::run_with_levels(lints, &target.tree, target.spans.as_ref(), levels);
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
        lint_json_with_levels(lints, target, src, file, &lint::LintLevels::default())
    }

    /// [`lint_json`] with per-lint level overrides applied. The level-free [`lint_json`] delegates
    /// here with an empty [`lint::LintLevels`].
    pub fn lint_json_with_levels(
        lints: &lint::LintSet,
        target: &Target,
        src: &str,
        file: Option<&str>,
        levels: &lint::LintLevels,
    ) -> (String, bool) {
        let diags = lint::run_with_levels(lints, &target.tree, target.spans.as_ref(), levels);
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

    /// Build the `Verified`-fix rewrite set for `lints` under `levels`: each NAMED lint that (a) is
    /// not suppressed (`Allow`) and (b) carries a fix of an eligible applicability contributes a
    /// `pattern → fix-template` rewrite rule, in catalog order (so a fix pass is deterministic).
    /// `include_heuristic` opts `Heuristic` fixes in; by default only `Verified` fixes apply
    /// (DESIGN-cadenza-lint §2 — a Heuristic fix is offered, not auto-applied).
    fn fix_ruleset(
        lints: &lint::LintSet,
        levels: &lint::LintLevels,
        include_heuristic: bool,
    ) -> RuleSet {
        let mut rules = Vec::new();
        for rule in &lints.rules {
            // A lint suppressed to `Allow` fires no diagnostic, so it applies no fix either.
            if rule.name.as_deref().and_then(|n| levels.effective(n))
                == Some(lint::LintLevel::Allow)
            {
                continue;
            }
            if let Some((template, app)) = &rule.fix {
                let eligible = matches!(app, lint::Applicability::Verified) || include_heuristic;
                if eligible {
                    rules.push(Rule::new(rule.pattern.clone(), template.clone()));
                }
            }
        }
        RuleSet::new(rules)
    }

    /// Apply the `Verified` (and, when `include_heuristic`, `Heuristic`) fixes of `lints` to `target`
    /// as a validated, formatting-preserving codemod: reuse [`apply_rewrite_preserving`] (splice only
    /// changed subtrees at their spans, leaving layout/comments verbatim), falling back to a whole-tree
    /// reprint when a span-splice can't be validated. `levels` gates which named lints fire (a lint set
    /// to `Allow` — by `--allow` or an `@allow` attribute — applies no fix). Fixes run to a fixed point,
    /// so a fix that exposes a nested idiom collapses in the same pass. Returns the fixed program text
    /// plus the number of sites rewritten; when nothing is fixable the source is returned with count 0.
    ///
    /// The `Verified` bar means every applied fix is semantically equivalent (licensed by the
    /// apply-and-recheck witness gate, DESIGN-cadenza-lint §6); the validated transaction here rejects
    /// any fix whose result does not re-parse to the intended tree, so a broken template never writes.
    pub fn lint_fix_with_levels(
        lints: &lint::LintSet,
        target: &Target,
        src: &str,
        surface: Format,
        levels: &lint::LintLevels,
        include_heuristic: bool,
        width: usize,
    ) -> Result<RewriteOutcome, String> {
        let rules = fix_ruleset(lints, levels, include_heuristic);
        if rules.rules.is_empty() {
            return Ok(RewriteOutcome {
                output: src.to_string(),
                count: 0,
            });
        }
        // Prefer the formatting-preserving splice (the DEFAULT for a codemod over hand-formatted
        // source): it needs spans and errors without them, so a spanless target (or a splice that
        // can't be validated) falls straight through to the whole-tree reprint below.
        if let Ok(o) =
            apply_rewrite_preserving(&rules, Strategy::BottomUp, target, src, surface, true)
        {
            return Ok(o);
        }
        apply_rewrite(&rules, Strategy::BottomUp, target, surface, width, true)
    }

    /// A per-source-label cache of [`LineIndex`], so resolving MANY clone sites in one source builds that
    /// source's index ONCE (O(len)) and binary-searches per site — turning the report's O(sites × len)
    /// newline re-scan into O(len + sites·log). Keyed by the file label a `CloneSite` carries.
    fn build_line_indices(
        sources: &std::collections::HashMap<String, String>,
    ) -> std::collections::HashMap<&str, LineIndex> {
        sources
            .iter()
            .map(|(label, src)| (label.as_str(), LineIndex::new(src)))
            .collect()
    }

    /// Format a clone site's location `LABEL:line:col` (or `LABEL:?:?` when no span). `sources` maps a
    /// file label to its source text; `indices` the matching prebuilt [`LineIndex`] per label (so the
    /// line:col lookup is a binary search, not a from-start scan — the O(N²)→O(N log N) fix for a report
    /// with many sites).
    fn site_loc(
        site: &clones::CloneSite,
        sources: &std::collections::HashMap<String, String>,
        indices: &std::collections::HashMap<&str, LineIndex>,
    ) -> String {
        let label = site.file.as_deref().unwrap_or("(stdin)");
        match site.span {
            Some(s) => match (sources.get(label), indices.get(label)) {
                (Some(src), Some(idx)) => {
                    let (l, c) = idx.line_col(src, s.start);
                    format!("{label}:{l}:{c}")
                }
                _ => format!("{label}:byte {}", s.start),
            },
            None => format!("{label}:?:?"),
        }
    }

    /// Render clone classes as human-readable text: a header per class (occurrence count + node size
    /// + exemplar) then one indented `LABEL:line:col` per site. `sources` maps file label → text.
    pub fn clones_report(
        classes: &[clones::CloneClass],
        sources: &std::collections::HashMap<String, String>,
    ) -> String {
        let indices = build_line_indices(sources);
        let mut out = String::new();
        for (i, cls) in classes.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!(
                "clone: {} occurrences, {} nodes: {}\n",
                cls.sites.len(),
                cls.size,
                cls.exemplar
            ));
            for site in &cls.sites {
                out.push_str(&format!("  {}\n", site_loc(site, sources, &indices)));
            }
        }
        out
    }

    /// Render clone classes as JSON: `[{exemplar, size, sites:[{file?, line?, col?}]}]`.
    pub fn clones_json(
        classes: &[clones::CloneClass],
        sources: &std::collections::HashMap<String, String>,
    ) -> String {
        let indices = build_line_indices(sources);
        let mut arr = json::Array::new();
        for cls in classes {
            let mut obj = json::Object::new();
            obj.string("exemplar", &cls.exemplar);
            obj.raw("size", &cls.size.to_string());
            let mut sites = json::Array::new();
            for site in &cls.sites {
                sites.raw(&site_json(site, sources, &indices));
            }
            obj.raw("sites", &sites.finish());
            arr.raw(&obj.finish());
        }
        arr.finish()
    }

    /// A JSON site object `{file?, line, col}` for a clone/near-clone site. `indices` supplies the
    /// prebuilt [`LineIndex`] per source label (binary-searched line:col, the O(N²)→O(N log N) fix).
    fn site_json(
        site: &clones::CloneSite,
        sources: &std::collections::HashMap<String, String>,
        indices: &std::collections::HashMap<&str, LineIndex>,
    ) -> String {
        let mut so = json::Object::new();
        if let Some(f) = &site.file {
            so.string("file", f);
        }
        let lc = site.span.and_then(|s| {
            let label = site.file.as_deref().unwrap_or("(stdin)");
            match (sources.get(label), indices.get(label)) {
                (Some(src), Some(idx)) => Some(idx.line_col(src, s.start)),
                _ => None,
            }
        });
        match lc {
            Some((l, c)) => {
                so.raw("line", &l.to_string());
                so.raw("col", &c.to_string());
            }
            None => {
                so.raw("line", "null");
                so.raw("col", "null");
            }
        }
        so.finish()
    }

    /// Render near-clone classes as human text: a header per class (occurrences + hole count + the
    /// inferred `,mK` pattern) then one indented `LABEL:line:col` per site.
    pub fn near_clones_report(
        classes: &[clones::NearCloneClass],
        sources: &std::collections::HashMap<String, String>,
    ) -> String {
        let indices = build_line_indices(sources);
        let mut out = String::new();
        for (i, cls) in classes.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            out.push_str(&format!(
                "near-clone: {} occurrences, {} hole(s): {}\n",
                cls.sites.len(),
                cls.hole_count,
                cls.pattern
            ));
            for site in &cls.sites {
                out.push_str(&format!("  {}\n", site_loc(site, sources, &indices)));
            }
        }
        out
    }

    /// Render near-clone classes as JSON: `[{pattern, size, holes, sites:[{file?, line, col}]}]`.
    pub fn near_clones_json(
        classes: &[clones::NearCloneClass],
        sources: &std::collections::HashMap<String, String>,
    ) -> String {
        let indices = build_line_indices(sources);
        let mut arr = json::Array::new();
        for cls in classes {
            let mut obj = json::Object::new();
            obj.string("pattern", &cls.pattern);
            obj.raw("size", &cls.size.to_string());
            obj.raw("holes", &cls.hole_count.to_string());
            let mut sites = json::Array::new();
            for site in &cls.sites {
                sites.raw(&site_json(site, sources, &indices));
            }
            obj.raw("sites", &sites.finish());
            arr.raw(&obj.finish());
        }
        arr.finish()
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
    use super::{Tree, tree_eq};

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
                        kind: ChangeKind::Remove {
                            old: a[i].to_sexpr(),
                        },
                    });
                }
                Align::Ins(j) => {
                    let mut p = path.clone();
                    p.push(j);
                    out.push(Change {
                        path: p,
                        kind: ChangeKind::Add {
                            new: b[j].to_sexpr(),
                        },
                    });
                    pos = j + 1;
                }
            }
        }
    }

    /// One LCS alignment op between two child slices. Public so the formatting-preserving edit path
    /// (`super::textedit`) can reuse the exact same child alignment the structural diff uses.
    pub enum Align {
        Keep(usize, usize), // (index in a, index in b) — structurally equal, recurse for inner diffs
        Del(usize),         // index in a
        Ins(usize),         // index in b
    }

    /// LCS alignment of two child slices by structural equality (the same shape as the line diff, on
    /// trees). Prefers keeping structurally-equal children aligned so Add/Remove land on the genuinely
    /// new/old ones. Exposed as `align_children` for reuse by the span-splicing edit path.
    pub fn align_children(a: &[Tree], b: &[Tree]) -> Vec<Align> {
        align(a, b)
    }

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
            path.iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(".")
        }
    }
}

/// FORMATTING-PRESERVING rewrite — splice changed subtrees into the ORIGINAL source at their spans,
/// leaving every unmatched byte (whitespace, newlines, comments, hand-alignment) exactly as it was.
///
/// The whole-tree printer path (`driver::apply_rewrite`) re-serializes the program, which reflows a
/// hand-formatted file onto the printer's canonical layout — unusable for editing a corpus meant to
/// be read and diffed line-by-line (ask-89). This module instead ALIGNS the original tree (which
/// carries a source span per node) against the rewritten tree and emits the MINIMAL set of textual
/// edits: a changed operand is one span-sized splice; a deleted list child is one span deletion
/// (widened to swallow its own line's leading indent + trailing newline, so no blank line dangles);
/// an inserted child is printed and spliced at the right offset. Every other byte is copied verbatim.
///
/// This mirrors how comby / ast-grep / jscodeshift edit at spans rather than reprint, and it depends
/// only on the span table the reader now produces for BOTH surfaces (ML and — via `read_spanned` —
/// s-expr, the corpus surface). Replacement text for new/changed nodes is rendered in the same
/// surface as the source, so the splice reads consistently with its neighbours.
pub mod textedit {
    use super::Tree;
    use crate::ast::Leaf;
    use crate::convert::Format;

    /// One primitive text edit: replace the original byte range `[start, end)` with `text`. A pure
    /// deletion has `text == ""`; a pure insertion has `start == end`.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Edit {
        pub start: usize,
        pub end: usize,
        pub text: String,
    }

    /// The result of a formatting-preserving rewrite: the edited source plus the number of primitive
    /// edits applied (0 ⇒ the edit was a no-op / the tree was unchanged).
    #[derive(Clone, Debug)]
    pub struct TextRewrite {
        pub output: String,
        pub edits: usize,
    }

    /// Compute the minimal edits turning `src` (whose tree is `old`, with per-node spans available
    /// via `old`'s provenance and a span lookup) into the program `new`, then apply them.
    ///
    /// `span_of` maps an original node to its source byte range. A node with no span (a synthetic /
    /// rewritten node) can't be edited in place — but the alignment only ever asks for the span of an
    /// ORIGINAL node (one drawn from `old`), which always has one, so `span_of` returning `None`
    /// there signals a bug and falls back to a whole-node print (never a panic).
    pub fn rewrite_preserving(
        src: &str,
        old: &Tree,
        new: &Tree,
        span_of: &dyn Fn(&Tree) -> Option<(usize, usize)>,
        surface: Format,
    ) -> TextRewrite {
        let mut edits = edits_preserving(src, old, new, span_of, surface);
        let n = edits.len();
        let output = apply_edits(src, &mut edits);
        TextRewrite { output, edits: n }
    }

    /// The primitive byte edits turning `src` (tree `old`) into `new`, in ascending `(start, end)` order —
    /// the STRUCTURAL patch as span-anchored text edits. This is what [`rewrite_preserving`] applies; exposed
    /// so a machine consumer (the `cdz check --json` fix channel) can HAND an agent the exact edits to apply
    /// (`src[start..end] := text`) rather than a hand-derived kind/prefix/suffix. Each edit is minimal and
    /// surface-correct (only changed subtrees; a wrap preserves the wrapped bytes and splices only the
    /// wrapper; an insert lands at the right child position) because it comes from the same alignment
    /// `rewrite_preserving` uses — no separate text logic to drift.
    pub fn edits_preserving(
        src: &str,
        old: &Tree,
        new: &Tree,
        span_of: &dyn Fn(&Tree) -> Option<(usize, usize)>,
        surface: Format,
    ) -> Vec<Edit> {
        let mut edits = Vec::new();
        diff_edits(src, old, new, span_of, surface, &mut edits);
        edits.sort_by_key(|ed| (ed.start, ed.end));
        edits
    }

    /// Render `t` as source text in `surface` (a single node, one line). Used for the replacement
    /// text of a changed / inserted node.
    fn render(t: &Tree, surface: Format) -> String {
        let arena = t.to_arena();
        match surface {
            // ML uses the pretty-printer at a generous width so a spliced node stays on one line
            // where it fits; the surrounding layout is untouched, so only this node is (re)printed.
            Format::Ml => crate::printer::print(&arena, 100),
            // sexpr and everything else: the direct one-line s-expression rendering.
            _ => crate::sexpr::print(&arena),
        }
    }

    /// Render an inserted CHILD `t` for splicing into a list whose head is `parent_head`, on `surface`.
    /// Most nodes render context-free via [`render`], but a few ML forms are CONTEXT-SENSITIVE — the ML
    /// printer only emits their in-context syntax when it sees them INSIDE their parent. The one that
    /// bites the corpus fixer: a `match` ARM `(pat body)` prints as `| pat => body` only inside a
    /// `(match …)`; rendered STANDALONE it prints as an APPLICATION `pat(body)` (e.g. `(D (trap …))` →
    /// `D(trap(…))`), which is invalid ML in arm position, so the InsertArms fix (CDZ0210 non-exhaustive
    /// match) was silently dropped on the ML surface at reparse/validate. (s-expr is context-free — an arm
    /// renders the same in or out of a match — so only ML needs this.) Fix: for an ML `match`-child, render
    /// a synthetic single-arm `match` and extract the `  | … => …` arm line(s), so the splice is valid ML.
    fn render_child(t: &Tree, parent_head: Option<&str>, surface: Format) -> String {
        if surface == Format::Ml && parent_head == Some("match") {
            // A match arm is a 2-element `(pat body)` list; anything else under a `match` head (the
            // scrutinee, or a malformed child) is not an arm — render it plainly.
            if let Tree::List(kids, _) = t
                && kids.len() == 2
            {
                // Build `(match cdz_arm_ctx <arm>)`, print it, and take everything after the `with` line
                // — the rendered arm line(s), correctly `| pat => body`. The sentinel scrutinee name is
                // arbitrary (never emitted into the result — only the arm tail is kept).
                let sentinel = Tree::Atom(Leaf::Name("cdz_arm_ctx".into()), None);
                let synthetic = Tree::List(
                    vec![
                        Tree::Atom(Leaf::Name("match".into()), None),
                        sentinel,
                        t.clone(),
                    ],
                    None,
                );
                let printed = render(&synthetic, surface);
                // The printer emits `match cdz_arm_ctx with\n  | <pat> => <body>`; keep everything after
                // the first newline (the arm line(s)), trimmed of leading indentation so the splice's own
                // separator controls placement.
                if let Some((_, arm)) = printed.split_once('\n') {
                    return arm.trim_start().to_string();
                }
            }
        }
        render(t, surface)
    }

    /// Walk `old`/`new` in parallel, appending edits for the sub-nodes that differ. The alignment
    /// rule matches `treediff`: identical ⇒ nothing; same-head lists ⇒ recurse (positional if equal
    /// arity, else LCS align — children Kept recurse, Removed delete their span, Inserted splice in);
    /// anything else ⇒ replace this whole node's span.
    fn diff_edits(
        src: &str,
        old: &Tree,
        new: &Tree,
        span_of: &dyn Fn(&Tree) -> Option<(usize, usize)>,
        surface: Format,
        out: &mut Vec<Edit>,
    ) {
        match (old, new) {
            // Two ALIGNED lists (same head): recurse into the children. Do NOT pre-`tree_eq` the whole
            // subtree here — that full O(subtree) walk was run at EVERY nesting level (each level's
            // `diff_children` re-`diff_edits`'d every child, whose first act was another deep `tree_eq`),
            // so a deep tree cost O(depth²) per diff, and a fix computed per-diagnostic made a file with N
            // fixable warnings O(N·depth²) → O(N³) on a deeply-nested program (a 200-deep-tuple match with
            // 200 unused binders: 897ms; 400: 7.3s). The recursion ALREADY emits no edits for an unchanged
            // child (aligned lists recurse to nothing; equal atoms are caught by the `Atom==Atom` arm
            // below), so the deep pre-check was pure redundant re-walking. Each node is now visited once →
            // O(tree) per diff.
            (Tree::List(a, _), Tree::List(b, _)) if same_head(a, b) => {
                diff_children(src, old, a, b, span_of, surface, out);
            }
            // Two atoms: equal → no edit (the leaf case the old top-level `tree_eq` guard covered);
            // differing → fall through to the whole-node reprint below.
            (Tree::Atom(la, _), Tree::Atom(lb, _)) if la == lb => {}
            _ => {
                // The new node does not align with the old by head. If `new` EMBEDS `old` as a descendant
                // (a WRAP — `old` becomes `(ctor old)`), preserve `old`'s original bytes: reprint only the
                // wrapper material around it, so a wrapped `E.get` keeps its spelling rather than being
                // canonically re-rendered. Otherwise fall back to a whole-node reprint.
                if let Some((s, e)) = span_of(old) {
                    if let Some((prefix, suffix)) = wrap_around_original(new, old, surface) {
                        // Splice the wrapper before/after the preserved original span (two insert edits).
                        if !prefix.is_empty() {
                            out.push(Edit {
                                start: s,
                                end: s,
                                text: prefix,
                            });
                        }
                        if !suffix.is_empty() {
                            out.push(Edit {
                                start: e,
                                end: e,
                                text: suffix,
                            });
                        }
                    } else {
                        out.push(Edit {
                            start: s,
                            end: e,
                            text: render(new, surface),
                        });
                    }
                }
            }
        }
    }

    /// If `new` embeds `original` (by identity — the SAME node, matched by its `origin`) as a descendant,
    /// return the `(prefix, suffix)` text that wraps `original`'s source bytes to produce `new` — reprinting
    /// only the WRAPPER, so the wrapped subtree keeps its original formatting. `None` when `new` does not
    /// contain `original` (a genuine whole-node replacement, reprinted normally). Realized by rendering
    /// `new` with `original` swapped for a HOLE placeholder, then splitting the rendered text on the hole:
    /// everything before is the prefix, everything after the suffix. The placeholder is a name that cannot
    /// occur in real source, so the split is unambiguous.
    fn wrap_around_original(
        new: &Tree,
        original: &Tree,
        surface: Format,
    ) -> Option<(String, String)> {
        let target = original.origin()?;
        // Require `original` to be a PROPER DESCENDANT of `new` — not `new` itself. When `new` merely
        // REPLACES `original` at the same position (a did-you-mean swaps one atom for another; both keep
        // the same origin), `new` "contains" the origin only as its own root, which is a plain replace, not
        // a wrap — reprinting it as prefix+HOLE+suffix would blank the whole node. Only a wrap embeds the
        // original strictly deeper.
        if new.origin() == Some(target) || !children_contain_origin(new, target) {
            return None;
        }
        // A placeholder that never occurs in source (matches `abi::WRAP_HOLE`'s intent; kept local so
        // `cadenza-syntax` needs no dependency on the compiler crate).
        const HOLE: &str = "\u{2026}HOLE\u{2026}";
        let holed = replace_origin_with_hole(new, target, HOLE);
        let rendered = render(&holed, surface);
        let (prefix, suffix) = rendered.split_once(HOLE)?;
        Some((prefix.to_string(), suffix.to_string()))
    }

    /// Whether `t` has a descendant (or is a node) whose `origin` is `target`.
    fn contains_origin(t: &Tree, target: crate::ast::StructId) -> bool {
        if t.origin() == Some(target) {
            return true;
        }
        match t {
            Tree::Atom(..) => false,
            Tree::List(items, _) => items.iter().any(|c| contains_origin(c, target)),
        }
    }

    /// Whether a PROPER DESCENDANT of `t` (a child or deeper — not `t` itself) has `origin == target`.
    fn children_contain_origin(t: &Tree, target: crate::ast::StructId) -> bool {
        match t {
            Tree::Atom(..) => false,
            Tree::List(items, _) => items.iter().any(|c| contains_origin(c, target)),
        }
    }

    /// Rebuild `t` with the node whose `origin` is `target` replaced by a bare `HOLE`-named atom — so
    /// `render` prints a placeholder where the preserved original goes. New/wrapper nodes are rendered
    /// normally; only the one preserved subtree becomes the hole.
    fn replace_origin_with_hole(t: &Tree, target: crate::ast::StructId, hole: &str) -> Tree {
        if t.origin() == Some(target) {
            return Tree::Atom(crate::ast::Leaf::Name(hole.into()), None);
        }
        match t {
            Tree::Atom(..) => t.clone(),
            Tree::List(items, o) => Tree::List(
                items
                    .iter()
                    .map(|c| replace_origin_with_hole(c, target, hole))
                    .collect(),
                *o,
            ),
        }
    }

    /// Do two child lists share a head name? Both empty, both headed by the SAME name, or both headed by
    /// non-names (rare — recursing beats a whole-node replace). But a NAME head vs a NON-name head (a list
    /// head) is a genuine head CHANGE — `(+ n 1)` vs `((. Int64 of) (+ n 1))` — NOT a positional align: the
    /// second WRAPS the first, so returning false here lets `diff_edits` take the wrap-preserve path (two
    /// clean insert edits) instead of an LCS align that fragments into leading-space inserts + empty deletes.
    fn same_head(a: &[Tree], b: &[Tree]) -> bool {
        match (a.first(), b.first()) {
            (None, None) => true,
            (Some(x), Some(y)) => match (name_of(x), name_of(y)) {
                (Some(nx), Some(ny)) => nx == ny,
                // Both heads unnameable (both list-headed) — alignable; recurse.
                (None, None) => true,
                // One named, one not — a head change (often a wrap): NOT alignable.
                _ => false,
            },
            _ => false,
        }
    }

    fn name_of(t: &Tree) -> Option<&str> {
        match t {
            Tree::Atom(crate::ast::Leaf::Name(n), _) => Some(n),
            _ => None,
        }
    }

    /// Diff the children of two same-head lists. Equal arity ⇒ positional recursion; unequal ⇒ LCS
    /// alignment producing span-anchored delete / insert / recurse edits.
    fn diff_children(
        src: &str,
        old_list: &Tree,
        a: &[Tree],
        b: &[Tree],
        span_of: &dyn Fn(&Tree) -> Option<(usize, usize)>,
        surface: Format,
        out: &mut Vec<Edit>,
    ) {
        if a.len() == b.len() {
            for (x, y) in a.iter().zip(b) {
                diff_edits(src, x, y, span_of, surface, out);
            }
            return;
        }
        let ops = super::treediff::align_children(a, b);
        // `anchor_end` tracks the byte offset just past the last old child seen (kept or deleted),
        // so an inserted new child lands right AFTER its preceding sibling. It starts just past the
        // parent's opening `(` (or its first child, the head) so a leading insertion is well-placed.
        let mut anchor_end = a
            .first()
            .and_then(span_of)
            .map(|(_, e)| e)
            .or_else(|| span_of(old_list).map(|(s, _)| s + 1))
            .unwrap_or(0);
        for op in ops {
            match op {
                super::treediff::Align::Keep(i, j) => {
                    diff_edits(src, &a[i], &b[j], span_of, surface, out);
                    if let Some((_, e)) = span_of(&a[i]) {
                        anchor_end = e;
                    }
                }
                super::treediff::Align::Del(i) => {
                    if let Some((s, e)) = span_of(&a[i]) {
                        let (ws, we) = widen_deletion(src, s, e, surface);
                        out.push(Edit {
                            start: ws,
                            end: we,
                            text: String::new(),
                        });
                        anchor_end = e;
                    }
                }
                super::treediff::Align::Ins(j) => {
                    // Render the new child in its PARENT's context (a `match` arm needs `| pat => body`
                    // arm-syntax, not a standalone application — see `render_child`). The separator: a
                    // match arm goes on its OWN line (arms are `|`-led, one per line), so an ML arm insert
                    // is newline-prefixed; every other insert is space-prefixed so tokens don't fuse.
                    // (Insertion is the rarer path — the corpus edit is a pure deletion; a splice the
                    // validator rejects falls back to a reprint upstream.)
                    let parent_head = match old_list {
                        Tree::List(items, _) => items.first().and_then(Tree::as_name),
                        _ => None,
                    };
                    let rendered = render_child(&b[j], parent_head, surface);
                    let sep = if surface == Format::Ml && parent_head == Some("match") {
                        "\n  "
                    } else {
                        " "
                    };
                    out.push(Edit {
                        start: anchor_end,
                        end: anchor_end,
                        text: format!("{sep}{rendered}"),
                    });
                }
            }
        }
    }

    /// Widen a deletion `[s, e)` to swallow the deleted node's own line when it sits ALONE on it:
    /// extend the start back over leading spaces/tabs to the line start, and the end forward over one
    /// trailing newline — so removing a `(needs …)` clause on its own line leaves no blank line and no
    /// dangling indent. If other non-space text shares the line before `s`, only trailing spaces up
    /// to `s` are kept (we don't eat a preceding token). If text follows on the same line after `e`,
    /// we don't eat the newline (so we don't join two lines).
    ///
    /// On the ML surface a sequencing block's non-final elements are joined by `;` (`a; b; c`), and a
    /// deleted element's span covers only the element — NOT the `;` that follows it. Leaving the `;`
    /// orphans the separator (`a; b; c` deleting `b` → `a; ; c`, a parse error). A `;` in ML is ONLY the
    /// sequence operator (never part of a token), so before the whitespace pass we extend the range over
    /// an adjacent separator: the `;` FOLLOWING the element (a non-final element, `b` in `a; b; c`), or —
    /// when none follows — the `;` PRECEDING it (the final element, `c`). Absorbing exactly one `;` keeps
    /// the surviving elements correctly separated. s-expr has no `;` separator (space-delimited), so this
    /// is ML-only; the whitespace/line widening below then tidies the rest.
    fn widen_deletion(src: &str, s: usize, e: usize, surface: Format) -> (usize, usize) {
        let bytes = src.as_bytes();
        // ML `;` sequencing separator: absorb the one adjacent to the deleted element so the surviving
        // elements stay separated (never orphaning a `;`). Prefer the FOLLOWING `;` (non-final element);
        // fall back to the PRECEDING one (final element). Only a run of spaces/tabs may sit between the
        // element and its `;` (a newline before a leading `;` would belong to the previous line — but ML
        // prints `expr;⏎ next`, so the `;` immediately follows the element, before any newline).
        let (mut s, mut e) = (s, e);
        if surface == Format::Ml {
            // Following `;` — scan past inline whitespace after the element.
            let mut f = e;
            while f < bytes.len() && matches!(bytes[f], b' ' | b'\t') {
                f += 1;
            }
            if f < bytes.len() && bytes[f] == b';' {
                e = f + 1;
            } else {
                // No following `;` (final element) — scan back past inline whitespace to a preceding `;`.
                let mut p = s;
                while p > 0 && matches!(bytes[p - 1], b' ' | b'\t' | b'\n' | b'\r') {
                    p -= 1;
                }
                if p > 0 && bytes[p - 1] == b';' {
                    s = p - 1;
                }
            }
        }
        // Extend start left over spaces/tabs.
        let mut ws = s;
        while ws > 0 && matches!(bytes[ws - 1], b' ' | b'\t') {
            ws -= 1;
        }
        let at_line_start = ws == 0 || bytes[ws - 1] == b'\n';
        // Is the remainder of the line (after e) only whitespace up to a newline?
        let mut k = e;
        while k < bytes.len() && matches!(bytes[k], b' ' | b'\t' | b'\r') {
            k += 1;
        }
        let line_tail_blank = k >= bytes.len() || bytes[k] == b'\n';
        if at_line_start && line_tail_blank {
            // The node owns its line: delete leading indent through the trailing newline (inclusive),
            // removing the whole line.
            let mut we = e;
            while we < bytes.len() && matches!(bytes[we], b' ' | b'\t' | b'\r') {
                we += 1;
            }
            if we < bytes.len() && bytes[we] == b'\n' {
                we += 1;
            }
            (ws, we)
        } else {
            // Shares its line: delete just the node plus one leading space (so `a b` → `a`, not
            // `a  `), without touching newlines.
            let start = if s > ws { s - 1 } else { s };
            (start, e)
        }
    }

    /// The single byte edit that DELETES the node occupying source span `[start, end)` from its parent
    /// list — the same edit `diff_edits`' `Align::Del` arm emits (`widen_deletion` + an empty-text edit),
    /// computed DIRECTLY from the known deleted span instead of via the LCS `align` of the whole parent's
    /// children. A delete fix already knows exactly WHICH child vanishes (its target), so the alignment
    /// DP is pure waste — and for a WIDE parent (a `do` block or match with N children) that DP is O(N²)
    /// `tree_eq` cells PER fix, so N such fixes on one file were O(N³) (a `do` of N discarded statements:
    /// N=100/200/400 = 33/207/1639ms, ~7×/dbl). This helper makes a delete fix's edit O(1) — no parent
    /// diff, no alignment. Byte-identical to the alignment path (same `widen_deletion`, same empty text).
    /// Returns `None` if the span is invalid — either degenerate (`start > end`) or out of bounds
    /// (`end > src.len()`) — so a caller passing a stale or miscomputed span gets `None` rather than a
    /// panic on the slice.
    pub fn delete_edit(src: &str, start: usize, end: usize, surface: Format) -> Option<Edit> {
        if start > end || end > src.len() {
            return None;
        }
        let (ws, we) = widen_deletion(src, start, end, surface);
        Some(Edit {
            start: ws,
            end: we,
            text: String::new(),
        })
    }

    /// Apply non-overlapping `edits` to `src`, producing the edited string. Edits are sorted by start
    /// offset; an insertion (`start == end`) at the same offset keeps input order. Overlapping edits
    /// (which the tree alignment never produces — each edit covers a distinct node's span) are applied
    /// in order, later ones skipped if they'd overlap an already-applied range.
    fn apply_edits(src: &str, edits: &mut [Edit]) -> String {
        edits.sort_by_key(|ed| (ed.start, ed.end));
        let mut out = String::with_capacity(src.len());
        let mut cursor = 0usize;
        for ed in edits.iter() {
            if ed.start < cursor {
                // Overlaps an already-emitted region — skip (defensive; shouldn't happen).
                continue;
            }
            out.push_str(&src[cursor..ed.start]);
            out.push_str(&ed.text);
            cursor = ed.end;
        }
        out.push_str(&src[cursor..]);
        out
    }
}

/// Structural LINTING — flag anti-patterns by shape rather than fix them. A lint rule is a pattern
/// plus a message and a severity; every match becomes a diagnostic. Batched over a codebase, this is
/// a Semgrep-lite structural checker / CI gate: it exits non-zero when any `error`-severity rule
/// fires. Purely syntactic (no scope/type), like the rest of this layer.
pub mod lint {
    use super::{
        Pattern, PatternError, Query, Span, SpanTable, Template, Tree, compile_pat, search_with,
    };

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

    /// Whether an autofix is always meaning-preserving (`Verified`, safe to auto-apply under `--fix`)
    /// or a suggestion that may change behavior/readability (`Heuristic`, offered but never applied
    /// without an explicit opt-in). `Verified` is licensed only by a round-trip apply-and-recheck
    /// witness test (DESIGN-cadenza-lint §6); a fix without that witness is `Heuristic`.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Applicability {
        Verified,
        Heuristic,
    }

    impl Applicability {
        /// Parse an applicability name; `None` for an unknown one.
        pub fn parse(s: &str) -> Option<Applicability> {
            match s {
                "verified" => Some(Applicability::Verified),
                "heuristic" => Some(Applicability::Heuristic),
                _ => None,
            }
        }

        pub fn as_str(self) -> &'static str {
            match self {
                Applicability::Verified => "verified",
                Applicability::Heuristic => "heuristic",
            }
        }
    }

    /// One lint rule: match `pattern`, and every match reports `message` at `severity`. A NAMED,
    /// fixable idiomatic lint additionally carries a namespaced `name` (e.g. `idiomatic/if-bool`,
    /// which level control keys off) and, where a canonical rewrite exists, a `fix` = an inline
    /// replacement `Template` over the pattern's metavars plus its `Applicability`. A bare report-only
    /// rule has `name: None` and `fix: None` — the existing surface, unchanged.
    #[derive(Clone, Debug)]
    pub struct LintRule {
        pub name: Option<String>,
        pub pattern: Pattern,
        pub message: String,
        pub severity: Severity,
        pub fix: Option<(Template, Applicability)>,
    }

    impl LintRule {
        /// Compile a rule from either lint form (the named one is a SUPERSET of the bare one):
        ///
        /// * `(lint PATTERN "message" [severity])` — the existing report-only rule, unchanged.
        /// * `(lint NAME PATTERN "message" [level] [=> TEMPLATE app])` — a NAMED idiomatic lint: a
        ///   leading bare-name atom is the namespaced lint name, and an optional trailing
        ///   `=> TEMPLATE app` clause carries a fix (a `Template` over the pattern's metavars + an
        ///   `Applicability` name `verified`/`heuristic`).
        ///
        /// The two are distinguished by the element after `lint`: a bare NAME atom (not a list, not a
        /// string) means the named form; anything else (a `(…)` pattern list) is the existing bare
        /// form. Severity/level defaults to `warning`; unknown severity or applicability names are
        /// rejected; a fix template that references a metavar the pattern never binds is rejected.
        pub fn compile_form(t: &Tree) -> Result<LintRule, PatternError> {
            let items = match t {
                Tree::List(items, _) if head_is(items, "lint") => items,
                _ => return Err(PatternError("expected a `(lint …)` form".into())),
            };
            // The named form is marked by a bare-NAME atom right after `lint` (a pattern is always a
            // `(…)` list or a metavar, never a plain namespaced name like `idiomatic/if-bool`).
            let named = matches!(items.get(1), Some(t) if is_lint_name_atom(t));
            if named {
                Self::compile_named(items)
            } else {
                Self::compile_bare(items)
            }
        }

        /// The existing `(lint PATTERN "message" [severity])` report-only form.
        fn compile_bare(items: &[Tree]) -> Result<LintRule, PatternError> {
            let (pat_tree, message, sev_tree) = match items {
                [_, p, msg] => (p, msg, None),
                [_, p, msg, sev] => (p, msg, Some(sev)),
                _ => {
                    return Err(PatternError(
                        "a lint rule is `(lint PATTERN \"message\" [severity])`".into(),
                    ));
                }
            };
            let message = as_str_leaf(message)
                .ok_or_else(|| PatternError("a lint rule's message must be a \"string\"".into()))?;
            let severity = parse_severity(sev_tree)?;
            Ok(LintRule {
                name: None,
                pattern: Pattern {
                    pat: compile_pat(pat_tree)?,
                },
                message: message.to_string(),
                severity,
                fix: None,
            })
        }

        /// The named `(lint NAME PATTERN "message" [level] [=> TEMPLATE app])` form.
        fn compile_named(items: &[Tree]) -> Result<LintRule, PatternError> {
            // Split off an optional trailing `=> TEMPLATE app` fix clause (3 tokens: the `=>` marker,
            // the template, the applicability name), so the head parses as the report part.
            let (head, fix_clause) = split_fix_clause(items)?;
            let (name_tree, pat_tree, message, sev_tree) = match head {
                [_, name, p, msg] => (name, p, msg, None),
                [_, name, p, msg, sev] => (name, p, msg, Some(sev)),
                _ => {
                    return Err(PatternError(
                        "a named lint rule is `(lint NAME PATTERN \"message\" [level] [=> TEMPLATE app])`"
                            .into(),
                    ));
                }
            };
            let name = name_tree
                .as_name()
                .ok_or_else(|| PatternError("a lint NAME must be a bare name".into()))?
                .to_string();
            let message = as_str_leaf(message)
                .ok_or_else(|| PatternError("a lint rule's message must be a \"string\"".into()))?;
            let severity = parse_severity(sev_tree)?;
            let pattern = Pattern {
                pat: compile_pat(pat_tree)?,
            };
            let fix = match fix_clause {
                None => None,
                Some((tmpl_tree, app_tree)) => {
                    let app_name = app_tree.as_name().ok_or_else(|| {
                        PatternError("a fix applicability must be a bare name".into())
                    })?;
                    let app = Applicability::parse(app_name).ok_or_else(|| {
                        PatternError(format!(
                            "unknown fix applicability `{app_name}` (want `verified` or `heuristic`)"
                        ))
                    })?;
                    let template = Template::from_tree(tmpl_tree.clone());
                    // A fix that references a metavar the pattern never binds can never instantiate —
                    // reject at compile so a broken rule is caught at load, not silently at 0 sites.
                    let bound = pattern.bound_metavars();
                    for referenced in template.referenced_metavars() {
                        if !bound.contains(&referenced) {
                            return Err(PatternError(format!(
                                "lint `{name}` fix references `,{referenced}`, which its pattern never binds"
                            )));
                        }
                    }
                    Some((template, app))
                }
            };
            Ok(LintRule {
                name: Some(name),
                pattern,
                message: message.to_string(),
                severity,
                fix,
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

        /// The BUILT-IN `idiomatic` lint catalog (DESIGN-cadenza-lint §2, Tier A — the first shipped
        /// pack) — the curated `LintSet` `cdz lint` uses by default so `cdz lint FILE` works out of the
        /// box. Each rule is a named, level-controllable idiomatic lint carrying a `Verified` structural
        /// fix where a canonical rewrite exists:
        ///
        /// - `idiomatic/if-bool` — `(if c true false)` → `c`; `(if c false true)` → `(not c)`. Two rules
        ///   (the arena distinguishes the arms), both Verified.
        /// - `idiomatic/redundant-let` — `(let ((x e)) x)` → `e` (bind then immediately return it),
        ///   Verified. NB the arena let-shape is the binding-LIST `(let ((x e)) x)`, not `(let x e x)`.
        /// - `idiomatic/double-negation` — `(not (not e))` → `e`, Verified.
        /// - `idiomatic/if-same-branch` — `(if c e e)` → `e` (both arms structurally equal, so the
        ///   condition is dead), Verified. Relies on the engine's non-linear metavar consistency: the
        ///   repeated `,e` matches only when the two arms are structurally equal.
        ///
        /// - `idiomatic/single-arm-match` — `(match s (p b))` → `(let ((p s)) b)` when the single arm's
        ///   pattern `p` is IRREFUTABLE (var/`_`/tuple/record), Verified. Expressed as a pure template by
        ///   the `is-irrefutable` metavar guard, which delegates to `match_to_let::is_irrefutable` (the
        ///   no-context form — a sum-ctor pattern is conservatively refutable, so the lint never fires on
        ///   a match whose refutability the `let` would erase). The clause's 2-element `(p b)` arity is
        ///   what pins "single arm, unguarded": a multi-arm match has more clauses, and a guarded arm is
        ///   not a bare `(p b)` pair — so the pattern structurally excludes both.
        /// - `naming/camel-case` — a `camelCase` binding name (a `def` name or a `let` binder) →
        ///   REPORT-ONLY warning (Cadenza convention is `snake_case`), via the `is-camel-case` metavar
        ///   guard. NO catalog fix: the auto-rename is Heuristic (it must rewrite every use site, needing
        ///   resolve info), so it is offered as a code-action, not applied by `--fix` (design §naming).
        ///   Two rules (def + let binder); a per-parameter form needs splice-element guards (later).
        /// - `idiomatic/deep-nesting` — a node whose call-CHAIN depth exceeds N (`calls-deeper-than 10`)
        ///   → REPORT-ONLY warning (the hoist-to-`let` fix is Heuristic, a code-action). N=10 is a
        ///   conservative egregious-only placeholder (concierge ruling 2026-08-09, pending the operator's
        ///   final threshold). Uses the `call_depth` metric (nested application forms only), NOT the
        ///   whole-subtree `list_depth` (which over-fires on structural nesting).
        /// - `idiomatic/nested-match` — a `match` whose SCRUTINEE is itself a `match` (matching on a
        ///   match RESULT — the operator's headline example) → REPORT-ONLY warning (the combine-into-one-
        ///   match fix is Heuristic — arm cross-product, unsettled tuple-scrutinee form — so a code-action,
        ///   not `--fix`). Matches ONLY `(match (match …) …)`, NOT an arm-body match (ordinary idiomatic
        ///   dispatch — 473 corpus hits vs 11 for the scrutinee form, so the arm-body shape would flood).
        ///
        /// Design §2 once listed `idiomatic/negated-eq` (`(not (== a b))` → `(!= a b)`); it is STRUCK
        /// (ruling 2026-08-09) as VACUOUS — there is no `!=` node to rewrite to. Core Cadenza has no
        /// `Prim::Ne` (only Lt/Gt/Le/Ge/Eq), no `!=` lexer token or `op_str` head (`!=` lives only in the
        /// cedar sublanguage), and `!=` desugars to `(not (= …))` — so `(not (= a b))` is ALREADY the
        /// canonical form. The catalog grows increment-by-increment (§2 open catalog).
        ///
        /// `compile` of this text is infallible (a unit test pins it), so `expect` is sound.
        pub fn builtin() -> LintSet {
            // A sequence of `(lint …)` forms — `LintSet::compile` reads them via `sexpr::read_all`,
            // which synthesizes the `(do …)` wrapper itself (so we must NOT write our own `(do …)`, or
            // it double-wraps and the inner `(do …)` fails the `(lint …)` head check).
            Self::compile(concat!(
                "(lint idiomatic/if-bool (if ,c true false) ",
                "\"redundant `if` on a Bool — use the condition directly\" => ,c verified)\n",
                "(lint idiomatic/if-bool (if ,c false true) ",
                "\"redundant `if` on a Bool — use `not(condition)`\" => (not ,c) verified)\n",
                "(lint idiomatic/redundant-let (let ((,x ,e)) ,x) ",
                "\"redundant `let` — binds a value then immediately returns it; use the value directly\" ",
                "=> ,e verified)\n",
                "(lint idiomatic/double-negation (not (not ,e)) ",
                "\"double negation — `not(not(e))` is just `e`\" => ,e verified)\n",
                "(lint idiomatic/if-same-branch (if ,c ,e ,e) ",
                "\"both `if` branches are identical — the condition is dead; use the branch directly\" ",
                "=> ,e verified)\n",
                "(lint idiomatic/single-arm-match (match ,s (,(p is-irrefutable) ,b)) ",
                "\"single irrefutable `match` — one arm that can never fail is clearer as a `let`\" ",
                "=> (let ((,p ,s)) ,b) verified)\n",
                // naming/camel-case — REPORT-ONLY (no fix): the auto-rename is Heuristic (it must rewrite
                // every use site, needing resolve info), so it is offered by a code-action, not the
                // catalog's `--fix`. Two highest-signal binding positions: a `def` name and a `let`
                // binder. A per-parameter guard needs splice-element guards (a later engine increment).
                "(lint naming/camel-case (def (,(n is-camel-case) ,@_) ,@_) ",
                "\"camelCase binding name — Cadenza convention is snake_case\")\n",
                "(lint naming/camel-case (let ((,(n is-camel-case) ,_)) ,@_) ",
                "\"camelCase binding name — Cadenza convention is snake_case\")\n",
                // idiomatic/deep-nesting — REPORT-ONLY (no fix): a call-CHAIN nested deeper than the
                // threshold is non-idiomatic (operator PR-2790 hm-collect.cdz); the fix (hoist inner
                // sub-expressions to let-bound names) is Heuristic — the names are the author's — so it is
                // offered as a code-action, never applied by --fix. Threshold N=10 is a CONSERVATIVE
                // egregious-only placeholder (concierge ruling 2026-08-09, pending the operator's final
                // threshold): the empirical study showed no clean cutoff (dense compiler-ml files light up
                // at N=6 and N=8), so N=10 flags only pathological nesting without redding dense-but-legit
                // code. Uses the call_depth metric (nested application forms only), NOT list_depth. NB it
                // fires on EVERY node whose call-depth exceeds N, so a single deep chain reports on its
                // outer nodes too (an outermost-only de-dup needs ancestor context the metavar guard lacks
                // — a follow-on once the operator pins the threshold).
                "(lint idiomatic/deep-nesting ,(x (calls-deeper-than 10)) ",
                "\"deeply nested call chain — consider hoisting inner sub-expressions to named `let` bindings\" ",
                "warning)\n",
                // idiomatic/nested-match — REPORT-ONLY (no fix): a `match` whose SCRUTINEE is itself a
                // `match` (matching on the RESULT of one match) — the operator's headline example. The
                // fix (hoist to one combined match over the inner scrutinee) is Heuristic — the arm
                // cross-product can change readability + the tuple-scrutinee form is unsettled (design
                // §nested-match) — so it is offered as a code-action, never applied by --fix. DELIBERATELY
                // matches ONLY the scrutinee-is-match shape, NOT a match nested in an arm BODY: an
                // arm-body match is ordinary idiomatic nested dispatch (measured 473 corpus hits vs 11 for
                // the scrutinee form), so flagging it would be a false-positive flood. `(match (match …) …)`
                // is the precise, low-false-positive anti-pattern.
                "(lint idiomatic/nested-match (match (match ,@_) ,@_) ",
                "\"match on the result of a match — consider one combined match over the inner scrutinee\" ",
                "warning)\n",
            ))
            .expect("the built-in idiomatic lint catalog compiles")
        }
    }

    /// The per-lint control level (DESIGN-cadenza-lint §3, the one net-new mechanism). Distinct from
    /// [`Severity`] — `Severity` is the rule's default REPORTING kind (error/warning/info); a `LintLevel`
    /// is the user's OVERRIDE of whether and how a NAMED lint fires: `Allow` suppresses it entirely,
    /// `Warn` reports it as a warning, `Deny` promotes it to an error (fails the run). Set by a module
    /// `(allow/warn/deny NAME)` directive or a `--allow/--warn/--deny NAME` CLI flag; only a NAMED lint
    /// (`name: Some(_)`) can be controlled (a bare report-only rule has no name to key off).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum LintLevel {
        Allow,
        Warn,
        Deny,
    }

    impl LintLevel {
        /// Parse a level name; `None` for an unknown one.
        pub fn parse(s: &str) -> Option<LintLevel> {
            match s {
                "allow" => Some(LintLevel::Allow),
                "warn" | "warning" => Some(LintLevel::Warn),
                "deny" => Some(LintLevel::Deny),
                _ => None,
            }
        }

        pub fn as_str(self) -> &'static str {
            match self {
                LintLevel::Allow => "allow",
                LintLevel::Warn => "warn",
                LintLevel::Deny => "deny",
            }
        }
    }

    /// A resolved set of per-lint level overrides, keyed by the namespaced lint name. A lookup honors
    /// GROUP prefixes: a level set for a bare group (`idiomatic`) applies to every lint under it
    /// (`idiomatic/if-bool`), with the MOST SPECIFIC key winning (an exact-name override beats a group
    /// override). Two layers compose by [`Self::overlay`] — the module directive layer first, then the
    /// CLI layer on top (CLI wins), matching the §3 resolution order CLI > module > rule-default.
    #[derive(Clone, Debug, Default)]
    pub struct LintLevels {
        /// name-or-group → level. Insertion order irrelevant; lookup picks the longest matching key.
        map: std::collections::BTreeMap<String, LintLevel>,
    }

    impl LintLevels {
        pub fn new() -> LintLevels {
            LintLevels::default()
        }

        /// Set `name` (an exact lint name or a group prefix) to `level`. A later set of the same key
        /// wins (last-write), so a CLI overlay naturally overrides a module one for the same key.
        pub fn set(&mut self, name: impl Into<String>, level: LintLevel) {
            self.map.insert(name.into(), level);
        }

        /// Overlay `other` ON TOP of `self` (other's keys win) — the CLI layer over the module layer.
        pub fn overlay(&mut self, other: &LintLevels) {
            for (k, v) in &other.map {
                self.map.insert(k.clone(), *v);
            }
        }

        /// The effective level for a lint `name`, or `None` if neither the name nor any of its group
        /// prefixes was overridden (the caller then uses the rule's default). A namespaced name
        /// `a/b/c` is probed most-specific first: `a/b/c`, then `a/b`, then `a` — the longest match
        /// wins, so an exact override beats a broader group one.
        pub fn effective(&self, name: &str) -> Option<LintLevel> {
            if let Some(&l) = self.map.get(name) {
                return Some(l);
            }
            // Walk group prefixes from most to least specific by trimming trailing `/segment`s.
            let mut probe = name;
            while let Some(cut) = probe.rfind('/') {
                probe = &probe[..cut];
                if let Some(&l) = self.map.get(probe) {
                    return Some(l);
                }
            }
            None
        }

        /// The lint levels declared by a program's `@`-ATTRIBUTE lint directives (operator directive:
        /// lint directives ride the existing `@`-attribute mechanism, not a bare list head). An item
        /// attribute `@allow("NAME") item` parses to `(@ (allow "NAME") item)` — the `@`-head wraps a
        /// two-element `(LEVEL "NAME")` attribute over the item; `LEVEL` ∈ `allow`/`warn`/`deny`, `NAME`
        /// is a STRING literal (a namespaced lint name like `"idiomatic/if-bool"` — a string so the `/`
        /// is not parsed as division). This collects every such attribute reachable in the tree into a
        /// program-wide level map (a later increment scopes an item attribute to only the subnode set it
        /// wraps — the operator's "attaches to a set of subnodes" — rather than program-wide; the
        /// program-root case is identical either way). A later directive of the same key wins.
        ///
        /// The module-level `@!allow NAME` form (Rust's `#![allow]`, desugars via the `@!`→`pragma`
        /// path) is a follow-on: it routes through the pragma registry, which needs to recognize the
        /// lint keys — a separate change. This reads the ITEM `@`-attribute form.
        pub fn from_attributes(program: &Tree) -> LintLevels {
            let mut levels = LintLevels::new();
            fn walk(t: &Tree, levels: &mut LintLevels) {
                if let Tree::List(items, _) = t {
                    // An `@`-attribute node is `(@ ATTR form)` — head `@`, a `(LEVEL "NAME")` attr, a form.
                    if items.first().and_then(|h| h.as_name()) == Some("@")
                        && let Some(attr) = items.get(1)
                        && let Tree::List(parts, _) = attr
                        && parts.len() == 2
                        && let Some(level) = parts
                            .first()
                            .and_then(|h| h.as_name())
                            .and_then(LintLevel::parse)
                        && let Some(name) = parts.get(1).and_then(as_str_leaf)
                    {
                        levels.set(name.to_string(), level);
                    }
                    // Descend into every child — attributes can nest (`@a def (… @b …)`) and appear at
                    // any depth (an item attribute inside a module body).
                    for child in items {
                        walk(child, levels);
                    }
                }
            }
            walk(program, &mut levels);
            levels
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
    /// This is the level-free path (every rule reports at its default severity).
    pub fn run(set: &LintSet, subject: &Tree, spans: Option<&SpanTable>) -> Vec<Diagnostic> {
        run_with_levels(set, subject, spans, &LintLevels::default())
    }

    /// Run every lint rule, applying the per-lint level overrides. For a NAMED rule, its effective
    /// level (from `levels`, else its default severity) decides the outcome: `Allow` DROPS every match
    /// (no diagnostic), `Deny` reports at `error` severity, `Warn` at `warning`. A rule with no name,
    /// or a named rule with no override, reports at its own `severity` unchanged. Match order is
    /// preserved (rules in order, matches pre-order).
    pub fn run_with_levels(
        set: &LintSet,
        subject: &Tree,
        spans: Option<&SpanTable>,
        levels: &LintLevels,
    ) -> Vec<Diagnostic> {
        let empty = Query::default();
        let mut out = Vec::new();
        for rule in &set.rules {
            // Resolve the effective severity for this rule. Only a NAMED rule can be level-controlled.
            let level = rule.name.as_deref().and_then(|n| levels.effective(n));
            let severity = match level {
                Some(LintLevel::Allow) => continue, // suppressed — emit nothing for this rule
                Some(LintLevel::Deny) => Severity::Error,
                Some(LintLevel::Warn) => Severity::Warning,
                None => rule.severity,
            };
            for m in search_with(&rule.pattern, &empty, subject, spans) {
                out.push(Diagnostic {
                    message: rule.message.clone(),
                    severity,
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

    /// Severity/level from the optional trailing name (shared by both lint forms): `warning` when
    /// omitted, an unknown name rejected.
    fn parse_severity(sev_tree: Option<&Tree>) -> Result<Severity, PatternError> {
        match sev_tree {
            None => Ok(Severity::Warning),
            Some(s) => {
                let name = s
                    .as_name()
                    .ok_or_else(|| PatternError("severity must be a bare name".into()))?;
                Severity::parse(name)
                    .ok_or_else(|| PatternError(format!("unknown severity `{name}`")))
            }
        }
    }

    /// Whether the element after `lint` marks the NAMED form — a bare name atom (e.g.
    /// `idiomatic/if-bool`). A bare `(lint …)` rule's second element is always the PATTERN, which is a
    /// `(…)` list or a `,meta` unquote (`(unquote …)` list), never a plain name atom — so a name atom
    /// there is unambiguous. (A string message is likewise not a name, so a zero-pattern malformed rule
    /// still falls through to the bare parser and its clearer error.)
    fn is_lint_name_atom(t: &Tree) -> bool {
        t.as_name().is_some()
    }

    /// Split an optional trailing `=> TEMPLATE app` fix clause off a named-lint form's items, returning
    /// the report-part head (without the clause) and the `(template, applicability)` trees if present.
    /// The clause is exactly the last three elements when the third-from-last is the `=>` marker name;
    /// otherwise there is no fix. A partial/misplaced `=>` (present but not in the 3-token tail shape)
    /// is a rejected malformation, not a silent no-fix.
    #[allow(clippy::type_complexity)]
    fn split_fix_clause(items: &[Tree]) -> Result<(&[Tree], Option<(&Tree, &Tree)>), PatternError> {
        // Find any `=>` marker among the elements after `lint`.
        let arrow_at = items
            .iter()
            .position(|t| t.as_name() == Some("=>"))
            .filter(|&i| i >= 1);
        match arrow_at {
            None => Ok((items, None)),
            Some(i) => {
                // A well-formed clause is `=> TEMPLATE app` at the very end: marker, then exactly two
                // trailing elements (template, applicability).
                if i + 2 != items.len() - 1 {
                    return Err(PatternError(
                        "a lint fix clause must be a trailing `=> TEMPLATE app`".into(),
                    ));
                }
                Ok((&items[..i], Some((&items[i + 1], &items[i + 2]))))
            }
        }
    }
}

/// A MERKLE content hash over the tree: a `u64` per node computed bottom-up, so structurally-equal
/// subtrees get the same hash wherever they appear (identity by content, not position — the git /
/// Unison move). It is the substrate for O(1) subtree-equality filtering, stable content-derived node
/// ids (the design doc's §8 open question), and clone detection.
///
/// Hashing matches [`super::tree_eq`] exactly, so `hash(a) == hash(b)` whenever `tree_eq(a, b)`:
/// - a leaf hashes its VALUE AS WRITTEN — an `Int` includes its radix (`42` and `0x2A` differ, as
///   they do under `tree_eq`); names hash their text (NO α-equivalence — `(let ((x …)) x)` and
///   `(let ((y …)) y)` differ, since binding/scope is the compiler's domain, not this layer's).
/// - a list hashes a tag plus its children's hashes in order.
///
/// The hash is TRUNCATED to 64 bits, so a collision is possible; every equality *decision* built on
/// it (clone classes) re-verifies with `tree_eq`. The hash is a fast filter, never the final word.
pub mod hash {
    use super::Tree;
    use crate::ast::{Leaf, Radix};
    use sha2::{Digest, Sha256};

    // Domain-separation tags, so an atom can never collide with a list and leaf kinds stay distinct.
    const TAG_LIST: u8 = 0x00;
    const TAG_INT: u8 = 0x01;
    const TAG_FLOAT: u8 = 0x02;
    const TAG_STR: u8 = 0x03;
    const TAG_BOOL: u8 = 0x04;
    const TAG_NAME: u8 = 0x05;
    const TAG_BYTES: u8 = 0x06;
    const TAG_BAD_ESCAPE: u8 = 0x07;
    const TAG_CHAR: u8 = 0x08;
    const TAG_BAD_CHAR: u8 = 0x09;
    const TAG_SYM: u8 = 0x0a;
    const TAG_SUFFIXED: u8 = 0x0b;
    // Non-finite float VALUES — a distinct digest tag each (NaN payloadless; infinity carries a sign
    // byte, mirroring TAG_BOOL), so the content hash separates NaN, +∞ and −∞ from each other and from
    // any finite float. Independent of the codec's kind tags (this is a separate digest scheme).
    const TAG_FLOAT_NAN: u8 = 0x0c;
    const TAG_FLOAT_INF: u8 = 0x0d;
    // Native compound HEAD leaves (M2) — a distinct digest tag each; a `Ctor` also folds its constructor
    // discriminant so a list vs tuple vs record vs map vs set head hashes distinctly. FieldPair/Member
    // are payloadless.
    const TAG_CTOR: u8 = 0x0e;
    const TAG_FIELD_PAIR: u8 = 0x0f;
    const TAG_MEMBER: u8 = 0x10;
    // The rational-literal HEAD leaf (seq-204) — a payloadless tag, its own distinct digest tag so a
    // rational node hashes distinctly (its num/den children hash via the List arm as ordinary Int leaves).
    const TAG_RATIONAL: u8 = 0x11;

    /// The 64-bit content hash of `t` (first 8 bytes of the SHA-256 Merkle digest, big-endian).
    pub fn hash_tree(t: &Tree) -> u64 {
        let d = digest(t);
        u64::from_be_bytes(d[..8].try_into().expect("sha256 is 32 bytes"))
    }

    /// The full 32-byte digest of `t` (used internally; `hash_tree` truncates it).
    fn digest(t: &Tree) -> [u8; 32] {
        let mut h = Sha256::new();
        match t {
            Tree::Atom(leaf, _) => hash_leaf(&mut h, leaf),
            Tree::List(items, _) => {
                h.update([TAG_LIST]);
                // Length-prefix so `(a (b c))` and `(a b c)` can't alias via child boundaries.
                h.update((items.len() as u64).to_be_bytes());
                for c in items {
                    h.update(digest(c));
                }
            }
        }
        h.finalize().into()
    }

    fn hash_leaf(h: &mut Sha256, leaf: &Leaf) {
        match leaf {
            Leaf::Int { value, radix } => {
                h.update([TAG_INT, radix_byte(*radix)]);
                let bytes = value.to_bigint().to_signed_bytes_le();
                h.update((bytes.len() as u64).to_be_bytes());
                h.update(&bytes);
            }
            Leaf::Float(d) => {
                h.update([TAG_FLOAT, d.negative as u8]);
                h.update(d.exponent.to_be_bytes());
                let sig = crate::ast::IntValue {
                    negative: false,
                    magnitude: d.significand.clone(),
                }
                .to_bigint()
                .to_signed_bytes_le();
                h.update((sig.len() as u64).to_be_bytes());
                h.update(&sig);
            }
            // Non-finite float values — payloadless NaN, sign-tagged infinity.
            Leaf::FloatNan => h.update([TAG_FLOAT_NAN]),
            Leaf::FloatInf { negative } => h.update([TAG_FLOAT_INF, *negative as u8]),
            Leaf::Str(s) => update_bytes(h, TAG_STR, s.as_bytes()),
            Leaf::Bytes(b) => update_bytes(h, TAG_BYTES, b),
            Leaf::Bool(b) => h.update([TAG_BOOL, *b as u8]),
            Leaf::Sym(s) => update_bytes(h, TAG_SYM, s.as_bytes()),
            Leaf::Name(n) => update_bytes(h, TAG_NAME, n.as_bytes()),
            Leaf::BadEscape(c) => {
                let mut buf = [0u8; 4];
                update_bytes(h, TAG_BAD_ESCAPE, c.encode_utf8(&mut buf).as_bytes());
            }
            Leaf::Char(c) => {
                let mut buf = [0u8; 4];
                update_bytes(h, TAG_CHAR, c.encode_utf8(&mut buf).as_bytes());
            }
            Leaf::BadChar(s) => update_bytes(h, TAG_BAD_CHAR, s.as_bytes()),
            // A TYPE-SUFFIXED literal: the suffix kind byte, then the body hashed as its bare int/float
            // would be — so `100N` hashes distinctly from a bare `100` yet stably across runs.
            Leaf::Suffixed { value, kind } => {
                h.update([TAG_SUFFIXED, *kind as u8]);
                match value {
                    crate::ast::SuffixBody::Int { value, radix } => {
                        h.update([TAG_INT, radix_byte(*radix)]);
                        let bytes = value.to_bigint().to_signed_bytes_le();
                        h.update((bytes.len() as u64).to_be_bytes());
                        h.update(&bytes);
                    }
                    crate::ast::SuffixBody::Float(d) => {
                        h.update([TAG_FLOAT, d.negative as u8]);
                        h.update(d.exponent.to_be_bytes());
                        let sig = crate::ast::IntValue {
                            negative: false,
                            magnitude: d.significand.clone(),
                        }
                        .to_bigint()
                        .to_signed_bytes_le();
                        h.update((sig.len() as u64).to_be_bytes());
                        h.update(&sig);
                    }
                }
            }
            // Native compound HEAD leaves (M2): Ctor folds its constructor discriminant; the marker
            // leaves are payloadless.
            Leaf::Ctor(c) => h.update([TAG_CTOR, *c as u8]),
            Leaf::FieldPair => h.update([TAG_FIELD_PAIR]),
            Leaf::Member => h.update([TAG_MEMBER]),
            Leaf::Rational => h.update([TAG_RATIONAL]),
        }
    }

    fn update_bytes(h: &mut Sha256, tag: u8, bytes: &[u8]) {
        h.update([tag]);
        h.update((bytes.len() as u64).to_be_bytes());
        h.update(bytes);
    }

    fn radix_byte(r: Radix) -> u8 {
        match r {
            Radix::Dec => 0,
            Radix::Hex => 1,
            Radix::Bin => 2,
        }
    }

    /// The number of nodes in `t` (atoms + lists) — the "size" used as a clone floor and to rank
    /// clone classes.
    pub fn node_size(t: &Tree) -> usize {
        match t {
            Tree::Atom(_, _) => 1,
            Tree::List(items, _) => 1 + items.iter().map(node_size).sum::<usize>(),
        }
    }

    // ---- shape hash (for NEAR-clone bucketing) ----

    const TAG_HOLE: u8 = 0x10;
    const TAG_HEAD: u8 = 0x11;

    /// The 64-bit SHAPE hash of `t`: like [`hash_tree`], but every LEAF is a hole EXCEPT a list's head
    /// name (its first child, when a name). So it buckets subtrees that share a skeleton of constructs
    /// and differ only in their operands — the candidate set for near-clone (Type-2) detection.
    /// `(scale x 2)` and `(scale y 3)` share a shape (`(scale _ _)`); `(scale …)` and `(shift …)` do
    /// not (different head).
    pub fn shape_hash(t: &Tree) -> u64 {
        let d = shape_digest(t);
        u64::from_be_bytes(d[..8].try_into().expect("sha256 is 32 bytes"))
    }

    fn shape_digest(t: &Tree) -> [u8; 32] {
        let mut h = Sha256::new();
        match t {
            // A standalone atom carries no shape — it is a hole. (A head name is hashed by its parent
            // list below, so it never reaches here as a standalone atom.)
            Tree::Atom(_, _) => h.update([TAG_HOLE]),
            Tree::List(items, _) => {
                h.update([TAG_LIST]);
                h.update((items.len() as u64).to_be_bytes());
                for (i, child) in items.iter().enumerate() {
                    match (i, child) {
                        // Preserve the head name so `(+ …)` and `(* …)` stay distinct shapes.
                        (0, Tree::Atom(Leaf::Name(n), _)) => {
                            h.update([TAG_HEAD]);
                            h.update((n.len() as u64).to_be_bytes());
                            h.update(n.as_bytes());
                        }
                        _ => h.update(shape_digest(child)),
                    }
                }
            }
        }
        h.finalize().into()
    }
}

/// ANTI-UNIFICATION — the inverse of the matcher. Given a set of concrete subtrees, compute their
/// least-general generalization: a pattern that matches them all, with a fresh metavariable wherever
/// they differ. Where [`crate::query`] matching goes pattern → instances, this goes instances →
/// pattern, so its output is a `,x`-metavariable pattern feedable straight back into `search`/
/// `rewrite`. It is the engine behind near-clone (Type-2) detection.
pub mod antiunify {
    use super::hash::hash_tree;
    use super::{Tree, tree_eq};
    use crate::ast::Leaf;
    use std::collections::HashMap;

    /// The result of anti-unifying N instances.
    #[derive(Clone, Debug)]
    pub struct Generalization {
        /// The pattern tree: the shared skeleton with `(unquote mK)` holes where instances differ.
        pub pattern: Tree,
        /// For each hole `mK` (index = K), the subtree each instance had there — `holes[k][i]` is
        /// instance `i`'s subtree at hole `k`. All inner vecs have length = number of instances.
        pub holes: Vec<Vec<Tree>>,
    }

    /// Anti-unify `instances` (≥1). Recurses positionally: equal sub-parts stay literal; a divergence
    /// becomes a hole. Holes are SHARED — two positions whose per-instance subtrees are element-wise
    /// equal get the SAME metavariable (so the emitted pattern's repeated-metavar consistency exactly
    /// re-captures the instances). With one instance, the result is that instance with no holes.
    pub fn anti_unify(instances: &[&Tree]) -> Generalization {
        assert!(!instances.is_empty(), "anti_unify needs ≥1 instance");
        // A hole is keyed by the vector of content hashes of its per-instance subtrees, so identical
        // difference-columns collapse to one metavariable.
        let mut holes: Vec<Vec<Tree>> = Vec::new();
        let mut by_key: HashMap<Vec<u64>, usize> = HashMap::new();
        let pattern = build(instances, &mut holes, &mut by_key);
        Generalization { pattern, holes }
    }

    /// Recursively generalize the `i`-th slice of instances at one position.
    fn build(
        instances: &[&Tree],
        holes: &mut Vec<Vec<Tree>>,
        by_key: &mut HashMap<Vec<u64>, usize>,
    ) -> Tree {
        let first = instances[0];
        // All equal ⇒ keep the literal subtree (no hole).
        if instances[1..].iter().all(|t| tree_eq(first, t)) {
            return strip(first);
        }
        // Same list head + same arity across ALL instances ⇒ recurse position-wise.
        if let Some(arity) = common_list_shape(instances) {
            let mut kids = Vec::with_capacity(arity);
            for k in 0..arity {
                let col: Vec<&Tree> = instances.iter().map(|t| child(t, k)).collect();
                kids.push(build(&col, holes, by_key));
            }
            return Tree::List(kids, None);
        }
        // Otherwise a divergence ⇒ a (possibly shared) hole.
        let key: Vec<u64> = instances.iter().map(|t| hash_tree(t)).collect();
        let idx = *by_key.entry(key).or_insert_with(|| {
            let idx = holes.len();
            holes.push(instances.iter().map(|t| strip(t)).collect());
            idx
        });
        metavar(idx)
    }

    /// If every instance is a list with the SAME head name and SAME arity, that arity; else `None`.
    fn common_list_shape(instances: &[&Tree]) -> Option<usize> {
        let (head0, arity0) = list_head_arity(instances[0])?;
        for t in &instances[1..] {
            let (h, a) = list_head_arity(t)?;
            if h != head0 || a != arity0 {
                return None;
            }
        }
        Some(arity0)
    }

    fn list_head_arity(t: &Tree) -> Option<(Option<&str>, usize)> {
        match t {
            Tree::List(items, _) => Some((items.first().and_then(as_name), items.len())),
            _ => None,
        }
    }

    fn as_name(t: &Tree) -> Option<&str> {
        match t {
            Tree::Atom(Leaf::Name(n), _) => Some(n),
            _ => None,
        }
    }

    fn child(t: &Tree, k: usize) -> &Tree {
        match t {
            Tree::List(items, _) => &items[k],
            _ => unreachable!("child() only called on lists of known arity"),
        }
    }

    /// A metavariable name for hole `idx`: `m0`, `m1`, … (bound by convention; the head is a name).
    pub fn metavar_name(idx: usize) -> String {
        format!("m{idx}")
    }

    /// A `(unquote mK)` metavariable node.
    fn metavar(idx: usize) -> Tree {
        Tree::List(
            vec![
                Tree::Atom(Leaf::Name("unquote".into()), None),
                Tree::Atom(Leaf::Name(metavar_name(idx).into()), None),
            ],
            None,
        )
    }

    /// Deep-copy `t` dropping provenance (the pattern is a fresh synthetic tree).
    fn strip(t: &Tree) -> Tree {
        match t {
            Tree::Atom(l, _) => Tree::Atom(l.clone(), None),
            Tree::List(items, _) => Tree::List(items.iter().map(strip).collect(), None),
        }
    }

    /// Render a generalization's pattern as readable s-expression text with `,mK` sugar for holes
    /// (instead of the explicit `(unquote mK)`), so it reads as — and is — a pattern for `rewrite`.
    pub fn render_pattern(t: &Tree) -> String {
        // A hole `(unquote mK)` prints as `,mK`; everything else prints normally via a fresh arena.
        if let Some(name) = as_metavar_name(t) {
            return format!(",{name}");
        }
        match t {
            Tree::Atom(_, _) => t.to_sexpr(),
            Tree::List(items, _) => {
                let parts: Vec<String> = items.iter().map(render_pattern).collect();
                format!("({})", parts.join(" "))
            }
        }
    }

    /// If `t` is a `(unquote NAME)` hole, its NAME.
    fn as_metavar_name(t: &Tree) -> Option<&str> {
        match t {
            Tree::List(items, _) => match items.as_slice() {
                [h, n] if as_name(h) == Some("unquote") => as_name(n),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Exact CLONE DETECTION: find subtrees that recur verbatim across a program (or a codebase). Groups
/// every subtree by its content [`hash`] and reports each class of ≥2 structurally-identical members
/// — the copy-paste an agent would extract into a shared definition. Purely structural (no
/// α-equivalence / semantics — that is the compiler's domain).
pub mod clones {
    use super::antiunify::{anti_unify, render_pattern};
    use super::hash::{hash_tree, node_size, shape_hash};
    use super::{Tree, tree_eq};
    use crate::span::Span;
    use crate::spans::SpanTable;
    use std::collections::HashMap;

    /// One occurrence of a cloned subtree.
    #[derive(Clone, Debug)]
    pub struct CloneSite {
        /// The source this occurrence came from (a file path / `(stdin)`), if the caller supplied one.
        pub file: Option<String>,
        pub node: Tree,
        pub span: Option<Span>,
    }

    /// A class of ≥2 structurally-identical subtrees. `exemplar` is one member's rendered s-expr;
    /// `size` is its node count; `sites` are all the occurrences (≥2).
    #[derive(Clone, Debug)]
    pub struct CloneClass {
        pub exemplar: String,
        pub size: usize,
        pub sites: Vec<CloneSite>,
    }

    /// One source to scan for clones: its tree, an optional span table, and an optional file label.
    pub struct Source<'a> {
        pub tree: &'a Tree,
        pub spans: Option<&'a SpanTable>,
        pub file: Option<String>,
    }

    /// Find clone classes in a single `subject`. Convenience wrapper over [`find_clones_multi`].
    pub fn find_clones(
        subject: &Tree,
        min_size: usize,
        spans: Option<&SpanTable>,
    ) -> Vec<CloneClass> {
        find_clones_multi(
            &[Source {
                tree: subject,
                spans,
                file: None,
            }],
            min_size,
        )
    }

    /// Find clone classes ACROSS all `sources` whose subtree has at least `min_size` nodes — clones
    /// may span files. A class is a set of subtrees with equal content hash AND (verified) `tree_eq`
    /// (the hash is a fast filter; `tree_eq` is the collision-safe decision).
    ///
    /// Only MAXIMAL clones are reported: once a cloned subtree is found, its inner clones are NOT
    /// reported separately (the walk stops descending), so `(f (g x))` recurring reports the whole
    /// `(f (g x))`, not also its `(g x)`/`x` parts. (Consequence: a subtree that recurs BOTH inside a
    /// larger clone and standalone reports only its maximal occurrences — the larger clone is the more
    /// useful signal.)
    ///
    /// Classes are returned largest-first (subtree size, then occurrence count) — biggest duplication
    /// first.
    pub fn find_clones_multi(sources: &[Source], min_size: usize) -> Vec<CloneClass> {
        // Pass 1: frequency of every subtree hash across ALL sources.
        let mut freq: HashMap<u64, usize> = HashMap::new();
        for src in sources {
            count(src.tree, min_size, &mut freq);
        }
        // Pass 2 (top-down, maximal): record each maximal recurring subtree, tagged with its source.
        let mut occ: Vec<(usize, &Tree)> = Vec::new();
        for (si, src) in sources.iter().enumerate() {
            collect_maximal(src.tree, si, min_size, &freq, &mut occ);
        }
        // Group by hash, verify each bucket by `tree_eq`, keep classes with ≥2 members.
        let mut by_hash: HashMap<u64, Vec<(usize, &Tree)>> = HashMap::new();
        for (si, t) in occ {
            by_hash.entry(hash_tree(t)).or_default().push((si, t));
        }
        let mut out: Vec<CloneClass> = Vec::new();
        for group in by_hash.values() {
            for members in split_by_eq(group) {
                if members.len() >= 2 {
                    out.push(CloneClass {
                        exemplar: members[0].1.to_sexpr(),
                        size: node_size(members[0].1),
                        sites: members
                            .iter()
                            .map(|(si, t)| {
                                let src = &sources[*si];
                                CloneSite {
                                    file: src.file.clone(),
                                    node: (*t).clone(),
                                    span: t
                                        .origin()
                                        .and_then(|id| src.spans.and_then(|s| s.get(id))),
                                }
                            })
                            .collect(),
                    });
                }
            }
        }
        // Largest subtree first; ties → more occurrences, then exemplar text (deterministic).
        out.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then(b.sites.len().cmp(&a.sites.len()))
                .then(a.exemplar.cmp(&b.exemplar))
        });
        out
    }

    /// Pass 1: tally each subtree hash (size ≥ `min_size`).
    fn count(node: &Tree, min_size: usize, freq: &mut HashMap<u64, usize>) {
        if node_size(node) >= min_size {
            *freq.entry(hash_tree(node)).or_insert(0) += 1;
        }
        if let Tree::List(items, _) = node {
            for c in items {
                count(c, min_size, freq);
            }
        }
    }

    /// Pass 2: record maximal clone occurrences (tagged with source index `si`). A node whose hash
    /// recurs (`freq ≥ 2`) is recorded and NOT descended into; otherwise descend into children.
    fn collect_maximal<'t>(
        node: &'t Tree,
        si: usize,
        min_size: usize,
        freq: &HashMap<u64, usize>,
        occ: &mut Vec<(usize, &'t Tree)>,
    ) {
        if node_size(node) >= min_size && freq.get(&hash_tree(node)).copied().unwrap_or(0) >= 2 {
            occ.push((si, node));
            return; // maximal: don't report clones nested inside this one
        }
        if let Tree::List(items, _) = node {
            for c in items {
                collect_maximal(c, si, min_size, freq, occ);
            }
        }
    }

    /// Partition a hash-bucket into verified-equal sub-groups (a collision splits into >1 group).
    fn split_by_eq<'t>(group: &[(usize, &'t Tree)]) -> Vec<Vec<(usize, &'t Tree)>> {
        let mut groups: Vec<Vec<(usize, &'t Tree)>> = Vec::new();
        for &(si, t) in group {
            match groups.iter_mut().find(|g| tree_eq(g[0].1, t)) {
                Some(g) => g.push((si, t)),
                None => groups.push(vec![(si, t)]),
            }
        }
        groups
    }

    // ---- near-clone (Type-2) detection via anti-unification ----

    /// A class of ≥2 subtrees that share a SHAPE but differ in some leaves. The inferred `pattern`
    /// (its `,mK`-metavariable form) matches every member; `holes[k]` is the per-site subtree at hole
    /// `k` (so `holes[k].len() == sites.len()`).
    #[derive(Clone, Debug)]
    pub struct NearCloneClass {
        /// The anti-unified pattern, rendered with `,mK` sugar — feedable straight into `rewrite`.
        pub pattern: String,
        /// Node count of the pattern skeleton (holes count as one node each) — the ranking size.
        pub size: usize,
        /// Number of holes (distinct metavariables) — how much the sites differ.
        pub hole_count: usize,
        pub sites: Vec<CloneSite>,
    }

    /// Find near-clone classes across `sources`: subtrees that share a skeleton (same [`shape_hash`])
    /// and generalize — via [`anti_unify`] — to a pattern with ≥1 hole. Only classes with ≥2 members,
    /// ≥1 hole (a 0-hole class is an EXACT clone, reported by [`find_clones_multi`] instead), and a
    /// skeleton of ≥ `min_size` nodes are kept. Maximal (top-down, don't descend into a reported
    /// near-clone). Ranked largest-first, then more occurrences.
    ///
    /// The emitted `pattern` is the inverse of the matcher: it re-matches every member, so it can be
    /// handed to `rewrite` to factor the sites into one call.
    pub fn find_near_clones(sources: &[Source], min_size: usize) -> Vec<NearCloneClass> {
        // Pass 1: shape-hash frequency across all sources.
        let mut freq: HashMap<u64, usize> = HashMap::new();
        for src in sources {
            shape_count(src.tree, min_size, &mut freq);
        }
        // Pass 2 (top-down, maximal): record subtrees whose SHAPE recurs.
        let mut occ: Vec<(usize, &Tree)> = Vec::new();
        for (si, src) in sources.iter().enumerate() {
            shape_collect_maximal(src.tree, si, min_size, &freq, &mut occ);
        }
        // Bucket by shape hash; each bucket is one candidate near-clone class.
        let mut by_shape: HashMap<u64, Vec<(usize, &Tree)>> = HashMap::new();
        for (si, t) in occ {
            by_shape.entry(shape_hash(t)).or_default().push((si, t));
        }
        let mut out: Vec<NearCloneClass> = Vec::new();
        for members in by_shape.values() {
            if members.len() < 2 {
                continue;
            }
            let trees: Vec<&Tree> = members.iter().map(|(_, t)| *t).collect();
            let g = anti_unify(&trees);
            // A 0-hole generalization means the members are exactly equal — that's an EXACT clone,
            // not a near-clone; skip it (find_clones reports those).
            if g.holes.is_empty() {
                continue;
            }
            out.push(NearCloneClass {
                pattern: render_pattern(&g.pattern),
                size: node_size(&g.pattern),
                hole_count: g.holes.len(),
                sites: members
                    .iter()
                    .map(|(si, t)| {
                        let src = &sources[*si];
                        CloneSite {
                            file: src.file.clone(),
                            node: (*t).clone(),
                            span: t.origin().and_then(|id| src.spans.and_then(|s| s.get(id))),
                        }
                    })
                    .collect(),
            });
        }
        out.sort_by(|a, b| {
            b.size
                .cmp(&a.size)
                .then(b.sites.len().cmp(&a.sites.len()))
                .then(a.pattern.cmp(&b.pattern))
        });
        out
    }

    /// Single-source near-clone convenience wrapper.
    pub fn find_near_clones_one(
        subject: &Tree,
        min_size: usize,
        spans: Option<&SpanTable>,
    ) -> Vec<NearCloneClass> {
        find_near_clones(
            &[Source {
                tree: subject,
                spans,
                file: None,
            }],
            min_size,
        )
    }

    /// Pass 1 for near-clones: tally each subtree SHAPE hash (size ≥ `min_size`).
    fn shape_count(node: &Tree, min_size: usize, freq: &mut HashMap<u64, usize>) {
        if node_size(node) >= min_size {
            *freq.entry(shape_hash(node)).or_insert(0) += 1;
        }
        if let Tree::List(items, _) = node {
            for c in items {
                shape_count(c, min_size, freq);
            }
        }
    }

    /// Pass 2 for near-clones: record maximal subtrees whose shape recurs; don't descend into them.
    fn shape_collect_maximal<'t>(
        node: &'t Tree,
        si: usize,
        min_size: usize,
        freq: &HashMap<u64, usize>,
        occ: &mut Vec<(usize, &'t Tree)>,
    ) {
        if node_size(node) >= min_size && freq.get(&shape_hash(node)).copied().unwrap_or(0) >= 2 {
            occ.push((si, node));
            return;
        }
        if let Tree::List(items, _) = node {
            for c in items {
                shape_collect_maximal(c, si, min_size, freq, occ);
            }
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

    #[test]
    fn tree_from_arena_and_to_arena_are_iterative_on_a_deep_arena() {
        // `Tree::of`/`from_arena` is the FIRST thing every codemod op (`cdz query`/`rewrite`/`lint`/
        // `clones`) does to a subject arena, and `to_arena`/`build` is the dual on the way out. Both were
        // native-recursive, and `codec::decode` accepts arbitrarily-deep valid-tree arenas (no cap,
        // unlike the reader's MAX_NESTING_DEPTH), so a deep subject overflowed the stack (SIGABRT) before
        // any matching even ran. Build a 100k-deep chain (past any native-stack limit) and assert the
        // round-trip arena → Tree → arena completes without overflow and preserves the tree (byte-equal
        // via codec::encode, itself a flat loop — the arena is already canonical so re-encode is stable).
        // Run on a 64 MB stack: `Tree::of`/`to_arena` are iterative (that's what this pins), but the deep
        // `Tree`/`Builder` structures this test BUILDS then DROPS are native-recursive on drop (a 100k-deep
        // nested chain drops one frame per level), which SIGABRTs a default ~2 MB test-worker stack before
        // the assertion — the same reason the sibling ML-printer deep test uses a big-stack worker.
        // (`run_with_compiler_stack` lives in rcdzc, not reachable here; spawn an inline big-stack thread +
        // resume_unwind so an assertion failure inside still fails the test.)
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let depth = 100_000usize;
                let mut b = Builder::new();
                let mut cur = b.name("x");
                for _ in 0..depth {
                    cur = b.list(vec![cur]);
                }
                let a = b.finish(cur);
                let a_bytes = crate::codec::encode(&a);

                let t = Tree::of(&a); // from_arena: must NOT overflow
                let back = t.to_arena(); // build: must NOT overflow
                assert_eq!(
                    crate::codec::encode(&back),
                    a_bytes,
                    "arena -> Tree -> arena preserves the deep tree"
                );
            })
            .expect("spawn big-stack arena-roundtrip worker");
        if let Err(p) = h.join() {
            std::panic::resume_unwind(p);
        }
    }

    #[test]
    fn a_malformed_json_load_reports_line_col_not_a_byte_offset() {
        // `query`/`rewrite` over a `.json` renders a parse error as `line:col` (like the convert path),
        // not a raw `at byte N` — a multi-line document points at a navigable place.
        let err = driver::load(b"{\n  \"a\": ,\n}", crate::convert::Format::Json).unwrap_err();
        assert!(
            err.contains(" at 2:") && !err.contains("byte"),
            "a malformed JSON query error must be line:col, not a byte offset; got {err}"
        );
    }

    #[test]
    fn a_malformed_sexpr_load_reports_line_col_not_a_byte_offset() {
        // The s-expr twin of the JSON case: `query`/`rewrite`/`clones`/`diff` over a malformed MULTI-LINE
        // `.sexp` render the position as `line:col`, not the raw `at byte N` this arm used to leak (it was
        // the last s-expr reader-error render still passing `e.0` verbatim; `cdz check` already mapped it).
        // The trailing `)` on line 2 is an `unexpected ')' at byte N` the multi-form fallback surfaces.
        let err = driver::load(b"(module m)\n(x))", crate::convert::Format::Sexpr).unwrap_err();
        assert!(
            err.contains(" at 2:") && !err.contains("byte"),
            "a malformed s-expr query error must be line:col, not a byte offset; got {err}"
        );
    }

    #[test]
    fn rewrite_is_iterative_not_recursive_on_a_deep_subject() {
        // `rewrite_node` descends the ENTIRE subject (both strategies), and a subject can be a decoded
        // arbitrarily-deep arena (from_arena is now iterative, so a deep subject BUILDS — and then this
        // walk must not overflow). A native-recursive rewrite overflowed the stack (SIGABRT) on a deep
        // subject via `cdz rewrite`. Assert a rewrite over a 100k-deep chain completes without overflow,
        // for a rule that matches at EVERY level (max work) and one that matches nowhere (pure descent).
        // Run on a 64 MB stack: `rewrite_node`/`Tree::of` are iterative (that's what this pins), but the
        // 100k-deep `Tree`/`Builder` structures this BUILDS then DROPS recurse one native frame per level
        // on Drop, SIGABRTing a default ~2 MB test-worker stack — same idiom as the sibling from_arena +
        // ml_print deep tests (run_with_compiler_stack is rcdzc-only; resume_unwind so an inner assert fails).
        let h = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let depth = 100_000usize;
                let mut b = Builder::new();
                let mut cur = b.name("x");
                for _ in 0..depth {
                    cur = b.list(vec![cur]);
                }
                let a = b.finish(cur);
                let t = Tree::of(&a);
                // No-match rule: pure full-depth descent, nothing rewritten.
                let none = rewrite(&pat("(nomatch ,x)"), &tmpl(",x"), &t);
                assert_eq!(none.count, 0, "a non-matching rule rewrites nothing");
                assert!(
                    tree_eq(&none.tree, &t),
                    "a no-op rewrite returns the subject unchanged"
                );
                // Match-every-level rule: `(,c)` (a 1-element list) matches every List node in the chain;
                // unwrap it to its child. Bottom-up, every one of the `depth` list levels fires — maximal
                // work, deepest stack. Must complete without overflow; the chain collapses to the leaf `x`.
                let all = rewrite(&pat("(,c)"), &tmpl(",c"), &t);
                assert_eq!(all.count, depth, "every list level is rewritten");
                assert!(
                    matches!(all.tree, Tree::Atom(Leaf::Name(ref n), _) if &**n == "x"),
                    "chain collapses to x"
                );
            })
            .expect("spawn big-stack rewrite worker");
        if let Err(p) = h.join() {
            std::panic::resume_unwind(p);
        }
    }

    fn pat(src: &str) -> Pattern {
        Pattern::compile(src).unwrap_or_else(|e| panic!("pattern {src:?}: {e}"))
    }

    fn tmpl(src: &str) -> Template {
        Template::compile(src).unwrap_or_else(|e| panic!("template {src:?}: {e}"))
    }

    /// A tiny deterministic PRNG (SplitMix64) — reproducible fuzz without a dependency, matching the
    /// lexer/parser/sexpr house style (the crate stays "plain").
    struct SplitMix64(u64);
    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
    }

    #[test]
    fn pattern_compile_and_match_never_panic_on_arbitrary_input() {
        // `Pattern`/`Template`/`RuleSet::compile` parse UNTRUSTED text (`cdz query PATTERN`, `--rule …`,
        // `--rules FILE`), so they must return a diagnostic, never panic — on any string, including
        // metavariable/splice/guard soup (`,x`, `,@xs`, `,_`, `,(x GUARD)`), unbalanced parens, and the
        // adjacent-splice/misplaced-splice/bad-guard cases these compilers explicitly reject. And a
        // pattern that DOES compile must MATCH against a subject without panicking (the match walk over
        // an arbitrary compiled pattern + a fixed subject). No result is asserted — the point is total,
        // panic-free compile + match, mirroring the reader/printer robustness fuzzes.
        let subject = subj("(f (g 1) (h a b) (nested (deep x)) 2 3)");
        let alphabet: Vec<char> = "(),@_ x0123456789+*-.\"#abchisltrue-literalhead-is`|:"
            .chars()
            .collect();
        let mut rng = SplitMix64(0x9e3d_71a5_c0de_0f3e);
        for len in 0..=28usize {
            for _ in 0..120 {
                let s: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                // Each compiler must return Ok/Err, never panic.
                if let Ok(p) = Pattern::compile(&s) {
                    // A compiled pattern must match without panicking (result unimportant).
                    let mut binds = Bindings::default();
                    let _ = p.matches(&subject, &mut binds);
                }
                let _ = Template::compile(&s);
                let _ = RuleSet::compile(&s);
            }
        }
        // Structural edge cases these compilers specifically reason about — all must be Err/Ok, no panic.
        for s in [
            ",",
            ",,",
            ",@",
            ",@,@",
            "(,@a ,@b)",
            "( ,@x )",
            ",()",
            ",(x)",
            ",(x bad-guard)",
            ",(x (head-is))",
            ",( )",
            "()",
            "(())",
            "((((",
            ",_",
            ",@_",
            "(a ,@x ,@y b)",
        ] {
            let _ = Pattern::compile(s);
            let _ = Template::compile(s);
            let _ = RuleSet::compile(s);
        }
    }

    #[test]
    fn apply_rewrite_never_panics_on_arbitrary_pattern_template_pairs() {
        // The FULL rewrite transaction on arbitrary (pattern, template) pairs — the loop the pattern-
        // compile fuzz above does not exercise: compile both → build a RuleSet → `driver::apply_rewrite`
        // over a fixed target (match → instantiate the template from the bindings → re-parse-VALIDATE →
        // project). Every stage must be TOTAL: `apply_rewrite` returns Ok/Err (a rewrite that produces
        // ill-formed text is a clean `Err`, never a panic), never crashes. This guards the instantiate +
        // validated-transaction path against a metavar/splice-arity or projection edge on odd inputs.
        let (target, _) = driver::load(
            b"(f (g 1) (h a b) (+ x 0) 2)",
            crate::convert::Format::Sexpr,
        )
        .unwrap();
        let alphabet: Vec<char> = "(),@_ x0123456789+*-fgh".chars().collect();
        let mut rng = SplitMix64(0x71e_5ec0_de0f_a5f1);
        for len in 0..=20usize {
            for _ in 0..100 {
                let ps: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                let ts: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                if let (Ok(p), Ok(t)) = (Pattern::compile(&ps), Template::compile(&ts)) {
                    let rules = RuleSet::new(vec![Rule::new(p, t)]);
                    // Must not panic — Ok(outcome) or Err(reject message), at a few widths + fixpoint.
                    for fixpoint in [false, true] {
                        let _ = driver::apply_rewrite(
                            &rules,
                            Strategy::BottomUp,
                            &target,
                            crate::convert::Format::Sexpr,
                            40,
                            fixpoint,
                        );
                    }
                }
            }
        }
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
    fn adjacent_splices_are_rejected() {
        // Two directly-adjacent splices have no anchor to divide the run — still rejected.
        let e = Pattern::compile("(f ,@a ,@b)").unwrap_err();
        assert!(e.0.contains("adjacent"), "got {e}");
    }

    #[test]
    fn two_splices_around_a_fixed_anchor_delete_a_clause() {
        // The clause-delete idiom (ask-88): `(F ,@before TARGET ,@after)` matches a target sitting
        // ANYWHERE in a variadic form, binding the runs on either side.
        let s = subj("(case a b (needs x) c d)");
        let m = search(&pat("(case ,@before (needs ,_) ,@after)"), &s, None);
        assert_eq!(m.len(), 1);
        let before: Vec<_> = m[0]
            .bindings
            .get_run("before")
            .unwrap()
            .iter()
            .map(|t| t.to_sexpr())
            .collect();
        let after: Vec<_> = m[0]
            .bindings
            .get_run("after")
            .unwrap()
            .iter()
            .map(|t| t.to_sexpr())
            .collect();
        assert_eq!(before, ["a", "b"]);
        assert_eq!(after, ["c", "d"]);
    }

    #[test]
    fn two_splices_target_at_the_front_and_back() {
        // The anchor at position 0 (no `before`) and at the end (no `after`) both bind empty runs.
        let front = subj("(case (needs x) c d)");
        let m = search(&pat("(case ,@before (needs ,_) ,@after)"), &front, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].bindings.get_run("before").unwrap().len(), 0);
        assert_eq!(m[0].bindings.get_run("after").unwrap().len(), 2);

        let back = subj("(case a b (needs x))");
        let m = search(&pat("(case ,@before (needs ,_) ,@after)"), &back, None);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].bindings.get_run("before").unwrap().len(), 2);
        assert_eq!(m[0].bindings.get_run("after").unwrap().len(), 0);
    }

    #[test]
    fn two_splices_delete_rewrite_drops_the_clause() {
        // The full delete-a-clause rewrite: `(case ,@a (needs ,_) ,@b) → (case ,@a ,@b)`.
        let s = subj("(case a b (needs x) c d)");
        let r = rewrite(
            &pat("(case ,@a (needs ,_) ,@b)"),
            &tmpl("(case ,@a ,@b)"),
            &s,
        );
        assert_eq!(r.count, 1);
        assert_eq!(r.tree.to_sexpr(), "(case a b c d)");
    }

    #[test]
    fn delete_edit_matches_the_align_path_and_skips_the_wide_parent_diff() {
        // REGRESSION (perf): a DELETE fix's byte-edit was computed by diffing the target's whole PARENT
        // list (`localized_change` → `edits_preserving` → `align`), whose LCS alignment DP calls `tree_eq`
        // over O(children²) cells. For a WIDE parent (a `do` block / match with N children) that is O(N²)
        // per fix, so N delete fixes on one file (each discarded `do` statement is a CDZ0307 delete) were
        // O(N³) (`cdz` timing: a `do` of N discarded statements N=100/200/400 = 33/207/1639ms). The
        // `textedit::delete_edit` fast path emits the edit DIRECTLY from the known deleted span, in O(1) —
        // no parent diff, no alignment. This test pins that it is BYTE-IDENTICAL to the align path.
        //
        // Build a wide `(do c0 c1 … c{n-1})`, delete a MIDDLE child, and compare: (a) `delete_edit(src,
        // child_span)` vs (b) the full align path `edits_preserving(src, parent, parent_without_child)`.
        // The two edit lists must be identical for EVERY position — so the fast path never drifts from the
        // alignment it replaces.
        use crate::convert::Format;
        fn parent_without_child(parent: &Tree, drop_ix: usize) -> Tree {
            let Tree::List(items, o) = parent else {
                panic!("parent is a list")
            };
            let kept: Vec<Tree> = items
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != drop_ix)
                .map(|(_, t)| t.clone())
                .collect();
            Tree::List(kept, *o)
        }
        // Read WITH spans (the s-expr corpus surface) so both paths see the same span table.
        let n = 12usize;
        let mut src = String::from("(do");
        for i in 0..n {
            src.push_str(&format!(" (s {i})"));
        }
        src.push(')');
        let (arena, spans) = sexpr::read_spanned(&src).expect("spanned parse");
        let tree = Tree::of(&arena);
        let span_of = |t: &Tree| -> Option<(usize, usize)> {
            t.origin()
                .and_then(|id| spans.get(id))
                .map(|s| (s.start, s.end))
        };
        // The parent is the `(do …)` list; its children are the head `do` + n statements. Delete a MIDDLE
        // statement (child index 6 of the do-list = statement 5).
        let Tree::List(children, _) = &tree else {
            panic!("root is the do list")
        };
        let drop_ix = 6usize;
        let child = &children[drop_ix];
        let (cs, ce) = span_of(child).expect("child has a span");
        // (a) the fast path.
        let fast = textedit::delete_edit(&src, cs, ce, Format::Sexpr).expect("delete edit");
        // (b) the align path over the parent pair.
        let without = parent_without_child(&tree, drop_ix);
        let slow = textedit::edits_preserving(&src, &tree, &without, &span_of, Format::Sexpr);
        assert_eq!(
            slow.len(),
            1,
            "deleting one child yields exactly one edit via the align path"
        );
        assert_eq!(
            vec![fast],
            slow,
            "the delete fast path must produce the byte-identical edit the align path does (same \
             widened span, empty text) — a drift here means `cdz fix`/`check --json` would apply a \
             different edit than the alignment intended"
        );
    }

    #[test]
    fn delete_edit_returns_none_for_an_invalid_span() {
        use crate::convert::Format;
        // `delete_edit` guards BOTH invalid-span cases (its documented contract): a degenerate span
        // (`start > end`) and an out-of-bounds span (`end > src.len()`) each yield `None` rather than a
        // slice panic — so a caller passing a stale/miscomputed span degrades gracefully. A valid span
        // still produces an edit (the guard doesn't reject good input).
        let src = "(do a b)";
        // Degenerate: start past end.
        assert_eq!(
            textedit::delete_edit(src, 5, 3, Format::Sexpr),
            None,
            "start > end is degenerate → None"
        );
        // Out of bounds: end past the source length.
        assert_eq!(
            textedit::delete_edit(src, 1, src.len() + 1, Format::Sexpr),
            None,
            "end > src.len() is out of bounds → None"
        );
        // A valid span still yields an edit — the guard rejects only invalid input.
        assert!(
            textedit::delete_edit(src, 4, 5, Format::Sexpr).is_some(),
            "a valid in-bounds span produces a delete edit"
        );
    }

    #[test]
    fn three_splices_two_anchors() {
        // Three splices with two fixed anchors — the backtracker places each run.
        let s = subj("(f a X b c Y d)");
        let m = search(&pat("(f ,@p X ,@q Y ,@r)"), &s, None);
        assert_eq!(m.len(), 1);
        let run = |n: &str| -> Vec<String> {
            m[0].bindings
                .get_run(n)
                .unwrap()
                .iter()
                .map(|t| t.to_sexpr())
                .collect()
        };
        assert_eq!(run("p"), ["a"]);
        assert_eq!(run("q"), ["b", "c"]);
        assert_eq!(run("r"), ["d"]);
    }

    #[test]
    fn two_splices_backtrack_when_the_first_greedy_run_blocks_the_anchor() {
        // If the anchor (`X`) appears more than once, the run before it must stop at the FIRST so
        // the tail can still find its own anchor. Backtracking (not a single greedy grab) handles it.
        let s = subj("(f a X b X c)");
        // pattern needs: ,@p then X then ,@q then X then ,@r
        let m = search(&pat("(f ,@p X ,@q X ,@r)"), &s, None);
        assert_eq!(m.len(), 1);
        let run = |n: &str| -> Vec<String> {
            m[0].bindings
                .get_run(n)
                .unwrap()
                .iter()
                .map(|t| t.to_sexpr())
                .collect()
        };
        assert_eq!(run("p"), ["a"]);
        assert_eq!(run("q"), ["b"]);
        assert_eq!(run("r"), ["c"]);
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
        assert_eq!(
            count(&lit, &subj("(+ a 1)")),
            0,
            "a is a name, not a literal"
        );
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
    fn guard_is_irrefutable_accepts_var_tuple_record_rejects_literal_and_ctor() {
        // `(match ,s (,(p is-irrefutable) ,b))` — the arm pattern must be irrefutable.
        let p = pat("(match ,s (,(p is-irrefutable) ,b))");
        // var / `_` / tuple / record binders are irrefutable.
        assert_eq!(count(&p, &subj("(match v (x (g x)))")), 1, "var arm");
        assert_eq!(count(&p, &subj("(match v (_ 0))")), 1, "wildcard arm");
        assert_eq!(
            count(&p, &subj("(match v ((tuple a b) a))")),
            1,
            "tuple arm"
        );
        assert_eq!(
            count(&p, &subj("(match v ((record (x a)) a))")),
            1,
            "record arm"
        );
        // A sum-ctor pattern and a literal are refutable (no-context form), so the guard fails.
        assert_eq!(
            count(&p, &subj("(match v ((Some y) y))")),
            0,
            "ctor arm refutable"
        );
        assert_eq!(
            count(&p, &subj("(match v (0 x))")),
            0,
            "literal arm refutable"
        );
    }

    #[test]
    fn is_camel_case_classifies_only_lowercase_initial_interior_upper_no_underscore() {
        // camelCase → flagged.
        assert!(is_camel_case("fooBar"));
        assert!(is_camel_case("myFunc"));
        assert!(is_camel_case("aB"));
        assert!(is_camel_case("parseHTTPResponse"));
        // NOT camelCase: snake_case, all-lowercase, Ctor/type/Module (leading upper), all-caps CONST,
        // a synth `__name`, empty.
        assert!(!is_camel_case("good_name"));
        assert!(!is_camel_case("foo"));
        assert!(!is_camel_case("Ctor"));
        assert!(!is_camel_case("MyType"));
        assert!(!is_camel_case("CONST"));
        assert!(!is_camel_case("__synth"));
        assert!(!is_camel_case("with_Upper_after_underscore")); // has `_` → snake-ish, not flagged
        assert!(!is_camel_case(""));
    }

    #[test]
    fn guard_is_camel_case_matches_only_camel_names() {
        let p = pat("(def (,(n is-camel-case) ,@_) ,@_)");
        assert_eq!(
            count(&p, &subj("(def (myFunc x) x)")),
            1,
            "camelCase def name"
        );
        assert_eq!(
            count(&p, &subj("(def (good_name x) x)")),
            0,
            "snake_case not flagged"
        );
        assert_eq!(
            count(&p, &subj("(def (f x) x)")),
            0,
            "plain lowercase not flagged"
        );
    }

    #[test]
    fn list_depth_counts_nested_list_levels() {
        // An atom is depth 0; a flat list is 1; each nesting level adds one.
        assert_eq!(list_depth(&subj("x")), 0, "an atom is depth 0");
        assert_eq!(list_depth(&subj("(f a b)")), 1, "a flat call is depth 1");
        assert_eq!(
            list_depth(&subj("(f (g x))")),
            2,
            "one nested call is depth 2"
        );
        assert_eq!(
            list_depth(&subj("(f (g (h (i 1))))")),
            4,
            "a 4-deep call chain is depth 4"
        );
        // Depth is the LONGEST chain — a wide-but-shallow form is not deep.
        assert_eq!(
            list_depth(&subj("(f a b c d e (g 1))")),
            2,
            "width does not add depth; the one nested arg gives depth 2"
        );
    }

    #[test]
    fn guard_deeper_than_fires_strictly_past_the_threshold() {
        // `,(x (deeper-than N))` matches a node whose list-depth is STRICTLY > N. `(f (g (h (i 1))))`
        // is depth 4.
        let deep = subj("(f (g (h (i 1))))");
        assert_eq!(
            count(&pat(",(x (deeper-than 3))"), &deep),
            1,
            "depth 4 > 3 fires"
        );
        assert_eq!(
            count(&pat(",(x (deeper-than 4))"), &deep),
            0,
            "depth 4 is not > 4"
        );
        assert_eq!(
            count(&pat(",(x (deeper-than 5))"), &deep),
            0,
            "depth 4 < 5 does not fire"
        );
        // A shallow form never fires a positive threshold.
        assert_eq!(
            count(&pat(",(x (deeper-than 2))"), &subj("(f a b)")),
            0,
            "flat call, depth 1"
        );
    }

    #[test]
    fn guard_deeper_than_rejects_a_non_integer_threshold() {
        assert!(Pattern::compile("(f ,(x (deeper-than foo)))").is_err());
        assert!(Pattern::compile("(f ,(x (deeper-than)))").is_err());
    }

    #[test]
    fn call_depth_counts_only_nested_application_forms() {
        // An atom / a bare name is call-depth 0.
        assert_eq!(call_depth(&subj("x")), 0, "an atom is call-depth 0");
        // A flat application is 1; each nested application adds one.
        assert_eq!(
            call_depth(&subj("(f a b)")),
            1,
            "a flat call is call-depth 1"
        );
        assert_eq!(
            call_depth(&subj("(f (g (h (i 1))))")),
            4,
            "a 4-deep call chain is call-depth 4"
        );
        // The KEY distinction from list_depth: STRUCTURAL nesting (keyword heads) does not COMPOUND
        // into call depth. A module/def/let spine with no real calls stays SHALLOW (call-depth <= 1 —
        // the lone def signature `(main)` reads as a depth-1 application, the documented bounded
        // imprecision) even though its list_depth is high. That gap is the whole point.
        let spine = subj("(module m (def (main) (let ((x 1)) x)))");
        assert!(
            call_depth(&spine) <= 1,
            "module/def/let keyword heads do not compound; the spine is call-shallow (got {})",
            call_depth(&spine)
        );
        assert!(
            list_depth(&spine) >= 4,
            "the same spine IS structurally deep (list_depth) — that is exactly why list_depth is the \
             wrong metric for deep-nesting"
        );
        // A call nested INSIDE a structural body still counts (structural heads are descended into).
        assert_eq!(
            call_depth(&subj("(def (main) (f (g (h 1))))")),
            3,
            "def wraps a depth-3 call chain; the def head contributes 0 but the body counts"
        );
        // Infix-operator heads (arena `+`, `=`, `<`, …) are NOT applications.
        assert_eq!(
            call_depth(&subj("(+ a (* b c))")),
            0,
            "arithmetic operators are not application forms"
        );
        // Equality's arena head `=` — shared with a Phase-B record field `(= name value)` — is an
        // operator head, so neither an equality nor a record field counts as a call.
        assert_eq!(
            call_depth(&subj("(= a b)")),
            0,
            "equality (arena head =) is an operator, not a call"
        );
        assert_eq!(
            call_depth(&subj("(record (= m 1) (= n 2))")),
            0,
            "a record literal and its (= field value) fields are not calls"
        );
        // Compound-value constructors (tuple/list/record/map) are data literals, not calls — their
        // heads contribute 0, so a nest of pure constructors is call-depth 0.
        assert_eq!(
            call_depth(&subj("(tuple (list 1 2) (list 3 4))")),
            0,
            "tuple/list constructors are data literals, not application forms"
        );
        // A constructor call applied to arguments (an uppercase callee) DOES count — it is an
        // application of a data constructor, the nesting the lint cares about.
        assert_eq!(
            call_depth(&subj("(Some (Wrap (Inner 1)))")),
            3,
            "constructor applications are calls"
        );
        // Width does not add depth — only the longest single chain.
        assert_eq!(
            call_depth(&subj("(f a b c d (g 1))")),
            2,
            "one nested-call arg gives call-depth 2 regardless of width"
        );
    }

    #[test]
    fn guard_calls_deeper_than_fires_strictly_past_the_threshold() {
        // `,(x (calls-deeper-than N))` matches a node whose CALL-depth is STRICTLY > N.
        // `(f (g (h (i 1))))` is call-depth 4.
        let deep = subj("(f (g (h (i 1))))");
        assert_eq!(
            count(&pat(",(x (calls-deeper-than 3))"), &deep),
            1,
            "call-depth 4 > 3 fires (on the outermost node)"
        );
        assert_eq!(
            count(&pat(",(x (calls-deeper-than 4))"), &deep),
            0,
            "call-depth 4 is not > 4"
        );
        // A structural spine with no calls never fires, however deep it nests.
        assert_eq!(
            count(
                &pat(",(x (calls-deeper-than 1))"),
                &subj("(module m (def (main) (let ((x 1)) x)))")
            ),
            0,
            "a call-free structural spine never fires calls-deeper-than"
        );
    }

    #[test]
    fn guard_calls_deeper_than_rejects_a_non_integer_threshold() {
        assert!(Pattern::compile("(f ,(x (calls-deeper-than foo)))").is_err());
        assert!(Pattern::compile("(f ,(x (calls-deeper-than)))").is_err());
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
        // The message LISTS the guard vocabulary (a closed set), so a near-typo (`is-litera` for
        // `is-literal`) is obvious and the message documents what a guard can be.
        let near = Pattern::compile("(f ,(x is-litera))").unwrap_err();
        assert!(
            near.0.contains("is-literal") && near.0.contains("(head-is NAME)"),
            "the unknown-guard message lists the valid guards: {near}"
        );
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
        assert!(
            m[0].node.to_sexpr().contains("return"),
            "the fn without raise"
        );
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
    fn a_rule_reports_the_template_metavars_its_pattern_never_binds() {
        // `Rule::unbound_template_metavars` is the STATIC check the `cdz rewrite` CLI runs up front: a
        // template metavar (single, splice, or the wildcard `,_` — which a pattern never binds) not bound
        // by the pattern can never be filled, so every site silently rewrites to nothing. Reporting it
        // turns a mystifying "rewrote 0 site(s)" into a named, actionable error.
        let rule = |p: &str, t: &str| Rule::new(pat(p), tmpl(t));
        assert_eq!(
            rule("(+ ,a ,b)", "(- ,a ,c)").unbound_template_metavars(),
            vec!["c".to_string()],
            "a stray single metavar is reported"
        );
        assert_eq!(
            rule("(+ ,@xs)", "(- ,@zs)").unbound_template_metavars(),
            vec!["zs".to_string()],
            "a stray splice metavar is reported"
        );
        assert_eq!(
            rule("(+ ,a ,b)", "(f ,a ,_)").unbound_template_metavars(),
            vec!["_".to_string()],
            "a template wildcard `,_` is unfillable (a pattern never binds `_`)"
        );
        // A well-formed rule (template metavars ⊆ pattern's) reports nothing — including a template that
        // uses only a SUBSET of the bound metavars.
        assert!(
            rule("(+ ,a ,b)", "(- ,a ,b)")
                .unbound_template_metavars()
                .is_empty()
        );
        assert!(
            rule("(+ ,a ,b)", "(id ,a)")
                .unbound_template_metavars()
                .is_empty()
        );
        assert!(
            rule("(+ ,@xs)", "(- ,@xs)")
                .unbound_template_metavars()
                .is_empty()
        );
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
    fn rewrite_fixpoint_is_idempotent_over_generated_subjects() {
        // The DEFINING property of a fixpoint, swept: `rewrite_fixpoint` reaches a STABLE result — feeding
        // its own output back in fires ZERO further rewrites and yields the byte-identical tree. Only ONE
        // hand case (`fixpoint_saturates_and_is_idempotent`) pinned this; if the fixpoint stopped early
        // (a saturation bug) OR kept firing on its own output (a non-terminating / oscillating rule), a
        // second run would differ — a real codemod defect (a `cdz rewrite --fixpoint` that leaves
        // rewritable sites, or loops). Sweep random subjects over a small algebra × a few
        // terminating rules; assert `fixpoint(fixpoint(s)) == fixpoint(s)` structurally AND that the
        // second run's rewrite count is 0.
        fn gen_subj(rng: &mut SplitMix64, depth: usize) -> String {
            let atoms = ["0", "1", "v", "x", "a"];
            if depth == 0 || rng.next().is_multiple_of(3) {
                return atoms[(rng.next() as usize) % atoms.len()].to_string();
            }
            let sub = |rng: &mut SplitMix64| gen_subj(rng, depth - 1);
            match rng.next() % 4 {
                0 => format!("(+ {} {})", sub(rng), sub(rng)),
                1 => format!("(* {} {})", sub(rng), sub(rng)),
                2 => format!("(f {})", sub(rng)),
                _ => format!("(+ 0 {})", sub(rng)), // bias the `(+ 0 x)` shape the rules target
            }
        }
        // Terminating simplification rules — each strictly SHRINKS or renames, so a fixpoint exists.
        let rules: [(&str, &str); 3] = [
            ("(+ 0 ,x)", ",x"), // additive identity
            ("(* 1 ,x)", ",x"), // multiplicative identity
            ("(f ,x)", ",x"),   // unwrap a call
        ];
        let mut rng = SplitMix64(0xf1_c0de_1de3_a5f1);
        let mut checked = 0usize;
        for _ in 0..3000 {
            let depth = 1 + (rng.next() as usize) % 4;
            let Ok(arena) = sexpr::read(&gen_subj(&mut rng, depth)) else {
                continue;
            };
            let s = Tree::of(&arena);
            let (ps, ts) = rules[(rng.next() as usize) % rules.len()];
            let (p, t) = (pat(ps), tmpl(ts));
            let once = rewrite_fixpoint(&p, &t, &s, 100);
            let twice = rewrite_fixpoint(&p, &t, &once.tree, 100);
            // Re-running the fixpoint on its own output is a NO-OP: identical tree, zero further rewrites.
            assert_eq!(
                twice.tree.to_sexpr(),
                once.tree.to_sexpr(),
                "fixpoint not stable for rule {ps}→{ts} on {}",
                s.to_sexpr()
            );
            assert_eq!(
                twice.count,
                0,
                "a re-run of a saturated fixpoint fired {} rewrites (should be 0) for rule {ps}→{ts} on {}",
                twice.count,
                s.to_sexpr()
            );
            checked += 1;
        }
        assert!(
            checked > 1000,
            "swept a meaningful fixpoint space, got {checked}"
        );
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
        assert!(
            reparsed.ok(),
            "rewrite result re-parses: {:?}",
            reparsed.errors
        );
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

    /// Build a random arena node (all leaf kinds + arbitrary arity incl. EMPTY lists — shapes the s-expr
    /// reader never produces but a decoded/hand-built arena can), returning its root id. Bounded by depth.
    fn gen_arena_node(rng: &mut SplitMix64, b: &mut Builder, depth: usize) -> StructId {
        if depth == 0 || rng.next().is_multiple_of(3) {
            let leaf = match rng.next() % 7 {
                0 => Leaf::Int {
                    value: crate::ast::IntValue::from_i64(rng.next() as i64),
                    radix: [crate::ast::Radix::Dec, crate::ast::Radix::Hex]
                        [(rng.next() % 2) as usize],
                },
                1 => Leaf::Str(
                    ["", "hi", "a\nb", "λ中"][(rng.next() % 4) as usize]
                        .to_string()
                        .into(),
                ),
                2 => Leaf::Bool(rng.next().is_multiple_of(2)),
                3 => Leaf::Char(['a', 'é', '\n'][(rng.next() % 3) as usize]),
                4 => Leaf::Bytes(vec![(rng.next() & 0xff) as u8].into()),
                5 => Leaf::Sym(["meter", "x"][(rng.next() % 2) as usize].to_string().into()),
                _ => Leaf::Name(
                    ["f", "x", "+", "record", "list"][(rng.next() % 5) as usize]
                        .to_string()
                        .into(),
                ),
            };
            b.atom_leaf(leaf)
        } else {
            let n = (rng.next() % 5) as usize; // 0..=4 children, incl. the empty list
            let kids: Vec<StructId> = (0..n).map(|_| gen_arena_node(rng, b, depth - 1)).collect();
            b.list(kids)
        }
    }

    /// Assert every node of an owned `Tree` carries a provenance `origin()` whose source `Struct` KIND
    /// matches the tree node's kind (Atom↔Atom, List↔List) — `from_arena` must tag each node with the id
    /// it was copied from, the invariant a search match's span report depends on.
    fn assert_provenance(t: &Tree, src: &Arenas) {
        let id = t
            .origin()
            .expect("from_arena records provenance on every node");
        match (t, src.get(id)) {
            (Tree::Atom(..), Struct::Atom(_)) => {}
            (Tree::List(kids, _), Struct::List(src_kids)) => {
                assert_eq!(
                    kids.len(),
                    src_kids.len(),
                    "provenance list arity mismatch at #{}",
                    id.0
                );
                for k in kids {
                    assert_provenance(k, src);
                }
            }
            _ => panic!("provenance kind mismatch at #{}", id.0),
        }
    }

    #[test]
    fn tree_arena_roundtrip_is_structural_identity_over_generated_arenas() {
        // `Tree::of(a).to_arena()` — the arena→owned-Tree→arena round-trip EVERY codemod op begins and
        // ends with — must be a STRUCTURAL IDENTITY. Both legs (`from_arena`, `build`) are explicit-stack
        // post-order walks with reversed-child pushes; the single hand case above can't catch a child-
        // order or arity drift in that bookkeeping. Sweep random arenas (all leaf kinds + arbitrary arity
        // incl. EMPTY lists, the shapes only a decoded/hand-built arena reaches) and assert: (a) the
        // round-trip is structurally equal to the source; (b) `to_sexpr()` equals the arena's own
        // `sexpr::print` (the rendering all the crate's other sweeps trust); (c) every owned-tree node's
        // `origin()` provenance points at a source id of the matching Struct kind.
        let mut rng = SplitMix64(0x77ee_c0de_a5f1_0003);
        for _ in 0..4000 {
            let mut b = Builder::new();
            let depth = 1 + (rng.next() % 4) as usize;
            let root = gen_arena_node(&mut rng, &mut b, depth);
            let arena = b.finish(root);
            let tree = Tree::of(&arena);
            // (a) round-trip is a structural identity.
            let back = tree.to_arena();
            assert!(
                back.structurally_eq(&arena),
                "Tree::of(a).to_arena() not structurally equal to a: {}",
                sexpr::print(&arena)
            );
            // (b) the Tree's own rendering agrees with the arena's printer.
            assert_eq!(
                tree.to_sexpr(),
                sexpr::print(&arena),
                "Tree::to_sexpr disagrees with sexpr::print on the same structure"
            );
            // (c) provenance: every node points back at a matching source id.
            assert_provenance(&tree, &arena);
        }
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
            assert_eq!(
                report.lines().filter(|l| l.contains(": x")).count(),
                1,
                "{report}"
            );
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
            let out = driver::apply_rewrite(
                &rules,
                Strategy::BottomUp,
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
        fn apply_rewrite_runs_a_multi_rule_set() {
            let (target, _) = driver::load(b"(f (+ a 0) (* b 1))", Format::Sexpr).unwrap();
            let rules = RuleSet::compile("(rule (+ ,x 0) ,x) (rule (* ,x 1) ,x)").unwrap();
            let out = driver::apply_rewrite(
                &rules,
                Strategy::BottomUp,
                &target,
                Format::Sexpr,
                100,
                false,
            )
            .unwrap();
            assert_eq!(out.count, 2);
            assert_eq!(out.output.trim(), "(f a b)");
        }

        // A comment/doc parsed INTO the representation (a leading `(comment …)` wrapper, a def's
        // `(doc …)` node) is an ordinary arena node the rewrite does not match — so it survives an
        // unrelated edit even through the whole-tree reprint path (which reserializes everything). This
        // is the structural-EDIT half of agent-authoring.md — the sidecar `Rewrite` surface (now built)
        // preserves documentation/comments attached to a part it does not change:
        //
        //= spec/capabilities/agent-authoring.md#documentation-survives-round-trip-and-edits
        //# A structural edit MUST preserve the documentation attached to a part of the program it does not change.
        //
        //= spec/capabilities/agent-authoring.md#comments-survive-round-trip-and-edits
        //# A structural edit MUST preserve a comment attached to a part of the program it does not change.
        #[test]
        fn a_rewrite_preserves_untouched_comment_and_doc_nodes() {
            // A leading comment wraps the following form: `(comment "lead" (def (f) (g 1)))`. Rewrite the
            // `g` call it wraps; the comment node (and its text) must remain.
            let (target, _) = driver::load(b"// lead\ndef f() = g(1)", Format::Ml).unwrap();
            let rules = RuleSet::new(vec![Rule::new(pat("(g ,x)"), tmpl("(h ,x)"))]);
            let out =
                driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Ml, 100, false)
                    .unwrap();
            assert_eq!(out.count, 1, "the g→h edit fired");
            assert!(
                out.output.contains("// lead"),
                "the leading comment must survive an unrelated rewrite:\n{}",
                out.output
            );
            assert!(
                out.output.contains("h(1)"),
                "the edit applied:\n{}",
                out.output
            );

            // A doc comment becomes a `(doc "…")` node in the def body; it likewise survives.
            let (target, _) = driver::load(b"/// docs for f\ndef f() = g(1)", Format::Ml).unwrap();
            let out =
                driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Ml, 100, false)
                    .unwrap();
            assert_eq!(out.count, 1);
            assert!(
                out.output.contains("/// docs for f"),
                "the doc comment must survive an unrelated rewrite:\n{}",
                out.output
            );

            // A comment on a node that IS itself matched is a targeted edit, not our concern here — this
            // test pins only that an UNTOUCHED comment/doc is never silently dropped.
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
            assert!(
                j.contains("\"x\":\"a\"") && j.contains("\"y\":\"b\""),
                "{j}"
            );
        }

        #[test]
        fn matches_json_no_match_is_empty_array() {
            let (target, _) = driver::load(b"(g x)", Format::Sexpr).unwrap();
            assert_eq!(
                driver::matches_json(&pat("(f ,x)"), &Query::new(), &target, None),
                "[]"
            );
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
            assert_eq!(
                driver::project_target(&target, Format::Sexpr, 100).unwrap(),
                "(+ x 0)"
            );
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
        fn line_index_matches_line_col_at_every_offset() {
            // `LineIndex::line_col` (binary search + per-line char count) must return EXACTLY what the
            // O(byte) `line_col` scan does at every byte offset — the byte-identity the `cdz clones`
            // report relies on (the O(N²)→O(N log N) fix must not shift any reported line:col). Cover an
            // empty line, a trailing newline, a byte past the end, AND multibyte chars (col counts CHARS,
            // so a `é`/emoji before the offset must count as one column, not its UTF-8 byte length).
            // A LONG single ASCII line (400 chars, no newline) — the O(N²) trigger `cdz exports`/`highlight`
            // hits on a corpus `(input …)` form: the column is `byte - line_start` chars from the ONE line
            // start, so every offset takes the ASCII O(1) fast path and MUST still equal `line_col`.
            let long_line: String = (0..400)
                .map(|i| char::from(b'a' + (i % 26) as u8))
                .collect();
            for src in [
                "abc\ndef\nghi",
                "",
                "\n\n\n",
                "no trailing newline",
                "x\n",                  // trailing newline → an empty final line
                "café\nnaïve\n😀 tail", // multibyte: é (2 bytes), ï (2), 😀 (4)
                long_line.as_str(),     // a long single ASCII line — the fast-path stress
            ] {
                let idx = driver::LineIndex::new(src);
                // Every valid byte boundary + a handful past the end.
                for byte in 0..=src.len() + 3 {
                    if byte <= src.len() && !src.is_char_boundary(byte) {
                        continue; // `line_col`/`LineIndex` are only queried at char boundaries (span starts)
                    }
                    assert_eq!(
                        idx.line_col(src, byte),
                        driver::line_col(src, byte),
                        "LineIndex disagrees with line_col at byte {byte} of {src:?}"
                    );
                }
            }
        }

        #[test]
        fn line_col_matches_an_independent_reference_over_generated_sources() {
            // `line_col` (and its `LineIndex` fast path) is the byte→(line,col) primitive EVERY surface's
            // error reporting funnels through (`locate_byte_in_message` rewrites "at byte N" → "line:col").
            // `line_index_matches_line_col_at_every_offset` proves the two INTERNAL impls agree — but they
            // could agree while BOTH being wrong. This pins them against an INDEPENDENT reference computed
            // a different way: line = 1 + count of '\n' strictly before `byte`; col = 1 + count of CHARS
            // since the last '\n' at-or-before `byte`. Swept over random sources rich in newlines + multibyte
            // (col counts CHARS, not bytes) at every char-boundary offset. A regression (counting bytes for
            // col, an off-by-one at a newline, a past-end mis-clamp) would misplace every reported error.
            fn reference(src: &str, byte: usize) -> (usize, usize) {
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
            // Alphabet weighted toward newlines + multibyte so lines/columns and byte≠char occur densely.
            let alphabet: &[char] = &['\n', 'a', 'b', ' ', 'é', '中', '😀', '\t'];
            let mut rng = SplitMix64(0x11e_c01d_1a7e_5eed);
            for _ in 0..3000 {
                let len = (rng.next() % 30) as usize;
                let src: String = (0..len)
                    .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                    .collect();
                let idx = driver::LineIndex::new(&src);
                for byte in 0..=src.len() + 2 {
                    if byte <= src.len() && !src.is_char_boundary(byte) {
                        continue; // queried only at char boundaries (span starts)
                    }
                    let want = reference(&src, byte);
                    assert_eq!(
                        driver::line_col(&src, byte),
                        want,
                        "line_col disagrees with the reference at byte {byte} of {src:?}"
                    );
                    assert_eq!(
                        idx.line_col(&src, byte),
                        want,
                        "LineIndex::line_col disagrees with the reference at byte {byte} of {src:?}"
                    );
                }
            }
        }

        #[test]
        fn lint_report_renders_location_severity_message_and_flags_error() {
            let (target, _) = driver::load(b"g(deprecated())", Format::Ml).unwrap();
            let set = crate::query::lint::LintSet::compile("(lint (deprecated ,@_) \"no\" error)")
                .unwrap();
            let (report, had_error) =
                driver::lint_report(&set, &target, "g(deprecated())", "in.ml");
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

        #[test]
        fn lint_report_with_levels_applies_allow_and_deny() {
            use crate::query::lint::{LintLevel, LintLevels};
            let (target, _) = driver::load(b"(if a true false)", Format::Sexpr).unwrap();
            let set = crate::query::lint::LintSet::compile(
                "(lint idiomatic/if-bool (if ,c true false) \"prefer the condition\")",
            )
            .unwrap();
            let src = "(if a true false)";
            // Deny → the warning-default rule reports as an error and flags the run.
            let mut deny = LintLevels::new();
            deny.set("idiomatic/if-bool", LintLevel::Deny);
            let (report, had_error) =
                driver::lint_report_with_levels(&set, &target, src, "in.sexp", &deny);
            assert!(had_error, "deny promotes to error: {report}");
            assert!(report.contains("error: prefer the condition"), "{report}");
            // Allow → suppressed entirely (empty report, no error).
            let mut allow = LintLevels::new();
            allow.set("idiomatic", LintLevel::Allow); // group prefix covers idiomatic/if-bool
            let (report, had_error) =
                driver::lint_report_with_levels(&set, &target, src, "in.sexp", &allow);
            assert!(
                !had_error && report.is_empty(),
                "allow suppresses: {report:?}"
            );
        }

        #[test]
        fn lint_fix_applies_verified_fixes_and_round_trips() {
            // The apply-and-recheck witness DESIGN-cadenza-lint §6 licenses `Verified`: fixing the
            // built-in `idiomatic/if-bool` rewrites `(if b true false)` → `b`, the result re-parses,
            // and re-linting the fixed program is clean (the idiom is gone).
            let src = "(module m (def (f (: b Bool)) (if b true false)) (export f))";
            let (target, _) = driver::load(src.as_bytes(), Format::Sexpr).unwrap();
            let set = crate::query::lint::LintSet::builtin();
            let levels = crate::query::lint::LintLevels::default();
            let out = driver::lint_fix_with_levels(
                &set,
                &target,
                src,
                Format::Sexpr,
                &levels,
                false,
                100,
            )
            .unwrap();
            assert_eq!(
                out.count, 1,
                "the one if-bool site was fixed: {}",
                out.output
            );
            assert!(
                out.output.contains("(def (f (: b Bool)) b)"),
                "if-bool collapsed to the condition: {}",
                out.output
            );
            // Re-lint the fixed program — the idiom is gone (apply-and-recheck).
            let (fixed, _) = driver::load(out.output.as_bytes(), Format::Sexpr).unwrap();
            let diags = crate::query::lint::run(&set, &fixed.tree, fixed.spans.as_ref());
            assert!(diags.is_empty(), "fixed program is idiom-clean: {diags:?}");
        }

        #[test]
        fn lint_fix_respects_allow_and_applies_no_fix() {
            // A lint suppressed to `Allow` (via a level, mirroring `--allow` / an `@allow` attribute)
            // fires no diagnostic AND applies no fix — the source is returned unchanged, count 0.
            use crate::query::lint::{LintLevel, LintLevels};
            let src = "(module m (def (f (: b Bool)) (if b true false)) (export f))";
            let (target, _) = driver::load(src.as_bytes(), Format::Sexpr).unwrap();
            let set = crate::query::lint::LintSet::builtin();
            let mut levels = LintLevels::new();
            levels.set("idiomatic/if-bool", LintLevel::Allow);
            let out = driver::lint_fix_with_levels(
                &set,
                &target,
                src,
                Format::Sexpr,
                &levels,
                false,
                100,
            )
            .unwrap();
            assert_eq!(out.count, 0, "an allowed lint applies no fix");
            assert_eq!(out.output, src, "source returned verbatim: {}", out.output);
        }

        #[test]
        fn lint_fix_leaves_verified_only_by_default_and_takes_heuristic_on_opt_in() {
            // A `Heuristic` fix is NOT applied by default (offered, not auto-applied); `include_heuristic`
            // opts it in. Uses a user rule so the applicability is under the test's control.
            let src = "(do (risky a))";
            let (target, _) = driver::load(src.as_bytes(), Format::Sexpr).unwrap();
            let set = crate::query::lint::LintSet::compile(
                "(lint style/risky (risky ,x) \"prefer safe\" => (safe ,x) heuristic)",
            )
            .unwrap();
            let levels = crate::query::lint::LintLevels::default();
            // Default: heuristic excluded → no change.
            let off = driver::lint_fix_with_levels(
                &set,
                &target,
                src,
                Format::Sexpr,
                &levels,
                false,
                100,
            )
            .unwrap();
            assert_eq!(
                off.count, 0,
                "heuristic fix withheld by default: {}",
                off.output
            );
            // Opt-in: heuristic applied.
            let on =
                driver::lint_fix_with_levels(&set, &target, src, Format::Sexpr, &levels, true, 100)
                    .unwrap();
            assert_eq!(on.count, 1, "heuristic fix applied under opt-in");
            assert!(on.output.contains("safe"), "the fix ran: {}", on.output);
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
                ChangeKind::Replace {
                    old: "b".into(),
                    new: "c".into()
                }
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
                ChangeKind::Replace {
                    old: "x".into(),
                    new: "(f y)".into()
                }
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

        #[test]
        fn diff_is_empty_exactly_when_the_trees_are_structurally_equal() {
            // The core SOUNDNESS biconditional of the structural diff: `treediff::diff(a, b)` is EMPTY
            // ⟺ `a` structurally-equals `b`. A false empty-diff (no change reported between DIFFERENT
            // trees) would make `cdz diff` / review tooling miss an edit; a spurious change on EQUAL trees
            // would report a phantom edit. Only one hand case (`identical_trees_have_no_changes`) pinned
            // the ⟸ direction and none swept ⟹. Generate random tree PAIRS — sometimes equal (same seed
            // regenerates the same program), sometimes differing — and assert diff-empty agrees with
            // structural equality both ways.
            use super::{SplitMix64, Tree, tree_eq};
            fn gen_tree(rng: &mut SplitMix64, depth: usize) -> String {
                let leaves = ["a", "b", "0", "1", "x"];
                if depth == 0 || rng.next().is_multiple_of(3) {
                    return leaves[(rng.next() as usize) % leaves.len()].to_string();
                }
                let heads = ["f", "g", "+", "*"];
                let head = heads[(rng.next() as usize) % heads.len()];
                let n = 1 + (rng.next() as usize) % 3;
                let kids: Vec<String> = (0..n).map(|_| gen_tree(rng, depth - 1)).collect();
                format!("({head} {})", kids.join(" "))
            }
            let mut rng = SplitMix64(0xd1ff_50f7_c0de_1a7e);
            let mut equal_seen = 0usize;
            let mut differ_seen = 0usize;
            for _ in 0..3000 {
                let da = 1 + (rng.next() as usize) % 4;
                let db = 1 + (rng.next() as usize) % 4;
                let (Ok(aa), Ok(ba)) = (
                    crate::sexpr::read(&gen_tree(&mut rng, da)),
                    crate::sexpr::read(&gen_tree(&mut rng, db)),
                ) else {
                    continue;
                };
                let (ta, tb) = (Tree::of(&aa), Tree::of(&ba));
                let empty = treediff::diff(&ta, &tb).is_empty();
                let eq = tree_eq(&ta, &tb);
                assert_eq!(
                    empty,
                    eq,
                    "diff-empty ({empty}) must agree with tree_eq ({eq}) for\n  a={}\n  b={}",
                    ta.to_sexpr(),
                    tb.to_sexpr()
                );
                // And the reflexive case explicitly: a tree never differs from itself.
                assert!(
                    treediff::diff(&ta, &ta).is_empty(),
                    "a tree diffs empty against itself: {}",
                    ta.to_sexpr()
                );
                if eq {
                    equal_seen += 1;
                } else {
                    differ_seen += 1;
                }
            }
            // The EQUAL arm is exercised DETERMINISTICALLY by the per-iteration reflexive `diff(ta, ta)`
            // check above (a tree always equals itself), so it does not rely on the generator happening to
            // produce a coincidental equal PAIR. `equal_seen`/`differ_seen` are only soft coverage hints:
            // assert just that the DIFFERING arm was meaningfully hit (the interesting direction the pair
            // sweep adds); the equal count is not asserted on (it depends on generator luck).
            let _ = equal_seen;
            assert!(
                differ_seen >= 100,
                "the pair sweep must hit differing pairs (the direction it adds), got {differ_seen}"
            );
        }
    }

    mod lint_tests {
        use super::subj;
        use crate::query::lint::{self, Applicability, LintLevel, LintLevels, LintSet, Severity};

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
        fn lint_set_compile_and_run_never_panic_on_arbitrary_input() {
            // `LintSet::compile` parses UNTRUSTED lint-rule text (`cdz lint --rule '…'` / `--rules FILE`)
            // — like Pattern/Template/RuleSet::compile it must return a diagnostic, never panic, on any
            // string: `(lint …)` soup, wrong arity, a non-string message, a bad severity name, a bad
            // pattern, unbalanced parens. And a set that DOES compile must RUN (lint_report) over a
            // subject without panicking. Mirrors the reader/query-compiler robustness fuzzes.
            let subject = "g(deprecated(), (todo x), 1)";
            let (target, _) =
                crate::query::driver::load(subject.as_bytes(), crate::convert::Format::Ml).unwrap();
            let alphabet: Vec<char> = "(lint ),@_\"xdeprecatedtoerrwान warnnote0123."
                .chars()
                .collect();
            let mut rng = super::SplitMix64(0x11_7ea5_c0de_0f3e);
            for len in 0..=28usize {
                for _ in 0..120 {
                    let s: String = (0..len)
                        .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                        .collect();
                    if let Ok(set) = LintSet::compile(&s) {
                        // A compiled set must run without panicking (report + json paths).
                        let _ = crate::query::driver::lint_report(&set, &target, subject, "in.ml");
                        let _ =
                            crate::query::driver::lint_json(&set, &target, subject, Some("in.ml"));
                    }
                }
            }
            // Targeted structural edges — each must be Err/Ok, never panic.
            for s in [
                "(lint)",                            // no pattern/message
                "(lint (x))",                        // missing message
                "(lint (x) 5)",                      // non-string message
                "(lint (x) \"m\" bogus)",            // unknown severity
                "(lint (x) \"m\" error extra)",      // too many operands
                "(lint (,@a ,@b) \"m\")",            // adjacent splices in the pattern
                "(lint (x) \"m\" \"notaname\")",     // severity not a bare name
                "(not-lint (x) \"m\")",              // wrong head
                "((((",                              // unbalanced
                "(lint (x) \"a\") (lint (y) \"b\")", // two rules
            ] {
                let _ = LintSet::compile(s);
            }
        }

        #[test]
        fn compile_reads_pattern_message_and_severity() {
            let set = LintSet::compile("(lint (deprecated ,@_) \"do not use\" error)").unwrap();
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
            let set =
                LintSet::compile("(lint (a ,@_) \"msg-a\" warning)\n(lint (b ,@_) \"msg-b\" info)")
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
            let set =
                LintSet::compile("(lint (+ ,x ,(z is-literal)) \"maybe redundant\")").unwrap();
            let diags = lint::run(&set, &subj("(do (+ a 0) (+ b c))"), None);
            assert_eq!(diags.len(), 1, "{diags:?}");
        }

        // --- cadenza-lint I1: the named `(lint NAME …)` superset form + optional fix clause. ---

        #[test]
        fn a_bare_lint_rule_still_has_no_name_and_no_fix() {
            // The existing report-only form is unchanged: no name, no fix, warning default.
            let set = LintSet::compile("(lint (todo ,@_) \"has a todo\")").unwrap();
            let r = &set.rules[0];
            assert_eq!(r.name, None);
            assert!(r.fix.is_none());
            assert_eq!(r.severity, Severity::Warning);
        }

        #[test]
        fn a_named_lint_carries_its_name_and_default_level() {
            // A leading bare name marks the named form; level still defaults to warning.
            let set = LintSet::compile(
                "(lint idiomatic/if-bool (if ,c true false) \"prefer the condition\")",
            )
            .unwrap();
            let r = &set.rules[0];
            assert_eq!(r.name.as_deref(), Some("idiomatic/if-bool"));
            assert_eq!(r.message, "prefer the condition");
            assert_eq!(r.severity, Severity::Warning);
            assert!(r.fix.is_none(), "no `=>` clause means warn-only");
            // and it still fires on a match, reporting the message.
            let diags = lint::run(&set, &subj("(do (if a true false) (g b))"), None);
            assert_eq!(diags.len(), 1, "{diags:?}");
            assert_eq!(diags[0].message, "prefer the condition");
        }

        #[test]
        fn a_named_lint_with_a_level_and_a_verified_fix_parses() {
            // The full form: NAME PATTERN message level => TEMPLATE app.
            let set = LintSet::compile(
                "(lint idiomatic/if-bool (if ,c true false) \"prefer the condition\" warning => ,c verified)",
            )
            .unwrap();
            let r = &set.rules[0];
            assert_eq!(r.name.as_deref(), Some("idiomatic/if-bool"));
            assert_eq!(r.severity, Severity::Warning);
            let (_tmpl, app) = r.fix.as_ref().expect("a `=>` clause yields a fix");
            assert_eq!(*app, Applicability::Verified);
        }

        #[test]
        fn a_named_lint_fix_without_a_level_parses() {
            // The level is optional even with a fix: NAME PATTERN message => TEMPLATE app.
            let set = LintSet::compile(
                "(lint idiomatic/redundant-let (let ,x ,e ,x) \"binds then returns it\" => ,e heuristic)",
            )
            .unwrap();
            let r = &set.rules[0];
            assert_eq!(r.severity, Severity::Warning);
            let (_tmpl, app) = r.fix.as_ref().expect("fix present");
            assert_eq!(*app, Applicability::Heuristic);
        }

        #[test]
        fn a_fix_template_referencing_an_unbound_metavar_is_rejected() {
            // `,z` is not bound by the pattern, so the fix could never instantiate — rejected at compile.
            let e =
                LintSet::compile("(lint idiomatic/bad (if ,c true false) \"m\" => ,z verified)")
                    .unwrap_err();
            assert!(e.0.contains("never binds"), "got {e}");
        }

        #[test]
        fn an_unknown_fix_applicability_is_rejected() {
            let e =
                LintSet::compile("(lint idiomatic/if-bool (if ,c true false) \"m\" => ,c maybe)")
                    .unwrap_err();
            assert!(e.0.contains("unknown fix applicability"), "got {e}");
        }

        #[test]
        fn a_malformed_trailing_fix_clause_is_rejected() {
            // A `=>` that is not the trailing `=> TEMPLATE app` 3-token shape is a malformation.
            let e = LintSet::compile("(lint idiomatic/if-bool (if ,c true false) \"m\" => ,c)")
                .unwrap_err();
            assert!(e.0.contains("trailing `=> TEMPLATE app`"), "got {e}");
        }

        #[test]
        fn applicability_parses_known_and_rejects_unknown() {
            assert_eq!(
                Applicability::parse("verified"),
                Some(Applicability::Verified)
            );
            assert_eq!(
                Applicability::parse("heuristic"),
                Some(Applicability::Heuristic)
            );
            assert_eq!(Applicability::parse("bogus"), None);
        }

        #[test]
        fn a_named_lint_still_accepts_an_explicit_error_level() {
            let set = LintSet::compile("(lint idiomatic/if-bool (if ,c true false) \"m\" error)")
                .unwrap();
            assert_eq!(set.rules[0].severity, Severity::Error);
            assert_eq!(set.rules[0].name.as_deref(), Some("idiomatic/if-bool"));
        }

        // --- cadenza-lint I1 (level layer): allow/warn/deny per named lint. ---

        #[test]
        fn lint_level_parses_known_and_rejects_unknown() {
            assert_eq!(LintLevel::parse("allow"), Some(LintLevel::Allow));
            assert_eq!(LintLevel::parse("warn"), Some(LintLevel::Warn));
            assert_eq!(LintLevel::parse("deny"), Some(LintLevel::Deny));
            assert_eq!(LintLevel::parse("bogus"), None);
        }

        #[test]
        fn allow_suppresses_a_named_lint_entirely() {
            let set = LintSet::compile(
                "(lint idiomatic/if-bool (if ,c true false) \"prefer the condition\")",
            )
            .unwrap();
            let s = subj("(do (if a true false) (g b))");
            // Without a level: fires.
            assert_eq!(lint::run(&set, &s, None).len(), 1);
            // Allowed: suppressed.
            let mut levels = LintLevels::new();
            levels.set("idiomatic/if-bool", LintLevel::Allow);
            assert!(lint::run_with_levels(&set, &s, None, &levels).is_empty());
        }

        #[test]
        fn deny_promotes_a_named_lint_to_error() {
            let set = LintSet::compile(
                "(lint idiomatic/if-bool (if ,c true false) \"prefer the condition\")",
            )
            .unwrap();
            let s = subj("(do (if a true false))");
            let mut levels = LintLevels::new();
            levels.set("idiomatic/if-bool", LintLevel::Deny);
            let diags = lint::run_with_levels(&set, &s, None, &levels);
            assert_eq!(diags.len(), 1);
            assert_eq!(diags[0].severity, Severity::Error);
            assert!(lint::has_error(&diags), "deny fails the run");
        }

        #[test]
        fn a_group_prefix_level_applies_to_every_lint_under_it() {
            let set = LintSet::compile(
                "(lint idiomatic/if-bool (if ,c true false) \"a\")\n\
                 (lint naming/camel-case (camelCase ,@_) \"b\")",
            )
            .unwrap();
            let s = subj("(do (if a true false) (camelCase x))");
            // Allowing the whole `idiomatic` group suppresses if-bool but NOT naming/camel-case.
            let mut levels = LintLevels::new();
            levels.set("idiomatic", LintLevel::Allow);
            let diags = lint::run_with_levels(&set, &s, None, &levels);
            assert_eq!(diags.len(), 1, "{diags:?}");
            assert_eq!(diags[0].message, "b");
        }

        #[test]
        fn an_exact_name_override_beats_a_group_override() {
            let set =
                LintSet::compile("(lint idiomatic/if-bool (if ,c true false) \"a\")").unwrap();
            let s = subj("(do (if a true false))");
            // Group allows, but the exact name denies — the most-specific key wins → error.
            let mut levels = LintLevels::new();
            levels.set("idiomatic", LintLevel::Allow);
            levels.set("idiomatic/if-bool", LintLevel::Deny);
            let diags = lint::run_with_levels(&set, &s, None, &levels);
            assert_eq!(diags.len(), 1);
            assert_eq!(diags[0].severity, Severity::Error);
        }

        #[test]
        fn a_cli_overlay_wins_over_a_module_level() {
            // module layer denies; CLI layer allows on top → allowed (CLI > module).
            let mut module = LintLevels::new();
            module.set("idiomatic/if-bool", LintLevel::Deny);
            let mut cli = LintLevels::new();
            cli.set("idiomatic/if-bool", LintLevel::Allow);
            module.overlay(&cli);
            assert_eq!(
                module.effective("idiomatic/if-bool"),
                Some(LintLevel::Allow)
            );
        }

        #[test]
        fn a_bare_unnamed_rule_is_not_level_controlled() {
            // A report-only rule has no name to key off, so a level override cannot touch it.
            let set = LintSet::compile("(lint (todo ,@_) \"has a todo\")").unwrap();
            let s = subj("(do (todo x))");
            let mut levels = LintLevels::new();
            levels.set("todo", LintLevel::Allow); // no effect — the rule is unnamed
            assert_eq!(lint::run_with_levels(&set, &s, None, &levels).len(), 1);
        }

        #[test]
        fn an_unoverridden_named_lint_keeps_its_default_severity() {
            let set = LintSet::compile("(lint idiomatic/x (bad ,@_) \"m\" info)").unwrap();
            let s = subj("(do (bad y))");
            let levels = LintLevels::new(); // empty
            let diags = lint::run_with_levels(&set, &s, None, &levels);
            assert_eq!(
                diags[0].severity,
                Severity::Info,
                "default severity preserved"
            );
        }

        // --- cadenza-lint I1 (@-attribute directives): operator ruled lint levels ride @-attributes. ---

        #[test]
        fn at_attributes_read_allow_warn_deny_from_the_parsed_tree() {
            // The `@allow("NAME")` item attribute parses to `(@ (allow "NAME") item)`; `from_attributes`
            // collects each such level directive. NAME is a STRING so a namespaced `idiomatic/if-bool`
            // is not misparsed as division.
            let program = subj(
                "(do (@ (allow \"idiomatic/if-bool\") (def (main) 0)) \
                 (@ (deny \"naming/camel-case\") (def (g) 1)) \
                 (@ (warn \"idiomatic/redundant-let\") (def (h) 2)))",
            );
            let levels = LintLevels::from_attributes(&program);
            assert_eq!(
                levels.effective("idiomatic/if-bool"),
                Some(LintLevel::Allow)
            );
            assert_eq!(levels.effective("naming/camel-case"), Some(LintLevel::Deny));
            assert_eq!(
                levels.effective("idiomatic/redundant-let"),
                Some(LintLevel::Warn)
            );
            assert_eq!(levels.effective("idiomatic/other"), None);
        }

        #[test]
        fn an_at_attribute_group_prefix_applies_to_the_group() {
            let program = subj("(do (@ (allow \"idiomatic\") (def (main) 0)))");
            let levels = LintLevels::from_attributes(&program);
            assert_eq!(
                levels.effective("idiomatic/if-bool"),
                Some(LintLevel::Allow)
            );
            assert_eq!(levels.effective("naming/camel-case"), None);
        }

        #[test]
        fn at_attributes_nest_and_are_found_at_any_depth() {
            // An attribute inside a nested form (a module body, a nested annotated def) is still found.
            let program = subj(
                "(module m (@ (deny \"idiomatic/if-bool\") (def (main) 0)) \
                 (@ (allow \"idiomatic/redundant-let\") (def (g) 1)))",
            );
            let levels = LintLevels::from_attributes(&program);
            assert_eq!(levels.effective("idiomatic/if-bool"), Some(LintLevel::Deny));
            assert_eq!(
                levels.effective("idiomatic/redundant-let"),
                Some(LintLevel::Allow)
            );
        }

        #[test]
        fn a_non_lint_at_attribute_is_ignored() {
            // A `@`-attribute whose head is not a lint level (`@requires`, `@tag`) is not a lint
            // directive — `from_attributes` skips it (no spurious level).
            let program = subj(
                "(do (@ (requires (> x 0)) (def (f (: x Int64)) x)) \
                 (@ (tag \"slow\") (def (g) 1)))",
            );
            let levels = LintLevels::from_attributes(&program);
            assert_eq!(levels.effective("requires"), None);
            assert_eq!(levels.effective("tag"), None);
        }

        #[test]
        fn a_non_string_at_attribute_name_is_ignored() {
            // The lint NAME must be a STRING literal. A bare-name arg (`(allow idiomatic)` — not a
            // string) is not the directive shape and is skipped, so a stray non-string never sets a
            // bogus level. (The surface always writes the string form `@allow("idiomatic/if-bool")`.)
            let program = subj("(do (@ (allow idiomatic) (def (main) 0)))");
            let levels = LintLevels::from_attributes(&program);
            assert_eq!(levels.effective("idiomatic"), None);
        }

        #[test]
        fn cli_overlay_wins_over_an_at_attribute() {
            // In-source `@deny`, CLI `--allow` on top → CLI wins (CLI > in-source attribute).
            let program = subj("(do (@ (deny \"idiomatic/if-bool\") (def (main) 0)))");
            let mut levels = LintLevels::from_attributes(&program);
            let mut cli = LintLevels::new();
            cli.set("idiomatic/if-bool", LintLevel::Allow);
            levels.overlay(&cli);
            assert_eq!(
                levels.effective("idiomatic/if-bool"),
                Some(LintLevel::Allow)
            );
        }

        // --- cadenza-lint I1 (Tier-A pack): the built-in `idiomatic` catalog. ---

        #[test]
        fn builtin_catalog_compiles_and_names_its_lints() {
            let set = LintSet::builtin();
            // if-bool (×2) + redundant-let + double-negation + if-same-branch + single-arm-match
            // (all fixable Verified) + naming/camel-case (×2, def + let binder — REPORT-ONLY, no fix)
            // + idiomatic/deep-nesting + idiomatic/nested-match (both REPORT-ONLY, Heuristic fixes).
            assert_eq!(set.rules.len(), 10, "10 Tier-A rules");
            let names: Vec<&str> = set.rules.iter().filter_map(|r| r.name.as_deref()).collect();
            assert_eq!(
                names,
                [
                    "idiomatic/if-bool",
                    "idiomatic/if-bool",
                    "idiomatic/redundant-let",
                    "idiomatic/double-negation",
                    "idiomatic/if-same-branch",
                    "idiomatic/single-arm-match",
                    "naming/camel-case",
                    "naming/camel-case",
                    "idiomatic/deep-nesting",
                    "idiomatic/nested-match",
                ]
            );
            // Every FIXABLE catalog rule's fix is Verified; the report-only lints (naming/camel-case —
            // Heuristic use-site rename; idiomatic/deep-nesting — Heuristic author-named hoist) carry no
            // catalog fix, offered as code-actions instead.
            for r in &set.rules {
                match r.fix.as_ref() {
                    Some((_, app)) => {
                        assert_eq!(*app, Applicability::Verified, "catalog fixes are Verified");
                        assert!(
                            r.name
                                .as_deref()
                                .is_some_and(|n| n.starts_with("idiomatic/")),
                            "only idiomatic lints carry a fix in the catalog"
                        );
                    }
                    None => assert!(
                        matches!(
                            r.name.as_deref(),
                            Some("naming/camel-case")
                                | Some("idiomatic/deep-nesting")
                                | Some("idiomatic/nested-match")
                        ),
                        "the report-only catalog lints are naming/camel-case + deep-nesting + nested-match"
                    ),
                }
            }
        }

        #[test]
        fn builtin_naming_camel_case_flags_camel_bindings_report_only() {
            let set = LintSet::builtin();
            // A camelCase `def` name and a camelCase `let` binder each fire; snake_case does not.
            let s = subj("(do (def (myFunc x) x) (def (f) (let ((fooBar 1)) fooBar)))");
            let diags = lint::run(&set, &s, None);
            let camel: Vec<_> = diags
                .iter()
                .filter(|d| d.message.contains("camelCase"))
                .collect();
            assert_eq!(
                camel.len(),
                2,
                "both the def name and the let binder fire: {diags:?}"
            );
            assert!(
                camel.iter().all(|d| d.severity == Severity::Warning),
                "report-only warnings"
            );
            // snake_case bindings fire nothing.
            let clean = subj("(do (def (good_name x) x) (def (g) (let ((foo_bar 1)) foo_bar)))");
            assert!(
                lint::run(&set, &clean, None)
                    .iter()
                    .all(|d| !d.message.contains("camelCase")),
                "snake_case bindings are not flagged"
            );
        }

        #[test]
        fn builtin_deep_nesting_fires_on_a_pathological_call_chain_report_only() {
            let set = LintSet::builtin();
            // A call chain deeper than N=10 fires the deep-nesting warning (report-only, no fix).
            // 12 nested applications -> call-depth 12 > 10.
            let deep = subj("(a (b (c (d (e (f (g (h (i (j (k (l 1))))))))))))");
            let diags = lint::run(&set, &deep, None);
            let dn: Vec<_> = diags
                .iter()
                .filter(|d| d.message.contains("deeply nested"))
                .collect();
            assert!(
                !dn.is_empty(),
                "a >10-deep call chain fires deep-nesting: {diags:?}"
            );
            assert!(
                dn.iter().all(|d| d.severity == Severity::Warning),
                "deep-nesting is a report-only warning"
            );
            // Ordinary shallow code (a structural spine + a shallow call) never fires it — this is the
            // whole point of call_depth over list_depth: structural nesting does not count.
            let shallow = subj("(module m (def (main) (let ((x 1)) (+ x (g 2)))))");
            assert!(
                lint::run(&set, &shallow, None)
                    .iter()
                    .all(|d| !d.message.contains("deeply nested")),
                "shallow structural code is not flagged as deep-nesting"
            );
        }

        #[test]
        fn builtin_nested_match_fires_only_on_a_match_scrutinee_not_an_arm_body() {
            let set = LintSet::builtin();
            // FIRES: a match whose SCRUTINEE is itself a match (matching on a match result).
            let on_result = subj("(do (def (g) (match (match z (1 p) (2 q)) (0 a) (3 c))))");
            let d = lint::run(&set, &on_result, None);
            let nm: Vec<_> = d
                .iter()
                .filter(|x| x.message.contains("match on the result of a match"))
                .collect();
            assert_eq!(nm.len(), 1, "scrutinee-is-match fires once: {d:?}");
            assert_eq!(nm[0].severity, Severity::Warning, "report-only warning");
            // QUIET: a match nested in an arm BODY is ordinary idiomatic dispatch, NOT flagged (else it
            // floods — 473 corpus hits vs 11 for the scrutinee form).
            let in_arm = subj("(do (def (h) (match x (0 (match y (1 a) (2 b))) (3 c))))");
            assert!(
                lint::run(&set, &in_arm, None)
                    .iter()
                    .all(|x| !x.message.contains("match on the result of a match")),
                "an arm-body match is idiomatic dispatch, not flagged"
            );
        }

        #[test]
        fn builtin_double_negation_and_if_same_branch_fire_and_fix() {
            let set = LintSet::builtin();
            // double-negation: `(not (not e))` → `e`.
            let s = subj("(do (not (not b)))");
            let diags = lint::run(&set, &s, None);
            assert_eq!(diags.len(), 1, "double-negation fires: {diags:?}");
            assert!(diags[0].message.contains("double negation"), "{diags:?}");
            // if-same-branch: `(if c e e)` → `e` (both arms structurally equal).
            let s2 = subj("(do (if c (g 1) (g 1)))");
            let d2 = lint::run(&set, &s2, None);
            assert_eq!(d2.len(), 1, "if-same-branch fires on equal arms: {d2:?}");
            // Differing arms must NOT fire (non-linear metavar consistency).
            let s3 = subj("(do (if c (g 1) (g 2)))");
            assert!(
                lint::run(&set, &s3, None).is_empty(),
                "distinct arms do not fire if-same-branch"
            );
            // The fix templates rewrite to the intended equivalents.
            let dn = set
                .rules
                .iter()
                .find(|r| r.name.as_deref() == Some("idiomatic/double-negation"))
                .unwrap();
            let (dtmpl, _) = dn.fix.as_ref().unwrap();
            assert_eq!(
                crate::query::rewrite(&dn.pattern, dtmpl, &subj("(not (not b))"))
                    .tree
                    .to_sexpr(),
                "b"
            );
            let sb = set
                .rules
                .iter()
                .find(|r| r.name.as_deref() == Some("idiomatic/if-same-branch"))
                .unwrap();
            let (stmpl, _) = sb.fix.as_ref().unwrap();
            assert_eq!(
                crate::query::rewrite(&sb.pattern, stmpl, &subj("(if c (g 1) (g 1))"))
                    .tree
                    .to_sexpr(),
                "(g 1)"
            );
        }

        #[test]
        fn builtin_single_arm_match_fires_only_on_an_irrefutable_arm() {
            let set = LintSet::builtin();
            let single = set
                .rules
                .iter()
                .find(|r| r.name.as_deref() == Some("idiomatic/single-arm-match"))
                .unwrap();
            // Irrefutable single arms fire: a var binder and a tuple destructure.
            for src in ["(match p (x (+ x 1)))", "(match p ((tuple a b) (+ a b)))"] {
                assert_eq!(
                    lint::run(&set, &subj(src), None).len(),
                    1,
                    "single-arm-match fires on irrefutable {src}"
                );
            }
            // A REFUTABLE single arm (a sum-ctor pattern) must NOT fire — the no-context
            // `is-irrefutable` guard treats every ctor as refutable, so the `let` can't erase a match's
            // refutability.
            assert!(
                lint::run(&set, &subj("(match p ((Some y) y))"), None).is_empty(),
                "a refutable ctor single-arm does not fire"
            );
            // A literal single arm is refutable too.
            assert!(
                lint::run(&set, &subj("(match p (0 x))"), None).is_empty(),
                "a literal single-arm is refutable"
            );
            // A multi-arm match is structurally excluded (the pattern is a 3-element `(match s (p b))`).
            assert!(
                lint::run(&set, &subj("(match p (x x) (y y))"), None).is_empty(),
                "a two-arm match does not fire single-arm-match"
            );
            // The fix template lowers `(match s (p b))` → `(let ((p s)) b)`.
            let (tmpl, _) = single.fix.as_ref().unwrap();
            assert_eq!(
                crate::query::rewrite(&single.pattern, tmpl, &subj("(match p (x (+ x 1)))"))
                    .tree
                    .to_sexpr(),
                "(let ((x p)) (+ x 1))"
            );
        }

        #[test]
        fn builtin_catalog_fires_on_the_idiomatic_shapes() {
            let set = LintSet::builtin();
            // `(if b true false)` → if-bool fires; `(let ((x e)) x)` → redundant-let fires.
            let s = subj("(do (if b true false) (let ((x (h))) x) (g 1))");
            let diags = lint::run(&set, &s, None);
            assert_eq!(
                diags.len(),
                2,
                "both if-bool and redundant-let fire: {diags:?}"
            );
            assert!(diags.iter().all(|d| d.severity == Severity::Warning));
            // A clean program (no idiomatic issue) fires nothing.
            let clean = subj("(do (if b (f 1) (g 2)) (let ((x (h))) (k x)))");
            assert!(
                lint::run(&set, &clean, None).is_empty(),
                "no false positives"
            );
        }

        #[test]
        fn builtin_if_bool_negated_arm_fires_separately() {
            let set = LintSet::builtin();
            // `(if b false true)` → the negated if-bool rule (fix `(not b)`), distinct from the `true false` rule.
            let s = subj("(do (if b false true))");
            let diags = lint::run(&set, &s, None);
            assert_eq!(diags.len(), 1, "the false/true arm fires: {diags:?}");
            assert!(diags[0].message.contains("not(condition)"));
        }

        #[test]
        fn builtin_verified_fixes_rewrite_to_the_equivalent_form() {
            // The §6 Verified witness: applying each catalog fix's pattern→template yields the intended
            // equivalent form. `(if c true false)` → `c`; `(if c false true)` → `(not c)`;
            // `(let ((x e)) x)` → `e`. (Applying is a separate `--fix`/code-action; this pins the fix
            // TEMPLATE is correct — the equivalence-preserving rewrite the applier would perform.)
            let set = LintSet::builtin();
            let cases = [
                (0usize, "(if a true false)", "a"),
                (1, "(if a false true)", "(not a)"),
                (2, "(let ((y (m))) y)", "(m)"),
            ];
            for (i, src, want) in cases {
                let (tmpl, _) = set.rules[i].fix.as_ref().expect("fix present");
                let subject = subj(src);
                let out = crate::query::rewrite(&set.rules[i].pattern, tmpl, &subject);
                assert_eq!(
                    out.tree.to_sexpr(),
                    want,
                    "rule {i} fix on {src} should rewrite to {want}"
                );
                assert_eq!(out.count, 1, "exactly one rewrite site");
            }
        }
    }

    mod hash_tests {
        use super::subj;
        use crate::query::hash::{hash_tree, node_size};

        #[test]
        fn hash_tree_agrees_with_tree_eq_over_every_generated_subtree() {
            // The SOUNDNESS invariant clone-detection stands on: `find_clones` groups subtrees BY
            // `hash_tree` and treats a shared bucket as a clone class, so the digest must agree with
            // structural equality in BOTH directions —
            //   hash_tree(a) == hash_tree(b)  ⟺  tree_eq(a, b)
            // A COLLISION (distinct trees, equal hash) reports a FALSE clone / merges two classes; the
            // reverse mismatch (equal trees, distinct hash) MISSES a real clone. Only a handful of hand
            // `assert_ne!` cases pinned this (radix, alpha-inequivalence, atom-vs-list); nothing swept the
            // whole space. Generate programs over an alphabet rich in near-misses (same shape/different
            // leaf, same leaves/different shape, distinct radices, string-vs-name), collect EVERY subtree
            // (via `node_size`-covering pre-order), and check the biconditional through TWO indexes, both
            // near-linear (no O(N²) pairwise scan):
            //   * no-collision (hash⇒eq): a hash→reps bucket — within a bucket every tree must be
            //     `tree_eq` (a distinct tree in the same bucket is a collision);
            //   * no-miss (eq⇒hash): a canonical-source→hash index — two subtrees are `tree_eq` IFF they
            //     render to the same canonical s-expr (`src_of`), so equal trees landing on DIFFERENT
            //     hashes is exactly one source string mapping to two hashes. O(N) via a map, replacing the
            //     old O(Σ bucket²)/all-pairs `tree_eq` cross-bucket scan (PR #500 CI-time nit).
            use super::{SplitMix64, Tree, tree_eq};
            use std::collections::HashMap;

            fn collect<'t>(t: &'t Tree, out: &mut Vec<&'t Tree>) {
                out.push(t);
                if let Tree::List(items, _) = t {
                    for c in items {
                        collect(c, out);
                    }
                }
            }
            let src_of = |t: &Tree| crate::sexpr::print(&t.to_arena());
            // An alphabet that makes near-miss subtrees actually occur: two heads, two operand names, a
            // shared name to force cross-position equal subtrees, the SAME numeric value in two radices
            // (`10` vs `0xA` — distinct leaves, must NOT collide), and a string literal vs a bare name.
            let atoms = ["f", "g", "a", "b", "x", "10", "0xA", "\"a\""];
            let mut rng = SplitMix64(0x0d1c_a5f0_c0de_5eed);
            // hash → representatives already seen with that hash (owned so they outlive one iteration).
            let mut buckets: HashMap<u64, Vec<Tree>> = HashMap::new();
            // canonical source → the hash every tree with that source got. A second, DIFFERENT hash for
            // the same source is the "equal trees hashed differently → missed clone" bug, caught in O(1).
            let mut by_source: HashMap<String, u64> = HashMap::new();
            let mut pairs_checked = 0usize;
            let mut equal_pairs = 0usize;
            for _ in 0..4000 {
                // Build a small random program textually so it routes through the real reader.
                fn gen_prog(rng: &mut SplitMix64, atoms: &[&str], depth: usize) -> String {
                    if depth == 0 || rng.next().is_multiple_of(3) {
                        return atoms[(rng.next() as usize) % atoms.len()].to_string();
                    }
                    let n = 1 + (rng.next() as usize) % 3;
                    let kids: Vec<String> =
                        (0..n).map(|_| gen_prog(rng, atoms, depth - 1)).collect();
                    format!("({})", kids.join(" "))
                }
                let depth = 1 + (rng.next() as usize) % 4;
                let src = gen_prog(&mut rng, &atoms, depth);
                let Ok(arena) = crate::sexpr::read(&src) else {
                    continue;
                };
                let tree = Tree::of(&arena);
                let mut subs = Vec::new();
                collect(&tree, &mut subs);
                for sub in subs {
                    let h = hash_tree(sub);
                    let reps = buckets.entry(h).or_default();
                    // no-collision (hash⇒eq): same hash MUST mean structurally equal.
                    for rep in reps.iter() {
                        assert!(
                            tree_eq(rep, sub),
                            "hash COLLISION between structurally-distinct subtrees:\n  {}\n  {}",
                            src_of(rep),
                            src_of(sub)
                        );
                        pairs_checked += 1;
                        equal_pairs += 1;
                    }
                    // Only stash a fresh representative once per structural class (keeps buckets O(1)).
                    if reps.iter().all(|rep| !tree_eq(rep, sub)) {
                        reps.push(sub.clone());
                    }
                    // no-miss (eq⇒hash), O(1): a subtree's canonical source pins its hash. If the same
                    // source ever maps to a DIFFERENT hash, two structurally-equal trees hashed apart —
                    // find_clones would MISS the clone. (`src_of` is a canonical rendering: tree_eq ⟺
                    // equal source.)
                    let src = src_of(sub);
                    match by_source.get(&src) {
                        Some(&prev) => {
                            assert_eq!(
                                prev, h,
                                "two structurally-EQUAL subtrees hashed DIFFERENTLY (missed clone): {src}"
                            );
                            pairs_checked += 1;
                        }
                        None => {
                            by_source.insert(src, h);
                        }
                    }
                }
            }
            assert!(
                equal_pairs >= 1,
                "the sweep never hit two equal subtrees in one bucket — alphabet too sparse to test the \
                 no-collision direction"
            );
            assert!(
                pairs_checked >= 100,
                "swept a meaningful digest-agreement space, got {pairs_checked} pairs"
            );
        }

        #[test]
        fn equal_subtrees_hash_equal_regardless_of_position() {
            // the two `(g x)` occurrences are structurally equal → equal hash.
            let s = subj("(f (g x) (g x))");
            let (a, b) = match &s {
                crate::query::Tree::List(items, _) => (&items[1], &items[2]),
                _ => panic!(),
            };
            assert_eq!(hash_tree(a), hash_tree(b));
        }

        #[test]
        fn different_subtrees_hash_differently() {
            assert_ne!(hash_tree(&subj("(g x)")), hash_tree(&subj("(g y)")));
            assert_ne!(hash_tree(&subj("(+ a b)")), hash_tree(&subj("(- a b)")));
        }

        #[test]
        fn radix_is_part_of_the_hash_matching_tree_eq() {
            // `42` and `0x2A` are distinct leaves under tree_eq, so their hashes differ too.
            assert_ne!(hash_tree(&subj("42")), hash_tree(&subj("0x2A")));
        }

        #[test]
        fn no_alpha_equivalence() {
            // binding is the compiler's domain; `x` vs `y` differ structurally, hence by hash.
            assert_ne!(
                hash_tree(&subj("(let ((x 1)) x)")),
                hash_tree(&subj("(let ((y 1)) y)"))
            );
        }

        #[test]
        fn atom_and_list_do_not_collide() {
            assert_ne!(hash_tree(&subj("x")), hash_tree(&subj("(x)")));
        }

        #[test]
        fn node_size_counts_all_nodes() {
            assert_eq!(node_size(&subj("x")), 1);
            assert_eq!(node_size(&subj("(f a b)")), 4); // f, a, b, + the list
        }

        #[test]
        fn shape_hash_ignores_operand_leaves() {
            use crate::query::hash::shape_hash;
            // same skeleton, different operands → same shape.
            assert_eq!(
                shape_hash(&subj("(scale x 2)")),
                shape_hash(&subj("(scale y 3)"))
            );
        }

        #[test]
        fn shape_hash_keeps_head_and_structure() {
            use crate::query::hash::shape_hash;
            // different head → different shape.
            assert_ne!(
                shape_hash(&subj("(scale x 2)")),
                shape_hash(&subj("(shift x 2)"))
            );
            // different arity / nesting → different shape.
            assert_ne!(shape_hash(&subj("(f a b)")), shape_hash(&subj("(f a)")));
            assert_ne!(
                shape_hash(&subj("(f a b)")),
                shape_hash(&subj("(f a (g b))"))
            );
        }
    }

    mod clone_tests {
        use super::subj;
        use crate::query::clones::find_clones;

        #[test]
        fn finds_a_repeated_subtree() {
            // `(g x)` occurs twice.
            let s = subj("(f (g x) (h (g x)))");
            let classes = find_clones(&s, 2, None);
            assert_eq!(classes.len(), 1, "{classes:?}");
            assert_eq!(classes[0].exemplar, "(g x)");
            assert_eq!(classes[0].sites.len(), 2);
        }

        #[test]
        fn min_size_filters_out_trivial_clones() {
            // `x` recurs but is a single node; min_size 2 drops it.
            let s = subj("(f x x x)");
            assert!(find_clones(&s, 2, None).is_empty());
            // min_size 1 would catch it (one class of the atom `x`).
            let small = find_clones(&s, 1, None);
            assert_eq!(small.len(), 1);
            assert_eq!(small[0].sites.len(), 3);
        }

        #[test]
        fn reports_maximal_clones_only() {
            // `(a (b c))` recurs; report the whole thing, NOT also its inner `(b c)`.
            let s = subj("(list (a (b c)) (a (b c)))");
            let classes = find_clones(&s, 2, None);
            assert_eq!(classes.len(), 1, "only the maximal clone: {classes:?}");
            assert_eq!(classes[0].exemplar, "(a (b c))");
        }

        #[test]
        fn ranks_largest_first() {
            // a big clone `(big p q r)` (twice) and a small `(m n)` (twice); big ranks first.
            let s = subj("(prog (big p q r) (big p q r) (m n) (m n))");
            let classes = find_clones(&s, 2, None);
            assert_eq!(classes.len(), 2);
            assert!(classes[0].size > classes[1].size, "{classes:?}");
            assert_eq!(classes[0].exemplar, "(big p q r)");
        }

        #[test]
        fn no_clones_when_all_distinct() {
            assert!(find_clones(&subj("(f (g a) (h b))"), 2, None).is_empty());
        }

        #[test]
        fn three_occurrences_are_one_class_of_three() {
            let s = subj("(do (k v) (k v) (k v))");
            let classes = find_clones(&s, 2, None);
            assert_eq!(classes.len(), 1);
            assert_eq!(classes[0].sites.len(), 3);
        }

        #[test]
        fn clone_detection_never_panics_and_holds_invariants_on_arbitrary_trees() {
            // `find_clones` / `find_near_clones` run over arbitrary parsed trees (the `cdz clones` codemod
            // on any program), with subtree hashing, maximal-clone filtering, and near-clone metavar
            // inference — none of which may PANIC, at any `min_size`. Fuzz over SplitMix64-generated
            // s-expr trees and assert the structural invariants each result must hold:
            //   * every clone class has ≥2 sites (the definition of a clone), a positive size, and (for
            //     exact clones) an exemplar that RE-READS to a tree (the printer emitted valid s-expr);
            //   * a near-clone class's inferred `,mK`-metavariable pattern likewise re-reads.
            use crate::query::clones::{Source, find_near_clones};
            let alphabet: Vec<char> = "() abcfgh0123".chars().collect();
            let mut rng = super::super::tests::SplitMix64(0x0c10_e5f0_0d1c_a5f1);
            for len in 0..=40usize {
                for _ in 0..80 {
                    let s: String = (0..len)
                        .map(|_| alphabet[(rng.next() as usize) % alphabet.len()])
                        .collect();
                    // Only well-formed trees — clone detection consumes a parsed tree, not raw text.
                    let Ok(arena) = crate::sexpr::read(&s) else {
                        continue;
                    };
                    let tree = crate::query::Tree::of(&arena);
                    for min_size in [1usize, 2, 3] {
                        // Exact clones: never panic; each class is a real (≥2-site, sized) class whose
                        // exemplar is valid s-expr.
                        for class in find_clones(&tree, min_size, None) {
                            assert!(class.sites.len() >= 2, "a clone class has ≥2 sites");
                            assert!(class.size >= min_size, "a clone respects min_size");
                            assert!(
                                crate::sexpr::read(&class.exemplar).is_ok(),
                                "exemplar re-reads: {:?}",
                                class.exemplar
                            );
                        }
                        // Near clones: never panic; the inferred pattern is valid s-expr.
                        let src = Source {
                            tree: &tree,
                            spans: None,
                            file: None,
                        };
                        for nc in find_near_clones(std::slice::from_ref(&src), min_size) {
                            assert!(
                                crate::sexpr::read(&nc.pattern).is_ok(),
                                "near-clone pattern re-reads: {:?}",
                                nc.pattern
                            );
                        }
                    }
                }
            }
        }
    }

    mod antiunify_tests {
        use super::subj;
        use crate::query::antiunify::{anti_unify, render_pattern};
        use crate::query::{Pattern, Tree, count};

        /// The count of holes (distinct metavars) in a generalization.
        fn hole_count(g: &crate::query::antiunify::Generalization) -> usize {
            g.holes.len()
        }

        #[test]
        fn generalizes_a_differing_operand_to_one_hole() {
            let a = subj("(scale x 2)");
            let b = subj("(scale x 3)");
            let g = anti_unify(&[&a, &b]);
            // shared `(scale x _)` with one hole for the last operand.
            assert_eq!(render_pattern(&g.pattern), "(scale x ,m0)");
            assert_eq!(hole_count(&g), 1);
            assert_eq!(g.holes[0].len(), 2); // one column entry per instance
            assert_eq!(g.holes[0][0].to_sexpr(), "2");
            assert_eq!(g.holes[0][1].to_sexpr(), "3");
        }

        #[test]
        fn two_differing_positions_are_two_holes() {
            let a = subj("(f 1 2)");
            let b = subj("(f 3 4)");
            let g = anti_unify(&[&a, &b]);
            assert_eq!(render_pattern(&g.pattern), "(f ,m0 ,m1)");
            assert_eq!(hole_count(&g), 2);
        }

        #[test]
        fn identical_columns_share_a_hole() {
            // both operands differ by the SAME per-instance values → one shared metavar.
            let a = subj("(pair k k)");
            let b = subj("(pair j j)");
            let g = anti_unify(&[&a, &b]);
            // column0 = [k, j], column1 = [k, j] → same key → shared `,m0`.
            assert_eq!(render_pattern(&g.pattern), "(pair ,m0 ,m0)");
            assert_eq!(hole_count(&g), 1);
        }

        #[test]
        fn recurses_into_nested_structure() {
            let a = subj("(let ((x 1)) (+ x 1))");
            let b = subj("(let ((x 2)) (+ x 2))");
            let g = anti_unify(&[&a, &b]);
            // 1↔2 appears in two positions with the SAME column [1,2] → shared hole.
            assert_eq!(render_pattern(&g.pattern), "(let ((x ,m0)) (+ x ,m0))");
            assert_eq!(hole_count(&g), 1);
        }

        #[test]
        fn differing_head_generalizes_the_whole_node() {
            // different heads can't align positionally → the whole thing is one hole.
            let a = subj("(f a)");
            let b = subj("(g a)");
            let g = anti_unify(&[&a, &b]);
            assert_eq!(render_pattern(&g.pattern), ",m0");
            assert_eq!(hole_count(&g), 1);
        }

        #[test]
        fn identical_instances_have_no_holes() {
            let a = subj("(f a b)");
            let b = subj("(f a b)");
            let g = anti_unify(&[&a, &b]);
            assert_eq!(hole_count(&g), 0);
            assert_eq!(render_pattern(&g.pattern), "(f a b)");
        }

        #[test]
        fn the_emitted_pattern_matches_every_instance() {
            // The round-trip that makes this the inverse of the matcher: anti-unify → the pattern,
            // compiled, matches each original instance.
            let insts = [
                subj("(scale x 2)"),
                subj("(scale x 3)"),
                subj("(scale x 9)"),
            ];
            let refs: Vec<&Tree> = insts.iter().collect();
            let g = anti_unify(&refs);
            let pat = Pattern::compile(&render_pattern(&g.pattern)).expect("valid pattern");
            for inst in &insts {
                assert_eq!(count(&pat, inst), 1, "pattern matches {}", inst.to_sexpr());
            }
        }

        #[test]
        fn shared_hole_pattern_enforces_consistency() {
            // `(pair ,m0 ,m0)` must match `(pair k k)` but NOT `(pair k j)` — the shared metavar is a
            // real consistency constraint, exactly as in the matcher.
            let a = subj("(pair k k)");
            let b = subj("(pair j j)");
            let g = anti_unify(&[&a, &b]);
            let pat = Pattern::compile(&render_pattern(&g.pattern)).unwrap();
            assert_eq!(count(&pat, &subj("(pair q q)")), 1);
            assert_eq!(count(&pat, &subj("(pair q r)")), 0);
        }

        #[test]
        fn the_anti_unified_pattern_matches_every_instance_over_generated_sets() {
            // The SOUNDNESS invariant of anti-unification (the inverse of the matcher), swept: for ANY set
            // of instances, the least-general generalization must MATCH EVERY instance it was built from —
            // that is what makes it a valid near-clone / refactor suggestion. Only `the_emitted_pattern_
            // matches_every_instance` pinned this on ONE hand set (3× `(scale x N)`). A generalization that
            // failed to match one of its own inputs (a hole/consistency-constraint or shape-alignment bug)
            // would emit a `cdz clones` / refactor pattern that does not cover its examples. Sweep random
            // instance-sets built by mutating a shared skeleton (so they DO share structure — the case
            // anti-unify is for) and assert the rendered pattern re-compiles and matches each instance.
            use super::{SplitMix64, Tree};
            fn gen_skeleton(rng: &mut SplitMix64, depth: usize) -> String {
                // A shared skeleton with `?` placeholders that each instance fills differently.
                let leaves = ["a", "b", "x", "?"]; // `?` = a per-instance-varying slot
                if depth == 0 || rng.next().is_multiple_of(3) {
                    return leaves[(rng.next() as usize) % leaves.len()].to_string();
                }
                let heads = ["f", "g", "scale", "pair"];
                let head = heads[(rng.next() as usize) % heads.len()];
                let n = 1 + (rng.next() as usize) % 3;
                let kids: Vec<String> = (0..n).map(|_| gen_skeleton(rng, depth - 1)).collect();
                format!("({head} {})", kids.join(" "))
            }
            // Fill each `?` in the skeleton with a per-instance leaf, so all instances share the skeleton
            // and differ only at the `?` slots — exactly what anti-unify generalizes.
            fn fill(skeleton: &str, rng: &mut SplitMix64) -> String {
                let fillers = ["0", "1", "2", "k", "y", "z"];
                let mut out = String::new();
                for ch in skeleton.chars() {
                    if ch == '?' {
                        out.push_str(fillers[(rng.next() as usize) % fillers.len()]);
                    } else {
                        out.push(ch);
                    }
                }
                out
            }
            let mut rng = SplitMix64(0xa274_de50_c0de_1a7e);
            let mut checked = 0usize;
            for _ in 0..2000 {
                let depth = 1 + (rng.next() as usize) % 4;
                let skeleton = gen_skeleton(&mut rng, depth);
                let k = 2 + (rng.next() as usize) % 3; // 2..=4 instances
                let texts: Vec<String> = (0..k).map(|_| fill(&skeleton, &mut rng)).collect();
                // Each instance must parse (the skeleton is well-formed s-expr with leaves filled in).
                let arenas: Vec<_> = texts
                    .iter()
                    .filter_map(|t| crate::sexpr::read(t).ok())
                    .collect();
                if arenas.len() != k {
                    continue;
                }
                let trees: Vec<Tree> = arenas.iter().map(Tree::of).collect();
                let refs: Vec<&Tree> = trees.iter().collect();
                let g = anti_unify(&refs);
                // The rendered pattern must RE-COMPILE (a generalization always renders a valid pattern)…
                let Ok(pat) = Pattern::compile(&render_pattern(&g.pattern)) else {
                    panic!(
                        "anti-unified pattern must compile: {}",
                        render_pattern(&g.pattern)
                    );
                };
                // …and MATCH EVERY instance it was generalized from (≥1 occurrence at the root).
                for (t, txt) in trees.iter().zip(&texts) {
                    assert!(
                        count(&pat, t) >= 1,
                        "anti-unified pattern {} must match its instance {txt}",
                        render_pattern(&g.pattern)
                    );
                }
                checked += 1;
            }
            assert!(
                checked > 500,
                "swept a meaningful anti-unification space, got {checked}"
            );
        }
    }

    /// Match `pattern` against `subject` and return the FIRST match's bindings (the capture lint runs
    /// per matched site).
    fn first_bindings(pattern: &str, subject: &str) -> Bindings {
        let s = subj(subject);
        let mut m = search(&pat(pattern), &s, None);
        assert!(
            !m.is_empty(),
            "pattern {pattern:?} did not match {subject:?}"
        );
        m.remove(0).bindings
    }

    #[test]
    fn capture_risk_flags_the_breaker_repro() {
        // The breaker's case: rewriting `(+ ,e 1)` -> `(let ((x 100)) (+ ,e x))` over a program where the
        // matched `,e` IS the variable `x`. The template introduces a binder `x`, and `,e` bound the tree
        // `x` (a free `x`), so splicing it under the new `let` silently re-scopes it — a capture. The lint
        // must flag (binder x, metavar e).
        let binds = first_bindings("(+ ,e 1)", "(+ x 1)");
        let risks = tmpl("(let ((x 100)) (+ ,e x))").capture_risks(&binds);
        assert_eq!(
            risks,
            vec![CaptureRisk {
                binder: "x".to_string(),
                metavar: "e".to_string(),
                is_splice: false,
            }],
            "the template's `x` binder captures the free `x` inside the matched `,e`"
        );
    }

    #[test]
    fn capture_risk_marks_a_splice_metavar_so_the_sigil_is_right() {
        // When the template references the metavar as a SPLICE (`,@rest`), the risk must record it so a
        // caller prints `,@rest` (not `,rest`). `,@rest` binds a run `[a x b]`; `x` is free in it, and the
        // template binds `x` in a `let`.
        let binds = first_bindings("(f ,@rest)", "(f a x b)");
        let risks = tmpl("(let ((x 0)) (g ,@rest))").capture_risks(&binds);
        assert_eq!(
            risks,
            vec![CaptureRisk {
                binder: "x".to_string(),
                metavar: "rest".to_string(),
                is_splice: true,
            }],
            "a splice metavar's risk carries is_splice=true"
        );
        // A SINGLE metavar (`,e`) is is_splice=false (the breaker repro already asserts the value; this
        // pins the contrast against the splice case above).
        let binds = first_bindings("(+ ,e 1)", "(+ x 1)");
        assert!(
            !tmpl("(let ((x 0)) ,e)").capture_risks(&binds)[0].is_splice,
            "a single metavar's risk is is_splice=false"
        );
    }

    #[test]
    fn no_capture_when_binder_name_is_fresh_or_metavar_tree_lacks_it() {
        // Same template shape, but `,e` bound `y` (not `x`): the template's `x` binder does NOT occur free
        // in `y`, so no capture.
        let binds = first_bindings("(+ ,e 1)", "(+ y 1)");
        assert!(
            tmpl("(let ((x 100)) (+ ,e x))")
                .capture_risks(&binds)
                .is_empty(),
            "a fresh binder name captures nothing"
        );
        // A template with NO binder at all never risks capture, whatever the match bound.
        let binds = first_bindings("(+ ,e 1)", "(+ x 1)");
        assert!(
            tmpl("(* ,e 2)").capture_risks(&binds).is_empty(),
            "a binder-free template has no capture risk"
        );
        // The matched tree contains `x` only in HEAD position (`(x 1)` — an application/operator head, not
        // a free variable reference), so a `let ((x …))` binder does not capture it.
        let binds = first_bindings("(f ,e)", "(f (x 1))");
        assert!(
            tmpl("(let ((x 0)) ,e)").capture_risks(&binds).is_empty(),
            "a name only in head position is not a captured free variable"
        );
    }

    #[test]
    fn capture_risk_recognizes_fn_and_def_and_nested_binders() {
        // A `fn` parameter binder captures a free occurrence in a matched metavar.
        let binds = first_bindings("(g ,body)", "(g (+ n 1))");
        assert_eq!(
            tmpl("(fn (n) ,body)").capture_risks(&binds),
            vec![CaptureRisk {
                binder: "n".to_string(),
                metavar: "body".to_string(),
                is_splice: false,
            }],
            "a fn parameter `n` captures the free `n` in the spliced body"
        );
        // A `def` signature binds the function name AND params; a param capture is flagged.
        let binds = first_bindings("(g ,body)", "(g (+ a 1))");
        assert_eq!(
            tmpl("(def (h a) ,body)").capture_risks(&binds),
            vec![CaptureRisk {
                binder: "a".to_string(),
                metavar: "body".to_string(),
                is_splice: false,
            }]
        );
        // Shadowing: an INNER binder that re-binds the name means the free occurrence under it is NOT free
        // there — but the metavar tree itself is what matters, and here `,body` bound `(+ n 1)` with a free
        // `n`, while the template's OUTER `fn (n)` still captures it. (Confirms we scan template binders
        // against the metavar's own free set, the intended lint.)
        let binds = first_bindings("(g ,body)", "(g (+ n 1))");
        assert!(
            !tmpl("(fn (n) (let ((k 0)) ,body))")
                .capture_risks(&binds)
                .is_empty()
        );
    }

    #[test]
    fn capture_risk_recognizes_match_arm_binders() {
        // A bare-name match-arm pattern `m` binds `m` for the arm; if a spliced metavar's tree has a free
        // `m`, that is a capture. `(match e (m ,body))` over `,body = (+ m 1)`.
        let binds = first_bindings("(g ,body)", "(g (+ m 1))");
        assert_eq!(
            tmpl("(match e (m ,body))").capture_risks(&binds),
            vec![CaptureRisk {
                binder: "m".to_string(),
                metavar: "body".to_string(),
                is_splice: false,
            }],
            "a match-arm binder `m` captures the free `m` in the spliced arm body"
        );
        // A CONSTRUCTOR-pattern arm `(Some n)`: `Some` is the constructor head (NOT a binder), `n` binds.
        let binds = first_bindings("(g ,body)", "(g (+ n 1))");
        assert_eq!(
            tmpl("(match e ((Some n) ,body))").capture_risks(&binds),
            vec![CaptureRisk {
                binder: "n".to_string(),
                metavar: "body".to_string(),
                is_splice: false,
            }],
            "the constructor `Some` is not a binder; the payload `n` is"
        );
        // A nullary constructor pattern `(C.R)` binds NOTHING — no false capture even when the metavar
        // tree mentions names.
        let binds = first_bindings("(g ,body)", "(g (f e))");
        assert!(
            tmpl("(match z ((C.R) ,body))")
                .capture_risks(&binds)
                .is_empty(),
            "a nullary constructor pattern binds nothing, so it cannot capture"
        );
        // A nested constructor pattern `(L.Cons h t)` binds both `h` and `t`.
        let binds = first_bindings("(g ,body)", "(g (+ h t))");
        let risks = tmpl("(match e ((L.Cons h t) ,body))").capture_risks(&binds);
        assert!(
            risks.contains(&CaptureRisk {
                binder: "h".to_string(),
                metavar: "body".to_string(),
                is_splice: false,
            }) && risks.contains(&CaptureRisk {
                binder: "t".to_string(),
                metavar: "body".to_string(),
                is_splice: false,
            }),
            "both `h` and `t` are arm binders; got {risks:?}"
        );
    }

    #[test]
    fn free_names_skips_heads_and_respects_inner_shadowing() {
        // `free_names` treats a head name as an operator (not a variable) and discounts names an inner
        // binder shadows. `(let ((x 1)) (+ x y))`: `x` is bound (not free), `+` is a head, `y` is free.
        let mut free = std::collections::BTreeSet::new();
        free_names(&subj("(let ((x 1)) (+ x y))"), &mut free);
        assert!(free.contains("y"), "y is free");
        assert!(!free.contains("x"), "x is bound by the let, not free");
        assert!(!free.contains("+"), "+ is an operator head, not a variable");
        assert!(
            !free.contains("let"),
            "the let keyword is a head, not a variable"
        );
    }

    #[test]
    fn inserting_a_match_arm_renders_valid_ml_arm_syntax_not_a_standalone_application() {
        // The CDZ0210 InsertArms fix (add a missing match arm) dropped on the ML surface: `textedit`'s
        // Ins splice rendered the new arm `(pat body)` STANDALONE, and a match arm only prints as
        // `| pat => body` INSIDE a `(match …)` — standalone it prints as an application `pat(body)`,
        // invalid ML in arm position, so the reparse/validate dropped the fix. `render_child` fixes it
        // by rendering a match-child as arm-syntax. This pins the ML insert produces VALID, REPARSEABLE
        // arm syntax (not `C(...)`). (s-expr was unaffected — an arm renders context-free there.)
        use crate::convert::Format;
        // A match missing the `C` arm; parse WITH spans so the preserving rewrite has anchors.
        let src = "def f(t) = match t with\n  | A => 1\n  | B => 2";
        let parsed = crate::parser::parse(src, crate::spans::FileId(0));
        assert!(parsed.errors.is_empty(), "parse: {:?}", parsed.errors);
        let old = Tree::of(&parsed.arenas);
        let spans = &parsed.spans;
        let span_of = |t: &Tree| -> Option<(usize, usize)> {
            t.origin()
                .and_then(|id| spans.get(id))
                .map(|s| (s.start, s.end))
        };
        // Build `new` = `old` with a `(C (trap "TODO: C"))` arm appended to the match. Locate the match
        // node (the def body) and append the arm.
        fn add_arm(t: &Tree) -> Tree {
            if let Tree::List(items, o) = t {
                if items.first().and_then(Tree::as_name) == Some("match") {
                    let mut kids = items.clone();
                    let arm = Tree::List(
                        vec![
                            Tree::Atom(Leaf::Name("C".into()), None),
                            Tree::List(
                                vec![
                                    Tree::Atom(Leaf::Name("trap".into()), None),
                                    Tree::Atom(Leaf::Str("TODO: C".into()), None),
                                ],
                                None,
                            ),
                        ],
                        None,
                    );
                    kids.push(arm);
                    return Tree::List(kids, *o);
                }
                return Tree::List(items.iter().map(add_arm).collect(), *o);
            }
            t.clone()
        }
        let new = add_arm(&old);
        let out = textedit::rewrite_preserving(src, &old, &new, &span_of, Format::Ml);
        // The spliced arm must be `| C => trap(...)` arm-syntax, NOT the standalone application `C(...)`.
        assert!(
            out.output.contains("| C => trap(\"TODO: C\")"),
            "arm inserted as `| C => …` arm-syntax; got:\n{}",
            out.output
        );
        assert!(
            !out.output.contains("C(trap("),
            "must NOT render the arm as a standalone application `C(trap(…))`; got:\n{}",
            out.output
        );
        // And the result must REPARSE cleanly (the whole point — an invalid render was dropped upstream).
        let reparsed = crate::parser::read_ml(&out.output);
        assert!(
            reparsed.ok(),
            "the ML with the inserted arm reparses: {:?}",
            reparsed.errors
        );
    }
}
