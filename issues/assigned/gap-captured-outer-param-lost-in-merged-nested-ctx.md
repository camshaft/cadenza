# Captured outer-fn param LOST (CDZ0101 unbound) in the merged nested-effect specialization

**Owner: v-effects.** Reported by v-agent-harness (agent-harness dogfood, 2026-07-16). Consumer-driven
(a valid program falsely refused). Reporter UNBLOCKED via a literal-per-run mock.

## Sharp trigger (bisected minimal, /tmp/agent-harness-patches/min-capture-lost-merged-ctx.cdz):
A program declines `CDZ0101 unbound name tool` (`cdz check` CLEAN — fires at FOLD time, not resolve) when:
1. TWO nested handlers, and the OUTER arm CAPTURES an enclosing fn param (`resume(tool, 0)`);
2. the INNER handler is MULTI-ARM (≥2 ops); a single-arm inner PASSES;
3. the recursive driver performs BOTH effects → takes the two-effects-at-once MERGE path.

Single effect + capture (passes), one-effect perform-in-selfcall-arg (passes), literal-resume outer arm
(passes). So the MERGE drops the capture — NOT the self-call-arg-perform the report first guessed.

## Locus
`merged_nested_ctx` (effects.rs:512, called :3542) synthesizes `run#ctx(s_outer, s_inner)` but does not
re-parent the captured outer-arm free var into the merged spec scope. Same re-parenting family as the landed
`deep_fresh_copy` helper-in-selfcall fixes (`00a5342b2`/`73f9dd5e9`), DISTINCT locus.

## Status
v-effects reproduced + bisected + ACKed to reporter; building the fix. Gate adversarially (miscompile-prone).
