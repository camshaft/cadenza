# Bool offers a total order, with false less than true

*2026-07-05*

**What happened.** An adversarial-corpus `/loop` run was extending the ordering-operator cases in
`spec/semantics/07-type-system.sexp` — pinning that `<` `>` `<=` `>=` type-check their operands the
same way `=` and the arithmetic operators do. It reached the case `(< true false)` and could not
record a definite outcome. `#Ordering Where Offered Is Total` said only that *a type that offers an
ordering* must offer a total order; it never said **which** types offer one, so whether Bool is
ordered was undetermined. Recording either outcome — a `false`/`true` result *or* a `CDZ0201`
rejection — would have invented an unreviewed design decision, so the case was dropped rather than
guessed. The operator later confirmed the intended design: Bool is totally ordered with
`false < true`, matching the conventional ordering a boolean carries (as in the prior implementation
language, where `false < true`).

**Why.** `#Ordering Where Offered Is Total` was written as a *conditional* invariant — "where
offered, an order is total and deterministic" — without an accompanying enumeration of which
primitive types actually offer an order. Int64's order is exercised throughout the corpus, so its
offering was never in question; Bool's was never stated either way. The gap is a specification
under-determination: a total-order property with no ground clause fixing the base types it applies
to. The seed reflected the gap by declining a Bool comparison ("non-integer operand to integer op"),
which is the correct reject-don't-miscompile response to an unspecified operation — but a decline is
not a specification, and every future generation would face the same ambiguity.

**The requirement it drove.** Added one sentence to
[`core-semantics.md` §"Ordering Where Offered Is Total"](../capabilities/core-semantics.md):
"The Bool type MUST offer a total order in which false is less than true." This turns the
conditional total-order invariant into a definite obligation for Bool and fixes the direction of the
order. The requirement is witnessed by execution: conformance cases in
[`spec/semantics/03-equality-and-observation.sexp`](../semantics/03-equality-and-observation.sexp)
record `(< false true)` → true, `(< true false)` → false, and the `>`/`<=`/`>=` companions, so the
behavior gate discharges the class thereafter (`conformance-gate.md` §"A Behavior Requirement Is
Covered Only By Execution"). The seed currently declines Bool ordering (the cases score *todo* under
reject-don't-miscompile), so the gate stays green while the requirement marks the emission a future
generation must add.
