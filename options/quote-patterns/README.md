# Decision — Quote Patterns

**The decision.** Whether — and how — a quasiquote template may appear in **pattern** position, so
that a program destructures an abstract-syntax-tree value by writing the shape it expects rather than
spelling out the `Ast.*` sum constructors by hand. The metaprogramming spec already fixes that quote
and quasiquote **construct** AST data and that the AST is an ordinary sum type "deconstructible by
pattern matching like any other sum type" (metaprogramming.md §"Quote Produces An AST Value"); it does
not fix whether the quasiquote *surface* extends to the pattern grammar, because that surface is the
choice this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The form reuses `match` and the pattern grammar rather than adding a new control construct — a quote
  pattern is a pattern like any other (core-semantics.md §"Pattern Matching").
- A quote pattern MUST be equivalent to the pattern built from the corresponding `Ast.*` sum
  constructors, so that structural equality and the AST encoding cannot distinguish a value matched
  through a quote pattern from the same value matched through the constructors — the pattern-position
  companion of the construction rule that `(quote 42)` and `(Ast.Int 42)` are one value
  (metaprogramming.md §"Quote Produces An AST Value"; ast-encoding.md §"The Encoding Is A Bijection
  With One Canonical Byte Form").
- Exhaustiveness is the existing rule: a match over an `Ast` scrutinee whose arms do not cover the AST
  sum is rejected `CDZ0210` (core-semantics.md §"Matching Is Exhaustive Or Rejected"); a quote pattern
  is not a special case.
- A quote pattern destructures over the **untyped** AST-analysis substrate, not the typed construction
  quote: the typed-quote obligation types the expression a macro *builds*, while a pattern may analyze
  arbitrary tree structure (metaprogramming.md §"A Typed Quote Carries The Type Of The Expression It
  Builds", 2nd sentence: the typed quote layers over the untyped analysis substrate).
- The AST it matches is the same value the compiler already builds by quasiquote and matches by the
  `Ast.*` constructors — the form adds a surface, not a new value or a second matching mechanism.

**Why this is an isolated decision.** The form is sugar over the AST sum type and the existing `match`:
a quasiquote pattern lowers to the nested `Ast.*` sum-patterns the language already matches (the corpus
already runs `(match (quote 42) ((Ast.Int n) n) …)` and `(match (quote (+ 1 2)) ((Ast.List elems) …))`
un-tagged, so the seed already destructures AST sums by constructor). The one genuinely new piece is a
**reader/lowering** step: recognizing a backtick in *pattern* position and desugaring it to those
constructor patterns. Changing the surface is an edit to a choice file here plus that lowering; it
touches no frozen contract, introduces no new value form, and reuses the existing exhaustiveness and
equality rules. It is realized by a later generation, not the seed
(`options/realized-capability-set/`); until then its corpus cases carry `(needs quote-patterns)` and
the seed's behavior gate skips them.

## Choices

- [`quasiquote-pattern`](./quasiquote-pattern.md) — the same `` ` ``/`,`/`,@` surface that constructs
  AST values, extended to pattern position: a literal subterm matches by equality, `,<pattern>` binds
  (or further matches) the sub-AST at its position, and a final `,@<name>` binds the remaining
  elements. **The default.**

DEFAULT: quasiquote-pattern
