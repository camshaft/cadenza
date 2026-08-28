# Frozen Contract — AST Binary Format

> **FROZEN CONTRACT.** This document pins the CONCRETE byte format of a Cadenza program's canonical
> stored form — the exact bytes that realize the abstract properties fixed by
> [ast-encoding.md](./ast-encoding.md). It exists so the binary AST is implementable and checkable by a
> third party (a clean-room decoder, a differential oracle, a non-Rust reader) from the specification
> alone, rather than only from the reference implementation. It is versioned and changed only by the
> coordinated act described in the constitution's Governance Floors.
>
> RFC-2119 key words are normative. [ast-encoding.md](./ast-encoding.md) states the encoding's abstract
> properties (bijection, self-contained prelude, versioning, canonical order); THIS document states the
> byte layout those properties are realized by. Where the two are ever in tension the abstract contract
> governs the intent and this document governs the bytes; they are kept in agreement by the same
> governance act.

## Purpose And Scope

[ast-encoding.md](./ast-encoding.md) pins that a program's canonical stored form is a binary
serialization of its abstract syntax tree and fixes that serialization's abstract properties, but
defers the concrete byte layout to the declared-default location. That left the only definition of the
bytes in the reference implementation, so a third party had nothing to implement the codec FROM. This
document is that concrete layout: the version header, the varint, the leaf pool, the structure pool,
and the root, at the byte level, with the validity rules a conformant decoder enforces.

This document pins the container version `cdzast\x00\x01`, which is the only version defined. A decoder
MUST refuse a container whose eight-byte version header it does not implement (see
[Versioning](./ast-encoding.md)) rather than misinterpret it; defining a new version is the coordinated
act described in the constitution's Governance Floors.

## Conventions

### Byte Order And Integer Encodings

A multi-byte scalar field is stated explicitly as either a variable-length integer (`varu64`) or a
fixed-width big-endian integer; there is no other integer encoding in this format.

A `varu64` MUST be an unsigned LEB128 varint: successive 7-bit groups of the value from least
significant to most significant, each group in the low 7 bits of a byte whose high bit is set on every
byte except the last, at most ten bytes.

A `varu64` MUST be minimal: the encoding of a value MUST use the fewest bytes possible, so that no value
has a longer non-canonical encoding; a decoder MUST refuse an over-long (non-minimal) or over-ten-byte
`varu64` rather than accept it. This minimality is part of what makes the whole encoding a bijection
with one byte form per tree.

A fixed-width big-endian integer field MUST be encoded as its two's-complement value in the stated
number of bytes, most significant byte first.

## Overall Structure

### A Binary AST Is A Header, A Leaf Pool, A Structure Pool, And A Root

A binary AST MUST be, in order and with no padding between them: the container header, the leaf pool,
the structure pool, and the root reference.

A decoder MUST consume the entire input; a binary AST with valid content followed by additional bytes
MUST be refused rather than accepted for the prefix that parses.

## The Container Header

### The Header Is Eight Bytes Naming The Format And Version

A binary AST MUST begin with the eight bytes `63 64 7A 61 73 74 00 01` — the six ASCII bytes `cdzast`
followed by the container-format version as a two-byte big-endian integer, which is `1` for the format
this document pins.

A decoder MUST refuse an input whose first eight bytes are not a container header it implements: fewer
than eight bytes present is a truncated input, and eight bytes present but not a recognized header is a
different or corrupt format, and either MUST be refused rather than misinterpreted.

## The Leaf Pool

### The Leaf Pool Is A Count Then That Many Leaves

The leaf pool MUST be a `varu64` leaf count followed by exactly that many leaf encodings in order; a
structure atom references a leaf by its zero-based index into this pool.

### A Leaf Is A Kind Byte Then A Kind-Determined Body

Each leaf MUST be a single kind byte followed by the body that kind defines; the kinds and their bodies
are exactly the following, and a decoder MUST refuse any other kind byte.

- `0`–`5` — an INTEGER literal. The kind byte folds the sign and the source radix: `0` positive
  decimal, `1` positive hexadecimal, `2` positive binary, `3` negative decimal, `4` negative
  hexadecimal, `5` negative binary. The body MUST be a `varu64` magnitude length followed by that many
  bytes of the magnitude as a big-endian, minimal (no leading zero byte) non-negative integer. The
  radix is a display distinction only — it records the spelling a textual syntax used and does not
  change the value. Zero MUST be encoded as the empty magnitude (length `0`) with a positive kind; a
  negative kind with an empty magnitude MUST NOT be produced.
