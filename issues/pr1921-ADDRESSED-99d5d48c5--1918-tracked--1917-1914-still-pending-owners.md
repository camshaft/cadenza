# PRs #1921 / #1918 / #1917 / #1914 review comments — LOW

## PR #1921 (cdz-agent-host, v-agent-harness-host) — 2 LOW
- policy_swap_e2e.rs:56 — comment says the executor serves "Now+Model" but the test only registers the NOW
  executor; align comment (or register Model). test/doc.
- host.rs:165 — rustdoc has awkward suffixes ("set_authorizer)s", "push_capabilities_changed)es") from a
  link-macro; fix the rendering. grammar/doc.

## PR #1918 (spec/19-sets.sexp:956, breaker) — doc/accuracy [VERIFY]
The doc says "Set has no `Set.empty`", but the spec treats `Set.empty` as part of the Set surface. Verify:
if Set.empty IS in the surface, the doc is wrong; if the case means something narrower (e.g. no LITERAL
empty-set syntax), reword precisely. LOW/doc.

## PR #1917 (rcdzc/src/effects.rs:3437, v-effects) — doc/accuracy
The doc says the guard targets only a *block-wrapped* branch-performing conditional, but the impl [per
Copilot] may match a non-block-wrapped shape too. Verify guard vs doc (same effects.rs region as the #1907
fix). LOW/doc.

## PR #1914 (cdz-kernel/src/name_store.rs:127, v-agent-harness) — cleanliness [dup of #1876]
POLICY_CURRENT (or the compiler-pointer const) documented "ONE source of truth" but raw string literals
still used at call sites — same as the #1876 nit; replace literals with the const. LOW/cleanliness.
