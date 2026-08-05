# PR #2060 review — cadenza-ast/src/codec.rs (v-syntax) — OPEN — test-precision [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/2060 (pin the Bytes-leaf codec wire — empty + high-byte). Copilot
(id 3714119223) flags the "leaves survive" assertion checks the pre-encode arena, not the decoded one.

## `a_bytes_leaf_round_trips…`: "Three DISTINCT Bytes leaves survive" asserts `a.leaves.len()` (the INPUT arena), not `back` (the DECODED arena) — doesn't test survival through the codec (Copilot, codec.rs:684) — test-precision [VERIFIED]
> The "Three DISTINCT Bytes leaves survive" assertion currently checks `a.leaves.len()`, which only
> validates how the test input was built (pre-encode) and doesn't actually assert anything about the
> decoded/canonical arena. If the codec were to drop a Bytes leaf (or otherwise alter leaf-pool
> cardinality), this check wouldn't catch it. Prefer asserting on `back` (or both `a` and `back`).

VERIFIED in the #2060 diff: `let a = b.finish(root);` (the INPUT arena), `let back = decode(&bytes)…;` (the
DECODED arena), then `assert_eq!(a.leaves.len(), 3, "three distinct Bytes leaves")` — asserts on `a`, the
pre-encode input. The test DOES round-trip + assert re-encode byte-identity (good), but the specific
"three distinct leaves SURVIVE" claim is about the codec preserving the leaf pool — and checking `a`
(which the test just built with 3 leaves) is tautological w.r.t. survival: a codec that dropped/merged a
Bytes leaf would change `back.leaves.len()` but leave `a.leaves.len() == 3`, so the check stays green. LOW/
test-precision — the byte-identity assert catches gross corruption, but the leaf-cardinality claim
specifically should assert the DECODED side. Fix per Copilot: `assert_eq!(back.leaves.len(), 3, …)` (or
assert both `a` and `back` for symmetry — pinning that neither the build nor the round-trip changed the
count). v-syntax owns cadenza-ast. PR OPEN → foldable.
