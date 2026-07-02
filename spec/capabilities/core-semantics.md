# Capability — Core Semantics

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines evaluation, binding, scope, control flow, pattern matching, failure and
> termination, equality and ordering, and the observable-behavior projection, and binds their
> behavior to the single executable-semantics corpus. Requirements realize
> [Core Principle III](../../constitution.md), [Core Principle V](../../constitution.md),
> [Core Principle IX](../../constitution.md), and [Core Principle XIV](../../constitution.md) and
> trace to [overview §3](../overview.md), [overview §4](../overview.md),
> [overview §10](../overview.md), and [overview §11](../overview.md).
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

The observable behavior of every language construct MUST match the construct's case in the executable-semantics corpus.

The compiler MUST NOT implement a construct's behavior in a way that disagrees with the corpus.

### Evaluation Is Deterministic Given Its Inputs And Capabilities

Evaluation of an expression MUST depend only on the expression, the bindings in scope, and the responses to the capabilities the expression invokes.

Evaluation MUST NOT depend on any outside influence the expression did not obtain through a binding in scope or a declared capability.

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

## Failure And Termination

### A Program Terminates In Exactly One Terminal Condition

A program run MUST end in exactly one terminal condition: a normal result, a trap of a defined kind, or exhaustion of the deterministic resource measure.

The terminal condition of a program run MUST be a deterministic function of its input and its declared capabilities' responses.

### A Trap Halts Execution At A Defined Point

A trap MUST halt the program at a defined point rather than continue with an unspecified value.

A trap MUST carry a defined kind that identifies why the program halted.

The kind of trap a given operation raises MUST be a deterministic function of the operation and its inputs.

### Partial Operations Have A Defined Outcome

An operation that has no result for some inputs MUST, on those inputs, either evaluate to a value the executable semantics defines or raise a trap of a defined kind.

An operation that has no result for some inputs MUST NOT produce an unspecified value.

## Equality And Ordering

### Equality Is Structural

Two values MUST be equal when they have the same type and their contents are equal component-wise.

Value equality MUST agree with the canonical byte form, so that two values are equal exactly when their canonical byte forms are identical.

### Floating-Point Equality Follows The Canonical Byte Form

A floating-point value MUST be equal to another floating-point value exactly when their canonical byte forms are identical, so that a negative zero is distinct from a positive zero and all not-a-number values are equal to one another.

### Ordering Where Offered Is Total

A type that offers an ordering MUST offer a total order over its values.

The ordering a type offers MUST be a deterministic function of the values compared.

## Observable Behavior

### Observable Behavior Is A Defined Projection Of A Run

The observable behavior of a program run MUST comprise its terminal condition, the value it produces on normal termination in canonical value form, and the ordered sequence of events it emitted.

The observable behavior of a program run MUST NOT include its internal representation, its timing, or its diagnostics.

### Emitted Events Are Ordered And Part Of Observable Behavior

The sequence of events a program emits MUST be observed in the order the program emitted them.

Two runs whose observable behaviors differ in any emitted event, in event order, or in terminal condition MUST be treated as behaving differently.
