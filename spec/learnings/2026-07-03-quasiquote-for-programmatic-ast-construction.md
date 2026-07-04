# Quasiquote for programmatic AST construction

*2026-07-03*

**What happened.** After adding `quote` to produce static AST values and establishing that the compiler operates on AST sums (not string-tagged structures), I realized: a compiler needs to construct AST **dynamically** — build instruction sequences where parts are computed. With only `quote`, constructing `(+ x 10)` where `x` is a variable means writing `(Ast.List (list (Ast.Name "+") x (Ast.Int 10)))` — verbose, unreadable, error-prone. A compiler building hundreds of instructions this way becomes unreadable. And when you need to splice a list of arguments into a call, manual construction is even worse.

**Why.** `quote` is uniform — it never evaluates. But metaprogramming needs **selective evaluation**: quote most of the structure, but evaluate specific holes. That's quasiquote. `` `(+ ,x 10)`` means "quote this structure, but evaluate `x` and insert its value here." The `,` (unquote) marks evaluation holes. And `,@` (unquote-splicing) splices a list: `` `(+ ,@args)`` with `args = (list 1 2 3)` produces `(+ 1 2 3)`, not `(+ (1 2 3))`. Without quasiquote, the compiler's instruction-building code is a wall of `Ast.List (list ...)` calls. With it, instruction construction reads like the instructions themselves. This is the difference between "you can build AST" (technically possible) and "building AST is ergonomic" (practically necessary for a compiler).

**The requirement it drove.**

`spec/capabilities/metaprogramming.md` §"Quasiquote Allows Selective Evaluation": The expression `` `<template>`` (quasiquote) MUST produce an AST value like `quote`, but with selective evaluation: any subexpression `,<expr>` (unquote) within the template MUST be evaluated and its result inserted into the AST at that position. Any subexpression `,@<list-expr>` (unquote-splicing) within the template MUST be evaluated to a list whose elements are spliced into the parent list at that position, not nested. Quasiquote MUST nest: ``` ``(+ ,,x)``` unquotes once to produce `` `(+ ,<x-value>)``. Unquote and unquote-splicing outside quasiquote MUST be a syntax error.

`spec/capabilities/compiler-pipeline.md` §"The Compiler Constructs Instructions Via Quasiquote": The compiler MUST use quasiquote to construct instruction AST values programmatically, so that instruction-building code is readable and maintainable rather than a wall of manual AST constructor calls.
