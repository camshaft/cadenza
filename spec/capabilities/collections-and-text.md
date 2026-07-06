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

### A String Offers Both A Scalar Length And A Byte Length

A string MUST offer a length counted in Unicode scalar values and a length counted in the bytes of its
UTF-8 encoding as two separately-named operations, so that neither meaning is the unqualified default an
author could confuse for the other.

The scalar length and the byte length MUST count the string's normalized contents, so that a length is a
function of the string's value rather than of an incidental byte spelling that normalization removes.

A string MUST NOT offer an unqualified length operation, so that every length query names whether it
counts scalar values or bytes.

The byte length MUST be obtainable without materializing the UTF-8 encoding as a separate value, so that
a size query an author expects to be cheap is not defined only in terms of an intermediate byte
sequence.

### A String Literal's Escapes Are A Closed Set

Within a string literal, a backslash MUST introduce an escape sequence rather than stand for itself.

A conforming reader MUST recognize exactly these escape sequences: `\n` (U+000A), `\t` (U+0009), `\r` (U+000D), `\\` (U+005C), and `\"` (U+0022).

A backslash followed by any character that does not begin one of the recognized escape sequences MUST be a compile-time error, so that an unrecognized escape is a rejected program rather than a silently-dropped backslash or an implementation-defined character.

### String Equality Follows Normalized Contents

Two strings MUST be equal exactly when their normalized contents are identical, under the text
normalization the hashing-and-encoding choice pins.

### String Comparison Is Defined On Scalar Values

An ordering over strings MUST be the lexicographic order of their Unicode scalar value sequences.

### Decoding Bytes To A String Is Total, Not Trapping

Decoding a byte sequence to a string MUST yield a result that distinguishes a successful decode from a
byte sequence that is not well-formed UTF-8, rather than trapping on ill-formed input, so that ill-formed
bytes are an ordinary value a program handles rather than a halt.

A pattern that decodes a string from a byte sequence MUST treat ill-formed UTF-8 as a non-match that the
match's exhaustiveness obligation forces the program to handle, so that the ill-formed case is covered by
a branch rather than by a trap.

Encoding a string to its UTF-8 byte sequence MUST be the inverse of decoding a well-formed byte sequence,
so that a string decoded from bytes and re-encoded yields those same bytes.

## Lists

### A List Is An Ordered Homogeneous Sequence

A list MUST be an ordered sequence whose elements share one type.

Two lists MUST be equal exactly when they have equal elements in the same order.

### Indexing And Lookup Are Fallible, Not Trapping

An operation that reads an element of a sequence by position — indexing a list, a string (by scalar or byte offset), or a `Bytes` value, or taking a sub-sequence slice — MUST be total, yielding an optional value that is present when the position is in bounds and absent when it is out of bounds, rather than trapping or producing an unspecified value.

Looking a key up in a map MUST likewise be total, yielding an optional value that is present when the map contains the key and absent when it does not.

A program that requires the present value of such an optional MUST obtain it through the optional's value-requiring operation carrying a mandatory message (core-semantics.md §"Requiring The Value Of An Optional Traps On Absence"), so that the boundary between handling absence as data and halting on absence is explicit at the point the program crosses it, not hidden inside the access operation.

## Maps

### A Map Associates Keys With Values

A map MUST associate keys of one type with values of one type.

A map MUST contain each key at most once.

Two maps MUST be equal exactly when they associate the same keys with equal values, independent of insertion order.

### Map Iteration Is Deterministic

Iterating a map MUST visit its entries in a deterministic order derived from the keys, not from insertion order.

The order in which a map's entries are visited MUST agree with the order its canonical byte form places them in.
