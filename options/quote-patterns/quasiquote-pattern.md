# Quote Patterns — Choice: quasiquote-pattern

> **The default choice for the `quote-patterns` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins the same `` ` ``/`,`/`,@` surface that
> constructs AST values, extended to pattern position, over the existing `Ast` sum type.

## The choice

The quasiquote surface serves two directions by reusing the constructor/pattern duality the language
already has — a variant `(Some 5)` builds and `(Some n)` destructures the same way:

- In **expression position**, `` `<template>`` **constructs** an `Ast` value (metaprogramming.md
  §"Quasiquote Constructs AST With Selective Evaluation"), with `,<expr>` embedding an evaluated
  subexpression and `,@<list-expr>` splicing a list of them.
- In **pattern position** (inside an ordinary `match`), `` `<template>`` **destructures** an `Ast`
  scrutinee: it matches the template's structure against the AST, matching literal subterms by
  equality, binding at each `,<pattern>`, and splicing the tail at a final `,@<name>`. The `` ` `` head
  constrains the scrutinee to `Ast`, exactly as `(Some n)` constrains it to a sum and `(bin …)`
  constrains it to `Bytes`.

No new control construct: a quote pattern is a pattern like any other, and it is **exactly equivalent**
to the pattern written with the `Ast.*` sum constructors — `` `(if ,c ,t ,e) `` *is*
`(Ast.List (list (Ast.Name "if") c t e))` as a pattern. The seed already matches the constructor form
(the un-tagged `(match (quote (+ 1 2)) ((Ast.List elems) …))` cases in 12-metaprogramming.sexp); the
quote pattern is the readable spelling of it.

## The template-to-pattern mapping

A quote pattern is read as a template and lowered position by position. Each syntactic form maps to the
`Ast.*` constructor pattern for that form:

| Template subterm | As a pattern, matches | Binds |
|---|---|---|
| a literal integer `42` | `(Ast.Int 42)` — by equality | — |
| a bare name `+` | `(Ast.Name "+")` — by equality | — |
| a literal string `"s"` | `(Ast.Str "s")` — by equality | — |
| a compound `(h a b …)` | `(Ast.List (list …))` of **exactly** that arity, each element matched positionally | — |
| `,<name>` | the sub-AST at this position | `<name>` to that `Ast` value |
| `,<pattern>` | the sub-AST at this position, further matched by `<pattern>` | `<pattern>`'s binders |
| `,@<name>` | **final list element only:** the remaining elements | `<name>` to the `(list …)` of remaining `Ast` values |

So a literal subterm is a match-by-equality against the AST node it denotes — the direct analogue of a
literal value pattern (`(match 2 (2 "two") …)`) and of a literal segment in a `bin` pattern. An unquote
is the binder: `,<name>` is the AST-position analogue of a bare name pattern, and `,<pattern>` nests an
ordinary pattern (including another quote pattern) at that position, so `` `(+ ,(Ast.Int n) ,y) ``
matches only an addition whose first operand is an integer literal and binds its value to `n`.

## Fixed arity by default

A compound template `` `(h a b) `` matches an `Ast.List` of **exactly** three elements; a form with a
different number of elements does not match. This is the direct reading of the constructor pattern
`(Ast.List (list (Ast.Name "h") a b))`, whose `(list …)` sub-pattern already fixes length. Variable
arity is expressed only through the explicit tail splice `,@`, never implicitly — the same discipline
`bin` uses, where the rest of a byte sequence is bound only through a final `(bytes rest)`.

## Tail splice is final-position only

`,@<name>` binds the remaining list elements and MUST be the **final** element of its enclosing
template — `` `(begin ,@stmts) `` binds `stmts` to the list of forms after `begin`. A `,@` anywhere but
last would require matching a variable-length gap in the middle of a fixed sequence, turning a single
positional scan into a search; it is an **ill-formed quote pattern**, rejected `CDZ0221` (the
`CDZ02xx` types-and-patterns band). This mirrors `bin`, where an unsized `(bytes rest)` is legal only
as the final segment. Splicing binds a **list**, never a single element: `` `(f ,@xs) `` binds `xs` to
the list of `f`'s arguments, the pattern-position dual of the construction rule that `,@` splices a
list's elements into the parent rather than nesting them.

## Exhaustiveness reuses the existing rule

A quote pattern never covers every AST value: a different head name, a different arity, or a scrutinee
that is a leaf where the pattern expects a list all fail to match. So a match over an `Ast` scrutinee
whose arms are only quote patterns does not cover the AST sum and is rejected `CDZ0210` — the same
rejection a sum match missing a variant gets (core-semantics.md §"Matching Is Exhaustive Or Rejected").
A bare-name pattern (equivalently, a `_` wildcard) matches any AST and so serves as the catch-all — the
`,`/`,@` marks are meaningful only inside a `` ` `` template, so a top-level catch-all is an ordinary
name, not an unquote. No special case: quote matching reuses exhaustiveness rather than adding a rule.

