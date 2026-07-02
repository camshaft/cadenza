# Capability — Core Semantics

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines evaluation, binding, scope, control flow, and pattern matching, and binds their
> behavior to the single executable-semantics corpus. Requirements realize
> [Core Principle III](../../constitution.md), [Core Principle V](../../constitution.md),
> [Core Principle IX](../../constitution.md), and [Core Principle XIV](../../constitution.md) and
> trace to [overview §3](../overview.md), [overview §10](../overview.md), and
> [overview §11](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes the invariants of evaluating a Cadenza program: how expressions reduce to
values, how names bind, how scope works, how control flow behaves, and what pattern matching
guarantees. It states the invariants; the concrete, case-by-case behavior of every construct is the
executable-semantics corpus under [`spec/semantics/`](../semantics/), which is this capability's
single source of truth. Where an invariant here and a corpus case appear to disagree, the corpus is
authoritative and the invariant is corrected to match.

## Evaluation

### Evaluation Is Deferred To The Corpus

The observable behavior of every surface construct MUST match the construct's case in the executable-semantics corpus.

The compiler MUST NOT implement a construct's behavior in a way that disagrees with the corpus.

### Evaluation Is Deterministic

Evaluation of an expression MUST depend only on the expression and the bindings in scope, and on no source of nondeterminism.

Evaluation of an expression MUST NOT observe an order among independent subexpressions beyond the order their data dependencies impose.

### Evaluation Is Bounded

Evaluation MUST be accountable against the deterministic resource measure so that a non-terminating program halts at a defined point rather than running unboundedly.

## Binding And Scope

### Binding Is Lexical

A name MUST resolve to the nearest enclosing binding of that name.

A reference to a name with no enclosing binding MUST be a compile-time error.

### Shadowing Is Well-Defined

A binding that shadows an outer binding of the same name MUST take effect for references in its scope as defined by the corpus.

## Control Flow

### Conditionals Evaluate One Branch

A conditional MUST evaluate only the branch its condition selects.

Every branch of a conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated branch cannot carry a deferred error.

## Pattern Matching

### Matching Is Exhaustive Or Rejected

A match whose patterns do not cover every value of the scrutinee's type MUST be a compile-time error.

A match MUST evaluate the branch of the first pattern that matches the scrutinee, as defined by the corpus.

### Bindings Introduced By A Pattern Are Scoped To Its Branch

A name a pattern binds MUST be in scope only in the branch guarded by that pattern.
