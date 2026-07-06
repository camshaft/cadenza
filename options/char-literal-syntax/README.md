# Decision — Char Literal Syntax

**The decision.** The reader spelling of a `Char` literal — the surface token that reads to a single
validated Unicode scalar (`Char`) node. The collections-and-text.md capability fixes the `Char` *type*
(a validated Unicode scalar in `U+0000..=U+10FFFF` excluding surrogates, with fallible conversion and
scalar-indexed string access); only how a scalar literal is *spelled* in text is left open, because —
like the `#"…"` symbol literal — it is a reader-level concern outside the compiler's trusted path
(ast-encoding.md §"Parsing And Printing Are Not In The Compiler's Trusted Path"). See
`spec/learnings/2026-07-05-char-is-a-validated-unicode-scalar-the-boundary-already-promises.md`.

**Why the language wants it.** A self-hosting lexer works one scalar at a time (`is-digit`, `is-alpha`,
peek-the-next-scalar), and comparing against scalar constants — "is this `(`?", "is this `0`?" — wants
a literal for the scalar, not a one-character `String` the program must then decompose. The `Char`
value is reachable without any reader change through `Char.from-int` (fallible) and `String.scalar-at`;
a literal is the ergonomic surface, so a generation MAY realize the value form before the literal
spelling.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- A char literal reads to a single Unicode scalar value; a literal naming a non-scalar (a surrogate or
  a code point above `U+10FFFF`) is a compile-time reader error `CDZ0002`
  (options/diagnostics-schema/coded-span-record.md; collections-and-text.md §"A Char Is A Single
  Unicode Scalar Value").
- The spelling MUST NOT collide with an existing reader token — the `'` quote shorthand is reserved for
  the homoiconic `quote` (spec/semantics/12-metaprogramming.sexp), and `#"…"` already reads to a
  `Symbol` (options/symbol-interning/).
- The literal MUST be able to denote any scalar, including non-printing and supplementary-plane
  scalars, so it carries an escape/numeric form as well as a direct-scalar form.
- The canonical tree carries a `Char` node (or the canonical `(Char.from-int …)` desugaring); the
  literal is display sugar, the way `a.b` carries only `(. a b)`.

## Choices

- [`hash-scalar-literal`](./hash-scalar-literal.md) — `#\a` for a direct scalar and `#\u+1F600` for a
  numeric scalar, reusing the `#`-sigil family that `#"…"` (symbol) already established, with the same
  string-escape set for the direct form. **The default.**

DEFAULT: hash-scalar-literal
