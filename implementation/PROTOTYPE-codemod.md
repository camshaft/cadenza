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

## CLI

```text
cdz-syntax query   PATTERN          [FILE] [--from FMT] [--count]
cdz-syntax rewrite PATTERN TEMPLATE [FILE] [--from FMT] [--to FMT] [--width N] [--fixpoint]
```

`--from`/`--to` are inferred from the FILE extension (`.cdz`/`.ml` → ml, `.sexp` → sexpr, `.bin` →
binary); `--to` defaults to the input format. With no FILE (or `-`), input is stdin.

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
```

- **query** prints each match as `byte START-END: <matched s-expr>` (the span comes from the parser's
  span table; ML input carries spans, s-expr/binary do not), followed by `  $name = …` binding lines.
  `--count` prints just the number.
- **rewrite** prints the rewritten program to stdout and the site count to stderr (so stdout stays a
  clean, pipeable program). It **validates as a transaction**: the result is re-printed to ML and
  re-parsed; if it does not round-trip, the rewrite is **rejected** (non-zero exit, no output) — never
  a half-applied edit.

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

## Library API (`cadenza_syntax::query`)

```rust
Pattern::compile(&str)  -> Result<Pattern, PatternError>
Template::compile(&str) -> Result<Template, PatternError>
search(&Pattern, &Tree, Option<&SpanTable>) -> Vec<Match>   // Match { node, span, bindings }
count(&Pattern, &Tree)  -> usize
rewrite(&Pattern, &Template, &Tree)              -> Rewrite  // Rewrite { tree, count }
rewrite_fixpoint(&Pattern, &Template, &Tree, max) -> Rewrite

// driver: load a target + project output; the CLI is a thin shell over this
query::driver::load(&[u8], Format)               -> Result<(Target, Vec<String /*warnings*/>), String>
query::driver::report_matches(&Pattern, &Target) -> String
query::driver::apply_rewrite(&Pattern, &Template, &Target, Format, width, fixpoint) -> Result<RewriteOutcome, String>
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

- **Type-directed queries** (`type-of`, `defines`, `refs`) — those reach into the compiler; this layer
  is dependency-free (`cadenza-syntax` depends on no compiler crate). They belong to the driver once
  it links `rcdzc`.
- **Addressed edits** (`insert`/`replace`/`delete`/`move` by node path/content-id) — the
  `content-addressed-nodes` structural-interface layer, above these primitives.
- **Type-checking the rewrite result** — the prototype validates *well-formedness* (re-parse +
  round-trip); full type validation is Rung 3.

## Tests

- `query` module unit tests (28): matching (metavars, consistency, variadic + anchoring, wildcard),
  rewriting (bottom-up, splice templates, unbound-var no-op, fixpoint bound), the driver.
- `tests/query_cli.rs` (9): the built binary driven over stdin — query/count/rewrite, cross-surface,
  broken-input recovery, bad-pattern rejection, no-op reprint.
