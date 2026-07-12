---
name: codemod
description: >-
  How to structurally search and rewrite Cadenza programs with the `cdz-syntax query` / `rewrite`
  codemod tool (in the cadenza-syntax crate). Read this whenever the task is finding or transforming
  code by SHAPE rather than text — structural search-and-replace, a rename/peephole/wrap refactor,
  a multi-rule simplifier pass, running a codemod across files/directories (apply in place, diff
  preview, or JSON), structurally diffing two programs (which subtrees changed), finding duplicated
  subtrees (clone detection), counting occurrences of a form, extracting spans of matching nodes, or
  building on the query/Tree matcher API. Covers the `,x`/`,@xs` pattern language, structural guards
  (`is-literal`/`head-is`/`matches`/`not`), relational context (`inside`/`has`), multi-rule sets +
  traversal strategy, multi-file/`--write`/`--diff`/`--json`, the `diff` (structural tree-diff) and
  `clones` (content-hash duplicate detection) subcommands, `lint` mode (anti-pattern checker / CI
  gate), the CLI, the library API, and the self-hosted sidecar map.
---

# Structural query & rewrite (codemod) for Cadenza

A codemod here is **structural search-and-replace over the homoiconic AST**, not a text patch.
Because every Cadenza form is `(head child…)` data, a pattern that matches code *is itself code* — a
rewrite rule reads in the shape of what it rewrites. The tool lives in `cadenza-syntax` (the `query`
module + the `cdz-syntax query`/`rewrite` subcommands). It is **Rung 2** of
`implementation/DESIGN-query-engine.md` (a built-in Rust driver) standing in for the eventual
self-hosted sidecar — see `implementation/PROTOTYPE-codemod.md` for the full write-up.

## The pattern language (not a new language)

A pattern and a rewrite template are ordinary **s-expression text** with two metavariable sigils the
reader already produces — no grammar is invented:

| Sigil  | Reads as             | Meaning                                              |
|--------|----------------------|------------------------------------------------------|
| `,x`   | `(unquote x)`        | bind **one** node to `x`                             |
| `,@xs` | `(unquote-splicing)` | bind a **run** of zero-or-more sibling nodes to `xs` |
| `,_`   |                      | wildcard: match one node, bind nothing               |
| `,@_`  |                      | wildcard run                                         |

Everything else is a **structural literal** that must match exactly. `(+ ,x 0)` matches an addition
whose second operand is the integer `0`, binding the first operand to `x`.

Rules to know:
- **Consistency (non-linear):** a repeated metavar must bind structurally-equal subtrees — `(+ ,x ,x)`
  matches `(+ a a)` and `(+ (f 1) (f 1))`, not `(+ a b)`. Wildcards `,_` are exempt.
