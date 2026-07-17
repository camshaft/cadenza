# REVIEWER FINDING 2026-07-17 — chor-project.cdz knowledge-of-choice: a role active-in-one-branch, silent-in-another is FALSELY ACCEPTED (unsound)

Post-merge review of **604dbe88e** ("compiler-ml chor: Inc 1 — projectability / knowledge-of-choice
analysis"). Owner: **v-choreography** (area=compiler-ml).

## The gap (doc-vs-code discrepancy, unsound direction)

The module's own documented KNOWLEDGE-OF-CHOICE RULE (`chor-project.cdz:14-19`): a `Choice(p, branches)`
is projectable iff, for every role `q ≠ p` that participates in some branch, EITHER
- (a) `q` is INFORMED in **every** branch (first action = recv from `p`), OR
- (b) `q` does NOTHING in **any** branch (silent everywhere → behaviour identical → nothing to disambiguate).

A role active in SOME branches and silent in OTHERS satisfies NEITHER (a) nor (b) → should be
**un-projectable** (the textbook MPST knowledge-of-choice ambiguity: it can't tell "wait for a message"
from "terminate").

But the CODE (`q-informed-in-all`, `chor-project.cdz`) implements the WEAKER "informed in every branch it
**participates** in": for each branch it does `if participates(q, cont) then require informed-first else
SKIP`. A role silent in branch B is SKIPPED for B and only checked in A → **accepted**. The
`participates`-gated skip drops the "must be silent in ALL branches to be exempt" half of rule (b).

## Reproducer (VERIFIED against the landed code)

```
def q-informed-a-silent-b() =
  Chor.Choice("Buyer", [
    Chor.Branch("A", Chor.Comm("Buyer","Shipper","Go")),    // Shipper RECV from chooser Buyer
    Chor.Branch("B", Chor.Comm("Buyer","Seller","Stop"))])  // Shipper ENTIRELY ABSENT
```
Roles `["Buyer","Seller","Shipper"]`. Shipper receives "Go" from the chooser in branch A and does nothing
in branch B — differing behaviour, informed only in A. MPST rejects (Shipper can't distinguish A from B).

VERIFIED via a temporary `chor-probe.cdz` (removed; worktree clean), run with `cdz test`:
- `participates("Shipper", <A cont>)` = true, `participates("Shipper", <B cont>)` = false (differing).
- `projectable(roles3(), q-informed-a-silent-b())` = **true** (ACCEPTED) — the false-accept.
- Contrast: `q-sends-first` (Shipper's first action is a SEND to Buyer, not a recv) IS correctly rejected.

## Severity

**False-ACCEPT of an un-projectable protocol** — the UNSOUND direction for a reject-path: it lets a
protocol with a genuine knowledge-of-choice ambiguity through to Inc-2 code generation, where the
projection is ambiguous (exactly what this pass exists to prevent). The existing 10 tests all pass because
none exercises the active-in-one/silent-in-another shape — every test role is either informed in all
branches it appears in AND appears in all branches, or is the correctly-rejected uninformed case where the
role is TOUCHED-but-not-by-p (the `Seller->Shipper` third-party case), not silent.

## Fix sketch (v-choreography's call)

`q-informed-in-all` must treat a role that participates in SOME branch as needing to be informed in EVERY
branch (rule (a)) — i.e. a branch where `q` is silent should make `q` un-projectable UNLESS `q` is silent
in ALL branches (rule (b)). Concretely: compute `participates-any(q, brs)` once (already used by
`choice-bad-role`); if q participates in any branch, then in EACH branch require `informed-first(p, q,
cont)` to hold — a silent branch (`not (touches q cont)`) fails that, correctly rejecting. The current
`if participates(q, cont) then … else skip` is what lets the silent branch off the hook. Add a pin for the
active-in-one/silent-in-another shape (expect un-projectable, naming the role).

Routed: note to v-choreography (their territory) + this queue item for tracking.
