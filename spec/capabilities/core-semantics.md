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

## Functions

### A Function Is A First-Class Value

A function MUST be a value that can be bound to a name, passed as an argument, returned as a result, and stored in a data structure, like any other value.

A function value MUST capture the bindings in scope at the point it is created, so that applying it later observes those captured bindings rather than the bindings in scope at the point of application.

### Functions Are Single-Arity

A function MUST take exactly one argument and return exactly one value.

Multi-parameter syntax `(fn (x y) body)` MUST desugar to curried form `(fn x (fn y body))`.

Multi-argument application `(f a b)` MUST desugar to curried application `((f a) b)`.

Partial application MUST be natural: applying a curried function to fewer arguments than its full chain returns a closure awaiting the remaining arguments.

### Applying A Function Binds Its Parameter To Its Argument

Applying a function to its argument MUST evaluate the function body in an environment that extends the function's captured environment with its parameter bound to the argument.

### Recursion Is Accountable Against The Resource Measure

A function that applies itself, directly or indirectly, MUST consume the deterministic resource measure so that unbounded recursion halts at a defined point rather than running unboundedly.

## Control Flow

### Conditionals Evaluate One Branch

A conditional MUST evaluate only the branch its condition selects.

Every branch of a conditional MUST be type-checked whether or not it is evaluated, so that an unevaluated branch cannot carry a deferred error.

## Sequencing

### A Sequencing Block Evaluates Its Forms In Order

A sequencing block MUST evaluate each of its forms in the order they are written.

A sequencing block MUST evaluate to the value of its last form.

A host call a form in a sequencing block makes MUST be observed before a host call made by a later form in the same block.

### A Declaration In A Sequencing Block Is Scoped To The Forms That Follow It

A declaration form in a sequencing block MUST bind its name for the forms that follow it in that block, so that a name a declaration introduces is in scope without a separate binding form.

## Pattern Matching

### Matching Is Exhaustive Or Rejected

A match whose patterns do not cover every value of the scrutinee's type MUST be a compile-time error.

A match MUST evaluate the branch of the first pattern that matches the scrutinee, as defined by the corpus.

### Bindings Introduced By A Pattern Are Scoped To Its Branch

A name a pattern binds MUST be in scope only in the branch guarded by that pattern.

## Types As First-Class Values

### Types Are First-Class Values

A Type MUST be a first-class value that can be bound to a name, passed as an argument, returned from a function, and inspected at runtime.

A type annotation `(: <expr> <Type>)` MUST carry its type as a value, not as a syntactic marker erased before evaluation.

The compiler MUST validate a type annotation against the annotated expression's static type at compile time.

The compiler MUST reject a program in which a type annotation's declared type does not match the annotated expression's static type before that program runs.

## Tuples

### A Tuple Is A Fixed-Size Positional Product

A tuple MUST be a fixed-size value whose elements are accessed positionally.

A tuple MAY hold elements of distinct types.

The empty tuple MUST be the unit value, so that unit and `()` are the same value.

A tuple MUST be deconstructible by pattern matching, so that `(tuple a b)` in pattern position binds the elements.

## Sum Types

### A Sum Type Constructor Is A Single-Arity Function Producing The Tagged Variant

A sum type constructor MUST be represented as a single-arity function that, when applied to exactly one argument, produces a Sum value tagged with the constructor's variant name.

A "nullary" variant MUST be a constructor whose argument type is Unit, not a pre-constructed Sum value.

Construction MUST be via application in all cases: `(Some 5)`, `(None unit)`, `(Sign.Zero unit)`.

A pattern matching a sum type constructor MUST have the form `(Ctor binder)` in all cases: `(Some x)`, `(None _)`, `(Sign.Zero _)`.

The prelude MUST bind Constructor values only for sum type variants, not pre-applied Sum values.

The pattern matcher MUST NOT special-case "nullary" vs "unary+" constructors by arity.

The pattern matcher MUST handle all constructor patterns uniformly as single-arity applications.

## Records, Maps, And Member Access

### A Record Has A Fixed Set Of Named Fields

A record MUST associate a fixed set of statically-known field names each with a value, where distinct fields may hold values of distinct types.

A map MUST associate keys with values as a dynamic homogeneous collection whose set of keys is not fixed by the value's form, distinct from a record's fixed field set.

### Member Access Projects A Record Field

Member access MUST project the field named by its key from the record it is applied to, evaluating to the value that field holds.

Member access applied to a value that is not a record MUST raise a trap of a defined kind rather than produce an unspecified value.

Member access naming a field the record does not contain MUST raise a trap of a defined kind rather than produce an unspecified value.

## Modules

### A Module Binds Its Name In Its Enclosing Scope

Evaluating a module MUST bind the module's declared name in the enclosing scope to the record of the module's exports, so that a module is named by its declaration without a separate binding form.

A reference to a module's name in its enclosing scope MUST resolve to that export record under the same lexical scope and shadowing rules as any other binding.

### A Module Evaluates To A Record Of Its Exports

Evaluating a module MUST produce a record whose fields are the names its definitions export bound to their values.

Each definition in a module MUST register its name and value as a field of the module's record.

A module's exported definition MUST be reachable by member access on the module's record.

### A Module Carries Its Manifest And Entry As Metadata

A module MUST carry the capabilities it declares as metadata separate from its exported fields, so that a declared capability is not itself an export.

A module's metadata MUST be reachable by a metadata key distinct from every export name, so that metadata access cannot collide with an export.

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

The Bool type MUST offer a total order in which false is less than true.

## Observable Behavior

### Observable Behavior Is A Defined Projection Of A Run

The observable behavior of a program run MUST comprise its terminal condition, the value it produces on normal termination in canonical value form, and the ordered sequence of host calls it made with the arguments it passed.

The observable behavior of a program run MUST NOT include its internal representation, its timing, or its diagnostics.

### Host Calls Are Ordered And Part Of Observable Behavior

The sequence of host calls a program makes MUST be observed in the order the program made them.

Two runs whose observable behaviors differ in any host call, in host-call order, or in terminal condition MUST be treated as behaving differently.

## The Unit Value

### An Expression Evaluated Only For Its Effect Yields The Unit Value

An expression evaluated only for the host call it makes MUST yield the value that host call returns, which is the unit value when the call's WIT signature returns unit.

A program that terminates normally without producing a value other than through the host calls it makes MUST produce the unit value as its normal-termination value.
