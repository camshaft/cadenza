# Frozen Contract — Deterministic Value Form

> **FROZEN CONTRACT.** This document pins the canonical byte form of a Cadenza value used
> wherever a value is hashed, compared for equality across a boundary, or serialized as a
> component's output. It is the form against which the determinism guarantee is measured. It is
> versioned and changed only by the coordinated act described in the constitution's Governance
> Floors. Its requirements realize [Core Principle III](../../constitution.md) and
> [Core Principle VI](../../constitution.md) and trace to [overview §4](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence under a
> stable heading. This contract fixes a canonical form's properties; the concrete encoding that
> realizes it is pinned at the declared-default location.

## Purpose And Scope

The determinism guarantee says two runs of a component over the same input produce byte-identical
output. That guarantee needs a fixed notion of "the bytes of a value": a canonical serialization
that is stable across runs and across compiler generations. This contract pins that canonical byte
form. It is distinct from the component ABI, which fixes the *calling convention* by which a value
crosses a boundary; this contract fixes the *serialization* used for hashing, equality, and a
component's emitted output.

## The Canonical Byte Form

### A Value Has One Canonical Byte Form

Each serializable value MUST have exactly one canonical byte encoding.

Two values that are equal under the language's structural equality MUST have identical canonical byte encodings.

Two values that are not equal under the language's structural equality MUST have distinct canonical byte encodings.

### Ordering Of Aggregate Members Is Fixed

The canonical encoding of an unordered aggregate MUST place its members in a fixed order derived from the members themselves, not from the order in which they were inserted or discovered.

The canonical encoding of an ordered aggregate MUST preserve its element order.

### The Unit Value Has A Canonical Byte Form

The unit value MUST have exactly one canonical byte encoding, so that a program that produces no value other than its emitted events has a serializable normal-termination value.

The canonical byte encoding of the unit value MUST be distinct from that of every other value, consistent with structural equality treating the unit value as equal only to itself.

## Numeric Serialization

### Numeric Values Serialize Deterministically

An exact numeric value MUST serialize to a byte form that is independent of how the value was computed.

Two floating-point values equal under structural equality MUST have identical canonical byte encodings.

Two floating-point values that are not equal under structural equality MUST have distinct canonical byte encodings.

A floating-point negative zero MUST serialize distinctly from a positive zero, consistent with structural equality treating them as distinct.

Every floating-point not-a-number value MUST serialize to one canonical byte form, consistent with structural equality treating all not-a-number values as equal.

## Additive Evolution

### Additive Evolution Of This Contract

A change to this contract that alters the canonical byte form of an already-serializable value MUST carry a version increment.

A change to this contract that alters the canonical byte form of an already-serializable value MUST carry a stated migration path.

A change to this contract that only defines a canonical byte form for a value that previously had none MUST be permitted as an additive change.
