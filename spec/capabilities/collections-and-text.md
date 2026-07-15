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

### A Char Is A Single Unicode Scalar Value

A char MUST be a single Unicode scalar value — a code point in the range `U+0000..=U+10FFFF` excluding the surrogate range `U+D800..=U+DFFF` — so that the element type of a string's scalar sequence is exactly a char and a char can never hold a value that is not a scalar.

A char's ordering MUST be the numeric order of its scalar value, so that a char order and the string order defined on scalar values agree by construction.

### A String's Scalars Are Addressable

Reading a string's scalar at a position MUST be total, yielding an optional char that is present when the position is in bounds and absent when it is out of bounds, so that scalar access is fallible in the same way list and byte indexing are rather than trapping.

### A Char Converts To And From An Integer Totally

Converting a char to its integer scalar value MUST be total, because every char is a scalar value that has an integer code point.

Converting an integer to a char MUST yield an optional char that is absent when the integer is not a Unicode scalar value — outside `U+0000..=U+10FFFF` or within the surrogate range — so that an out-of-range integer is handled as data rather than producing a char that is not a valid scalar.

### Decoding Bytes To A String Is Total, Not Trapping

Decoding a byte sequence to a string MUST yield a result that distinguishes a successful decode from a
byte sequence that is not well-formed UTF-8, rather than trapping on ill-formed input, so that ill-formed
bytes are an ordinary value a program handles rather than a halt.

A pattern that decodes a string from a byte sequence MUST treat ill-formed UTF-8 as a non-match that the
match's exhaustiveness obligation forces the program to handle, so that the ill-formed case is covered by
a branch rather than by a trap.

Encoding a string to its UTF-8 byte sequence MUST be the inverse of decoding a well-formed byte sequence,
so that a string decoded from bytes and re-encoded yields those same bytes.

## Collections

### A Collection's Homogeneity Violation Is A Malformed Collection

The built-in collections — lists, maps, and sets — are each HOMOGENEOUS: a list's elements share one type, and a map associates keys of one type with values of one type (*A List Is An Ordered Homogeneous Sequence*, *A Map Associates Keys With Values*, *A Set Is A Collection Of Unique Elements*).

A construction whose elements, keys, or values do not share one type MUST be rejected as a malformed collection with the diagnostic code `CDZ0201`, so that a heterogeneous collection is treated as the collection being unbuildable rather than as a value of some other type.

The malformed-collection code a heterogeneous construction takes MUST be the same code independent of the collection kind — list, map, or set — so that the diagnostic names one category rather than one per collection kind.

The malformed-collection code a heterogeneous construction takes MUST be the same code independent of how the construction is written, whether a literal or a functional-construction operation such as append, replace-at-index, concatenate, or insert, so that the code does not vary with the construction form.

The malformed-collection code a heterogeneous construction takes MUST be the same code independent of how the element types differ, whether a cross-kind clash, a numeric mix that does not silently promote, or two same-kind values of different shape, so that a consumer branching on the code sees one category for "this collection is not homogeneous" rather than a code that varies with the incidental shape of the disagreement (*diagnostics.md §Every Diagnostic Has A Stable Code*).

This is distinct from the type-conflict code (`CDZ0203`), which names a two-types-must-AGREE unification conflict — a conditional's branches disagreeing, a value annotation contradicting its expression, or a comparison of two values whose shapes do not match. A collection's internal heterogeneity is NOT such a conflict: it is the collection being unbuildable, so it takes the malformed-collection code, not the mismatch code.

## Lists

### A List Is An Ordered Homogeneous Sequence

A list MUST be an ordered sequence whose elements share one type.

Two lists MUST be equal exactly when they have equal elements in the same order.

### A List Is Grown By Functional Construction

A list MUST offer an operation that appends an element and an operation that replaces the element at an index, each of which MUST produce a new list value and leave its operand list unchanged, so that a list is immutable under growth exactly as it is under reading.

The replace-at-index operation MUST be defined only for an index that is in bounds, consistent with the fallible reading rule below, so that growth never observes an element at a position the list does not have.

A list MUST also offer an operation that concatenates two lists, producing a new list whose elements are those of the first list in order followed by those of the second, and leaving both operand lists unchanged. Concatenation MUST be defined only when both operands share one element type — the result is a list of that type — consistent with *A List Is An Ordered Homogeneous Sequence*; concatenating with the empty list on either side MUST yield a list equal to the other operand.

