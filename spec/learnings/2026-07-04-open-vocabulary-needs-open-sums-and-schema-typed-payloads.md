# Open vocabulary needs open sums and schema-typed payloads: a fold is total over an extensible kind space

*2026-07-04*

> **PARTIALLY SUPERSEDED (2026-08-25).** The **schema-typed-payload decode** half of this learning was
> removed: the operator ruled that the `decode` / `payload-of` / `Int64-schema` prelude surface (with the
> synthesized `Schema` / `DecodeError` sums and type-system.md §"An Open Sum's Payload May Be Schema-Typed",
> now retracted) was never greenlit and had zero consumers. **Open sum types themselves remain realized and
> unaffected** — only the schema-typed-payload decode mechanism this doc also motivated was deleted. Read
> the schema-payload sections below as historical rationale for a feature that no longer exists.

**What happened.** The target's vocabulary is **open**: event kinds are not a fixed enumeration in code
— a kind is introduced by publishing its schema as an event, and **a fold that has no handler for an
event's kind MUST treat it as a no-op rather than fail**. This imposes two things on Cadenza that the
current type universe does not yet provide, and one of them **pulls forward a feature the rows learning
had deferred**:
1. **Open sum types (extensible variants).** A fold matches the event kinds it knows and is **inert** to
   the rest. A *closed* sum type ([[2026-07-04-records-are-rows-open-by-default]] committed records to
   rows but left the sum dual — polymorphic variants — as "future work") cannot express "these known
   variants, plus an open tail I ignore." The target makes the sum dual **required**, not future work:
   a fold's scrutinee type is *open*, its match covers the known kinds, and a wildcard/`else` arm makes
   it total over the unknown tail.
2. **Schema-typed opaque payloads.** An event's payload is *opaque bytes* interpreted only against the
   declaration named by `(kind, kind version)`. So a fold decodes a payload **against a schema resolved
   at run time** — the payload's static type is known only once the kind is matched. This is
   parse-bytes-into-a-typed-value at a boundary, with the type determined by the matched variant.

**Why the open-sum requirement is now load-bearing (and re-opens the deferred item).** The rows learning
gave records row polymorphism and explicitly deferred the sum analogue. The target removes the option:
- **Forward compatibility is a core invariant of the target** ("unknown kinds are inert to folds",
  stated in both the event-schema contract and the fold capability). A fold authored today MUST tolerate
  event kinds declared tomorrow. That is *exactly* an **open/polymorphic variant** type: the set of
  variants is not closed at the fold's compile time.
- **Exhaustiveness must remain checkable.** Cadenza's `core-semantics.md` requires a match be exhaustive
  or rejected. Over an *open* sum, exhaustiveness is satisfied by a mandatory **open-tail arm** (the
  no-op/`else`) — so the compiler still proves totality, but totality now *includes* the "ignore unknown
  kinds" case. Without open sums, either exhaustiveness checking or forward compatibility breaks; with
  them, both hold: the fold is total, and its totality *is* the inert-to-unknown-kinds guarantee.

**Why schema-typed payloads fit the existing design.**
- **It is the reader/printer at a data boundary.** Decoding an opaque payload against a resolved schema
  is the reader ([[2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer]]) applied to event
  payloads: bytes → typed value, where the target type is chosen by the matched `(kind, version)`.
- **The result is a `Result`, not a trap.** A payload that does not validate against its declared schema
  yields a typed failure the fold handles, consistent with decline/reject-don't-miscompile and the
  Result idiom already in the corpus — a fold must not trap on a malformed payload it can instead route.
- **Version resolution is ordinary sum matching.** `(kind, kind version)` selecting a schema is the same
  nominal-identity mechanism ([[2026-07-04-nominal-is-orthogonal-tag-over-structural-types]]) — a kind
  version is a qualified identity, and two versions of a kind are distinct schemas the fold may handle
  differently.

**Consequences to hold.**
- **Open sums compose with monomorphization/erasure.** Like row-polymorphic records, an open sum is a
  compile-time typing device; the runtime representation is a tagged value, and the "open tail" is just
  the tags a given match does not name. No boundary change — the ABI's `variant` mapping
  (`options/type-mapping/`) already carries tagged unions.
- **The AST sum is closed; event kinds are open — deliberately different.** The `Ast` type is a fixed,
  closed sum ([[2026-07-04-macros-are-typed-and-hygienic]]) — the language's own syntax is not
  user-extensible at the variant level (no reader macros —
  [[2026-07-04-macro-phases-and-the-reader-stays-fixed]]). Event *kinds* are open because the target's
  vocabulary is data. Cadenza must support **both** a closed sum (Ast) and an open sum (event kinds) —
  which is why open-ness must be an explicit property of a sum type, not a global default.

**The requirements it drives.** `spec/capabilities/type-system.md` §"The Declarable Type Universe" gains
an **open sum / extensible variant** subsection (the dual of row-polymorphic records): a sum type MAY be
open, a match over an open sum is exhaustive via a mandatory open-tail arm, and open variants monomorphize
to tagged values before the boundary. `spec/capabilities/core-semantics.md` §"Matching Is Exhaustive Or
Rejected" is annotated that exhaustiveness over an open sum is satisfied by the open-tail arm (so
inert-to-unknown is a *checked* totality, not an unchecked fallthrough). A schema-typed-payload
requirement (in `type-system.md` or a collections/boundary spec) states that decoding an opaque payload
against a run-time-resolved schema yields a typed `Result`, not a trap. Composes with
[[2026-07-04-records-are-rows-open-by-default]] (the record dual), 
[[2026-07-04-fold-modules-are-provably-pure]], and
[[2026-07-04-host-is-value-agnostic-compiler-owns-reader-printer]].
