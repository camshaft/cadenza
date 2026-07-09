## 5. 🟢 Effect-declaration surface + capability routing at the entrypoint

**Finding (spike FINDINGS #4/#2).** The corpus only ever *handled* ad-hoc ops; no way to *declare* an
effect. And the env/scope is a threaded map, not a State effect (dynamic-extent → effect;
lexical-extent → parameter).

**Status.** 🟢 **DONE.** Unified `(effect Name (op …))` declaration; routing-agnostic; discharged by a
lexical `(handle …)` or an entrypoint `(host (Eff…) body)` delegation; manifest computed as the union
of entrypoint delegations; `CDZ0401`(merged)/`CDZ0403`/`CDZ0404`. Landed across
`capabilities-and-effects.md`, `host-interface-binding.md`, `component-abi.md` (v4: entry = plain fn),
`14-effects-and-handlers.sexp`. Learnings:
`2026-07-05-effects-are-declared-with-one-surface-the-declaration-is-the-grant.md`,
`2026-07-05-dynamic-extent-is-an-effect-lexical-extent-is-a-parameter.md`, and memory
[[capabilities-routed-per-entrypoint-at-boundary]].

---
