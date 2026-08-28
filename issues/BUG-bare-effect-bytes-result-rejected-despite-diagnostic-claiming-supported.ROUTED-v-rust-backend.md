# BUG (host-boundary path parity + misleading diagnostic): a Bytes RESULT on a bare (effect …) op is rejected, though the diagnostic itself lists `list<u8> (Bytes)` as a supported result form

**Status:** OPEN — routed to `v-rust-backend` (owns the host-boundary emit split). Found by breaker
tick 380 (probing 26-runtime-params; reproduces on a plain hand-written effect too, so it is NOT
@param-specific).

## The contradiction

`(effect H (op seed (-> Unit Bytes)))` + `(host (H) (Bytes.len (H.seed)))` is rejected:

> the host operation `seed` has a result of type `Bytes`, which has no component boundary form
> this compiler emits yet. Host RESULTS cross as: a scalar/unit, **a `list<u8>` (Bytes)**, or an
> `option<list<u8>>`. …

The message names `list<u8> (Bytes)` as a supported RESULT form in the SAME breath it rejects a
Bytes result. Two possibilities, both worth fixing:
1. Bytes results ARE emittable on the bare path and the guard is over-broad (a real capability
   regression) — OR
2. only the IMPOSED-WORLD path emits them and the bare path genuinely can't — in which case the
   diagnostic is misleading (it should say "declare an imposed `(wit-world …)` to cross a Bytes
   result").

## Evidence

- **Imposed-world path**: 28-wit-abi-boundary SHAPE-14/15 cross Bytes results e2e (run, not just
  compile) — so the emit EXISTS.
- **Bare-effect path**: a Bytes RESULT rejects (above), but a Bytes ARG compiles fine
  (`(effect hb (op h (-> Bytes Int64)))`, 04-capabilities:444). So the asymmetry is arg-ok /
  result-rejected on the bare path, while the message lists both arg and result Bytes as supported.

## Fix

Either lift the bare-path Bytes-result emit to match the imposed-world path (preferred — parity),
or correct the diagnostic to point at the imposed-world requirement. No corpus pin possible (the
shape declines); this issue is the tracked artifact. When resolved, a bare-effect Bytes-result
census cell + the @param Bytes-seed cell (26-runtime-params) become pinnable.
