# PR #1942 review — rcdzc/src/effects.rs (v-effects) — MERGED — precision/over-decline [VERIFIED-PLAUSIBLE, LIVE ON TRUNK]

https://github.com/camshaft/cadenza/pull/1942 — MERGED 2026-08-04T04:07:46Z (adv-69 c3-nested: the
scrutinee/statement scan now descends into a nested Handle's body keeping the outer ctx). Copilot (id
3709366737) flags that the UNCONDITIONAL descent can OVER-decline the same-effect re-handle (shadow) case.
NOTE: soundness-SAFE direction (over-decline = honest Todo, not a miscompile) — this is a
completeness/precision concern, the mirror image of the #1933 under-scan I filed.

## `body_has_block_wrapped_scrutinee_or_statement_branch_perform` descends into EVERY nested `Handle` body with the OUTER ctx; a same-effect inner re-handle shadows the outer for that op, so an inner-handled perform is misread as outer → spurious adv-69 decline (Copilot, effects.rs:3559) — precision [VERIFIED-PLAUSIBLE]
> `body_has_block_wrapped_scrutinee_or_statement_branch_perform` now always descends into a nested
> `Handle`'s body using the OUTER `ctx`. For re-entrant same-effect nesting (an inner `handle` that
> discharges the same effect as `ctx`), the inner body is *not* under the outer handler for that effect
> (it's the inside-out shadow case), but this scan would still treat inner-handled performs as "outer" and
> can spuriously force an adv-69 decline. Consider gating the descent so it skips nested handles whose
> discharged effect is already present in `ctx` (same-effect re-handle), while still descending for
> different-effect nested handles (the intended fix).

STRUCTURALLY VERIFIED. The new descent (effects.rs:3559-area diff) is
`if let Resolved::Handle { body, .. } = resolved_of(db, node) { return
body_has_block_wrapped_scrutinee_or_statement_branch_perform(db, body, ctx); }` — UNCONDITIONAL, keeps the
outer `ctx`, no discharged-op comparison against the inner handle's arms. The perform-match chain
(`block_wrapped_branch_performs` → `conditional_branch_performs` → `subtree_performs` →
`subtree_performs_uncached`) bottoms out at `is_perform(db, head, ctx)`, which matches on
`(decl-occ, op-index)` against `ctx.arms` (HandlerCtx.arms, keyed exactly by that pair). So for a
same-effect re-handle — an inner `(handle E … (handle E …))` where the inner discharges the SAME op — a
block-wrapped branch perform of E inside the inner body IS shadowed by the inner handler at runtime, but
`is_perform(head, outer_ctx)` still matches (same decl-occ/op-index) → the scan reports it as an outer
perform → `reduce_handle` declines a case the inner handler actually serves correctly.

Contrast the a4/let-init sibling I filed (#1933): there the risk was UNDER-scan (missing init); here it's
OVER-scan (declining a shadowed inner perform). Both stem from the same "descend into nested body with
outer ctx, ctx-keyed" design; the ctx-key guards against DIFFERENT-effect inner performs but NOT
same-op re-handle shadowing. Copilot's fix is right in shape: skip the descent (or mask the shadowed op in
ctx) when the nested handle discharges an op already in `ctx`, still descending for different-effect nests.

CAVEAT for v-effects: (a) soundness is not at risk — over-decline is an honest Todo, so this is
lower-severity than a miscompile; (b) reachability depends on whether a same-effect re-handle with a
block-wrapped branch perform of the shadowed op in the inner body is a shape the corpus/users hit — you'll
know if it's currently over-declining any working case (a green→decline flip in the flip battery) or is
purely theoretical. If it can over-decline a real working shape, gate the descent on
"inner discharged op ∉ ctx"; if unreachable, a comment noting the shadow assumption suffices. MED-LOW /
precision. v-effects owns effects.rs.
