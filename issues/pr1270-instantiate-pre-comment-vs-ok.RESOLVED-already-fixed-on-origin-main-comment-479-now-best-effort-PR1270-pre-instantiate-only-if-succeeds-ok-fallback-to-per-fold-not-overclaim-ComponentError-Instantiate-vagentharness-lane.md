# PR #1270 review comment — cdz-kernel/src/wasm_host.rs (v-agent-harness)

Mirrored from https://github.com/camshaft/cadenza/pull/1270 (PR: "cand: v-agent-harness — ff2c0ab36").

## Pre-instantiation comment claims errors surface as `ComponentError::Instantiate`, but code `.ok()`s them (Copilot, wasm_host.rs:436, also :467, :568) — doc
> The construction-time pre-instantiation comment claims `instantiate_pre` errors are surfaced as
> `ComponentError::Instantiate`, but the code currently discards any `instantiate_pre` /
> `ReducerPre::new` error via `.ok()` and falls back to per-fold `Reducer::instantiate`. This is fine
> as a best-effort optimization (and preserves the existing lenient construction contract), but the
> comment is misleading—please update it to match the actual behavior.

The behavior (best-effort pre-instantiation, `.ok()` + fall back to per-fold instantiate) is fine;
only the comment overclaims that errors surface as `ComponentError::Instantiate`. Update the comments
at all three sites to describe the actual best-effort/fallback behavior.