- `6` — a finite FLOAT (an exact decimal). The body MUST be: one byte that is `1` if the value is
  negative and `0` otherwise; then the base-ten exponent as an eight-byte big-endian two's-complement
  integer; then a `varu64` significand length and that many bytes of the significand as a big-endian,
  minimal non-negative magnitude. The value denoted is `(-1)^negative × significand × 10^exponent`; the
  significand carries no sign (its sign is the leading byte).
- `7` — a STRING literal. The body MUST be a `varu64` byte length followed by that many UTF-8 bytes.
- `8` — the boolean `false`. The body MUST be empty.
- `9` — the boolean `true`. The body MUST be empty.
- `10` — a NAME (an identifier or a construct head). The body MUST be a `varu64` byte length followed by
  that many UTF-8 bytes.
- `11` — a BYTES literal (an arbitrary byte sequence, not necessarily UTF-8). The body MUST be a
  `varu64` byte length followed by that many raw bytes.
- `12` — a BAD-ESCAPE marker (a lexically malformed string escape the reader preserves). The body MUST
  be a `varu64` byte length followed by that many UTF-8 bytes encoding exactly one Unicode scalar (the
  offending escape). A decoder MUST refuse a body that is not valid UTF-8 or that decodes to zero or to
  more than one scalar, so that the leaf is injective.
- `13` — a CHARACTER literal (a single Unicode scalar value). The body MUST be a `varu64` byte length
  followed by that many UTF-8 bytes encoding exactly one Unicode scalar. A decoder MUST refuse a body
  that is not valid UTF-8 or that decodes to zero or to more than one scalar, so that the leaf is
  injective (a one-character `a` and a two-character `ab` cannot collide).
- `14` — a BAD-CHARACTER marker (a character literal spelling a non-scalar the reader preserves). The
  body MUST be a `varu64` byte length followed by that many UTF-8 bytes of the offending text.
- `15` — a SYMBOL literal. The body MUST be a `varu64` byte length followed by that many UTF-8 bytes.
- `16` — a TYPE-SUFFIXED numeric literal (a surface spelling such as `100N` or `0.5R`). The body MUST
  be: one suffix byte, `0` for the `BigInt` suffix (`N`) or `1` for the `Rational` suffix (`R`); then
  one body-shape byte, `0` for an integer body or `1` for a float body; then the body encoded exactly
  as a bare integer or float leaf's body is — for an integer body, an integer kind byte (`0`–`5`) then
  a `varu64` magnitude length and magnitude bytes; for a float body, the float body defined for kind
  `6`.
- `17` — the floating-point not-a-number value. The body MUST be empty.
- `18` — positive floating-point infinity. The body MUST be empty.
- `19` — negative floating-point infinity. The body MUST be empty.
- `20` — a LIST constructor head — the head leaf of a list literal `(<list-ctor> element…)`. The body
  MUST be empty; the leaf carries its meaning in its kind alone, so a compound literal's constructor is
  recognized by this kind identity rather than by comparing a head's text against a reserved spelling.
- `21` — a TUPLE constructor head — the head leaf of a tuple literal `(<tuple-ctor> element…)`. The body
  MUST be empty.
- `22` — a RECORD constructor head — the head leaf of a record literal `(<record-ctor> field…)` whose
  fields are FIELD-PAIR entries (kind `25`). The body MUST be empty.
- `23` — a MAP constructor head — the head leaf of a map literal `(<map-ctor> entry…)` whose entries are
  FIELD-PAIR entries (kind `25`). The body MUST be empty.
- `24` — a SET constructor head — the head leaf of a set literal `(<set-ctor> element…)`. The body MUST
  be empty.
- `25` — a FIELD-PAIR head — the head leaf of a record/map entry `(<field-pair> key value)` (the `=`
  marker). The body MUST be empty; the entry marker is recognized by this kind identity, distinct from a
  NAME leaf spelling `=` (kind `10`), which remains an ordinary name.
- `26` — a MEMBER-ACCESS head — the head leaf of a projection `(<member> object key)` (the `.` marker).
  The body MUST be empty; the projection marker is recognized by this kind identity, distinct from a NAME
  leaf spelling `.` (kind `10`), which remains an ordinary name.

## The Structure Pool

### The Structure Pool Is A Count Then That Many Structure Entries

The structure pool MUST be a `varu64` structure-entry count followed by exactly that many structure
encodings in order; a list child and the root reference a structure entry by its zero-based index into
this pool.

### A Structure Entry Is An Atom Or A List

Each structure entry MUST be a single tag byte followed by the body that tag defines, and a decoder
MUST refuse any other tag byte:

- `0` — an ATOM: the body MUST be a `varu64` index into the leaf pool. An atom is a leaf standing as a
  node of the tree.
