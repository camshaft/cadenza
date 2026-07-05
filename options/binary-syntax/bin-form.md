# Binary Syntax — Choice: bin-form

> **The default choice for the `binary-syntax` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It pins one `bin` keyword serving both binary
> construction and binary matching, over the existing `Bytes` value form.

## The choice

One keyword, `bin`, serves two directions by reusing the constructor/pattern duality the language
already has — a variant `(Some 5)` builds and `(Some n)` destructures the same way:

- In **expression position**, `(bin <segment>...)` **constructs** a `Bytes` value by encoding each
  segment in order and concatenating the results.
- In **pattern position** (inside an ordinary `match`), `(bin <segment>...)` **destructures** a `Bytes`
  scrutinee: it reads each segment in order, binding names and matching literals, and the arm fires only
  when every segment matches and the byte accounting works out. The `bin` head constrains the scrutinee
  to `Bytes`, exactly as `(Some n)` constrains it to a sum.

No new control construct: a binary pattern is a pattern like any other, and a `bin` result is an
ordinary `Bytes` value indistinguishable from one built with `Bytes.of`/`Bytes.concat`.

## Segments

A `bin` is an ordered sequence of segments. Each is written like a constructor
`(<kind> <slot> <modifier>...)` — the head names the encoding, the slot is a value (when building) or a
binder/literal (when matching):

| Segment | Construct | Match |
|---|---|---|
| `(u8 v)` `(u16 v)` `(u32 v)` `(u64 v)` | emit `v` as an unsigned N-bit integer, **big-endian by default** | bind an unsigned N-bit integer |
| `(i8 v)` `(i16 v)` `(i32 v)` `(i64 v)` | emit `v` as a signed N-bit integer, two's complement | bind a signed N-bit integer |
| `(uNN v le)` / `(iNN v le)` | the `le` modifier selects little-endian byte order | same, reading little-endian |
| `(bits v k)` | emit the low `k` bits of `v`; `k` is a **compile-time constant** | bind the next `k` bits as an integer |
| `(bytes b)` | splice all of `b` | **final segment only:** bind the remaining bytes |
| `(bytes b n)` | splice `b`, whose length must be `n` | bind exactly `n` bytes; `n` MAY be a name bound by an earlier segment (**dependent size**) |

A **literal** in the slot means match-by-equality — the direct analogue of a literal value pattern
(`(match 2 (2 "two") …)`). `(bin (u32 0x89504E47) (bytes rest))` matches a scrutinee whose first four
bytes are `0x89504E47` and binds the remainder to `rest` — a hex literal (01-literals.sexp) names the
magic number legibly, which is the reason radix literals pair so naturally with binary matching.
Fixed-width integer segments default to **big-endian** (network order, and the order the
wasm/self-hosting idiom needs); the `le` modifier is the only way to select little-endian, so byte
order is always explicit.

The **dependent-size** `bytes` segment — `(bytes body n)` where `n` was bound by an earlier segment in
the same pattern — is the form's reason to exist: it expresses length-prefixed framing directly, is
entirely value-level (so erasure and monomorphization are untouched), and needs no dependent-type
machinery because "not enough bytes remain" is simply a non-match.

## Byte-alignment is static

The whole `bin` MUST be byte-aligned. Because `bits` widths are compile-time constants, the running
sum of bit widths between byte boundaries is known at compile time, so misalignment is caught
**statically**, not at run time. The following are **ill-formed binary forms**, rejected `CDZ0220`
(the `CDZ02xx` types-and-patterns band):

- bit-field widths that do not close a whole byte (e.g. `(bin (bits x 1) (bits y 3))` — 4 bits);
- a non-final **unsized** `(bytes b)` segment (it would consume the rest, so nothing after it is
  reachable) — a sized `(bytes b n)` may appear anywhere;
- a `bits` width that is not a compile-time constant.

## Runtime fit is a trap

A value with no encoding in its segment has no defined result, so **construction traps** with reason
`"binary value does not fit segment"` (the companion of the `Bytes` out-of-range trap) rather than
truncating or wrapping:

- a value above an unsigned segment's range — `(bin (u8 256))` traps;
- a **negative** value in an **unsigned** segment — `(bin (u8 -1))` traps (whereas the **signed**
  `(bin (i8 -1))` encodes `-1` as the two's-complement byte `255`; unsigned and signed segments differ
  precisely here);
- a value wider than a bit-field's width — `(bin (bits 2 1))` traps.

## Exhaustiveness reuses the existing rule

A `bin` pattern never covers every byte sequence: the empty sequence, a shorter sequence, and one whose
literal segments differ all fail to match. So a match over `Bytes` whose only arm is a `(bin …)` pattern
does not cover the scrutinee's type and is rejected `CDZ0210` — the same rejection a sum match missing a
variant gets. A bare final `(bin (bytes rest))` matches any byte sequence and so serves as a catch-all.
No special case: binary matching reuses exhaustiveness rather than adding a rule.

## Resolved forks

Two design forks were resolved when this choice was adopted:

- **One `bin` keyword, dual** (not two forms `Bytes.pack`/`Bytes.unpack`). Reusing the
  constructor/pattern duality keeps patterns un-namespaced (the language namespaces no other pattern)
  and adds one keyword rather than two names. The alternative — namespacing the forms under the `Bytes`
  module — was rejected because it would make binary patterns the only namespaced pattern.
- **Byte-granular with in-byte bit-fields** (not a distinct bit-granular `Bits` value form). `Bytes`
  stays the only value form; a `bin` may pack sub-byte bit-fields but the whole is always byte-aligned.
  This covers the wasm/LEB128/length-framing needs a self-hosting compiler has, with the simplest
  alignment story and no second value form to give its own equality, canonical encoding, and length
  semantics. A first-class arbitrary-bit-length `Bits` value form is left for a later decision, to be
  taken only if a real bit-packed protocol (one whose fields are not byte-aligned as a whole) demands
  it.

## What it replaces

The form subsumes the hand-rolled byte layout the corpus currently expresses with masks and shifts.
Length-framing:

```
; before — hand-rolled big-endian u16 prefix
(Bytes.concat (Bytes.of (list (& (>> len 8) 255) (& len 255))) payload)
; after
(bin (u16 len) (bytes payload))
```

A packed flags byte (1-bit flag ++ 3-bit tag ++ 4-bit value):

```
; before
(Int.to-byte (| (| (<< flag 7) (<< tag 4)) val))
; after — 1 + 3 + 4 = 8 bits, statically closes one byte
(bin (bits flag 1) (bits tag 3) (bits val 4))
```
