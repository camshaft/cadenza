# PR #2147 review — rcdzc/src/effects.rs (v-effects) — OPEN — 1 correctness [PLAUSIBLE-HIGH] + 1 doc [VERIFIED LOW]

https://github.com/camshaft/cadenza/pull/2147 (FIX ao10 — inner abort drops a PERFORMING-CONDITION +
branch outer-advance 110→111; Site-5 #cv-lift for a performing cond, gated by refold-servability). Copilot
2 inline. NOTE: ao10 is a class I've tracked (abort-outer-advance) — the fix adds a servability gate so the
5b #cv-lift stands down where the refold serves.

## `body_served_by_oneshot_refold` omits the `count_resumes == 1 || !body_reaches_foreign_perform(...)` guard that the REAL refold gate (`reduce_handle`, effects.rs:2457) carries → the shared servability predicate is LOOSER than the refold it mirrors, so it can return true where the refold actually DECLINES, standing the Site-5 5b `#cv`-lift down incorrectly (Copilot, effects.rs:3529) — correctness [PLAUSIBLE-HIGH, verify in-context]
> `body_served_by_oneshot_refold` is intended to match the E5 two-hole refold gate in `reduce_handle`,
> but it currently omits the multi-shot/foreign-perform guard (`count_resumes == 1 ||
> !body_reaches_foreign_perform(...)`). As written, it can return true in cases where the refold will
> actually decline, causing the Site-5 5b `#cv`-lift to stand down incorrectly.

VERIFIED the ASYMMETRY (not the end-to-end miscompile — that's deep effects semantics): the new helper
(#2147 diff:34-38) returns:
  `!ctx.abortive.contains(&(decl,idx))
   && !is_tail_resumptive_arm(db, arm.body)
   && peel_resume_from_arm_body(db, arm.body).is_none()
   && !arm_partially_resumes(db, arm.body)`
— 4 conjuncts. But the ACTUAL refold gate in `reduce_handle` (effects.rs:2457, on trunk) is:
  `... && (count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db, body, &ctx))`
i.e. the refold ALSO requires one-shot-OR-no-foreign-perform. The helper's own doc says it "Mirrors the
two-hole refold's gate" — but it drops that conjunct. So `body_served_by_oneshot_refold` is STRICTLY LOOSER
than the refold: for a multi-shot arm that reaches a foreign perform, the helper returns true (says "refold
serves it, stand 5b down") while the real refold at 2457 would DECLINE (its `count_resumes==1 ||
!foreign` fails). Net: the ao10 branch-advance-preservation `#cv`-lift stands down, AND the refold declines
→ NEITHER transform runs → the exact lost-advance the ao10 fix exists to prevent, for that shape.
CONFIDENCE: PLAUSIBLE-HIGH. The asymmetry is real and verified against source; whether a multi-shot +
foreign-perform arm can actually reach this Site-5 5b path (vs being excluded upstream) is the in-context
question only v-effects can settle — but the helper claiming to "mirror" a gate while dropping a conjunct
is a real bug-shape, and the failure mode (both transforms decline) is precisely the ao10 regression.
Fix per Copilot: add the same `(count_resumes(db, arm.body) == 1 || !body_reaches_foreign_perform(db,
body, ctx))` conjunct to `body_served_by_oneshot_refold` so the shared predicate is EXACTLY the refold's
gate — otherwise "shared servability predicate so the two never contend" isn't true (they diverge on
multi-shot/foreign). v-effects: worth an ao10-adjacent test with a multi-shot performing-condition arm.

## the pre-existing `///` block documenting `hoist_resumptive_conditional` now sits directly above the newly-inserted `body_served_by_oneshot_refold` (no blank line) → rustdoc merges both `///` blocks onto the new helper, orphaning `hoist_resumptive_conditional`'s doc (Copilot, effects.rs:3499 & :3531) — doc [VERIFIED, LOW]
> These `///` rustdoc lines describe `hoist_resumptive_conditional`, but after inserting
> `body_served_by_oneshot_refold` they now attach to the helper instead (so rustdoc for both items
> becomes misleading). Convert this block to non-doc comments (or move it) so it no longer documents the
> wrong function.
VERIFIED in the diff: the old `///` block (ends "…`None`-free (returns the rewritten tree…)", diff:6-8)
is immediately followed by the NEW helper's own `///` block (diff:9-20) with NO blank line, then `fn
body_served_by_oneshot_refold` (diff:21), then `fn hoist_resumptive_conditional` (diff:40). Consecutive
`///` blocks with no separator concatenate into ONE doc comment on the NEXT item — so both blocks now
document `body_served_by_oneshot_refold`, and `hoist_resumptive_conditional` loses its rustdoc. LOW/doc.
Fix: separate them — the OLD block belongs on `hoist_resumptive_conditional` (move it down to sit directly
above that fn), leaving only the new helper's block on the new helper.

Both foldable pre-merge. The omitted-guard one is the one that matters (correctness on the ao10 fix
itself). v-effects owns rcdzc effects. Copilot bot is reliable; the guard asymmetry is source-verified.
