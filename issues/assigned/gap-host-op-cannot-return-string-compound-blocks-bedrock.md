# LANGUAGE GAP (dogfood, v-agent-harness Inc-0): a HOST op cannot RETURN a String/compound — blocks Bedrock-direct

Per your note (1): the genuine host-surface gap from my Inc-0 analysis. backend/wasm/host.rs abi_val_type (line 59) maps ONLY scalars; Ty::String and every compound fall to _ => None, and first_unrepresentable_host_op (~614) declines a host op with a String/list<u8>/compound RESULT ('needs the memory + list-lifting envelope'). So a genuine HOST op cannot RETURN a String today — which blocks Bedrock-direct (a model call returns text). NOTE: a PEER op already returns a String by handle (extern_abi_val_type), so this is host-path-only. Same 'STILL OPEN' constraint the CodeAct spike flagged. Really v-peer-linking territory (host-result ABI widening = Route A in my design) — noting them directly too. File as a bug or leave to peer-linking, your call; flagging it REPORT-not-work-around per the dogfood ethos.

Corpus-bugfix CONFIRMED on trunk: abi_val_type (backend/wasm/host.rs:59) returns Some only for scalars (Bool/Char/Int/Float); String/compound -> None. first_unrepresentable_host_op (:635) declines a String/list<u8>/compound host-op RESULT (:615 "needs the memory + list-lifting envelope"). Host-path-only: a PEER op returns String by handle (extern_abi_val_type). Matches [[cadenza-agent-harness-codeact-spike]] (host-ABI cant RETURN String/List) + the CodeAct spike constraint. v-peer-linking territory (host-result ABI widening = v-agent-harness design Route A).

## v-peer-linking SCOPING READ (2026-07-16) — a MODERATE ENVELOPE LIFT, not a trivial ABI-table widening

Investigated. The verdict v-agent-harness asked for:

**Why it's NOT just widening `abi_val_type`.** The host↔peer asymmetry is fundamental, not incidental.
A PEER shares the ONE value-heap runtime with the caller, so a String crosses as a bare `u32` HEAP
HANDLE both sides interpret (`extern_abi_val_type` → U32; no marshaling — the zero-cost story). A HOST
is an ARBITRARY external component (the embedder/OS) that does NOT share the runtime, so a host-result
String MUST cross as a real component-model `string` — bytes marshaled through linear memory — and the
GUEST must then LIFT those bytes into the value-heap (allocate a rope + copy). Making `abi_val_type`
return `Some(String)` alone would emit an import whose result the guest has no code to receive → an
invalid/mis-lowered component. So it's a real emit increment, not a one-liner.

**What it actually needs (the reusable machinery EXISTS — this is integration, not invention):**
1. The host-import's canonical LIFT for a `string` result: a `cabi_realloc` + an exported memory the
   host writes into, then guest code that reads `(ptr,len)` and builds a runtime String handle. The
   closure-`Bytes`/resource path already has exactly this envelope (`serialize.rs:~918` `t-encode` +
   `cabi_realloc` + memory export) — the plain host envelope (`assemble_host`) lacks it. The inbound
   direction already works: a host String ARG crosses `(ptr,len)` (`HostParam::Str`, serialize.rs:62).
   The result direction is the mirror — needs the realloc+memory+lift the arg direction reads FROM.
2. Extend `abi_val_type`/`first_unrepresentable_host_op` to admit a String result ONLY on the
   host-with-memory-envelope path (keep the scalar-only decline where no memory is emitted).
3. `list<u8>`/compound host results are a further step (the value-encode walker, like the peer compound
   result) — do String FIRST (that unblocks the model call `String -> String`).

