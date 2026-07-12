# Prototype — structural query & rewrite (codemod) for Cadenza

**Status:** working prototype, landed in `cadenza-syntax` (the `query` module + the `cdz-syntax
query`/`rewrite` subcommands). This is **Rung 2** of `DESIGN-query-engine.md`: a built-in set of
structural transforms over the AST, run by a Rust driver, projecting output through the existing
surfaces. It stands in for the eventual self-hosted **sidecar** (Rung 3), and is shaped so that
end-state drops in without changing the driver or the pattern surface.

## The one idea

A codemod is **structural search-and-replace over the homoiconic AST**, not a text patch. Because
every Cadenza form is `(head child…)` data, a pattern that matches code *is itself code* — a rewrite
rule reads in the shape of what it rewrites. The prototype does not invent a query language: it reuses
the s-expression surface plus two metavariable sigils the reader already produces.

## The pattern language

A pattern (and a rewrite template) is ordinary s-expression text with two metavariables:

| Sigil    | Reads as             | Meaning                                                        |
|----------|----------------------|---------------------------------------------------------------|
| `,x`     | `(unquote x)`        | bind **one** node to `x`                                       |
| `,@xs`   | `(unquote-splicing)` | bind a **run** of zero-or-more sibling nodes to `xs`          |
| `,_`     |                      | wildcard: match one node, bind nothing                        |
| `,@_`    |                      | wildcard run: match any run, bind nothing                     |

Everything else is a **literal** that must match structurally. Rules:

- **Consistency (non-linear).** A repeated metavariable must bind structurally-equal subtrees:
  `(+ ,x ,x)` matches `(+ a a)` and `(+ (f 1) (f 1))`, but not `(+ a b)`. (The Semgrep / ast-grep /
  Comby convention. Wildcards `,_` are exempt — each is independent.)
- **One splice per list.** At most one `,@` may appear among a list's direct children (an
  unambiguous run boundary). It may be **anchored** by fixed nodes on either side:
  `(call ,head ,@mid ,last)` pins the first and last argument and binds the middle run.
