# PR #1219 review comment — rcdzc/src/tests.rs (v-memory-safety)

Mirrored from https://github.com/camshaft/cadenza/pull/1219 (PR: "cand: v-memory-safety — 01ce9c4fa").

## Leak-bound `live <= 16` vs a test documenting "exactly ONE live cell" (amazon-q, tests.rs:5203) — test-tightness, VERIFY intent
> Logic Error: The leak bound assertion `live <= 16` is too permissive for a test that documents an
> "exactly ONE live cell" leak. This allows 16x the expected leak to pass silently, defeating the
> purpose of detecting regressions. If new code introduces additional leaks (e.g., 2-15 cells), this
> test will incorrectly pass.
> suggested: `live > 0 && live <= 2`

Worth a look, but YOUR call on the exact bound — you know whether `<= 16` is deliberate slack. The
real point is the mismatch between the docstring ("exactly ONE live cell") and a bound that admits up
to 16: either the doc overstates the precision, or the bound is looser than the invariant you mean to
pin. If the count is genuinely deterministic here (not backend/allocator-sensitive like some reclaim
batteries), tightening toward the documented "1" (amazon-q suggests `<= 2`) would catch a 2–15 cell
regression the current bound silently passes. If 16 is intentional headroom, tighten the DOC instead
so it doesn't claim exactness the assertion doesn't enforce. Not asserting the `<= 2` value is right —
just flagging the doc-vs-assertion gap on a leak-regression test.

## 2. (later Copilot inline) Recursion-count comment wrong (Copilot, tests.rs:5182) — doc
> The comment says `main` "recurses once (`walk 1`)", but with the current condition `(>= n 0)` the
> call sequence for `n=1` is `walk 1 → walk 0 → walk -1` (two recursive calls before the base case).
> This makes the witness description misleading for future debugging.

Fold this into the same follow-up as the leak-bound tighten: correct the "recurses once" description
to the actual `walk 1 → walk 0 → walk -1` sequence. (A third Copilot inline at :5207 merely restates
the leak-bound point above — no separate action.)
