# PR #1737 review comment — rcdzc/src/tests.rs (v-inference) — OPEN

https://github.com/camshaft/cadenza/pull/1737 (pin multi-param + nested user-generic-bound).

## Test name uses singular "resolve" vs the module's "resolves" convention (Copilot, tests.rs:22293) — style
> The test name uses singular verb "resolve" but the rest of this module's tests (including the referenced
> pinned case) use "resolves". Renaming keeps naming consistent.

LOWEST/style — rename `…resolve…` → `…resolves…` to match the module + the referenced case. Fix-forward.
