# Capability — Value Interchange

> **CAPABILITY SPECIFICATION.** Behavior and invariants, free of implementation detail. This document
> defines **value interchange**: the surface by which a program turns a value into bytes and reads a
> value back, so that a value can be persisted, handed to another component, or stored across compiler
> generations and reconstituted as the same value. Requirements realize
> [Core Principle III](../../constitution.md) and [Core Principle VI](../../constitution.md) and trace
> to [overview §4](../overview.md) and [overview §8](../overview.md).
>
> RFC-2119 key words are normative. Each requirement is a single self-contained sentence carrying
> exactly one obligation, under a stable heading. This capability states the surface's invariants; the
> concrete operations, the schema-identity function, and the header layout are pinned at the declared-
> default location.

## Purpose And Scope

The language already fixes that a program has a durable, hashable, third-party-checkable stored form
(ast-encoding.md) and that every value has one canonical byte form (deterministic-value-form.md). Value
interchange gives a *value* the property code already has: a program can serialize a value to a stable
byte sequence and another party can decode that sequence back to the same value. This capability fixes
the invariants of that surface — that its bytes are the canonical value form and not a second encoding,
that decode inverts encode and refuses anything else, that a serialized value may carry a self-checking
header identifying its type, and that the header's identity is a function of the type's content — while
leaving the concrete operations, the exact schema-identity function, and the header's byte layout to the
declared-default location. It does not restate the canonical byte form, which the value-form contract
governs, nor the calling convention by which a value crosses a live component boundary, which the
component ABI governs; interchange is the *serialization* a program invokes, distinct from both.

## Interchange Is Over The Canonical Value Form

### Serialized Bytes Are The Canonical Value Form

The bytes a program obtains by serializing a value MUST be that value's canonical byte form (deterministic-value-form.md §"The Canonical Byte Form"), so that interchange introduces no second value encoding and a value's interchange bytes are identical to its bytes for hashing and equality.

Two values equal under structural equality MUST serialize to identical interchange bytes, and two values that are not equal MUST serialize to distinct interchange bytes, inherited from the canonical value form rather than re-established by this capability.

### Decode Inverts Serialize And Refuses Otherwise

Decoding the serialized bytes of a value against the type of that value MUST yield a value equal to the original under structural equality (deterministic-value-form.md §"The Canonical Byte Form Has A Decode That Inverts It").

Decoding a byte sequence that is not the serialization of any value of the expected type MUST yield the absence of a value rather than a value, consistent with the language's fallible readers that yield an optional result rather than trapping (collections-and-text.md).

## A Serialized Value May Carry A Self-Describing Header

### Serialization Is Available With Or Without A Type Header

A program MUST be able to serialize a value without a type header, for use where the decoding party already knows the value's type, so that the common intra-program path carries no header overhead.

A program MUST be able to serialize a value with a type header prepended to the same canonical value bytes, for use where the decoding party must confirm the type of the bytes it received, so that a value can be "passed around" and checked on receipt.

The header-carrying serialization of a value MUST be its type header followed by exactly the bytes its headerless serialization produces, so that there is one payload format offered with or without a header rather than two payload formats.

### The Type Header Identifies The Value's Type By Its Content

The type header MUST be a function of the value's type alone, deterministic and independent of the order in which types were encountered, mirroring the discovery-order-independent canonical prelude of a stored program (ast-encoding.md §"The Prelude Order Is Canonical").

The type header MUST distinguish types that are not interchangeable and MUST agree for types that are, so that a decoder can decide from the header alone whether the bytes are meant for the type it expects.

Whether a nominal type's tag participates in the header MUST be a property of which serialization operation the program invokes rather than a value carried in the payload, so that a nominal value and its underlying structural value serialize to identical payload bytes and differ only in the header a tag-participating operation computes (type-system.md §"Nominal Is An Orthogonal Modifier Over Any Structural Type").

### A Decoder Refuses A Header That Does Not Match The Expected Type

Decoding header-carrying bytes MUST verify the header against the type the decode expects and, on a mismatch, yield the absence of a value without decoding the payload, mirroring a host that refuses a runtime whose content address does not match rather than substituting one (component-abi.md; reproducible-derivation.md).

A decoder MUST NOT dispatch on the header to select a type to decode into as a requirement of this capability, so that header-directed dispatch is a facility a program may build over interchange rather than an obligation interchange imposes.

## The Realization Is A Declared-Default Decision

### The Interchange Surface Is Pinned At The Declared-Default Location

The concrete serialization and decoding operations a program invokes MUST be pinned at the declared-default location, so that two builds agree on the surface a program may rely on.

### The Schema-Identity Function And Header Layout Are Pinned At The Declared-Default Location

The function that derives a type header from a type, its length, and its byte layout MUST be pinned at the declared-default location, and — because the header's bytes cross a boundary and identify a value's type — a change to that pinned form MUST be a coordinated change under the constitution's Governance Floors with a stated migration path.

The canonical byte form of a type from which a type header is derived MUST be pinned at the declared-default location, defined where a canonical form for a type previously had none, so that adding it is an additive change (deterministic-value-form.md §"Additive Evolution"; ast-encoding.md §"New Constructs Do Not Bump The Encoding Version").

## Open Decisions This Capability Leaves To The Declared-Default Location

### The Points A Choice Must Resolve

A choice realizing this capability MUST resolve the concrete interchange operations, including how the type a headerless decode expects is supplied so that decode is directed by a known type rather than by the payload.

A choice realizing this capability MUST resolve the schema-identity function — over what canonical form of a type it is computed, which reduction to a fixed-size header it applies, and how a type's evolution is reflected so that an evolved type yields a distinct header rather than a colliding one.

A choice realizing this capability MUST resolve whether, and by what surface, a value whose type contains a form the canonical value form does not yet serialize is rejected, so that a not-yet-serializable value form is a detected gap rather than a silent one (deterministic-value-form.md §"Additive Evolution").

A choice realizing this capability MAY resolve on-the-wire interoperability with an external serialization format, but such interoperability MUST NOT weaken the requirement that a program's own interchange bytes are the canonical value form, so that interoperating with a foreign format is an additional surface rather than a replacement of the canonical one.
