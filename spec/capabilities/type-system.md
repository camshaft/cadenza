# Capability — Type System

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the type universe, inference, checking, soundness, and erasure. Requirements
> realize [Core Principle VII](../../constitution.md) and trace to [overview §5](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes what it means for a program to be well-typed: that every expression has a
statically determined type, that inference finds those types with minimal ceremony, that annotations
constrain rather than contradict inference, that a well-typed program does not go wrong at runtime in
a way the semantics classifies as a type error, and that types leave no trace in the runnable form.
It states the invariants of the type system, not the algorithm that realizes them.

## Static Typing

### Every Expression Has A Static Type

Every expression in a well-formed program MUST have a type determined before the program is compiled to a component.

A program that is not well-typed MUST be rejected at compile time rather than compiled to a component carrying a deferred type error.

## Inference

### Inference Is Principal-Type Inference By Unification

Type inference MUST determine types by unification over type variables — solving the equality constraints a program's structure imposes — so that a type is derived from how each binding is used rather than assumed or guessed from a single use site.

The type inference determines for an expression MUST be its principal type: the most general type from which every other valid type of that expression is an instance, so that inference commits to no more than the program's uses require.

A value that escapes to the host whose type contains a type variable no use constrains MUST be rejected at compile time with the type-determination fault code, rather than crossing the boundary with an invented type, so that a serialized value's type header is always fully determined. A bare `None` returned as the program result (type `Option ?`, the payload free), an `Ok` whose `Err` parameter is never constructed, or an empty list indexed to `None` (element free) is rejected for its unresolved type — the fix is an annotation that determines the variable — not for its export shape. A CONSUMED such value (matched, or passed to a typed parameter) constrains the variable and type-checks without annotation; the ambiguity bites only at an unannotated escape.

Inference MUST propagate a determined type to every occurrence of the binding it constrains, so that a parameter used in one position is typed consistently at every other occurrence and at every call site.

A program for which unification has no solution — a use that imposes contradictory constraints on a type variable — MUST be rejected at compile time with the machine-readable code for the conflicting-use type error, rather than compiled.

### A Let-Bound Definition Is Generalized

A let-bound definition whose inferred type contains free type variables MUST be generalized over those variables, so that the definition may be used at different instantiations within its scope, consistent with the generics being type-valued parameters.

A type variable that is constrained by an enclosing binding MUST NOT be generalized, so that generalization does not escape the scope in which a variable is still being solved.

### An Unannotated Program Is Accepted When It Has A Valid Typing

An unannotated program that has a valid typing MUST be accepted without requiring the author to write that typing, so that inference relieves the author of restating what the structure already determines.

### Annotations Constrain, Never Contradict

An explicit type annotation MUST participate in inference as an additional constraint unified with the type the system infers, rather than override it.

A program whose annotation cannot be unified with the type inference determines MUST be rejected rather than have the annotation silently replace the inferred type.

### Inference And First-Class Types Meet At A Bidirectional Boundary

Unification-based inference MUST range over a non-computational term core, so that the type variables inference solves never carry a computation whose reduction principal-type inference cannot decide.

A position that binds a type-valued parameter MUST be a bidirectional-checking boundary, at which a type is either synthesized by monomorphization from the concrete type-value supplied or checked against an explicit annotation, rather than solved by unification, so that first-class computable types are reconciled with principal-type inference instead of contradicting it.

### A Type Rejection Reports The Minimal Conflict At Both Sites

A rejection for a failed unification MUST report the minimal unsatisfiable set of constraints rather than the first constraint that failed, so that the diagnostic names the actual conflict and not an arbitrary casualty of it.

A rejection for a failed unification MUST name both source locations whose requirements disagree, rather than blame one site, so that an agent sees both ends of the contradiction that together are the route to a fix.

## The Declarable Type Universe

### The Structural Types Are Record, Tuple, And Sum

A program MUST be able to form a structural type — a record of named fields, a tuple of positional elements, or a sum of named variants — whose identity is its shape, equal to any type of the same shape.

A structural type's shape MUST be its constituent types in their defining positions — a record's field names with their types, a tuple's element types in order, a sum's variant names with their payload types — so that two structural types are equal exactly when those constituents coincide.

### Never Is The Empty Sum

The type universe MUST include the empty sum — a sum type with zero variants — as the dual of the unit type, which is the empty tuple, so that the zero of the sum constructor is a nameable type exactly as the zero of the product constructor is.

The empty sum MUST be uninhabited — it MUST have no constructor and no value — because a sum over zero variants offers no variant to construct.

The type of an expression that diverges rather than producing a value — a trap, or requiring the value of an absent optional — MUST be the empty sum, and that type MUST unify with any expected type, because a diverging expression yields no value that could be of the wrong type.

A match on a scrutinee of the empty sum type MUST be exhaustive with zero arms, because there is no variant left to cover — the degenerate base case of the exhaustiveness rule, not an exception to it.

### Records Are Rows, Open By Default Under Inference

A record type MUST be expressible as a row — a set of field-name-to-type associations that MAY carry a row variable standing for the fields not named — so that a function can accept any record that has at least the fields it uses.

A function that uses only some of a record's fields MUST be inferrable at a row type open over the fields it does not use, so that row polymorphism, not a fixed record shape, is what inference assigns where the program does not pin the shape.

The row variable of an open record type MUST be resolved to a closed set of fields before the value crosses a component boundary, so that the emitted component carries a concrete record shape and no row variable (consistent with monomorphization and the component ABI).

Comparison of two records MUST remain a comparison of closed shapes: a program that compares a subset of one record against another MUST first project the compared fields explicitly and compare the projections, rather than rely on an equality overloaded to ignore extra fields, so that `=` is never silently widened by row polymorphism.

### A Record Row Is Reshaped Only Through An Explicit Operation Yielding A New Value

A program MUST be able to derive a new record from existing records by an explicit row operation — restricting to named fields, dropping named fields, or combining two records — rather than by an implicit widening or narrowing that inference introduces, so that a shape change is always something the program wrote.

A record row operation MUST yield a new record value and MUST NOT alter the operand records, consistent with the immutable value heap, so that reshaping a record is the derivation of a new value with a new shape and not a mutation of an existing one.

The shape of a record row operation's result MUST be determined statically from the operands' shapes, so that the emitted component carries a concrete closed record shape and the operation introduces no runtime field set.

### A Record Is Restricted To A Named Set Of Its Fields

A program MUST be able to project a record onto a stated set of field names, yielding a record whose fields are exactly those names bound to the values the operand holds for them, so that narrowing a record to a sub-shape is an explicit operation rather than an overloaded equality.

A projection that names a field the operand record does not contain MUST be rejected at compile time with the machine-readable code for a required field that is absent, so that a projection cannot silently produce a field the operand never held.

### A Record Is Reduced By Dropping A Named Set Of Its Fields

A program MUST be able to derive a record that drops a stated set of field names from an operand record, yielding a record whose fields are exactly the operand's remaining fields, so that removing a field is the complement of projecting the fields kept.

A drop that names a field the operand record does not contain MUST be rejected at compile time with the machine-readable code for a required field that is absent, so that dropping a field the record never held is a static error rather than a no-op.

### Two Records Are Combined Only When Their Field Sets Are Disjoint

A program MUST be able to combine two records into one whose field set is the union of the operands' field sets, each field bound to the value its source record holds, so that merging records is the row analogue of forming a record from two groups of fields.

A combination of two records whose field sets share a name MUST be rejected at compile time with the machine-readable code for a field that is already present, so that a combined record never has to choose which operand's value a shared field takes and the fixed-field-set invariant is preserved.

### A Field Is Added To Or Replaced In A Record By A Derived Operation

A program MUST be able to derive a record that adds a field absent from an operand record, and a combination that adds a field the operand already contains MUST be rejected at compile time with the machine-readable code for a field that is already present, so that adding a field never silently overwrites an existing one.

A program MUST be able to derive a record that replaces a field present in an operand record with a new value of a possibly different type, so that updating a field is an explicit operation distinct from adding one and the replacement's type is whatever the new value holds.

A field update whose named field is absent from the operand record MUST be rejected at compile time with the machine-readable code for a required field that is absent, so that updating a field the record never held is a static error rather than an addition.

### A Tuple Is Reshaped Positionally By An Explicit Operation Yielding A New Value

A program MUST be able to derive a new tuple from existing tuples by an explicit positional operation — concatenating two tuples or splitting one at a stated position — rather than by an implicit change of arity, consistent with a tuple being a fixed-size positional value whose length is part of its type.

A tuple positional operation MUST yield a new tuple value and MUST NOT alter the operand tuples, consistent with the immutable value heap, so that reshaping a tuple is the derivation of a new value and not a mutation.

The arity of a tuple positional operation's result MUST be determined statically from the operands' arities, so that the emitted component carries a concrete tuple shape and the operation introduces no runtime-length tuple.

### Two Tuples Are Concatenated Into One Of Their Combined Length

A program MUST be able to concatenate two tuples into one whose elements are the first tuple's elements in order followed by the second tuple's elements in order, so that its arity is the sum of the operands' arities and each element keeps the type of its source position.

### A Tuple Is Split At A Position Into A Prefix And A Suffix

A program MUST be able to split a tuple at a stated position into a pair of tuples — a prefix holding the elements before the position and a suffix holding the elements from the position onward — so that partitioning a tuple positionally is an explicit operation yielding both parts.

A split position that is not within the operand tuple's static arity range MUST be rejected at compile time as a type error, consistent with a positional tuple access whose index is out of the tuple's static arity being rejected, so that a split can never name a position the tuple does not have.

### The Effect Row Is A Row Over The Same Machinery

A function's effect row MUST be tracked by the same row machinery as an open record, so that principal-type inference over effects reuses row unification rather than a separate effect-inference system.

A function polymorphic over its effect row MUST have that row resolved to a closed set — the empty row for a pure function — before it crosses a component boundary, so that the emitted component's import world is exactly the manifest and carries no effect-row variable (host-interface-binding.md §The Manifest Is A Projection Of The Escaping Effect Row).

### Nominal Is An Orthogonal Modifier Over Any Structural Type

A program MUST be able to declare a nominal type by tagging any structural type — record, tuple, or sum — with a name, so that nominal-versus-structural is one orthogonal choice available over every structural type rather than a property of one kind of type.

A nominal type MUST be represented as its underlying structural value together with a compile-time tag naming the type, so that a nominal type and its underlying structural type are one runtime mechanism and the tag adds nothing to the value's runtime representation.

A nominal type's identity MUST be its fully-qualified name — the module path in which it is declared together with its declared name — so that its identity is unique across the whole program and does not depend on its shape.

Two nominal types MUST be distinct whenever their fully-qualified names differ, even when their underlying structures and their declared local names are identical, so that a module cannot forge a value of another module's nominal type by re-declaring a same-shape same-name type.

### Nominal Types Are Not Comparable Across Their Boundary

A comparison whose operands are of two different nominal types MUST be rejected by a type-tracking generation, because the purpose of declaring a type nominal is to give its values an identity that is not interchangeable with a same-shape value of another type.

A comparison between a nominal value and the underlying structural value of the same shape MUST be rejected by a type-tracking generation, so that a nominal value never silently compares equal to the untagged shape it was declared distinct from.

An untyped evaluation that does not track the name tag MUST compare two values by their shared structure alone, giving two same-shape values a defined outcome of equal, so that the dynamic semantics recorded for such a comparison is total where a type-tracking generation instead rejects the comparison as a type error.

### A Nominal Value Is Convertible To Its Underlying Structural Value

A program MUST be able to strip a nominal type's name tag to obtain the underlying structural value, so that a value declared nominal can be compared or used structurally when the program explicitly asks for it rather than silently.

The stripped structural value MUST be the same value the nominal value already is at runtime, so that removing the tag is a compile-time reinterpretation and not a copy or conversion of the value.

### An Abstract Type's Representation Is Not Observable Across Its Boundary

A built-in structural comparison whose operand is a value of an abstract type — a type whose handle a module made visible without making its constructors visible ([modules-and-namespaces.md](modules-and-namespaces.md) §A Type's Handle And Its Constructors Are Independently Visible) — MUST be rejected outside the declaring module, so that the abstract type's representation is not observed through equality and a module that wants its abstract type compared publishes a comparison operation rather than exposing its structure.

Stripping an abstract type's name tag to its underlying structural value MUST be rejected outside the declaring module, so that the escape hatch to a nominal type's structure is available only where that type's constructors are, and a handle-only export does not leak the representation it withheld.

### Structural Values Are Comparable Only When Their Shapes Match

Two records MUST be comparable only when their sets of field names are identical, two tuples only when their lengths are identical, and two sums only when their variant sets are identical, because values of different shapes have no meaningful equality.

A comparison of two structural values whose shapes differ MUST be rejected by a type-tracking generation as a type error rather than reported as unequal, so that a shape mismatch is caught rather than answered.

An untyped evaluation that does not track types MUST give such a mismatched comparison a defined dynamic outcome of not-equal rather than trapping, so that the dynamic semantics recorded for such a comparison is total while a type-tracking generation rejects the comparison earlier.

### Sum Types Are Constructed And Deconstructed

A value of a sum type MUST be constructed through one of its variants.

A value of a sum type MUST be deconstructed only through a match that the exhaustiveness rule governs.

### A Single-Variant Single-Field Sum Is A Nominal Type Over Its Payload

A sum type with exactly one variant carrying exactly one payload field MUST be a nominal type over that field's type, so that declaring `(type T (V U))` names the structural type `U` nominally (§"Nominal Is An Orthogonal Modifier Over Any Structural Type") rather than introducing a distinct tagged representation.

The runtime value of such a single-variant single-field sum MUST be its payload value itself, carrying only the compile-time nominal tag and nothing at runtime, consistent with the nominal rule that the tag adds nothing to the value's runtime representation: constructing `(V u)` yields the value `u`, and matching the single variant `(V binder)` binds `u` directly. The nominal type `T` is a compile-time tag over the structural type of `u` (§"Nominal Is An Orthogonal Modifier Over Any Structural Type"), not a distinct runtime value; the type annotation a returned such value carries in its observable canonical form is governed by the value form the executable-semantics corpus records, which is not uniformly the nominal name `T`.

A sum type with more than one variant, or whose variant is nullary, or whose variant carries more than one payload field MUST retain its variant tag in the value — its value is the tagged variant, not an erased payload — so that only the single-variant single-field form is a nominal newtype and every other sum is a genuine tagged union whose value carries its variant.

### A Match Is Exhaustive Against The Sum Type's Variant Set

The exhaustiveness rule governing a match MUST be checked against the scrutinee sum type's variant set, so that a match covering fewer than all variants is a compile-time rejection determined by that variant set rather than a runtime outcome.

### A Sum Type May Be Open, With A Mandatory Open-Tail Arm

A program MUST be able to declare an open sum — a variant set that MAY carry a row variable standing for variants not named — so that a value can range over an extensible vocabulary of variants the declaring module does not close, dual to an open record's extensible field set.

A match on an open sum MUST carry an open-tail arm covering the variants not named, and a match that omits it MUST be a compile-time rejection, so that exhaustiveness holds for an open sum exactly as it does for a closed one and an unknown variant is handled rather than unmatched.

A closed sum MUST remain the default: a sum declared without a row variable is closed, and the abstract syntax tree type MUST be a closed sum, so that a compiler's match over the AST is checked against a fixed, known variant set.

### The Abstract Syntax Tree Is An Ordinary Sum Type

The abstract syntax tree type MUST be an ordinary sum type of the language — a variant per syntactic form (an integer, a float, a string, a boolean, a name, and a list of child nodes) with the list variant carrying a list of the same type — rather than a primitive the type system special-cases.

The abstract syntax tree type MUST carry a distinct first-class variant for each collection constructor — a list, a tuple, a record, a map, and a set (`Ast.ListCtor`, `Ast.TupleCtor`, `Ast.RecordCtor`, `Ast.MapCtor`, `Ast.SetCtor`), each carrying its child abstract syntax trees — rather than reflecting a collection as a generic child-node list headed by the collection's name, so that a reflected collection is its own variant and no collection is a string- or name-headed node.

The abstract syntax tree type MUST additionally carry a distinct variant for a key-value field pair (`Ast.FieldPair`, a key and a value abstract syntax tree) and a distinct variant for a member access (`Ast.Member`, an operand and a key abstract syntax tree), so that a record's and a map's child abstract syntax trees are field-pair variants and the reflected form of `(= key value)` and of `(. obj key)` is likewise a first-class variant rather than a name-headed node.

The abstract syntax tree type MUST additionally carry a distinct variant for a rational literal (`Ast.Rational`, a numerator and a denominator abstract syntax tree), so that the reflected form of a rational literal such as `3/2` is a first-class variant carrying its two integer child nodes rather than a name-headed node, and reflection stays total over every well-formed literal leaf.

The generic child-node-list variant MUST remain, carrying the syntactic forms that are not collections — a name-headed form such as a conditional, a function, a match, or an application, whose head reflects as a name — so that giving collections their own variants removes string- and name-headed *collections* without removing the generic node for the name-headed forms that keep it.

The AST sum type MUST be constructed and deconstructed by the same variant-construction and match mechanisms as any other sum type, so that a compiler written in the language walks a program as data with no reflection primitive.

### Types Are First-Class Values Whose Type Is The Type Of Types

A type MUST be expressible as an ordinary first-class value that can be bound, passed, and returned, so that the language needs no separate term-and-type syntax to name a type.

The type of a type-value MUST be the type of types, so that the kind level is itself a type in the system rather than an untyped meta-level.

### Generics Are Type-Valued Parameters, Not A Separate Polymorphism Mechanism

A generic definition MUST be expressed as an ordinary definition that takes type-valued parameters, so that generics reuse the first-class-type machinery rather than introducing a separate parametric-polymorphism construct.

A type parameter MUST be resolvable to a concrete type at compile time, so that a type-value never flows from runtime data into a position that determines a type.

A generic type constructor — a type parameterized by another type, such as a list of a given element type or an optional of a given type — MUST be a compile-time function from types to a type, applied by ordinary application, so that a parameterized type like an optional integer is the result of applying a type constructor rather than special syntax.

### A Generic Constraint Is A Compile-Time Predicate Over Type-Values

A type parameter's constraint MUST be expressed as a compile-time predicate over the type-value bound to the parameter, so that constraint checking reuses compile-time evaluation rather than a separate trait-resolution system.

The compiler MUST reject a generic instantiation whose type argument fails the parameter's constraint, with the machine-readable diagnostic code for the unsatisfied constraint, so that a constraint violation is a compile-time rejection rather than a runtime failure.

### Ad-Hoc Polymorphism Is An Explicitly Passed Dictionary

A trait MUST be an ordinary record type whose fields are the operations it declares, and an instance MUST be an ordinary value of that record type, so that ad-hoc polymorphism reuses records and first-class values rather than a separate trait construct.

A definition that is polymorphic over a trait MUST receive the instance as an ordinary explicit parameter, so that ad-hoc polymorphism is the existing type-valued-and-value-valued parameter mechanism rather than a separate resolution engine.

The compiler MUST NOT resolve a trait instance from ambient or global scope, so that which implementation a use site gets is visible at the call and no orphan rule or global-coherence assumption is needed for ad-hoc polymorphism to compose with content-addressed modules.

An explicitly passed instance MUST be monomorphized into the use site, so that a component carries no runtime dictionary lookup and no dispatch the manifest did not declare.

An implicit resolution of a trait instance MAY be offered only as an optional elaboration that provably rewrites to the explicit passing above without changing emitted bytes, so that the mandatory mechanism stays explicit and any convenience layer is meaning-preserving.

### A Generic Definition Is Monomorphized Before The Component Boundary

The compiler MUST monomorphize a generic definition — specialize it to each concrete set of type arguments it is used with, by compile-time reduction with those type-values bound — before the definition crosses a component boundary, consistent with the component ABI.

Monomorphization MUST be the same compile-time reduction by which the compiler specializes any definition applied to compile-time-known arguments, so that a generic instantiation is not a distinct lowering path from ordinary compile-time application.

### Subtyping Is Explicit Or Absent

The type system MUST NOT introduce an implicit subtyping coercion that the program did not write.

## Soundness

### A Well-Typed Program Does Not Go Wrong

A well-typed program MUST NOT reach, at runtime, a state the executable semantics classifies as a type error.

## Erasure

### Types Are Erased From The Component

The compiler MUST erase types from the emitted component so that the runnable form carries no runtime type reflection.

The behavior of an emitted component MUST NOT depend on any type information the compiler could not erase.
