# PEER GAP: a String/Bytes ARGUMENT to a peer op is not emitted (invalid component) — the CRITICAL-PATH cell for Bedrock-as-peer

**Owner:** v-peer-linking. **Filed:** 2026-07-16 (v-agent-harness probed the full String-crossing matrix + corrected the earlier host-result framing).

## The corrected priority (v-agent-harness's e2e matrix probe, trunk e1506bd7c)
A String crosses NO peer/host boundary in a runnable model-call shape TODAY except a peer RESULT:
- ✅ peer op RESULT = String — WORKS e2e (byte-len read off a consumed result).
- 🔴 **peer op ARG = String/Bytes — the CRITICAL-PATH blocker for `(-> String String)`** (this file).
- 🔴 entrypoint's own result escaping a String → resource-escape decline (peerbug-list-at-…md / task #6).
- 🔴 host op ARG = non-const String → declines. host op RESULT = String → declines (gap-host-op-… / task #7).
- 🔴 entrypoint PARAM = String → declines ("String has no component boundary representation").

So the naive Bedrock peer binding `(effect Bedrock (op converse (-> String String)))` declines on the
PROMPT ARG (this cell) AND on returning the completion from `main` (task #6). Route B (Bedrock-as-peer)
needs BOTH the String-ARG emit (this) and the result-escape (task #6) for the full `(String -> String)`
shape; the ARG cell is the one v-agent-harness named critical-path.

## Root cause (isolated 2026-07-16 — probed with my PL24 decline temporarily disabled)
`extern_abi_val_type(String) = Some(U32)` (a peer String SHOULD cross as a runtime handle, like any
compound — the peer shares the value-heap runtime). But the ARGUMENT EMISSION lowers the source String
as a component-model `string` (a `canon lower` with a `mem` option) instead of building + passing a
runtime rope HANDLE. So the emitted consumer imports `("mem","mem")` that the peer envelope never
supplies → INVALID component ("missing module instantiation argument named `mem`"). Confirmed on the
isolated shape `(op len (-> String Int64))` + `(S.len "hello")` (Int64 result, so NO result-escape) →
invalid component. PL24 currently DECLINES this (report-don't-miscompile).

## The fix (a moderate emit build — the inbound-rope-handle emit)
For a PEER-BOUND op's String/Bytes argument, the consumer must:
1. Build a runtime String/Bytes HANDLE from the source rope (the value-heap `bytes-*`/`arr-alloc`
   construction the in-guest String path already uses) — so the consumer IMPORTS `cadenza:runtime/heap`
   (an arg-only-string peer consumer becomes a runtime-importing component; today only a compound arg/
   result triggers that — extend the trigger to a String/Bytes peer arg).
2. Pass the u32 HANDLE as the peer-call argument (an i32 on the stack), NOT a `(ptr,len)` component
   string. select.rs's peer `Core::HostCall` arg emit (the `CallExternImport` arg loop) must treat a
   String/Bytes arg like a compound (push the handle) rather than routing it through the string-lowering.
3. The peer op's component PARAM functype must be `u32` (the handle valtype), not `string` — mirror
   `extern_abi_val_type`→U32 in `extern_op_comp_functype` (mod.rs). (The RESULT direction already does
   this — a peer String result is a u32 handle; the arg is the same handle, inbound.)
4. Remove/relax PL24's decline (STRING_ARG_ACROSS_PEER_MESSAGE, compile.rs ~2392) once the emit works —
   keeping the decline only for the still-unemittable HOST String arg (non-const) if that stays separate.

SIZE: ~80-150 lines (arg-emit routing in select.rs + the functype + the runtime-import trigger). MODERATE
byte risk — the peer RESULT direction is a working template (a String result already crosses as a
handle). Byte-validate the consumer with `wasm-tools validate`.

## Sequencing note
For the FULL `(-> String String)` model call, BOTH this (arg) and task #6 (result-escape, since `main`
returns the peer's String) are needed. v-agent-harness ships Route B incrementally; this ARG cell is the
first unblock. I OWN it. Repros at /tmp/inc1a/ (consumer.sexp = full shape; argonly2.sexp = isolated arg).
Supersedes task #7 (host-result) as the Bedrock critical path — task #7 is the eventual in-Cadenza-SigV4 cleanup.

## REFINED ROOT CAUSE (2026-07-16, deep probe — the functype is ALREADY right; two OTHER layers are wrong)
Probed the emitted consumer for `argonly2.sexp` (String literal arg, Int64 result) with PL24 disabled:
- ✅ The peer op's COMPONENT functype is ALREADY correct: `(param "p0" u32) (result s64)` — a HANDLE, not
  a component `string`. So `HostParam::Scalar(U32)` (host.rs:320-326, peer-bound → extern_abi_val_type)
  + `extern_op_comp_functype` are RIGHT. My earlier "functype = string" guess was WRONG.
- 🔴 The emitted CORE module has a spurious `(import "mem" "mem" (memory 1))` AND a `canon lower` with a
  Memory option — but the peer op takes a u32, so NO `mem` is needed. The core instance is instantiated
  with only `"peer"` provided (not `"mem"`) → "missing module instantiation argument named `mem`" →
  INVALID. Source: `core_module_with_extern`/the extern envelope emits a memory import unconditionally
  when the body contains a String (the ConstStr rope-build touches memory ops), even though the peer
  boundary itself needs none. FIND + suppress the spurious mem import/option on the peer-only path.
- 🔴 The consumer takes `assemble_extern` (peer-ONLY, `imports.is_empty()` at mod.rs ~995) NOT
  `assemble_extern_runtime`. But `Core::ConstStr` emit (select.rs:5421) builds the rope via `bytes-alloc`
  /`bytes-set` — RUNTIME ops. So the consumer MUST import `cadenza:runtime/heap` (via
  `assemble_extern_runtime`) for the rope-build, which means the runtime-op collection (`imports`) must
  COUNT the ops the ConstStr arg emits. Today `imports` is empty for this shape → the rope-build's
  `bytes-alloc` import is also unsatisfied (a second invalidity behind the mem one).

## Corrected fix (THREE layers, ~100-180 lines — a multi-tick focused build, NOT a tick-end rush)
1. RUNTIME WIRING: collect the runtime ops a peer String/Bytes ARG's rope-build emits (bytes-alloc/
   bytes-set / str-from-bytes) into `imports`, so `!imports.is_empty()` routes to `assemble_extern_runtime`
   (peer + runtime) and the consumer imports `cadenza:runtime/heap`. The ConstStr → handle emit
   (select.rs:5421) already produces a u32 handle; it just needs its runtime ops counted for THIS shape.
2. SUPPRESS the spurious `mem`: the peer-op core import + `canon lower` must NOT carry a Memory option
   for a u32-handle param. Locate where `core_module_with_extern[_runtime]` / the envelope adds the mem
   import+option for a String-containing body and gate it off for the peer path (a peer handle never
   touches component-model linear memory — that's the HOST string path only).
3. RELAX PL24 (compile.rs ~2392): once (1)+(2) emit a valid component, drop the String/Bytes-peer-arg
   decline (keep it only if a residual unemittable sub-case remains).
Then the FULL `(-> String String)` also needs task #6 (result-escape). Test: `argonly2.sexp` (arg only,
scalar result) must VALIDATE + run first; then a both-sides-source round trip (peer reads the crossed
String, returns a scalar). Byte-validate each step with `wasm-tools validate`.
