# Decision — Ad-Hoc Polymorphism

**The decision.** How Cadenza resolves an operation whose implementation depends on the type it is
used at (what other languages call type classes, traits, or implicits). The type system requires
principal-type inference and forbids a separate polymorphism engine that fights content-addressed
modules, but it does not fix the resolution mechanism, which is what this decision pins.

**Requirements any choice must satisfy (from the spec — do not weaken):**
- Resolution is deterministic and a function of the source (constitution II, III).
- Resolution is compatible with content-addressed modules — no global coherence assumption and no
  orphan-rule that a content-addressed module cannot honor (type-system.md).
- The mechanism monomorphizes away before the boundary, leaving no runtime dispatch the manifest did
  not declare (type-system.md; component-abi.md).

## Choices

- [`explicit-dictionaries`](./explicit-dictionaries.md) — a trait is a dictionary record type and an
  instance is an ordinary value of it, passed to a polymorphic definition as an **ordinary explicit
  parameter**; there is no resolution engine at all, so no scoped search, no global coherence, no
  orphan rule, and no ambiguity — the caller names the instance, and monomorphization inlines it. An
  implicit-resolution convenience MAY be added later only as a meaning-preserving elaboration that
  desugars to explicit passing. **The default.**

DEFAULT: explicit-dictionaries
