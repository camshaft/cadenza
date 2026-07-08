# Decision — Value Interchange

**The decision.** The program-facing surface and byte form by which a Cadenza value is turned into
bytes and read back — a *stable value form* that a program can persist, hand to another component, or
store across compiler generations, and that another party can decode into the same value. The value-
interchange capability fixes the invariants this surface must hold (value-interchange.md): that its
bytes are the canonical value form, that decode inverts encode and refuses anything else, that a
serialized value may carry a content-derived type header, and that a decoder refuses a header that does
not match. The value-form contract fixes the canonical byte form itself, injective with structural
equality, and now its decode inverse (deterministic-value-form.md). What those requirements leave to
the declared-default location — the concrete operations, the schema-identity function and header
layout, and the canonical byte form of a *type* from which the header is derived — is the choice this
decision pins.

**Why the language wants it.** A value that can leave a running program and come back unchanged is the
foundation of durable state, inter-component messaging, and content-addressed caching keyed on values
rather than on source. It is the value-level companion of what the language already has for code: a
program has a durable, hashable, third-party-checkable stored form (ast-encoding.md); a *value* should
have the same. The pattern is Erlang's `term_to_binary`/`binary_to_term` — a value serializes to a
stable blob and reconstitutes on the other side — adapted to a statically-typed, tagless runtime: the
bytes stay tag-free and the type is known (or checked) at the boundary rather than carried per-node.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The bytes a value serializes to MUST be the canonical byte form the value-form contract already pins
  (deterministic-value-form.md §"The Canonical Byte Form"), so interchange does not introduce a second,
  competing value encoding: equal values serialize identically and unequal values serialize distinctly,
  inherited rather than re-established. A serialized value's numeric, unit, float, and aggregate-order
  bytes are exactly those the value-form contract fixes (numeric-model.md; deterministic-value-form.md).
- Serialization MUST be deterministic and stable across runs and across compiler generations, so a value
  serialized by one generation reads back identically under another (deterministic-value-form.md;
  reproducible-derivation.md §"Derivation Is A Function Of Source And Toolchain").
- Adding this surface MUST be additive: it defines a boundary/interchange form and a decode direction
  where the value-form contract previously pinned only encode, and it defines a canonical form for a
  *type* that previously had none — both permitted by deterministic-value-form.md §"Additive Evolution"
  and, for the type form, by the same self-contained-prelude construction ast-encoding.md already uses
  for code (§"The File Carries Its Own Symbol Prelude", §"A Prelude Symbol Is Namespaced And May Be
  Versioned"). No frozen-contract version increment is required to define a form for a value or type that
  had none; a change to an *already-defined* form is an ABI-level coordinated act (below).
- Decode MUST be the inverse of encode on the values encode produces: decoding the bytes of a value
  yields that value, exactly as ast-encoding.md §"The Encoding Is A Bijection With One Canonical Byte
  Form" requires for code. Decode is total-with-refusal: bytes that are not the encoding of a value of
  the expected type MUST be refused (a `None`, never a misread value), consistent with the language's
  existing fallible readers returning `Option` (collections-and-text.md; the `String.from-bytes` UTF-8
  validator) and with ast-encoding.md §"A reader MUST refuse … rather than misinterpret".
- When a serialized value carries a schema tag, that tag MUST be a deterministic function of the type's
  content — never of the order in which types were seen or interned — mirroring the canonical, discovery-
  order-independent prelude of ast-encoding.md §"The Prelude Order Is Canonical" and the content-derived
  identity required of symbols (symbol-interning). A decoder handed a tag that does not match the type it
  expects MUST refuse rather than proceed, exactly as a host refuses a runtime whose content address does
  not match (component-abi.md §"A host that cannot locate a runtime … MUST refuse"; reproducible-
  derivation.md).

**Why this is an isolated decision.** The interchange payload *is* the canonical value form the language
already realizes — this decision adds no new value bytes, only a program-facing name for producing and
consuming them, plus an optional self-checking header. The header's schema hash is computed over a
canonical form of the value's *type*, which is itself expressible as a normalized type term encoded by
the already-pinned AST encoding (ast-encoding.md) and hashed by the already-pinned hash
(options/hashing-and-encoding). Nominal identity never enters the payload — a nominal value and its
underlying structural value serialize to identical bytes, because the nominal tag is a compile-time
distinction the tagless runtime does not carry (type-system.md #Nominal Is An Orthogonal Modifier Over
Any Structural Type; the value-heap runtime owns a tag-free representation, component-abi.md). The tag therefore
appears *only* in the schema hash, and whether it participates is a property of which operation a program
calls, not a new kind of value. So this decision touches no frozen contract's existing bytes and no
capability's existing requirement: it is a new surface and two additive forms (the decode direction and
the canonical type form) realized by a later generation, not the seed (options/realized-capability-set).
Until then its corpus cases carry a `(needs value-interchange)` tag and the seed's behavior gate skips
them.

**A note on external interoperability.** A byte-exact study of an external positional binary wire format
of the same *shape* — a tag-free positional payload with an optional truncated-schema-hash prefix — is
recorded under [`references/`](./references/positional-binary-wire-format.md). Its *architecture*
(optional prefix chosen by which operation you call; a SHA-256 schema hash truncated to 8 bytes; a
verify-and-refuse receiver) is the direct inspiration for the default choice below and fits Cadenza's
grain closely. Its *concrete bytes*, however, are a big-endian positional layout with `u64` length
prefixes and no defined floating-point form — which does not match Cadenza's already-frozen deterministic
value form (deterministic CBOR, width-indexed integers). Adopting that layout on the wire for direct
interop is therefore a genuine but separate choice with real costs (a second, non-canonical value
encoding; an unresolved float form); it is described as an authorable alternative here rather than the
default, because the default must not fight a frozen contract. See the choice file's "External interop"
section for the exact points of divergence.

## Choices

- [`schema-hashed-envelope`](./schema-hashed-envelope.md) — a program-facing `to-bytes`/`from-bytes`
  pair over the canonical value form, plus an optional 8-byte schema-hash prefix (SHA-256 over the
  normalized type's AST encoding, truncated) selected by calling the tagged operation; nominal-tag-
  participating and structural-only hashing are distinct operations; the receiver verifies the prefix and
  refuses a mismatch. Reuses the frozen value form, AST encoding, and hash unchanged. **The default.**

An `external-format-compatible` choice — adopting the external positional layout from `references/` on
the wire for direct third-party interop, accepting a second value encoding and a to-be-defined float
form — is a genuine alternative a build MAY author when on-the-wire interoperability with that format
outweighs staying on the canonical value form. It is deliberately not the default; see the note above.

DEFAULT: schema-hashed-envelope

## Change discipline

The interchange payload and the schema-hash construction fix bytes that cross a boundary and identify a
value's type, so a change to either — the payload encoding (inherited from the value form and hashing
choices) or the type-hash algorithm, truncation length, or prefix layout — is an ABI-level change under
the constitution's Governance Floor "The Component ABI Changes Only By Coordinated Act," evaluated
against already-serialized values with a stated migration path. Defining a form for a value or type that
previously had none remains a permitted additive change.