- **Unbound template variable ⇒ that site is left unchanged** (reject-don't-corrupt), never a
  half-instantiated tree.

These sigils are exactly the quote-pattern surface the structural-editing corpus pins as the
end-state (`spec/semantics/20-structural-editing.sexp`: `` `(+ ,x 0) `` ⇒ `x`), so patterns written
today against the prototype read identically to the self-hosted rewrite rules later.

### Guards (structural predicates on a metavar)

A metavariable can carry conjunctive **structural** constraints: `,(name guard…)`. All guards are
purely syntactic — deliberately no scope/binding or type predicates (`refs`/`defines`/`type-of`),
which need the compiler's resolver/checker and live there, not in this syntax-only layer.

| Guard              | Holds when the node is…                              |
|--------------------|------------------------------------------------------|
| `is-literal`       | a literal atom (int/float/string/bool — not a name)  |
| `is-name`          | a name atom                                          |
| `is-int` / `is-float` / `is-str` / `is-bool` | that specific literal kind |
| `is-atom` / `is-list` | any atom / any list                               |
| `(head-is NAME)`   | a list whose head name is `NAME`                     |
| `(matches PAT)`    | itself matches sub-pattern `PAT` (its captures are a pure test, not leaked) |
| `(not GUARD)`      | the negation                                         |

```
(+ ,(x is-literal) ,y)      # an addition whose first operand is a literal
(f ,(g (head-is *)))        # a call whose argument is a `*` application
(f ,(x is-atom (not is-name)))   # conjunctive: a non-name atom, i.e. a literal
```
An unknown guard is rejected at compile time. Guards compose with consistency: `(+ ,(x is-name) ,(x is-name))` needs two *equal names*.

### Relational context (structural ancestry / containment)

A `query` can be filtered by where a match sits in the tree (purely structural — no scope):

| Constraint            | Keeps a match when…                                        |
|-----------------------|------------------------------------------------------------|
| `--inside PAT`        | some **ancestor** matches `PAT`                            |
| `--has PAT`           | some strict **descendant** matches `PAT`                   |
| `--not-inside PAT`    | no ancestor matches `PAT`                                  |
| `--not-has PAT`       | no descendant matches `PAT`                               |

Each is repeatable and they compose conjunctively (`Query::inside/has/not_inside/not_has` in the API).

### Multi-rule sets & traversal strategy

A `rewrite` can apply an ordered **rule set** in one traversal — the peephole-simplifier shape. A
rules file is a sequence of `(rule PATTERN TEMPLATE)` forms; at each node the **first** matching rule
fires. Traversal is **bottom-up** by default (children first, so a rule that exposes a new match is
caught in the same pass); `--top-down` matches outermost-first (one rewrite per node per pass; combine
with `--fixpoint` to saturate).

```
;; peephole.rules
(rule (+ ,x 0) ,x)
(rule (* ,x 1) ,x)
(rule (* ,_ 0) 0)
```

## CLI

```text
cdz-syntax query   PATTERN [FILE|DIR…] [--from FMT] [--count] [--json]
                   [--inside PAT] [--has PAT] [--not-inside PAT] [--not-has PAT]
cdz-syntax rewrite PATTERN TEMPLATE [FILE|DIR…] [--from FMT] [--to FMT] [--width N]
                   [--fixpoint] [--top-down] [--diff | --write | --json]
cdz-syntax rewrite --rules FILE     [FILE|DIR…] …same flags…
cdz-syntax diff    FILE-A FILE-B    [--from FMT] [--json]     # structural (subtree) diff
cdz-syntax lint    [FILE|DIR…] --rules FILE | --rule '(lint …)' [--from FMT] [--json]
```

`--from`/`--to` are inferred from each FILE extension (`.cdz`/`.ml` → ml, `.sexp` → sexpr, `.bin` →
binary); `--to` defaults to the input format. With no FILE (or `-`), input is stdin. **Multiple FILEs
and directories** are accepted; a directory is recursed, picking up every file whose extension maps to
a surface (`--from` forces one). Results are path-sorted.

```console
$ printf 'f(a + 0, b * 1)' | cdz-syntax query '(+ ,x 0)' --from ml
byte 2-7: (+ a 0)
  $x = a

$ printf 'g(x + 0) + (y + 0)' | cdz-syntax query '(+ ,e 0)' --from ml --count
2

$ printf 'f(a + 0, b + 0)' | cdz-syntax rewrite '(+ ,x 0)' ',x' --from ml --to ml
cdz-syntax: rewrote 2 site(s)
f(a, b)

$ printf '(risky a b)' | cdz-syntax rewrite '(risky ,@args)' '(log (risky ,@args))' --from sexpr
cdz-syntax: rewrote 1 site(s)
(log (risky a b))

# guard: only additions with a literal first operand
$ printf '(do (+ 1 a) (+ b c))' | cdz-syntax query '(+ ,(x is-literal) ,y)' --from sexpr --count
1

# relational: an `x` only where it sits inside a (danger …)
$ printf '(do (safe x) (danger (g x)))' | cdz-syntax query 'x' --from sexpr --inside '(danger ,@_)'
#0: x

# a multi-rule peephole set applied in one pass
$ printf '(f (+ a 0) (* b 1) (* c 0))' | cdz-syntax rewrite --rules peephole.rules --from sexpr
cdz-syntax: rewrote 3 site(s)
(f a b 0)

# query a whole directory; --json is a flat, machine-readable array of matches
$ cdz-syntax query '(+ ,e 0)' src/ --json
[{"file":"src/a.ml","span":{"start":2,"end":7},"matched":"(+ x 0)","bindings":{"e":"x"}}, …]

# preview a rewrite as a unified diff (file untouched)
$ cdz-syntax rewrite '(+ ,x 0)' ',x' src/a.ml --diff
--- a/src/a.ml
+++ b/src/a.ml
@@ -1,1 +1,1 @@
-f(a + 0)
+f(a)

# apply in place across a directory (only files that change and validate are written)
$ cdz-syntax rewrite '(+ ,x 0)' ',x' src/ --write
cdz-syntax: src/a.ml: rewrote 1 site(s)

# STRUCTURAL diff of two programs — which subtrees changed (not text lines)
$ cdz-syntax diff before.ml after.ml
1: replace (+ a 0) => a

# LINT: flag anti-patterns from a rule set; exits non-zero on any `error` (a CI gate)
$ cat house.lint
(lint (deprecated ,@_) "deprecated call — replace it" error)
(lint (. (. ,_ ,_) ,_)  "deep member chain"           warning)
$ cdz-syntax lint src/ --rules house.lint
src/a.ml:2:3: error: deprecated call — replace it
$ echo $?
1
```

- **query** prints each match as `byte START-END: <matched s-expr>` (the span comes from the parser's
  span table; ML input carries spans, s-expr/binary do not), followed by `  $name = …` binding lines.
  `--count` prints the number (per file + a `total:` across a multi-file run). `--json` emits a flat
  array `[{file?, span, matched, bindings}]`. Over several FILEs/a DIR, human output is grouped by
  `=== file ===`.
- **rewrite** prints the rewritten program to stdout and the site count to stderr (so stdout stays a
  clean, pipeable program). `--diff` previews a unified diff instead (the file is not touched);
  `--write` applies in place (FILE inputs only, only files that actually change); `--json` emits
  `{file?, count, rewritten}`. `--write` is mutually exclusive with `--diff`/`--json`. Either way it
  **validates as a transaction**: the result is re-printed to ML and re-parsed; if it does not
  round-trip, the rewrite is **rejected** (non-zero exit, nothing written) — never a half-applied edit.
- **diff** structurally diffs two programs and reports the changed SUBTREES, each addressed by a path
  (the child-index route from the root): `PATH: replace OLD => NEW` / `add NEW` / `remove OLD`, or
  `--json` `[{path, kind, old?, new?}]`. Unlike `rewrite --diff` (a line-based unified diff), this is a
  *tree* diff: two same-head lists recurse positionally (a changed operand is one point-change, not a
  whole-form replace); differing arity aligns children by LCS (add/remove); a changed head or an
  atom↔list is a whole-node replace. Formatting-independent — it sees nodes, not lines.
- **lint** flags structural anti-patterns from a rule set — `(lint PATTERN "message" [severity])`
  forms (`--rules FILE` and/or inline `--rule '(lint …)'`), severity ∈ `error`/`warning`/`info`
  (default `warning`). Every match is a diagnostic `FILE:line:col: SEVERITY: message` (or `--json`
  `[{file?, line, col, severity, message, matched}]`). It **exits non-zero iff any `error`-severity
  diagnostic fired** — a structural-checker CI gate — while `warning`/`info` report without failing.
  Lint patterns use the full pattern language (guards, splices). It's a Semgrep-lite for the AST,
  built on the same matcher.

Because the parser is a recovering parser, `query` works over **broken input** too: it reports the
recoverable parse error on stderr and still runs the query over the recovered tree — the "total query
over incomplete source" the tooling capability calls for.

## Design (semantics)

- **Value model.** Everything operates on an owned `query::Tree` (`Atom | List`), the mirror of the
  built-in `Ast` sum a self-hosted sidecar destructures. Convert at the edges with `Tree::of(&arena)`
  / `Tree::to_arena()`; each node keeps its source `StructId` as provenance so a match reports a span.
- **Search** is top-down, reporting every match (nested matches included).
- **Rewrite** is **bottom-up**: children are rewritten first, then a node is matched against its
  *already-rewritten* form, so a rule that exposes a new match in its result is caught in the same
  pass (e.g. `(+ ,x 0) → ,x` collapses `(+ (+ x 0) 0)` fully). `--fixpoint` re-runs until stable,
  **bounded** (64 passes) to survive a rule whose output re-matches its input.
- **Tree-diff** recurses positionally through same-head/same-arity lists (a changed operand is a
  point-change at its path, not a whole-form replace), aligns unequal-arity children by LCS over
  structural equality (add/remove), and replaces a whole node on a changed head or atom↔list mismatch
  — so the change set reads like the edit, independent of layout.
- **Lint** runs each rule (a pattern + message + severity) over a program; every match is a
  diagnostic. It is a thin layer on the matcher (`search_with`) + a byte-span→`(line, col)` map. The
  only new "signal" is the exit-code contract: any `error`-severity diagnostic fails the run.

## Library API (`cadenza_syntax::query`)

```rust
Pattern::compile(&str)  -> Result<Pattern, PatternError>   // supports `,(x guard…)` guards
Template::compile(&str) -> Result<Template, PatternError>
search(&Pattern, &Tree, Option<&SpanTable>) -> Vec<Match>   // Match { node, span, bindings }
count(&Pattern, &Tree)  -> usize

// relational context (structural ancestry / containment; no scope)
Query { inside, has, not_inside, not_has: Vec<Pattern> }    // builder: .inside(p).has(p)…
search_with(&Pattern, &Query, &Tree, Option<&SpanTable>) -> Vec<Match>
count_with(&Pattern, &Query, &Tree) -> usize

// single-rule (convenience) and multi-rule + strategy
rewrite(&Pattern, &Template, &Tree)               -> Rewrite   // Rewrite { tree, count }
rewrite_fixpoint(&Pattern, &Template, &Tree, max) -> Rewrite
Rule::new(Pattern, Template) / Rule::compile_form(&Tree)
RuleSet::new(Vec<Rule>) / RuleSet::compile(&str)            // "(rule PAT TMPL) …"
Strategy::{BottomUp, TopDown}
rewrite_rules(&RuleSet, &Tree, Strategy)          -> Rewrite
rewrite_rules_fixpoint(&RuleSet, &Tree, Strategy, max) -> Rewrite

// driver: load a target + project output; the CLI is a thin shell over this
query::driver::load(&[u8], Format)                       -> Result<(Target, Vec<String /*warnings*/>), String>
query::driver::report_matches(&Pattern, &Query, &Target) -> String
query::driver::apply_rewrite(&RuleSet, Strategy, &Target, Format, width, fixpoint) -> Result<RewriteOutcome, String>
query::driver::matches_json(&Pattern, &Query, &Target, file: Option<&str>) -> String  // [{file?,span,matched,bindings}]
query::driver::rewrite_json(file: Option<&str>, count, rewritten) -> String           // {file?,count,rewritten}
query::driver::project_target(&Target, Format, width) -> Result<String, String>        // the "before" side of a --diff
query::driver::changes_report(&Tree, &Tree) -> String     // human tree-diff: "PATH: replace OLD => NEW" …
query::driver::changes_json(&Tree, &Tree)   -> String     // [{path, kind, old?, new?}]

// structural tree-diff
query::treediff::diff(&Tree, &Tree) -> Vec<Change>        // Change { path: Vec<usize>, kind: Replace|Add|Remove }
query::treediff::path_str(&[usize]) -> String             // "2.0" (or "<root>")

// structural lint (a pattern + message + severity; error-severity fails a run)
query::lint::LintSet::compile(&str) -> Result<LintSet, PatternError>   // "(lint PAT \"msg\" [severity]) …"
query::lint::run(&LintSet, &Tree, Option<&SpanTable>) -> Vec<Diagnostic>   // { message, severity, span, matched }
query::lint::has_error(&[Diagnostic]) -> bool
query::driver::line_col(src, byte) -> (usize, usize)                       // 1-based line:col
query::driver::lint_report(&LintSet, &Target, src, label) -> (String, bool /*had_error*/)
query::driver::lint_json(&LintSet, &Target, src, file) -> (String, bool)

// small dependency-free helpers (no serde)
query::json::{quote, Object, Array}     // JSON string builder
query::diff::unified(old, new, old_label, new_label) -> String   // LCS-based unified LINE diff, 3 lines context
```

## Mapping to the self-hosted end state (Rung 3)

| Prototype (now, Rust)                    | Self-hosted sidecar (later, Cadenza)                          |
|------------------------------------------|---------------------------------------------------------------|
| `query::Tree` (`Atom`/`List`)            | the built-in `Ast` sum (`Ast.Int`/`Ast.Name`/`Ast.List`)     |
| `Pattern` / `Template` (`,x` / `,@xs`)   | quote patterns `` `(+ ,x 0) `` in a `match` arm               |
| `search` / `rewrite` (Rust)             | `select` / `rewrite` combinators (§4 of the design doc)       |
| `driver::apply_rewrite` validation       | the engine's re-parse + **type-check** before accept (§5)     |
| `cdz-syntax query/rewrite` subcommands   | same driver, loading a user sidecar component (same ABI)      |

The gap to close for Rung 3 is the generics + recursion-over-sum-types work already in progress; the
prototype's `Tree` matcher is the executable spec for what those combinators must do.

## What is deliberately NOT here

- **Scope- / binding-based queries and guards** (`refs`, `defines`, scope-aware rename, free-var
  analysis) — these need a name resolver, which the **compiler** owns. Keeping them out avoids
  duplicating the compiler's scope logic; every guard and relational constraint here is purely
  structural (ancestry/containment/shape), never scope.
- **Type-directed queries** (`type-of`, typed metavars) — those reach into the checker; this layer is
  dependency-free (`cadenza-syntax` depends on no compiler crate). They belong to the driver once it
  links `rcdzc`.
- **Addressed edits by stable id** (`insert`/`replace`/`delete`/`move` by node path/content-id) — the
  `content-addressed-nodes` structural-interface layer, above these primitives. (Pattern-driven
  replace/apply/preview across files IS here now — via `rewrite --write`/`--diff`/`--json`.)
- **Type-checking the rewrite result** — the prototype validates *well-formedness* (re-parse +
  round-trip); full type validation is Rung 3.

## Tests

- `query` module unit tests (80): matching (metavars, consistency, variadic + anchoring, wildcard),
  **guards** (each predicate, `matches`/`not`, conjunction, consistency interaction, compile-time
  rejection), **relational context** (inside/has/not-*, strict-descendant, composition), **multi-rule
  sets + strategy** (first-match-wins, rule-file compile, bottom-up vs top-down, fixpoint), the
  **json** writer + **diff** engine, **tree-diff** (leaf/nested change, head-replace, add/remove,
  atom↔list, multiple changes), **lint** (severity parse/default, compile, run, multi-rule order,
  bad message/severity, full pattern language) + `line_col` + the driver's JSON/`project_target`.
- `tests/query_cli.rs` (34): the built binary driven over stdin AND over temp files/dirs — query/
  count/rewrite, guards, relational flags, `--rules`, `--top-down`, **multi-file & directory walk**,
  **`--json`** (query + rewrite), **`--diff`** (preview, file untouched), **`--write`** (in-place,
  no-op skip, stdin-rejected, mutually-exclusive-with-diff), **`diff` subcommand** (changed subtree +
  path, JSON, identical), **`lint` subcommand** (location+severity, error→exit 1, warning→exit 0,
  clean→exit 0, JSON, rules-file over a dir, no-rules error), cross-surface, broken-input recovery,
  bad-pattern / unknown-guard rejection.
