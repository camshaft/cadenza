---
name: codemod
description: >-
  How to structurally search and rewrite Cadenza programs with the `cdz-syntax query` / `rewrite`
  codemod tool (in the cadenza-syntax crate). Read this whenever the task is finding or transforming
  code by SHAPE rather than text — structural search-and-replace, a rename/peephole/wrap refactor,
  counting occurrences of a form, extracting spans of matching nodes, or building on the query/Tree
  matcher API. Covers the `,x`/`,@xs` pattern language, the CLI, the library API, and how it maps to
  the eventual self-hosted sidecar.
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
```

- `query` prints matches (span + bindings) or, with `--count`, the number. No match ⇒ empty, exit 0.
- `rewrite PATTERN TEMPLATE` prints the rewritten program to **stdout** and the site count to
  **stderr** (so stdout stays a clean, pipeable program). `--fixpoint` re-applies until stable
  (bounded). It **validates as a transaction**: the result is re-printed to ML and re-parsed; if it
  doesn't round-trip, the rewrite is **rejected** (non-zero exit, no output) — never a half-applied edit.
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

// or the whole driver (what the CLI uses): load a target + project output, with validation
let (target, warnings) = query::driver::load(bytes, Format::Ml)?;
let report  = query::driver::report_matches(&pat, &target);
let outcome = query::driver::apply_rewrite(&pat, &tmpl, &target, Format::Ml, 100, false)?;
```

`search` is top-down (nested matches included). `rewrite` is **bottom-up** — a node is matched against
its already-rewritten children, so a rule that exposes a new match collapses in one pass
(`(+ ,x 0) → ,x` fully reduces `(+ (+ x 0) 0)`).

## What is NOT here (yet)

- **Type-directed queries** (`type-of`, `defines`, `refs`) — those reach into the compiler; this layer
  is dependency-free. They belong to the driver once it links `rcdzc` (Rung 3).
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