## Equality and encoding are the constructor form's

Because a quote pattern lowers to the `Ast.*` constructor patterns, a value matched through it is
matched through the very sum patterns the corpus already runs, so structural equality and the AST
encoding cannot tell the two spellings apart — the pattern-position companion of the construction
requirement that `(quote 42)`, `` `,1 `` embedding `1`, and `(Ast.Int 42)` are one AST value
(metaprogramming.md §"a quoted integer equals the same node built by the Ast.Int constructor";
ast-encoding.md §"The Encoding Is A Bijection With One Canonical Byte Form"). The pattern adds a
surface, not a second matching mechanism.

## Why this matters for self-hosting

A self-hosting compiler's core is AST **analysis** — it decodes the input program to an `Ast` value and
pattern-matches over it to lower each form (compiler-pipeline.md §"The Compiler Operates On AST
Values"; the AST-construction/evaluation split fixes that the compiler needs construction and analysis,
not `eval`, spec/learnings/2026-07-03-ast-construction-vs-ast-evaluation.md). Written with raw
constructors, recognizing a two-operand addition reads:

```
(match node
  ((Ast.List (list (Ast.Name "+") a b)) (lower-add a b))
  (_                                     (lower-other node)))
```

With a quote pattern the arm reads as the form it recognizes — the pattern-position mirror of the
construction idiom the compiler already uses to *build* instructions (metaprogramming.md §"quasiquote
makes instruction construction readable"):

```
(match node
  (`(+ ,a ,b) (lower-add a b))
  (other      (lower-other other)))
```

The `,`/`,@` marks are meaningful only **inside** a `` ` `` template; the catch-all `other` is an
ordinary bare-name pattern, exactly as in any sum match — a bare `,other` outside a quasiquote is the
existing "unquote outside quasiquote" syntax error (`CDZ0401`), not a pattern.

This is the payoff the form exists for: the compiler's largest body of code is a `match` over the AST,
and quote patterns let every arm read as the surface it lowers.

## Resolved forks

- **One quasiquote surface, dual** (not a separate `ast-match` construct or a distinct pattern
  namespace). Reusing the `` ` ``/`,`/`,@` reader in pattern position keeps patterns un-namespaced (the
  language namespaces no other pattern) and adds no keyword — the reader already lexes the marks; only
  their appearance in pattern position is new. A separate matching construct was rejected because it
  would give AST destructuring its own control form when `match` already suffices.
- **Fixed arity with an explicit final `,@`** (not Scheme-style ellipsis with mid-sequence
  repetition). A single tail splice covers the length-variable forms a compiler meets (an operator's
  arguments, a block's statements) while keeping a pattern a single positional scan; general
  mid-sequence repetition (`...` between fixed elements) is left to a later decision, taken only if a
  real pattern needs to match a variable gap flanked by fixed tails — the same posture `bin` takes on a
  first-class `Bits` value form.
- **Untyped destructuring substrate** (the quote pattern analyzes arbitrary tree structure; the typed
  quote types only the expression a macro *builds*). This is the metaprogramming spec's existing split,
  not a new one: "the typed quote MUST layer over the untyped abstract-syntax-tree analysis substrate
  rather than replace it, so that a macro may still analyze arbitrary tree structure while the
  expression it emits is type-checked."
