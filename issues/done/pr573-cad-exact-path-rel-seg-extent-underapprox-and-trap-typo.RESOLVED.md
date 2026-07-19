# pr573 — cad/exact.cdz: relative path segs under-approximate bounding extent + trap-msg typo (2 Copilot)

Mirrored from GitHub PR #573 review comments (Copilot).
PR: https://github.com/camshaft/cadenza/pull/573 (9-MR publish batch)
File: `implementation/cad/src/exact.cdz` — v-cad territory. Both VERIFIED against `git show trunk`.

## SUBSTANTIVE — id 3607817624 (exact.cdz:145) — *Rel path segs treated as absolute → extent under-approximates
> `path-seg-extent` treats `*Rel` segments as if their coordinates were absolute points (`ax(p)`), but
> the type docs say `*Rel` is relative to the current cursor. That makes `path-half-extent` (and thus
> `profile-half-extent`/`bounding-box` for `PathProfile`) potentially under-approximate extents for
> paths that include multiple relative segments (e.g. two `LineToRel(10,0)` steps reach x=20 but the
> current code only sees max |dx|=10).

VERIFIED: `path-seg-extent` (exact.cdz:145) applies the same `ax(p)` (component-wise abs of the point)
to `MoveToRel`/`LineToRel`/`CubicToRel` as to the `*Abs` variants. For a relative seg the Vec2 is a
DELTA from the running cursor, not an absolute point — so the fold in `path-extent-go` never
accumulates the cursor position and the max-abs is taken over deltas, not reached points. Two
`LineToRel(10,0)` reach x=20 but the code sees |dx|=10. Result: `bounding-box`/`profile-half-extent`
for a `PathProfile` built from relative segments UNDER-approximates the enclosing box — a real
correctness bug (a too-small bound is unsound, unlike the doc's intended "over-approximates safely").
Fix: thread the running cursor through `path-extent-go` and, for `*Rel` segs, accumulate cursor+delta
(and include the cursor's own reached point) before taking abs. (Note `CubicToRel` control points are
also relative → same treatment.)

## NIT — id 3607817639 (exact.cdz:376) — trap-message typo "tool" → "too"
> Typo in trap messages: "tool" should be "too" (these are asserting the matched constructor shape,
> not referencing a tool).

VERIFIED: traps read `trap("s2 tool is a Sphere")` / `trap("s1 tool is a Sphere")` in a `@test`; the
sibling traps read "s2 is a Difference". Should be "s2 too is a Sphere". Trivial; fold into the same
edit.

## Owner
v-cad (`implementation/cad/*`). The extent bug is the substantive one.

---
RESOLVED (corpus-bugfix 2026-07-19, verified on trunk 3578bb130): the SUBSTANTIVE bug (relative-path-seg
extent under-approximation) is FIXED in implementation/cad/src/exact.cdz. `path-extent-go` (line 170) now
THREADS the running cursor: `path-seg-cursor` advances it (Abs sets to the point, Rel adds the delta),
`path-seg-reach` resolves each segment's ABSOLUTE reached point (a cubic also folds its two control points),
and v2max folds the resolved absolute points — not raw deltas. Doc (160-165) cites the reviewer's exact
counterexample: "two LineToRel(10,0) reach x=20, not 10." So PathProfile bounding-box/profile-half-extent no
longer under-approximate. The trap-msg "typo" (tool→too) is not present on trunk (the "tool ... too" at 458-459
is intentional CSG subtract-tool wording, not the flagged typo). Owner (v-cad) resolved — no corpus-bugfix action.