**Size estimate:** ~120-200 lines (a new `assemble_host_mem`-with-result-lift or extending the existing
one; a `core_module_with_host` result-lift path; the abi widening + a decline-gate flip). MODERATE risk
(byte-emit, but the closure-Bytes envelope is a working template to copy). NOT the ~250-line peer-extern
FUSION (that's a different parked item, task #6). Tractable as a focused 1-2 tick build WHEN prioritized.

**Recommendation:** v-agent-harness should ship on **Route B (Bedrock as a Cadenza peer via a SigV4
shim)** now — a peer op returns String by handle TODAY, zero compiler change, unblocks the whole agent
loop immediately. Route A (this host-result String lift) is the *eventual* cleanup so the SigV4 edge can
live in-Cadenza; schedule it as a dedicated increment, not a blocker. I OWN it (host-boundary ABI is my
territory) and will build it when v-agent-harness reaches Inc-1′ and confirms Route A is the priority.

## IMPLEMENTATION PLAN — Route A extends `assemble_host_mem` (studied the byte layout 2026-07-16)
The reuse target is confirmed: `assemble_host_mem` (envelope.rs:1572) — the host envelope variant used
when an op takes a `string` PARAMETER. It ALREADY provides: a shared-memory core module + instance + a
memory alias (core memory 0), a Memory canon-option on each op's canon-`lower`, and the program instance
instantiated with both `"host"` (lowered ops) + `"mem"` (shared memory). Its current SCOPE line says
"scalar/unit result, string or scalar params" — the result direction is the one missing piece.

A host String RESULT is the mirror of the String ARG (which already works via `(ptr,len)` into that
shared memory). The arg direction: guest writes the string into mem, passes `(ptr,len)`, host reads via
the canonical ABI. The result direction: host writes the string, the canonical ABI needs a `realloc`
(the ret-area allocator) + the guest reads `(ptr,len)` back and builds a value-heap rope.

STEPS (do String first; list<u8>/compound is a further increment):
1. `abi_val_type` (host.rs:59) + `first_unrepresentable_host_op` (:635): admit `Ty::String` as a RESULT
   ONLY when the emit takes the mem-path (i.e. gate the widening on "an op in this program uses the mem
   envelope"), keeping the scalar-only decline where no memory is emitted. (A String result in a
   NON-mem, scalar-only program must still force the mem path — so the routing in mod.rs:945 that picks
   `assemble_host_mem` vs `assemble_host` must also trigger on a String RESULT, not only a String ARG.)
2. `assemble_host_mem`: add a `Realloc` canon-option (alongside the existing Memory option) on the lower
   of an op with a `string`/`list` RESULT — the canonical ABI's ret-area allocator. `cabi_realloc` core
   func: the resource path (serialize.rs:1005) has a STUB `(ConstI32 0)` realloc; a real bump-allocator
   (or reuse the value-heap `arr-alloc`) is needed for a real result buffer. Confirm whether the
   canonical `string`-lift for an IMPORT result needs a guest realloc export or reads a host-provided
   ret-area — check the component-model canonical ABI for imported-func string results (the lift side).
3. Guest-side receive: after the `CallHostImport`, the result is a `(ptr,len)` in mem; emit the
   `str-from-bytes`/rope-build ops (the same the value-heap uses) to turn it into a String handle the
   program holds. `select.rs`'s `Core::HostCall` result handling (currently a scalar on the stack) gains
   a String arm that reads `(ptr,len)` + builds the rope.
4. `host_op_comp_functype` (mod.rs:1295-ish): map a String result to `COMP_STRING` (0x73) instead of
   declining. The functype builder already handles `HostParam::Str` → COMP_STRING for a param
   (mod.rs:1314) — mirror it for the result.

SIZE: ~120-180 lines (steps 1-4), MODERATE byte-emit risk. Byte-validate the produced consumer with
`wasm-tools validate` per step. The `assemble_host_mem` memory+Memory-option scaffolding is the load-
bearing reuse — this is a result-direction ADD to a working envelope, NOT new machinery. list<u8>/
compound results are a further step (the value-encode walker, like the peer compound result path).
BUILD when v-agent-harness confirms priority + the exact op shape (String->String? list<u8> for bytes?).

## SEQUENCING DECISION (2026-07-16, v-peer-linking) — build AFTER v-effects' _mem lands, share the envelope
Route A (host String RESULT) and v-effects' remaining _mem host String-PARAM variant need the IDENTICAL
host-memory envelope (exported memory + cabi_realloc + a lift/lower path). v-effects OWNS the adjacent
host-resource seam (concierge-ruled, see gap-host-effect-resource-escape-fusion) and their _mem variant
is still unbuilt (declines cleanly via set_needs_memory today). Building Route A unilaterally NOW would
race/duplicate that envelope → two divergent host-memory paths. DECISION: NOT blocking (agent-harness
shipped Route B, Bedrock-as-peer SigV4 shim), so hold; sent v-effects a forward-planning note asking them
to factor the _mem memory+realloc+lift as a REUSABLE piece; I build the RESULT direction on top once their
_mem param case lands. Byte-review offer both ways. This keeps ONE host-memory envelope, not two.
