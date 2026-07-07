# A mid-flight signature change turns the gate red — the corpus must follow the seed, and the spec wording must be reconciled

*2026-07-07*

**What happened.** ask-38 (`Ast.decode` must be total, not trap) landed — and the standing gate check caught it
by going **RED**: 4 corpus cases failing ("contradict the recorded semantics"). The cause was a mid-flight
signature change: the seed changed `Ast.decode` from `Bytes → Ast` (bare) to `Bytes → Result<Ast, e>` (total —
`(Ok ast)` for a canonical encoding, `(Err reason)` for malformed or trailing bytes), but the 4 existing
round-trip corpus cases still asserted the bare form (`(= (Ast.decode (Ast.encode x)) x)` → true), which is now
`(= (Ok ast) x)` → false. The binary had rebuilt *during* the cycle (mtime moved under the probe again), so only
re-running against the current binary revealed decode was now Result-typed.

Restoring green was the priority (never leave the gate red), and it was the loop's job — the seed moved to the
correct behavior, so the corpus had to follow. Migrating the 4 cases to the `Result` surface hit a subtlety: the
natural `(match … ((Ok a) (= a x)) ((Err _) false))` form was itself REJECTED ("CDZ0201: comparison between
values of different types" on the leaf; "CDZ0401: undeclared capability: g" on the compound) — an explicit `Err`
arm tripped a seed limitation. The `(else …)` catch-all form worked: `(match (Ast.decode (Ast.encode x)) ((Ok a)
(= a x)) (else false))` → true. With the 4 migrated and 2 new error-case cases added (garbage → Err, trailing →
Err), the gate returned to green (569), and both ask-38 clauses are now pinned and met: invalid bytes → error
value (not trap), trailing bytes → error value (not silent drop).

**Why.** Three lessons, escalating in scope.

*A signature change is a gate event, and the corpus is downstream of the seed.* When the seed changes an
operation's type to the correct one, the corpus cases written against the old type are not wrong-the-seed, they
are stale-the-corpus — the loop migrates them. This is the inverse of the usual withheld-case flow (where the
corpus is ahead and waits for the seed): here the seed got ahead and the corpus had to catch up in the same
cycle to restore green. The standing gate check is what turns "a sibling changed decode" into "4 red cases I
must migrate now" — without it, a green-looking commit would have shipped a corpus that disagrees with the seed.

*Migrate to the shape the seed accepts, not the shape you'd write by hand.* The obvious `((Ok a) …) ((Err _)
…)` form was rejected by a seed limitation (an explicit `Err` arm mis-typed), and iterating on a red gate is
dangerous. Probing for a working shape first — `(else …)` — before editing the corpus is the safe path: find the
form the current seed accepts, then write the cases in it, then run the gate once. (The `Err`-arm limitation is
noted on the ask, not chased on the red gate.)

*The seed picked Result where the spec wording said Option — that divergence must be reconciled, not silently
accepted.* `value-interchange.md` says decode yields "the absence of a value" (Option-shaped, `None`); the seed
implemented `Result<Ast, e>` (carrying the reject reason). Both are total and satisfy "not trapping," and Result
is arguably richer — but the literal spec wording and the implementation now disagree on shape. The loop's job
is not to pick the winner (that's an operator call — bless Result in the spec, or return Option in the seed) but
to *surface the disagreement* rather than let a green gate paper over it: the gate passing means the corpus
agrees with the seed, NOT that the seed agrees with the spec. Flagged on ask-38 for the operator.

**The requirement it drove.** The 4 round-trip cases migrated to the `Result`/`Ok` form (gate green), and 2 new
cases pin the error clauses ask-38 required: *"decoding bytes that are not a canonical AST yields the error case,
not a trap"* (garbage → Err) and *"decoding canonical bytes followed by a trailing byte yields the error case"*
(valid++[99] → Err). ask-38 moved open → done with both clauses met, plus two flags for the operator: the
Option-vs-Result spec-wording reconciliation, and the minor `Err`-arm match limitation (a note, not a new ask,
since `(else …)` is a clean workaround). General lesson: **when the seed changes a signature to the correct
behavior, the corpus is downstream and must migrate in the same cycle to hold the gate green — but a green gate
after migration means the corpus matches the SEED, and a separate check (does the seed match the SPEC?) is still
owed when the implementation chose a shape the spec did not literally name.**