### A List's Representation Is Unspecified And Unobservable

A conforming implementation MAY back a list with any internal representation — a contiguous array, a persistent tree, or a structure it selects and changes by size or usage — and MUST NOT let that choice be observable, so that two lists with equal elements in the same order are indistinguishable by every operation, including equality, length, indexing, and the list's canonical byte form, regardless of how each is stored. This realizes memory-and-resource-model.md §"Sharing Is Not Observable" for the list type; it introduces no way for a program to name, select, or branch on a list's representation.

### Indexing And Lookup Are Fallible, Not Trapping

An operation that reads an element of a sequence by position — indexing a list, a string (by scalar or byte offset), or a `Bytes` value, or taking a sub-sequence slice — MUST be total, yielding an optional value that is present when the position is in bounds and absent when it is out of bounds, rather than trapping or producing an unspecified value.

Looking a key up in a map MUST likewise be total, yielding an optional value that is present when the map contains the key and absent when it does not.

A program that requires the present value of such an optional MUST obtain it through the optional's value-requiring operation carrying a mandatory message (core-semantics.md §"Requiring The Value Of An Optional Traps On Absence"), so that the boundary between handling absence as data and halting on absence is explicit at the point the program crosses it, not hidden inside the access operation.

## Maps

### A Map Associates Keys With Values

A map MUST associate keys of one type with values of one type.

A map MUST contain each key at most once.

Two maps MUST be equal exactly when they associate the same keys with equal values, independent of insertion order.

### A Map Is Built By Functional Construction

A map MUST offer an empty map value, an operation that adds or replaces the association for a key, and an operation that removes the association for a key. Each MUST produce a new map value and leave its operand map unchanged, so that a map is immutable under update exactly as a list is under growth (*A List Is Grown By Functional Construction*).

Adding a key already present MUST replace that key's value rather than introduce a second entry, preserving the *A Map Associates Keys With Values* rule that a map contains each key at most once. Removing a key the map does not contain MUST yield a map equal to the operand rather than trapping, so that removal is total.

A map MUST report the number of keys it associates, and that count MUST equal the number of distinct keys added and not since removed.

The add-or-replace and the remove operations MUST each come in two forms: a plain form yielding only the new map, and a form that additionally yields the value the key held before the operation as an optional — present when the key was associated beforehand and absent when it was not — paired with the new map. The plain form is the common case that discards the prior value; the value-yielding form lets a program observe what an add replaced or a remove dropped in a single operation, without a separate lookup. The two forms MUST agree on the resulting map, so that the only difference is whether the prior value is reported.

### Keys Are Compared By Value, Not Representation

Whether a map contains a key, and which entry a lookup or removal names, MUST be decided by the key's value under *core-semantics.md §Equality Is Structural* — two keys that are equal as values name the same entry regardless of how each was constructed or stored. A map therefore MUST NOT expose or depend on any hashing, ordering, or internal placement of its keys as observable behavior; only membership, association, size, equality, and the deterministic iteration order below are observable. This realizes *memory-and-resource-model.md §Sharing Is Not Observable* for the map type, exactly as *A List's Representation Is Unspecified And Unobservable* does for lists.

### A Map Renders As Its Entries In Canonical Key Order

A map's canonical form MUST present its entries as key-value pairs in the deterministic order of *Map Iteration Is Deterministic*, so that two equal maps have identical canonical forms regardless of the order their entries were added. The canonical form MUST be distinguishable from a record's, so that a map and a record are never confused by their rendered form even when they carry the same keys and values (a map's keys are values of one key type; a record's field names are fixed compile-time labels).

### Map Iteration Is Deterministic

Iterating a map MUST visit its entries in a deterministic order derived from the keys, not from insertion order.

The order in which a map's entries are visited MUST agree with the order its canonical byte form places them in.

## Sets

### A Set Is A Collection Of Unique Elements

A set MUST be a collection of elements of one type.

A set MUST contain each element at most once.

Two sets MUST be equal exactly when they contain equal elements, independent of insertion order.

### Set Membership Is Total

Testing whether a set contains an element MUST be total, yielding a boolean rather than trapping.

A set MUST NOT offer access to an element by position, because a set is unordered and has no positional element to address.

### Set Iteration Is Deterministic

Iterating a set MUST visit its elements in a deterministic order derived from the elements, not from insertion order.

The order in which a set's elements are visited MUST agree with the order its canonical byte form places them in.
