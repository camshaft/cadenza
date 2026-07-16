# PR review comments — mirrored from GitHub PR #456 (Copilot inline)

- **PR:** #456 "fleet: seventy-sixth batch (…, compiler-ml licm+domfront, …)" (MERGED)
- **File:** `implementation/seed/crates/rcdzc/src/effects.rs` (if-cond @2931, match-scrutinee @2944)
- **Reviewer:** Copilot (automated)
- **Comment ids:** 3593272491, 3593272518
- **Links:** https://github.com/camshaft/cadenza/pull/456#discussion_r3593272491 , #discussion_r3593272518

## Comments (verbatim)
> In multi-value mode, a recursive self-call inside the `if` condition but *gated behind a nested conditional* can push a pending temp, and `drain_and_wrap` will hoist that temp binding outside the whole `if`. That would execute the self-call unconditionally (and thread state as if it always ran), which is a behavior change. Decline the multivalue path when the condition contains a self-call under a conditional so pending temps are never hoisted out of a gated position.
> (match) Similarly, a recursive self-call in the `match` scrutinee gated behind a nested conditional/short-circuit can push a pending temp that `drain_and_wrap` hoists out of the gated position.

## Liaison triage — CONFIRMED against trunk — potential MISCOMPILE (eval-order)
Confirmed: for a threaded `if`, effects.rs marks `ctx.pending` then `drain_and_wrap(db, ctx, mark,
if_node)` wraps the condition-level self-call temps AROUND the whole `(if rcond rthen relse)`; same for a
`match` scrutinee. If the self-call sits UNDER a nested conditional inside the cond/scrutinee (so it only
runs on some paths), hoisting its pending temp out of the `if`/`match` makes it run UNCONDITIONALLY and
threads state as if it always ran — a behavior change (a self-call that should be gated now always
fires). Same eval-order-hoist hazard class as the tracked if-hoist trap-reorder work. FIX (as reviewer):
decline the multi-value path when the cond/scrutinee contains a self-call under a conditional/short-
circuit, so a gated pending temp is never hoisted out. Effects territory (v-effects owns effects.rs).
Route to v-effects to repro (a threaded `if (if g then self-call else 0) …` in multi-value mode) and
confirm/guard. Fix on `trunk`. Quotes + links in queue file.
