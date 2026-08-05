# PR #2290 review — cdz-kernel/src/wasm_host.rs (v-agent-harness) — OPEN — panic-safety [VERIFIED, LOW-MED] + fixes my #2285 c1/c2

https://github.com/camshaft/cadenza/pull/2290 (async handle-ABI kv — serve a handle-lowered reducer's bound
`cadenza:agent-kernel/kv` get/put; closes the genesis `Kv.put` no-op — the §4b work my #2285 c2 flagged the
placeholder for). Copilot 1 inline (id 3723778037, wasm_host.rs:2932, +lines 2960, 2975).

## the bound-kv host closures index `params[0]`/`params[1]` BEFORE validating `params.len()` → a wasmtime arity mismatch (signature mismatch during linking) PANICS on OOB index instead of returning a structured trap/error (Copilot, wasm_host.rs:2932/2960/2975) — panic-safety [VERIFIED, LOW-MED]
> `params[0]`/`params[1]` are indexed before validating `params.len()`. If wasmtime ever invokes this host
> func with an unexpected arity (e.g., due to a signature mismatch during linking), this will panic instead
> of returning a structured trap/error. This issue also appears at line 2960 and 2975.

VERIFIED against the #2290 diff: the `kv.put` closure does `let (Val::U32(key_h), Val::U32(val_h)) =
(&params[0], &params[1]) else { …structured err… }` and the `kv.get` closure does `let Val::U32(key_h) =
&params[0] else { … }`. The `let-else` guards the TYPE (Val::U32) but NOT the arity — `params[0]`/`params[1]`
are indexed first, so a call with `params.len() < 2` (put) or `< 1` (get) panics with an index-out-of-bounds
BEFORE the else arm can produce the structured `kv_err`/trap. LOW-MED / panic-safety.

Reachability caveat (relayed as such): whether wasmtime can actually invoke a host func with an arity that
disagrees with its declared WIT signature is the OWNER's call — the linker binds these to the kv interface's
typed signature, so in practice arity should match. But defense-in-depth on a host-boundary closure that a
guest reducer drives re-entrantly is cheap and worthwhile. Fix per Copilot: check `params.len()` up front
(e.g. `let [key_h, val_h] = params else { return structured kv_err("put", "expected 2 args") }`, or an
explicit `if params.len() != 2` guard) at all three sites (2932/2960/2975) so a bad arity returns a trap, not
a panic.

## ALSO — #2290 FIXES my two #2285 fix-forward doc nits (verify-on-land when #2290 merges):
- c1 (id 3723378864): heap_marshal.rs stale `apply_handle_lowered_async` → this PR corrects the doc-link to
  `apply_handle_lowered` (heap_marshal.rs diff line ~13).
- c2 (id 3723378900): the "its Kv.put) no-op'd" test comment (wasm_host.rs) is REWRITTEN — the fixture imports
  only runtime heap, KV propagation now lands with this stage-2 kv work + its own kv-asserting test.
Both marked resolved when #2290 lands on trunk (grep trunk HEAD, not this open diff).

v-agent-harness owns cdz-kernel/src. PR OPEN → the arity guard is foldable pre-merge.
