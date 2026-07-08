# Value Interchange — Choice: schema-hashed-envelope

> **The default choice for the `value-interchange` decision** (see [README.md](./README.md) for the
> decision and the requirements a choice must satisfy). It names the program-facing operations that turn
> a value into bytes and back, and the optional self-checking header that lets a decoder confirm it was
> handed the bytes of the type it expects.
>
> The interchange payload is the canonical value form the language already realizes
> (deterministic-value-form.md; options/hashing-and-encoding). This choice adds a surface and a header
> over that form; it introduces no new value bytes. Because the header's bytes and the payload's bytes
> cross a boundary and identify a value's type, a change to either is a coordinated change under the
> constitution's Governance Floors, with a migration path.

## The value form is inherited, not redefined

A value's interchange bytes are exactly its canonical byte form under the value-form contract and the
`hashing-and-encoding` choice: deterministic CBOR, NFC-normalized string contents, byte-wise-ordered
unordered-aggregate members, width-indexed integers, one canonical float/NaN form
(deterministic-value-form.md; options/hashing-and-encoding/sha256-deterministic-cbor.md). This choice
does not restate those bytes; it names the operations that expose them and the header that guards them.
Equal values therefore serialize identically and unequal values distinctly — a property inherited from
the value form rather than re-established here.

## The surface

Interchange is a pair of operations over the existing `Bytes` value:

| Operation | Type | Meaning |
|---|---|---|
| `to-bytes` | `T → Bytes` | the value's canonical byte form, no header (the *bare* form) |
| `from-bytes` | `Bytes → Option<T>` | decode bare bytes *against the expected `T`*; `None` if the bytes are not a valid encoding of a `T` |
| `to-tagged-bytes` | `T → Bytes` | the schema-hash header (nominal tag participating) followed by the bare bytes |
| `to-tagged-bytes-structural` | `T → Bytes` | the schema-hash header computed over the *structural* type (nominal tag erased) followed by the bare bytes |
| `from-tagged-bytes` | `Bytes → Option<T>` | verify the header against the expected `T`, then decode; `None` on header mismatch or invalid payload |

These are ordinary type-directed operations in the dotted-method form the language already uses
(`Bytes.of`, `String.from-bytes`, `Int64.checked-add`): a generic surface monomorphized at each use site
against the statically-known `T` (host-value-agnostic; generics = monomorphization). `from-bytes` is the
value-level companion of the existing fallible `String.from-bytes : Bytes → Option<String>` UTF-8
validator — same shape, same `Option` refusal discipline — generalized from "is this valid UTF-8" to "is
this a valid encoding of a `T`".

**Bare is the tail of tagged.** `to-tagged-bytes` produces exactly the 8-byte header followed by the
bytes `to-bytes` would produce; `from-tagged-bytes` strips and checks the header, then runs the same
decode `from-bytes` runs. There is one payload format, offered with or without a header — not two
formats. A program uses the bare form on paths where both sides already agree on `T` (the type is known
statically, so the header would be pure overhead) and the tagged form where a value is "passed around"
to a decoder that must confirm what it received.

## Nominal versus structural is a choice of operation, not a mode

Whether a type's nominal tag participates in the schema hash is decided by *which operation a program
calls*, never by a flag byte on the wire:

- `to-tagged-bytes` hashes the type *including* its nominal tag, so a `UserId` (a nominal `Int64`) and a
  bare `Int64` produce different headers and do not interchange — the boundary companion of the existing
  `CDZ0202` rejection that makes a nominal value non-comparable to the plain value of its shape
  (type-system.md #Nominal Is An Orthogonal Modifier Over Any Structural Type).
- `to-tagged-bytes-structural` hashes the *structural* type only, erasing the nominal tag, for generic
  transport where a receiver keys on shape rather than on nominal identity.

In every case the *payload* is identical — nominal identity never reaches the bytes, because the tagless
value-heap runtime does not carry it (component-abi.md #A Runtime Value Crosses As An Opaque Handle). The
nominal/structural distinction lives entirely in the header.

## The schema-hash header

The header is an 8-byte prefix: the first 8 bytes of `SHA-256` over a *canonical byte form of the
value's type*, laid out most-significant-byte first.

- **What is hashed.** A type is normalized to a type term and encoded by the already-pinned AST encoding
  (ast-encoding.md): a tree of nodes, each a namespaced, optionally-versioned symbol applied to ordered
  children, carrying its own canonical symbol prelude. A `list<u8>`, a `record` with named fields, a
  `sum` with named variants, a nominal tag over a structural shape — each is a type term with one
  canonical encoding. The schema hash is `SHA-256` of that encoding (options/hashing-and-encoding fixes
  the hash), truncated to 8 bytes.
- **Why the AST encoding.** Reusing it gives the type hash three properties for free that the requirements
  demand: one canonical byte form per type (ast-encoding.md §"The Encoding Is A Bijection With One
  Canonical Byte Form"), a discovery-order-independent prelude so the hash is a function of the type's
  content and not of the order types were seen (§"The Prelude Order Is Canonical"), and namespaced,
  versioned symbols so a type's meaning can evolve without a hash collision and an evolved type simply
  produces a different hash (§"A Prelude Symbol Is Namespaced And May Be Versioned"). This makes the value
  form and the code form structurally the same construction — a type is hashed the way a program is
  hashed.
- **Why 8 bytes.** Sixty-four bits is negligible next to any real payload and gives ample collision margin
  for a schema guard whose job is to catch "wrong type / wrong version," not to be a cryptographic
  commitment. Full-digest and other truncation lengths are reserved as ABI-versioned changes should a use
  ever need them.

## Receiver behavior — verify and refuse

`from-tagged-bytes` reads the 8-byte header, compares it to the schema hash of the `T` it was asked to
decode into, and:

- **matches** → decodes the remaining bytes against `T` (the bare-decode path), yielding `Some(value)` or
  `None` if the payload is not a valid `T`;
- **mismatches** → returns `None` without decoding.

Refusal-on-mismatch is the value-level instance of the pattern the language already follows at every
boundary: a reader refuses a construct it does not understand rather than misinterpreting it
(ast-encoding.md §"A reader MUST refuse … rather than misinterpret"); a host refuses a runtime whose
content address does not match rather than substituting one (component-abi.md; reproducible-derivation.md).

**No registry, no dispatch.** This choice pins verify-and-refuse only. Dispatch — looking a header up in a
`schema-hash → handler` table to decide *which* type to decode into — is deliberately out of scope; a
program that wants it can build it in userland over `Map<Bytes, …>`, and a language-level intern table for
type hashes (the type-level companion of symbol-interning) is a possible future decision, not this one.

## Additivity

Adding this surface changes no already-defined bytes:

- The payload is the existing canonical value form; no value's encoding changes.
- The *decode* direction is defined where the value-form contract previously pinned only encode — a value
  that previously had a canonical encoding but no specified inverse now has one, and decode is the inverse
  of encode on the values encode produces (deterministic-value-form.md §"Additive Evolution").
- The canonical *type* form is defined where none existed, by reusing the AST encoding — permitted by the
  same additive-evolution clause and requiring no container-encoding version bump (ast-encoding.md §"New
  Constructs Do Not Bump The Encoding Version").

Floating-point interchange rides the value form's canonical float form when a build realizes floats in the
value form; until then, a type containing a float simply has no interchange form yet, which is a permitted
"a value that previously had none" gap rather than a defect (deterministic-value-form.md §"Additive
Evolution"). A type whose interchange form is added later produces a distinct schema hash, so a decoder
built before the addition refuses the newer bytes rather than misreading them.

## External interop (why the concrete bytes are not adopted here)

The architecture above is drawn from an external positional binary wire format of the same shape, studied
byte-exactly in [`references/positional-binary-wire-format.md`](./references/positional-binary-wire-format.md).
Its *design* — an optional prefix selected by which operation you call, a SHA-256 schema hash truncated to
8 bytes, a verify-and-refuse receiver — is exactly what this choice adopts. Its *concrete payload bytes*
are not, because they diverge from Cadenza's already-frozen value form on three points that matter:

- **Payload encoding.** The external format is a big-endian positional layout with `u64` length prefixes
  on variable-length data; Cadenza's canonical value form is deterministic CBOR
  (options/hashing-and-encoding). Adopting the external layout on the wire would introduce a *second*
  value encoding competing with the canonical one, which the requirements forbid for the default.
- **Integers.** The external format uses fixed-width big-endian scalars (and an opt-in LEB128 varint);
  Cadenza's numeric model is width-indexed `(Int N)`/`(UInt N)` (options/numeric-model). A faithful
  mapping is possible but is its own additive design.
- **Floating-point.** The external format's core defines *no* float encoding; Cadenza's value form
  mandates a canonical float/NaN/−0.0 form. There is no float layout to interoperate *with* until one is
  defined on both sides.

An `external-format-compatible` alternative choice — adopting the external payload layout on the wire and
defining the missing float form — is authorable by a build for which direct on-the-wire interoperability
with that format outweighs staying on the canonical value form. It is not the default, because the default
must not stand up a second value encoding or fight a frozen contract.

## Why these choices

- **One value encoding, reused.** Interchange bytes are the canonical value form, so there is exactly one
  answer to "the bytes of a value" and a third party needs only the primitives it already has.
- **Optional header by operation, not by flag.** Selecting bare versus tagged by which operation a program
  calls keeps the payload format singular and pushes the nominal/structural and tagged/untagged choices to
  the call site, where the type is known — no runtime branch, no wire flag to misread.
- **The type is hashed like the code.** Computing the schema hash over the AST encoding of the type unifies
  the value form with the code form and inherits canonicality, discovery-order independence, and versioned
  evolution without inventing a new type-serialization.
- **Refuse, never misread.** A mismatched header is a detected error returning `None`, matching the
  language's boundary discipline everywhere else.
