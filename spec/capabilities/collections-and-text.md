# Capability — Collections And Text

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This
> document defines the operational semantics of strings and of the built-in collections — lists and
> maps — including their equality, ordering, and determinism. Requirements realize
> [Core Principle III](../../constitution.md) and [Core Principle VII](../../constitution.md) and
> trace to [overview §4](../overview.md) and [overview §5](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading.

## Purpose And Scope

This capability fixes the behavior of text and the built-in collections, which the type mapping names
but does not give semantics: what a string's unit of length and comparison is, how lists and maps
behave and compare, and how a map's iteration is made deterministic. It states the behavior; the
concrete byte forms are governed by the deterministic-value-form contract and the hashing-and-encoding
choice.

## Text

### A String Is A Sequence Of Unicode Scalar Values

A string MUST be a sequence of Unicode scalar values, so that its contents are independent of any byte
encoding.

A string's length MUST be counted in Unicode scalar values.

### String Equality Follows Normalized Contents

Two strings MUST be equal exactly when their normalized contents are identical, under the text
normalization the hashing-and-encoding choice pins.

### String Comparison Is Defined On Scalar Values

An ordering over strings MUST be the lexicographic order of their Unicode scalar value sequences.

## Lists

### A List Is An Ordered Homogeneous Sequence

A list MUST be an ordered sequence whose elements share one type.

Two lists MUST be equal exactly when they have equal elements in the same order.

### List Operations Are Total Or Trap

An operation that indexes a list outside its bounds MUST raise a trap of a defined kind rather than produce an unspecified value.

## Maps

### A Map Associates Keys With Values

A map MUST associate keys of one type with values of one type, with each key present at most once.

Two maps MUST be equal exactly when they associate the same keys with equal values, independent of insertion order.

### Map Iteration Is Deterministic

Iterating a map MUST visit its entries in a deterministic order derived from the keys, not from insertion order.

The order in which a map's entries are visited MUST agree with the order its canonical byte form places them in.
