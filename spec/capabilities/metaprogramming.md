# Capability — Metaprogramming

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines compile-time evaluation and structural macros: the affordance by which a program
> is transformed as data, kept deterministic and hygienic. Requirements realize
> [Core Principle II](../../constitution.md) and [Core Principle III](../../constitution.md) and
> trace to [overview §3](../overview.md) and [overview §13](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

Because a program's canonical representation is code as data, a program can be transformed by other
code before it is compiled. This capability fixes that compile-time evaluation and macros are pure,
hygienic, and reproducible — so that metaprogramming, the affordance that makes Cadenza structurally
malleable by agents, cannot become a hole in determinism, capability-safety, or reproducibility. It
states the invariants; the concrete macro surface is governed by the code-shape default.

## AST Construction

### Quote Produces An AST Value

The expression `(quote <expr>)` MUST evaluate to an AST sum type value representing the structure of `<expr>`, without evaluating `<expr>` itself.

Quoting a collection construction — a list, a tuple, a record, a map, or a set — MUST produce that collection's own first-class AST variant (`Ast.ListCtor` / `Ast.TupleCtor` / `Ast.RecordCtor` / `Ast.MapCtor` / `Ast.SetCtor`, *type-system.md §The Abstract Syntax Tree Is An Ordinary Sum Type*) rather than a name-headed generic node, so that a reflected record is a `RecordCtor` of `FieldPair` values, a reflected map a `MapCtor` of `FieldPair` values, a reflected set a `SetCtor`, and a reflected member access `(. obj key)` an `Ast.Member` — no collection is reflected as a string- or name-headed node.

The AST MUST be a sum type with variants for each syntactic form, deconstructible by pattern matching like any other sum type.

### Quasiquote Constructs AST With Selective Evaluation

The expression `` `<template>`` (quasiquote) MUST produce an AST value like `quote`, but with selective evaluation at marked positions.

Any subexpression `,<expr>` (unquote) within a quasiquote template MUST evaluate `<expr>` normally and insert its result into the AST being constructed at that position.

Any subexpression `,@<list-expr>` (unquote-splicing) within a quasiquote template MUST evaluate `<list-expr>` to a list whose elements are spliced into the parent list at that position, not nested as a single element.

Quasiquote MUST nest, so that ``` ``(+ ,,x)``` evaluates the inner `,` to produce `` `(+ ,<x-value>)``.

Unquote and unquote-splicing outside a quasiquote context MUST be a syntax error.

Quote and quasiquote are construction primitives that produce AST data.

### A Quasiquote In Pattern Position Destructures An AST

A quasiquote template `` `<template>`` appearing in pattern position MUST destructure an abstract-syntax-tree scrutinee, matching the template's structure against the tree.

A quasiquote pattern MUST be equivalent to the pattern formed from the corresponding abstract-syntax-tree sum constructors, so that a value matched through a quasiquote pattern cannot be distinguished by structural equality or by the encoding from the same value matched through the constructors.

A literal subterm within a quasiquote pattern MUST match the abstract-syntax-tree node it denotes by equality, and a `,<pattern>` (unquote) subterm MUST match the sub-tree at its position against `<pattern>`, binding the sub-tree when `<pattern>` is a name.

A `,@<name>` (unquote-splicing) subterm within a quasiquote pattern MUST bind the remaining elements of its enclosing list as a list, and MUST appear only as the final element of its template.

A match over an abstract-syntax-tree scrutinee whose arms are quasiquote patterns MUST be subject to the exhaustiveness rule exactly as any other match, so that a quasiquote pattern is not a special case (core-semantics.md §"Matching Is Exhaustive Or Rejected").

A quasiquote pattern MUST layer over the untyped abstract-syntax-tree analysis substrate, so that it may destructure arbitrary tree structure — the dual of the construction quote, which carries the type of the expression it builds (§"A Typed Quote Carries The Type Of The Expression It Builds").

### Reflecting A Type To Its Definition AST

A program MUST be able to reflect a type value to the abstract-syntax-tree of that type's definition, so that a compiler authored in the language can inspect not only how an expression was written (quote) but how a type is defined, then analyze that definition with the ordinary AST machinery. The reflected value MUST be an ordinary AST value of the definition's declaration form (the same AST a quote of that declaration produces), reusing the abstract-syntax-tree sum's existing variants rather than a bespoke descriptor, so that it prints, encodes, decodes, and pattern-matches like any other AST.

Reflecting a type to its definition AST MUST offer both the generic form and the instantiated form: the generic form MUST reflect the declaration verbatim with its type parameters intact, and the instantiated form MUST substitute the type's concrete arguments for its parameters in the declaration, so that a caller chooses whether to observe the definition as declared or as specialized. For a type with no parameters the two forms MUST coincide.

Reflecting a type to its definition AST MUST be total over concrete types — a nominal or sum type reflects its declaration, and a structural type (a record, tuple, list, map, set, primitive) reflects its canonical type-surface form — and MUST reject a type that is not concrete (one carrying an unresolved type variable) rather than fabricate a definition.

The instantiated form MUST remain finite for a recursive or mutually-recursive type: substituting a type's concrete arguments MUST replace only the declaration's own parameter binders in its own body, leaving every nested type reference — including a self-reference — a named application that is not unfolded.

## AST Evaluation (Optional)

### Eval Is Optional For Macros And Interactive Use

The expression `(eval <ast-value>)` evaluating an AST value as code is an optional metaprogramming affordance for macros and interactive evaluation, not a core compiler requirement.

A generation that realizes macros or a REPL MAY provide `eval` to execute compile-time or interactive code.

A generation that does not realize macros or interactive evaluation need not provide `eval`.

The compiler MUST NOT require `eval` to compile programs — the compiler constructs and analyzes AST but does not execute dynamically-constructed AST.

## Compile-Time Evaluation

### Compile-Time Evaluation Is One Tier

Macro expansion, generic reduction, monomorphization, and constant folding MUST be the same compile-time evaluation mechanism rather than separate subsystems, so that there is one place the meaning of compile-time computation lives and the four cannot drift apart.

A macro MUST be an ordinary compile-time function over the abstract syntax tree, so that a macro is not a distinct construct but an application of the one compile-time tier to a program's data.

### Compile-Time Evaluation Is Pure

Code evaluated at compile time MUST run in the empty effect row, so that its purity is a consequence of the effect model rather than a rule stated only for compile time (capabilities-and-effects.md §The Manifest Is The Escaping Effect Row).

Code evaluated at compile time MUST NOT reach a host function, so that it performs no ambient input or output and depends on no wall-clock time or source of randomness.

## Macros

### Expansion Operates On The Canonical Representation

A macro MUST receive values of the canonical representation, so that it transforms a program as data rather than as text.

A macro MUST produce values of the canonical representation, so that it transforms a program as data rather than as text.

### A Macro Is Dispatched By Binding, Not By Spelling

A macro MUST be dispatched by resolving the binding at the head of a form to a macro definition, so that whether a form is a macro use is determined by binding rather than by a heuristic over the head's spelling.

The reader MUST NOT be extensible by a program: syntax MUST grow at the abstract-syntax-tree level through macros rather than through reader macros, so that the text-to-canonical-representation reader stays outside the compiler's trusted path.

### A Tagged Template Is A Binding-Dispatched Compile-Time Macro Over Literal Chunks And Holes

A tagged template — an identifier written immediately before a string literal, with no intervening whitespace, such as `tag"…text…{expr}…"` — MUST lex to a single canonical abstract-syntax-tree node carrying the tag name, the literal string chunks between the interpolation holes, and the holes, so that an embedded foreign syntax is captured as ordinary program data.

The reader MUST NOT run any program code or learn any grammar when lexing a tagged template, so that the reader stays outside the compiler's trusted path exactly as it does for every other form.

The reader MUST, when lexing a tagged template, only split the string body into literal chunks and `{…}` holes.

Each interpolation hole `{expr}` MUST be parsed as an ordinary expression of the language.

Each parsed interpolation hole MUST appear in the tagged-template node as one of its holes, so that the tag function receives the hole expressions in source order.

The count of literal chunks in a tagged template MUST be exactly one greater than the count of holes, so that the chunks and holes reconstruct the original text in order.

The tag of a tagged template MUST be dispatched by binding, not by spelling, so that a program adds an embedded domain-specific syntax by defining or importing a function rather than by extending the reader.

The compiler MUST resolve the tag name to a binding and require it to be a compile-time function from a list of the chunk strings and a list of the hole expressions to an abstract syntax tree.

The compiler MUST evaluate that tag function on the one-tier compile-time evaluation mechanism, applied to the chunks and holes.

The compiler MUST splice the tag function's resulting abstract syntax tree in the tagged template's position, expanding to a fixpoint before type checking, so that a tagged template is meaning-equivalent to the hand-written program its tag function produces and is type-checked as ordinary code.

### A Typed Quote Carries The Type Of The Expression It Builds

A quote used to build an expression MUST carry the type of the expression it constructs, so that a macro that produces an ill-typed expression is rejected at the macro rather than downstream at the macro's expansion site.

The typed quote MUST layer over the untyped abstract-syntax-tree analysis substrate rather than replace it, so that a macro may still analyze arbitrary tree structure while the expression it emits is type-checked.

### Macros Are Hygienic

A name a macro introduces MUST NOT capture a name at the macro's use site unless the macro explicitly requests it.

A name a macro introduces MUST NOT be captured by a name at the macro's use site unless the macro explicitly requests it.

Hygiene MUST be realized by tracking the set of scopes an identifier carries, so that a name's binding is resolved by its scope set rather than by its spelling alone.

### Expansion Runs In Phases To A Fixpoint

Macro expansion MUST run as a distinct phase that precedes type checking, expanding to a fixpoint so that a macro whose output is itself a macro use is fully expanded before the program is type-checked.

A macro definition MUST be available in an earlier phase than the code that uses it, so that the phase in which a definition runs is distinct from the phase in which its expansion is checked.

### Expansion Is Reproducible

Expanding the same program MUST produce the same expanded representation on every conforming compiler.

## Meaning After Expansion

### Expansion Precedes And Feeds The Core Guarantees

The expanded representation MUST be subject to type checking exactly as if it had been written directly.

The expanded representation MUST be subject to capability checking exactly as if it had been written directly.

The expanded representation MUST be subject to the determinism guarantees exactly as if it had been written directly.

A macro MUST NOT be able to produce an expanded representation that reaches a capability the program's manifest does not enumerate.
