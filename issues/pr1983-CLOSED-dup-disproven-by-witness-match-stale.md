# PR #1983 review — rcdzc/src/effects.rs (v-effects) — MERGED — 1 correctness (foreign-effect dup) + 1 likely-stale [VERIFIED]

https://github.com/camshaft/cadenza/pull/1983 (FIX the recursive-branch-perform state-drop — a HIGH
self-probed miscompile). Copilot 2 inline. One is a VERIFIED correctness gap present on trunk; the other
appears STALE (the reviewed helper/doc isn't on merged trunk).

## the abort-hoist's `pure` guards use ctx-only `subtree_performs`, so a duplicated subtree performing a FOREIGN effect (handled by an OUTER handler) is treated as pure and its effect fires twice (Copilot, effects.rs:4802→trunk 3215/3300) — correctness [VERIFIED-PLAUSIBLE]
> `cond_pure` is currently defined as `!subtree_performs(cond)` (plus `#cv` checks), but
> `subtree_performs` only detects performs of the effect being discharged by this handler. If the
> condition performs a *foreign/host* effect (e.g. an effect handled by an outer handler), the merged
> per-slot state selectors duplicate `cond` evaluation and can duplicate that foreign effect, changing
> program behavior. The merge should be gated on the condition being effect-free in the broader sense (no
> foreign performs either), or rewritten to ensure the condition is evaluated exactly once and shared.

VERIFIED the mechanism on trunk. Both abort-hoist sites gate on `!subtree_performs(db, x, ctx)`:
- effects.rs:3215 (`others_pure` for sibling operands + `cond_pure`) — distributes an operand into BOTH
  `if` branches via `rebuild`, duplicating the OTHER operands + head into each branch.
- effects.rs:3300 (`preceding_pure` + `cond_pure`) — the let-init variant, duplicating preceding inits
  into both branches.
`subtree_performs` → `is_perform(head, ctx)` keys on `ctx.arms` = ONLY this handler's discharged op (I
verified this scope on a prior tick). So a sibling/preceding subtree (or `cond`) that performs a FOREIGN
effect — one discharged by an OUTER handler, or a host effect — returns `subtree_performs == false`, is
judged "pure", and gets DUPLICATED into both branches. If only one branch runs, a foreign effect that was
meant to fire once now fires once-per-taken-branch (fine for 1 branch, but the SIBLING copy in the untaken
branch is dead) — the real hazard is when the duplicated subtree is EVALUATED in both (e.g. the operand is
in strict position in both rebuilt `op` calls): the foreign perform then executes twice. The comment says
"duplicating it is fine only if pure" — but "pure" here means pure-w.r.t.-THIS-handler, not
effect-free-overall. MED (HIGH-class fix, subtle): correctness under nested/foreign handlers. Fix per
Copilot: broaden the guard to "no performs of ANY effect" (a ctx-agnostic perform scan) for the duplicated
positions, OR restructure so the duplicated subtree is evaluated once + shared (bind it above the `if`).
v-effects should confirm reachability with a witness: an abort-hoist candidate whose sibling/cond performs
an OUTER-handled effect → check it doesn't double-fire.

## `thread_branch_local_abort_with_out` / Match-arm doc mismatch — appears STALE (not on merged trunk) (Copilot, effects.rs:4377) — VERIFY, likely already-resolved
> The doc for `thread_branch_local_abort_with_out` says the `If`/`Match` arms use this helper to merge
> per-branch out-states, but only the `If` arm was updated to call it; the `Match` arm still calls
> `thread_branch_local_abort` and discards per-arm out-state. Either implement for `Match` or fix the doc.

LIKELY STALE: on MERGED trunk there is NO `thread_branch_local_abort_with_out` fn (grep: 0 hits). BOTH the
`If` arm (effects.rs:4742-4743) and the `Match` arm use the plain `thread_branch_local_abort`, and the
Match arm carries a comment explaining the choice: "The out-state is the post-scrutinee state (the
single-return shape does not observe a per-arm out-state)." So the reviewed intermediate (a `_with_out`
helper + an If-only update) appears to have been reworked before merge into a symmetric If/Match form with
an explanatory comment — the asymmetry + the overclaiming doc Copilot saw are gone. Relaying so v-effects
CONFIRMS the merged form is the intended one (If and Match symmetric on `thread_branch_local_abort`, no
`_with_out`, Match out-state deliberately post-scrutinee) — if so, no action; the finding was against a
pre-merge revision. LOW (verify-only).
