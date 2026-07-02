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

### Inference Yields The Most General Type

Where the type system determines an expression's type without an annotation, the determined type MUST be the most general type consistent with the expression's uses.

An unannotated program that has a valid typing MUST be accepted without requiring the author to write that typing.

### Annotations Constrain, Never Contradict

An explicit type annotation MUST be checked against the type the system would otherwise determine.

A program whose annotation conflicts with the type the system determines MUST be rejected rather than have the annotation silently override inference.

## The Declarable Type Universe

### User Types Are Declarable As Nominal Or Structural

A program MUST be able to declare a nominal type whose identity is its declared name, distinct from any structurally identical type of a different name.

A program MUST be able to declare a structural type whose identity is its shape, equal to any type of the same shape.

### Sum Types Are Declarable, Constructed, And Deconstructed

A program MUST be able to declare a sum type as a set of named variants, each optionally carrying data.

A value of a sum type MUST be constructed through one of its variants.

A value of a sum type MUST be deconstructed only through a match that the exhaustiveness rule governs.

### Generics Are Parameterized And Monomorphized

A definition MUST be able to take type parameters so that it applies to more than one concrete type.

A type parameter MUST be able to carry the constraints the definition's body requires of it.

The compiler MUST monomorphize a generic definition to concrete types before it crosses a component boundary, consistent with the component ABI.

### Subtyping Is Explicit Or Absent

The type system MUST NOT introduce an implicit subtyping coercion that the program did not write.

## Soundness

### A Well-Typed Program Does Not Go Wrong

A well-typed program MUST NOT reach, at runtime, a state the executable semantics classifies as a type error.

## Erasure

### Types Are Erased From The Component

The compiler MUST erase types from the emitted component so that the runnable form carries no runtime type reflection.

The behavior of an emitted component MUST NOT depend on any type information the compiler could not erase.
