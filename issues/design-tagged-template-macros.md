# Vertical-ready: tagged-template macros (embedded DSLs), JSX as flagship library

**Design doc:** `implementation/design/DESIGN-tagged-template-macros.md` (landed on trunk).
**Subsystems:** `cadenza-syntax` (reader/printer — the fixed lex rule) + metaprogramming/rcdzc +
`implementation/compiler-ml/` (the expansion rule + the JSX library, in Cadenza).
**Suggested vertical area:** `metaprogramming` (owns the macro form), coordinating with `v-syntax`
(owns the reader/printer form in Increment 1).

## The one-line what
Ship a generic **tagged-template macro**: the reader lexes `tag"…text…{expr}…"` into a canonical node
carrying literal chunks (`List String`) + interpolation holes (`List Ast`); the tag is dispatched **by
binding** to an ordinary compile-time function `List String -> List Ast -> Ast`, evaluated on the
existing one-tier compile-time evaluator, whose returned `Ast` is spliced and type-checked. **JSX is a
library** on top (`jsx : List String -> List Ast -> Ast` emitting plain `(Tag (record …) (list …))`
calls — no blessed `View` type); `sql"…"`, `re"…"`, `css"…"` come free.

## Why it's legal (the key constraint)
`spec/capabilities/metaprogramming.md` §*A Macro Is Dispatched By Binding…* forbids reader extension by a
program. Resolution: the reader has ONE fixed rule (splits a string on holes, learns no grammar, runs no
user code); all DSL grammar lives in the compile-time function. See doc §1–§2.

## First increment
**Increment 0 (spec-first, blocking):** add a normative §*A Tagged Template Is A Binding-Dispatched
Compile-Time Macro Over Literal Chunks And Holes* to `metaprogramming.md`, plus a new
`spec/semantics/NN-tagged-templates.sexp` (id-echo macro, hole-splicing macro, malformed-tag reject, a
fixpoint case), and register the capability gate in `options/realized-capability-set/`.
**Then Increment 1:** the reader form in `cadenza-syntax` (lex `tag"…{…}…"` → `TaggedTemplate`, print
back, round-trip/codec/no-panic) — parses & prints only, no expansion yet.

Increments 2–5 (expansion rule → diagnostics → the JSX library/stress test → a second DSL) are laid out
in the doc §10. Open decisions (hole delimiter `{}` vs `${}`, lowercase-tag convention, typed quotes,
library location) each have a chosen default in doc §9.
