# Capability — Metaprogramming

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines compile-time evaluation and structural macros: the affordance by which a program
> is transformed as data, kept deterministic, hygienic, and bounded. Requirements realize
> [Core Principle II](../../constitution.md), [Core Principle III](../../constitution.md), and
> [Core Principle V](../../constitution.md) and trace to [overview §3](../overview.md) and
> [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

Because a program's canonical representation is code as data, a program can be transformed by other
code before it is compiled. This capability fixes that compile-time evaluation and macros are pure,
bounded by the deterministic resource measure, hygienic, and reproducible — so that metaprogramming,
the affordance that makes Cadenza structurally malleable by agents, cannot become a hole in
determinism, capability-safety, or reproducibility. It states the invariants; the concrete macro
surface is governed by the code-shape default.

## Quoting And The AST As A Sum Type

### Quote Produces An AST Value

The expression `(quote <expr>)` MUST evaluate to an AST sum type value representing the structure of `<expr>`, without evaluating `<expr>` itself.

The AST MUST be a sum type with variants for each syntactic form, deconstructible by pattern matching like any other sum type.

### Unquote Evaluates An AST Value As Code

The expression `(unquote <ast-value>)` MUST evaluate the AST value as code, producing the result the quoted expression would have produced.

Unquoting an AST value that does not represent a well-formed expression MUST trap of a defined kind.

### Quote And Unquote Are Inverses

Evaluating `(unquote (quote <expr>))` MUST produce the same result as evaluating `<expr>` directly, so that quote and unquote are inverses.

## Quasiquote For Programmatic AST Construction

### Quasiquote Allows Selective Evaluation

The expression `` `<template>`` (quasiquote) MUST produce an AST value like `quote`, but with selective evaluation at marked positions.

Any subexpression `,<expr>` (unquote) within a quasiquote template MUST be evaluated and its result inserted into the AST at that position.

Any subexpression `,@<list-expr>` (unquote-splicing) within a quasiquote template MUST be evaluated to a list whose elements are spliced into the parent list at that position, not nested as a single element.

Quasiquote MUST nest, so that ``` ``(+ ,,x)``` evaluates the inner unquote to produce `` `(+ ,<x-value>)``.

Unquote and unquote-splicing outside a quasiquote context MUST be a syntax error.

## Compile-Time Evaluation

### Compile-Time Evaluation Is Pure

Code evaluated at compile time MUST NOT perform ambient input or output.

Code evaluated at compile time MUST NOT depend on a wall-clock time.

Code evaluated at compile time MUST NOT depend on a source of randomness.

### Compile-Time Evaluation Is Bounded

Compile-time evaluation MUST be accountable against the deterministic resource measure so that it halts at a defined point.

## Macros

### Expansion Operates On The Canonical Representation

A macro MUST receive values of the canonical representation, so that it transforms a program as data rather than as text.

A macro MUST produce values of the canonical representation, so that it transforms a program as data rather than as text.

### Macros Are Hygienic

A name a macro introduces MUST NOT capture a name at the macro's use site unless the macro explicitly requests it.

A name a macro introduces MUST NOT be captured by a name at the macro's use site unless the macro explicitly requests it.

### Expansion Is Reproducible

Expanding the same program MUST produce the same expanded representation on every conforming compiler.

### Expansion Terminates

Macro expansion MUST terminate, halting at a defined point if it exceeds the deterministic resource measure.

## Meaning After Expansion

### Expansion Precedes And Feeds The Core Guarantees

The expanded representation MUST be subject to type checking exactly as if it had been written directly.

The expanded representation MUST be subject to capability checking exactly as if it had been written directly.

The expanded representation MUST be subject to the determinism guarantees exactly as if it had been written directly.

A macro MUST NOT be able to produce an expanded representation that reaches a capability the program's manifest does not enumerate.
