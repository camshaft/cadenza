# PR#758 review comment — effects.rs param_apply_extra_handled `depth` comment says "callers pass 0" but external caller passes active depth

Mirrored from GitHub PR review comment (Copilot), id `3626207998`.
PR: https://github.com/camshaft/cadenza/pull/758 (merged; fix still belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/effects.rs:1159`

## Comment (verbatim)

> The new `depth` parameter comment says "External callers pass 0", but the current call site passes
> the active `check_no_home_walk` recursion `depth`. Either start this budget at 0 from callers, or
> update the comment to match the actual usage so future edits don't assume a different contract.

## Liaison verification (CONFIRMED on trunk)

- `param_apply_extra_handled` (effects.rs:1146) has a `depth: u32` param (1159) whose comment
  (1157-1158) ends: "External callers pass 0."
- Call sites (grep):
  - 1206 — INTERNAL recursive follow: `param_apply_extra_handled(db, head, sub_body, args.len(), depth + 1)` (fine).
  - 1377 — the EXTERNAL entry (inside `check_no_home_walk`, effects.rs:1302):
    `param_apply_extra_handled(db, head, callee_body, args.len(), depth)` — passes the ACTIVE
    `check_no_home_walk` recursion `depth`, NOT `0`.

So the "External callers pass 0" contract is violated by the sole external caller. Impact is benign in
practice (seeding the inter-procedural budget at the caller's current depth just makes the `depth < 32`
follow-gate trip EARLIER — more conservative, still terminating, still sound), but the comment is
misleading for future edits.

Fix (per Copilot, either): pass `0` at the 1377 call site (start the inter-procedural budget fresh),
OR update the comment to "seeded from the caller's current walk depth" to match actual usage. Prefer
whichever matches the intended budget semantics — if the 32-follow budget is meant to be independent of
the check_no_home_walk depth, pass 0; if it's meant to share the stack-depth budget, fix the comment.

Doc/contract-clarity, no correctness bug. Owner: v-effects (`rcdzc/src/effects.rs`). Routed as a note.
