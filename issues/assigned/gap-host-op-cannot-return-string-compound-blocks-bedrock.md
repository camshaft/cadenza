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