- `1` — a LIST: the body MUST be a `varu64` child count followed by that many `varu64` indices into the
  structure pool, in order. A node is a head followed by its arguments, all as list children, so the
  container form is independent of which node kinds the language defines.

## The Root

### The Root Is A Reference To The Top Structure Entry

The binary AST MUST end with a `varu64` index into the structure pool naming the tree's root entry.

## Referential Integrity And Validity

### A Decoder Enforces Referential Integrity, Tree-ness, And Exact Consumption

A decoder MUST refuse a binary AST in which any atom's leaf index is greater than or equal to the leaf
count, any list child index or the root index is greater than or equal to the structure count, or any
such index does not fit in an unsigned 32-bit integer.

A decoder MUST refuse a binary AST in which the structure reachable from the root is not a tree — that
is, in which some reachable structure entry is reached more than once, whether by a cycle or by a
shared subtree — so that a recursive consumer cannot diverge or be forced to expand a shared subtree
exponentially. Structure entries not reachable from the root are permitted and ignored.

A decoder MUST refuse a binary AST with bytes remaining after the root reference.

## The Canonical Byte Form

### Equal Trees Encode To Identical Bytes

An encoder MUST produce, for a given abstract syntax tree, the one canonical byte sequence: the leaf
pool MUST list each distinct leaf exactly once (leaves are deduplicated by value) and the leaf pool and
the structure pool MUST be ordered by a deterministic function of the tree alone, independent of the
order in which nodes were constructed, so that two equal trees produce identical bytes.

The deterministic order is fixed as follows, so that a third party can produce the canonical bytes and
not merely read them. The structure pool MUST be in POST-ORDER: an entry appears after all of its
children, so the root is the LAST structure entry. The leaf pool MUST be in FIRST-ENCOUNTER order under
a pre-order (parent-before-children) walk of the tree, each distinct leaf placed at its first
occurrence and deduplicated by value thereafter. Two leaves are the same for deduplication when they are
equal by value, and a `Name` leaf's text MUST be Unicode NFC-normalized before that comparison so that
two canonically-equal spellings intern to one leaf. An integer or float significand magnitude MUST be
minimal (no leading zero byte), zero MUST be the empty magnitude, and zero MUST NOT take a negative
kind; these together give each value one magnitude form.

Decoding a canonical binary encoding MUST yield the abstract syntax tree it was encoded from, and
re-encoding that tree MUST reproduce the same bytes.

A decoder MUST accept the pools in the order the bytes present them and MUST NOT require them to be in
the canonical order; the canonical order is an encoder obligation, and referential-integrity and
tree-ness (above) are what a decoder enforces. The leaf pool realizes the self-contained symbol prelude
that [ast-encoding.md](./ast-encoding.md) requires: a node names its kind by an atom referencing a
`Name` leaf in the pool by index, and the file carries every leaf it references.

## Worked Byte Examples

### Per-Field Encodings

The following are the exact body bytes for representative leaves (kind byte first), which a conformant
encoder produces and a conformant decoder accepts:

- The integer `300` (positive, decimal): `00 02 01 2C` — kind `0` (positive decimal), magnitude length
  `2`, magnitude `0x012C`.
- The integer `-1` (decimal): `03 01 01` — kind `3` (negative decimal), magnitude length `1`, magnitude
  `0x01`.
- The integer `0`: `00 00` — kind `0` (positive decimal), magnitude length `0` (empty).
- The string `"hi"`: `07 02 68 69` — kind `7`, length `2`, UTF-8 `hi`.
- The name `+`: `0A 01 2B` — kind `10`, length `1`, UTF-8 `+`.
- The boolean `true`: `09` — kind `9`, no body.
- The float `1.5` (that is, `15 × 10^-1`): `06 00 FF FF FF FF FF FF FF FF 01 0F` — kind `6`, not
  negative, exponent `-1` as an eight-byte big-endian two's-complement integer, significand length `1`,
  significand `0x0F` (fifteen).
- The not-a-number value: `11` — kind `17`, no body.
- The list constructor head: `14` — kind `20`, no body.
- The record constructor head: `16` — kind `22`, no body.
- The field-pair (`=`) head: `19` — kind `25`, no body.
- The member-access (`.`) head: `1A` — kind `26`, no body.

## Additive Evolution

### Additive Evolution Of This Contract

The addition of a new leaf kind or structure tag MUST be assigned a previously unused byte and MUST NOT
change the bytes of any tree that does not use it, consistent with the general-and-stable property of
[ast-encoding.md](./ast-encoding.md).

A change to this document that alters the bytes of an already-encodable tree MUST carry a container
version increment and a stated migration path; a change that only pins bytes for a construct that
previously had none MUST be permitted as an additive change.
