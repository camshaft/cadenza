# PR#810 review comment — s2_arg_ok function-level doc stale: says String/Bytes DEFERRED but the arm now admits them

Mirrored from GitHub PR review comment (Copilot), id `3634769280`.
PR: https://github.com/camshaft/cadenza/pull/810 (merged; fix belongs on trunk)
Location: `implementation/seed/crates/rcdzc/src/backend/rust/mod.rs:86`

## Comment (verbatim)

> `s2_arg_ok` now admits `Ty::String | Ty::Bytes`, but the function-level doc comment above still says
> String/Bytes args are deferred. This is now misleading for readers trying to understand which
> host-closure shapes are supported; please update the doc comment to match current behavior.

## Liaison verification (CONFIRMED on trunk)

- The FUNCTION-level doc (mod.rs ~72-77, above `fn s2_arg_ok`) still reads: "A String/Bytes ARG (the
  harness rebuilds a String literal but the closure-side ABI differs) … stays DEFERRED — … those
  cases stay a clean `todo`."
- But the body now has `Ty::String | Ty::Bytes => true` (mod.rs:87, added `db3926605` "admit a
  String/Bytes closure ARG applied in-guest…"). So String/Bytes ARE now admitted.
- Note: the ARM's OWN inline comment (mod.rs ~82-86) is ACCURATE — it correctly explains the nuance
  (an in-guest-APPLIED String/Bytes is fine; a String PASSED FROM THE HOST at the boundary is still
  deferred). So only the FUNCTION-level doc paragraph is stale, not the arm comment.

Fix: update the function-level doc to say String/Bytes args applied in-guest are now admitted (with the
host-boundary-String still-deferred caveat the arm comment already states) — align the summary with the
arm. Doc-only, no behavior change. Owner: v-rust-backend (`backend/rust/mod.rs`; commit `db3926605`).
Routed as a note.
