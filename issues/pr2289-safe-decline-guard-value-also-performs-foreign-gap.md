# PR #2289 review — rcdzc/src/effects.rs (v-effects) — OPEN — safe-decline coverage gap [VERIFIED-plausible, MED]

https://github.com/camshaft/cadenza/pull/2289 (safe-decline the outer-perform-in-next-state-slot silent
miscompile — as2/as1; the arc I've been tracking, ties to
[[as-family-outer-perform-in-next-state-slot-eval-order-spec-question]]). Copilot 1 inline, comment id
3723670680 at effects.rs:4819.

## the safe-decline guard fires only when the foreign perform is EXCLUSIVE to `next_state` (`next_state has foreign && value does NOT`), but a direct foreign perform in `next_state` is unsound REGARDLESS of whether `value` also performs one → `(resume (A.get) (A.get))` / `(resume (+ t (A.get)) (+ t (A.get)))` still leaks the silent miscompile (Copilot, effects.rs:4819) — coverage-gap [VERIFIED-plausible, MED]
> The safe-decline guard only triggers when the foreign perform is *exclusive* to `orig_next_state`.
> However, a direct foreign perform in `next_state` is unsound regardless of whether the `value` slot also
> performs a foreign op (e.g. `(resume (+ t (A.get)) (+ t (A.get)))` / `(resume (A.get) (A.get))`): the
> threaded state is still an unevaluated expression, so the foreign perform in `next_state` can still be
> dropped or duplicated. This check should decline whenever `orig_next_state` directly performs a foreign op.

VERIFIED the guard shape against the #2289 diff (effects.rs, `thread_bounded` tail-resume arm):
```rust
if let Some((orig_value, orig_next_state)) = peel_resume_from_arm_body(db, arm.body)
    && next_state_directly_performs_foreign(db, orig_next_state, ctx)
    && !next_state_directly_performs_foreign(db, orig_value, ctx)
{ return None; }
```
So it declines iff `next_state` has a DIRECT foreign perform AND `value` does NOT.

Checking the author's own rationale for the `!...(orig_value)` clause: the doc says the proven-correct forms
MUST stay folding — as3 = `(resume (+ t (A.get)) t)` (foreign in the VALUE slot, next_state clean) and as7 =
the let-lift. But as3 has NO foreign in next_state, so the FIRST clause
(`next_state_directly_performs_foreign(orig_next_state)`) already excludes as3 — the second `!...(orig_value)`
clause is NOT what protects as3. What the second clause actually excludes is the BOTH-perform case: a foreign
in next_state AND a foreign in value. Copilot's point is that those cases still have the unsound next_state
foreign (dropped/duplicated), so excluding them from the decline lets exactly the class the PR targets slip
through when the value happens to also perform.

MED / coverage-gap on a silent-miscompile guard (the whole point of the decline is to catch the dropped/dup'd
next-state foreign; a value-slot foreign doesn't make the next-state one sound). Whether
`(resume (A.get) (A.get))` actually miscompiles under fold vs. is caught elsewhere is subtle eval-order
semantics — I relay it as PLAUSIBLE; v-effects owns the call. Fix per Copilot: drop the
`&& !next_state_directly_performs_foreign(db, orig_value, ctx)` clause so the guard declines whenever
`orig_next_state` directly performs a foreign op. If v-effects KEEPS the clause, worth an as-family control
pin for `(resume (A.get) (A.get))` documenting why the both-perform case is sound (or a decline pin if it
isn't). v-effects owns rcdzc effects. PR OPEN → foldable pre-merge.
