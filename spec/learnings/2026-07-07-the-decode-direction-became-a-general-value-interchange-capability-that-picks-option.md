# The decode direction became a general value-interchange capability — and it picked Option, resolving the signature

*2026-07-07*

**What happened.** Last cycle the operator corrected the `Ast.decode` design — a decode consumes bytes that can
come from an external source, so it must be **total** (return a Result/Option), never trap — and left the
concrete signature (Option vs Result) as an open call, filed in ask-38. This cycle a sibling landed a whole new
capability spec, `spec/capabilities/value-interchange.md`, that resolves it normatively and generalizes it: it
defines **value interchange** — the surface by which *any* value serializes to bytes and decodes back — and its
§"Decode Inverts Serialize And Refuses Otherwise" states *"Decoding a byte sequence that is not the serialization
of any value of the expected type MUST yield the ABSENCE OF A VALUE rather than a value, consistent with the
language's fallible readers that yield an optional result rather than trapping."*

So the operator's one-off correction about `Ast.decode` became a general, normative principle over all values, and
it picked the **Option** side ("absence of a value" = `None`) — matching the existing `String.from-bytes : Bytes →
Option<String>` precedent. `Ast.decode` is now just one instance that must conform: `Bytes → Option<Ast>`.

Re-probing the seed against the new normative requirement: `Ast.decode` still returns a bare `Ast` and **traps**
on invalid bytes (a `match … ((Some a) …)((None _) …)` over it declines "match does not cover the scrutinee" —
decode is not Option-typed). So the spec moved but the seed has not: the total-decode requirement is unmet, on
both clauses (invalid bytes → should be `None`, still traps; trailing bytes → should be `None`, still silently
dropped — the trailing-bytes clause lives in `deterministic-value-form.md`).

**Why.** Two observations worth keeping.

*A one-off operator correction is worth promoting to a general capability, and this cycle showed the loop and the
spec authors doing exactly that.* The operator's guidance was phrased about `Ast.decode`, but the underlying
principle — a decode over untrusted input is total — is not AST-specific; it holds for any value crossing a
trust boundary (persisted, sent to another component, read across compiler generations). Landing it as a general
`value-interchange` capability rather than an `Ast.decode` footnote means every future serializable type inherits
the total-decode obligation, and `Ast.decode` becomes a conformance instance rather than a special case. This is
the right shape for a correction that names a boundary condition: generalize to the boundary, not the instance.

*The signature question resolved itself by matching the existing fallible-reader surface.* The spec chose Option
("absence of a value," "consistent with the language's fallible readers") over Result, and it did so by pointing
at the precedent (`String.from-bytes`, indexing-yields-Option, the whole fallible-reader family) rather than
inventing a decode-error type. That is the conservative resolution — a new fallible operation should wear the
language's existing fallible surface unless it has a reason to carry error detail — and it means the migration is
mechanical (the 9 existing `Ast.decode` round-trip corpus cases become `Some`/match forms).

**The requirement it drove.** No corpus case landed (the error-case cases stay withheld until the seed makes
`Ast.decode` Option-returning — adding them now fails the gate, since decode isn't Option-typed yet). ask-38 is
updated with the resolved signature (`Bytes → Option<Ast>`, per value-interchange.md) and the two unmet clauses
(invalid → `None`, trailing → `None`), plus the ready-to-land corpus once the seed conforms: the error cases
become ordinary VALUE cases (`(match (Ast.decode <garbage>) ((Some _) 1) ((None _) 0))` → 0), no trap oracle. The
seed fix is a signature change (`Ast.decode : Bytes → Option<Ast>`) plus the EOF check for trailing bytes, and a
migration of the round-trip corpus to `Some`. General lesson: **when an operator correction names a boundary
condition (here, "decode of external bytes must not fail hard"), the durable form is a general capability at that
boundary, not a patch to the one operation that surfaced it — and a new fallible operation should adopt the
language's existing fallible surface (Option, the fallible-reader family) rather than a bespoke error channel,
unless it needs the detail.**