- **One splice per list,** anchorable on both sides: `(call ,head ,@mid ,last)`.
- **Unbound template var ⇒ that site is left unchanged** (reject-don't-corrupt).

These are the same quote-pattern shapes (`` `(+ ,x 0) ``) that `spec/semantics/20-structural-editing.sexp`
pins as the self-hosted end state, so a rule written today reads identically later.

**Guards** constrain a metavar structurally: `,(name guard…)` (conjunctive). Guards are
`is-literal`, `is-name`, `is-int`/`is-float`/`is-str`/`is-bool`, `is-atom`/`is-list`,
`(head-is NAME)`, `(matches PAT)`, `(not GUARD)`. E.g. `(+ ,(x is-literal) ,y)`, `(f ,(g (head-is *)))`.
An unknown guard is rejected at compile time. **All guards are purely structural — there are NO
scope/binding or type guards** (`refs`/`defines`/`type-of`); binding analysis is the compiler's job,
not this layer's, to avoid duplicating the resolver.

## CLI

The binary is `cdz-syntax` (at `target/<profile>/cdz-syntax`, or `cargo run -p cadenza-syntax --bin
cdz-syntax --`). `--from`/`--to` infer from a FILE extension (`.cdz`/`.ml`→ml, `.sexp`→sexpr,
`.bin`→binary); stdin needs an explicit `--from`.

```console
# find every additive-identity site; prints "byte START-END: <form>" + "$var = …" bindings
$ printf 'f(a + 0, b * 1)' | cdz-syntax query '(+ ,x 0)' --from ml
byte 2-7: (+ a 0)
  $x = a

# just the count
$ printf 'g(x + 0) + (y + 0)' | cdz-syntax query '(+ ,e 0)' --from ml --count
2

# rewrite: (+ ,x 0) -> ,x   (result on stdout, "rewrote N site(s)" on stderr)
$ printf 'f(a + 0, b + 0)' | cdz-syntax rewrite '(+ ,x 0)' ',x' --from ml --to ml
cdz-syntax: rewrote 2 site(s)
f(a, b)

# wrap a call with a splice template
$ printf '(risky a b)' | cdz-syntax rewrite '(risky ,@args)' '(log (risky ,@args))' --from sexpr
(log (risky a b))

# a guard + a relational constraint
$ printf '(do (+ 1 a) (+ b c))' | cdz-syntax query '(+ ,(x is-literal) ,y)' --from sexpr --count
1
$ printf '(do (safe x) (danger (g x)))' | cdz-syntax query 'x' --from sexpr --inside '(danger ,@_)'
#0: x

# a multi-rule peephole set (first match wins), applied in one bottom-up pass
$ printf '(f (+ a 0) (* b 1) (* c 0))' | cdz-syntax rewrite --rules peephole.rules --from sexpr
(f a b 0)

# run over a whole directory; --json for machine-readable output
$ cdz-syntax query '(+ ,e 0)' src/ --json
[{"file":"src/a.ml","span":{"start":2,"end":7},"matched":"(+ x 0)","bindings":{"e":"x"}}, …]

# preview a rewrite (--diff, file untouched), then apply in place across a dir (--write)
$ cdz-syntax rewrite '(+ ,x 0)' ',x' src/a.ml --diff
$ cdz-syntax rewrite '(+ ,x 0)' ',x' src/ --write

# STRUCTURAL diff of two programs — which subtrees changed (not text lines)
$ cdz-syntax diff before.ml after.ml
1: replace (+ a 0) => a

# LINT: flag anti-patterns; exits non-zero on any `error` (a CI gate)
$ cdz-syntax lint src/ --rule '(lint (deprecated ,@_) "avoid" error)'
src/a.ml:2:3: error: avoid

# CLONES: find duplicated subtrees (copy-paste) within/across files
$ cdz-syntax clones src/ --min-size 4
clone: 3 occurrences, 4 nodes: (validate config strict)
  src/a.ml:1:11
  src/a.ml:2:11
  src/b.ml:1:11
```

- **Multiple FILEs and directories** are accepted (a DIR is recursed by extension); with no FILE,
  input is stdin. Human output over several files is grouped by `=== file ===`.
- `query` prints matches (span + bindings), `--count` the number (per file + a `total:`), or `--json`
  a flat array `[{file?, span, matched, bindings}]`. No match ⇒ empty, exit 0. Filter by structural
  context: `--inside`/`--has`/`--not-inside`/`--not-has PAT` (repeatable, conjunctive; ancestry/
  containment only — no scope).
- `rewrite PATTERN TEMPLATE` (or `rewrite --rules FILE`) prints the rewritten program to **stdout** and
  the count to **stderr** (stdout stays a clean, pipeable program). `--rules FILE` = `(rule PAT TMPL)`
  forms (first match wins); `--top-down` (default bottom-up); `--fixpoint` (bounded). Output modes:
  `--diff` previews a unified diff (file untouched), `--write` applies in place (FILE inputs only,
  changed files only), `--json` emits `{file?, count, rewritten}` (mutually exclusive with `--write`).
  Always **validates as a transaction**: the result is re-printed to ML + re-parsed; if it doesn't
  round-trip it is **rejected** (non-zero exit, nothing written) — never a half-applied edit.
- `diff FILE-A FILE-B` is a **structural** (subtree) diff, not a line diff: it reports each changed
  node by path — `PATH: replace OLD => NEW` / `add NEW` / `remove OLD`, or `--json`
  `[{path, kind, old?, new?}]`. Same-head lists recurse positionally (a changed operand is one
  point-change), differing arity aligns by LCS. Use it to review what a rewrite/edit changed to the
  tree, independent of formatting. (Distinct from `rewrite --diff`, which is a line-based unified diff.)
- `lint [FILE|DIR…] --rules FILE | --rule '(lint …)'` flags structural anti-patterns. A rule is
  `(lint PATTERN "message" [severity])`, severity ∈ `error`/`warning`/`info` (default `warning`),
  patterns use the full language (guards/splices). Each match → `FILE:line:col: SEVERITY: message`
  (or `--json`). **Exits non-zero iff any `error`-severity diagnostic fired** — a CI gate; warnings
  don't fail. Semgrep-lite for the AST.
- `clones [FILE|DIR…] [--min-size N]` finds **duplicated subtrees** (copy-paste) within and across
  files — the refactoring signal for "extract a shared def". Each subtree gets a Merkle content hash;
  a clone class is ≥2 structurally-equal subtrees (hash-bucketed, `tree_eq`-verified). `--min-size N`
  (node-count floor, default 3) drops trivial dupes; only maximal clones are reported, ranked
  biggest-first. Output: `clone: N occurrences, M nodes: <exemplar>` + `LABEL:line:col` per site, or
  `--json`. Purely structural (no α-equivalence).
- Because the parser recovers from errors, `query` works over **broken input** too: it warns on stderr
  and still runs the query over the recovered tree.

## Library API — `cadenza_syntax::query`

Reach for this when scripting a codemod in Rust (a multi-rule pass, a custom report). Everything
operates on an owned homoiconic `Tree` (`Atom | List`), the mirror of the built-in `Ast` sum; convert
at the edges and each node keeps its source `StructId` so a match reports a span.

```rust
use cadenza_syntax::query::{self, Pattern, Template, Tree};

let arena = /* from parser::read_ml / sexpr::read / codec::decode */;
let tree  = Tree::of(&arena);
let pat   = Pattern::compile("(+ ,x 0)")?;
let tmpl  = Template::compile(",x")?;

let hits  = query::search(&pat, &tree, Some(&spans));  // Vec<Match { node, span, bindings }>
let n     = query::count(&pat, &tree);
let out   = query::rewrite(&pat, &tmpl, &tree);          // Rewrite { tree, count }; bottom-up
let sat   = query::rewrite_fixpoint(&pat, &tmpl, &tree, 64);

// relational context (structural only): search filtered by ancestry/containment
use query::Query;
let q     = Query::new().inside(Pattern::compile("(danger ,@_)")?).not_has(Pattern::compile("(ok)")?);
let hits2 = query::search_with(&pat, &q, &tree, Some(&spans));

// multi-rule set + strategy
use query::{Rule, RuleSet, Strategy};
let rules = RuleSet::compile("(rule (+ ,x 0) ,x) (rule (* ,x 1) ,x)")?;   // or RuleSet::new(vec![Rule::new(p, t)])
let out2  = query::rewrite_rules(&rules, &tree, Strategy::BottomUp);       // or Strategy::TopDown
let sat2  = query::rewrite_rules_fixpoint(&rules, &tree, Strategy::BottomUp, 64);

// or the whole driver (what the CLI uses): load a target + project output, with validation
let (target, warnings) = query::driver::load(bytes, Format::Ml)?;
let report  = query::driver::report_matches(&pat, &q, &target);
let outcome = query::driver::apply_rewrite(&rules, Strategy::BottomUp, &target, Format::Ml, 100, false)?;

// machine-readable output + diff (dependency-free helpers)
let mjson = query::driver::matches_json(&pat, &q, &target, Some("a.ml"));    // [{file?,span,matched,bindings}]
let rjson = query::driver::rewrite_json(Some("a.ml"), outcome.count, &outcome.output);
let before = query::driver::project_target(&target, Format::Ml, 100)?;       // "before" side of a diff
let d = query::diff::unified(&before, &outcome.output, "a/a.ml", "b/a.ml");   // unified LINE diff text

// structural (subtree) diff of two trees
let changes = query::treediff::diff(&tree_a, &tree_b);  // Vec<Change { path: Vec<usize>, kind }>
let human   = query::driver::changes_report(&tree_a, &tree_b);   // "PATH: replace OLD => NEW" …
let cjson   = query::driver::changes_json(&tree_a, &tree_b);     // [{path, kind, old?, new?}]

// structural lint (pattern + message + severity; error-severity fails a run)
use query::lint::{self, LintSet};
let set   = LintSet::compile("(lint (deprecated ,@_) \"avoid\" error)")?;
let diags = lint::run(&set, &tree, Some(&spans));   // Vec<Diagnostic { message, severity, span, matched }>
let gate  = lint::has_error(&diags);                // true → CI should fail

// content hash + clone detection
let h       = query::hash::hash_tree(&tree);        // u64 Merkle hash; == iff tree_eq (fast eq filter)
let classes = query::clones::find_clones(&tree, 3, Some(&spans));   // Vec<CloneClass { exemplar, size, sites }>
// cross-file: query::clones::find_clones_multi(&[Source { tree, spans, file }], min_size)
```

Multi-file / `--write` / directory-walk plumbing lives in the CLI (the bin), not the library — the
library stays pure. Reach for the driver + `std::fs` if scripting a batch run in Rust.

`search` is top-down (nested matches included). `rewrite` is **bottom-up** — a node is matched against
its already-rewritten children, so a rule that exposes a new match collapses in one pass
(`(+ ,x 0) → ,x` fully reduces `(+ (+ x 0) 0)`).

## What is NOT here (and why)

- **Scope- / binding-based queries and guards** (`refs`, `defines`, scope-aware rename) — these need
  a name resolver, which the **compiler** owns. Kept out on purpose to avoid duplicating scope logic;
  every guard and relational constraint here is purely structural (shape/ancestry/containment).
- **Type-directed queries** (`type-of`, typed metavars) — reach into the checker; this layer is
  dependency-free. They belong to the driver once it links `rcdzc` (Rung 3).
- **Addressed edits** (`insert`/`replace`/`delete`/`move` by node path/content-id) — the
  `options/structural-interface/content-addressed-nodes.md` layer, above these primitives.
- **Type-checking a rewrite result** — the tool validates *well-formedness* (re-parse + round-trip);
  full type validation is Rung 3.

## Gotchas

- **Patterns are the s-expr surface, always** — write `(+ ,x 0)`, not `x + 0`. (The subject can be any
  surface via `--from`; the pattern/template text is s-expr.)
- **`rewrite` writes the program to stdout, the count to stderr** — capture stdout to get a clean
  result; don't grep stdout for "rewrote".
- **A repeated metavar is a constraint, not just a name** (`,x … ,x` demands equal subtrees). Use a
  fresh name or `,_` when you don't want that.
- **`--fixpoint` is bounded** (64 passes) precisely because a rule whose output re-matches its input
  (e.g. `,x → (w ,x)`) would otherwise loop; a bounded, non-fixed result is returned, not an error.
