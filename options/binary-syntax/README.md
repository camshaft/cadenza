# Decision — Binary Syntax

**The decision.** The surface and semantics of Cadenza's binary construction-and-matching form — how a
program builds a `Bytes` value from typed segments and how it destructures one by pattern — over the
`Bytes` value form the language already realizes. The constitution and the collections spec fix that
`Bytes` is an immutable byte-sequence value with construction, equality, length, concatenation, and
total-or-trap indexing (spec/semantics/10-bytes.sexp; collections-and-text.md); they do not fix a
syntax for reading and writing structured binary layouts, because that surface is the choice this
decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- The form reuses `match` and the pattern grammar rather than adding a new control construct — a binary
  pattern is a pattern like any other (core-semantics.md §"Pattern Matching").
- Exhaustiveness is the existing rule: a match over `Bytes` whose arms do not cover every byte sequence
  is rejected `CDZ0210` (core-semantics.md §"Matching Is Exhaustive Or Rejected"); a binary pattern is
  not a special case.
- A partial construction (a value with no encoding in its segment) has a defined outcome — a trap of a
  defined kind, never an unspecified or truncated value (core-semantics.md §"Partial Operations Have A
  Defined Outcome").
- Byte order and signedness are explicit in the syntax, never an implicit host-endianness or
  sign-convention choice — consistent with deterministic value forms (determinism-and-fuel.md;
  deterministic-value-form.md).
- The result is an ordinary `Bytes` value, indistinguishable from one built with `Bytes.of`/`Bytes.concat`
  (10-bytes.sexp), so the form adds a surface, not a new value type.

**Why this is an isolated decision.** The form is sugar over the existing `Bytes` value form and the
existing `match`: it lowers to byte construction and structural destructuring the language already has.
Changing the segment grammar is an edit to a choice file here plus a lowering in the compiler; it
touches no frozen contract and no capability requirement, and it introduces no new value form (a `bin`
result is a `Bytes`). It is realized by a later generation, not the seed (`options/realized-capability-set/`);
until then its corpus cases carry `(needs binary-matching)` and the seed's behavior gate skips them.

## Choices

- [`bin-form`](./bin-form.md) — one `bin` keyword serving both directions (construct in expression
  position, destructure in pattern position), with fixed-width integer segments (explicit endianness
  and signedness), sub-byte bit-fields, and dependent-size `bytes` segments. **The default.**

DEFAULT: bin-form
