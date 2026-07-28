# PR#739 review comment — infer.rs width_fault_against_ty comment claims a `let` descent that doesn't exist

Mirrored from GitHub PR review comment (Copilot), id `3622941668`.
PR: https://github.com/camshaft/cadenza/pull/739 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/infer.rs:1111`

## Comment (verbatim)

> The comment says this Float32 overflow check is reached via runtime `if`/`match`/`let` descent,
> but `width_fault_against_ty` only has explicit descent for `if` and `match` (no `let` case here).
> This is misleading for future maintainers; either add the `let` descent that's being described, or
> update the comment to match the actual control flow.

## Liaison verification (CONFIRMED on trunk)

`fn width_fault_against_ty` (infer.rs:1070) has explicit descent arms only for:
- `Core::If` (~1085: `let Resolved::If { then_, else_, .. } = resolved_of(...)`)
- `Core::Match` (~1101: `let Resolved::Match { arms, .. } = resolved_of(...)`)

There is NO `Core::Let` arm. But the Float32-overflow comment (~1107-1112) says the check "fires
through the runtime `if`/`match`/`let` descent above". The `let` mention is inaccurate — corroborated
by the landing commits themselves, whose subjects only ever say "if/match" (`ffa733b67` "…in a runtime
if/match branch", `61eae7af1` "descends a runtime `match`", `1a037eaa2` "descends a runtime `if`").

Fix (per Copilot, pick one):
- Drop `let` from the comment (make it "if/match descent"), OR
- If a `let`-bound narrow annotation SHOULD be caught (e.g. `(: (let (x ...) 1.0e300) Float32)`), add
  the `Core::Let` descent arm + a corpus case — but that's a scope decision, likely a follow-up.

Recommend the doc fix now; if the `let` gap is real, file it as a separate check-vs-emit increment.
Doc-only (or small feature). Owner: v-inference (owns rcdzc infer/unify/resolve; commit `ffa733b67`).
Routed as a note.
