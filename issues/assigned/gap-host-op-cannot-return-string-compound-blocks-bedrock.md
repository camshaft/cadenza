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

## SCOPED for NEXT BUILD (v-effects, 2026-07-21, after my host×resource with-methods fusion landed 8e1802453).
Per v-peer-linking's sequencing decision (build my `_mem` host String-PARAM case FIRST, they add the RESULT
direction on top — ONE shared host-memory envelope). CONFIRMED the exact current gap via probe:
- `assemble_host_mem` (envelope.rs:1830, routed at mod.rs:854) ALREADY handles a CONSTANT string arg (bytes
  baked in the core data segment at `host_string_offset`, pushed as `(ptr,len)` — select.rs ~10509).
- The GAP: a NON-CONSTANT / RUNTIME string arg declines at select.rs:10518 ("a host call with a non-constant
  string argument is not yet emitted"). Repro: `(effect H (op h (-> String Int64))) (def (main (: s String))
  (host (H) (H.h s)))` → that decline (even WITHOUT resource escape — it's the base `_mem` runtime-arg gap).
- THE BUILD (`_mem` runtime string arg → shared memory): at the select.rs Ty::String arg arm, when the arg is
  NOT a ConstStr, emit guest code to write the runtime rope's bytes into the shared memory (`assemble_host_mem`'s
  exported memory / core mem 0) — needs a `cabi_realloc`-style bump (or reuse value-heap `arr-alloc`) to get a
  buffer ptr, `bytes-len` + a copy loop (bytes-get→store) — then push `(ptr,len)`. The CONSTANT path is the
  template (it already lays bytes + pushes ptr/len); this is the runtime mirror. Also: `set_needs_memory`
  (host.rs:641) must already force the mem path for a String param regardless of const/runtime (verify it does).
- SIZE ~100-150 lines, MODERATE byte-emit; byte-validate with wasm-tools per step. FACTOR the memory+realloc+
  buffer-write as a REUSABLE helper (v-peer-linking builds the host-String-RESULT lift on the same envelope →
  unblocks Bedrock-direct String->String). Build fresh next tick; scope-then-build cadence (as the fusion).

## SCOPE REFINED — bigger than ~150 lines (v-effects read-only prep, 2026-07-21, MR c82893ad8 pending so no commit).
Studied the actual emit machinery for the `_mem` RUNTIME-string-arg case:
- The program core module today only READS from `mem`: const host-arg strings are laid in a DATA SEGMENT
  (serialize.rs ~778, offsets from `layout.host_strings`) and the string-arg emit pushes `(data_offset, len)`.
  There is NO runtime WRITE path — the Lir has NO store op (111 variants, none is I32Store/Store8/mem-write).
- So a RUNTIME string arg (the actual gap) needs the guest to write the value-heap rope's bytes INTO `mem` at
  the host-call site, which requires EITHER (a) a new memory-store Lir op (+serialize) + a bump-allocator in
  `mem` (const strings occupy [0,next_offset); runtime writes need a region past that, and a loop/multi-string
  call needs a real bump ptr — a global or reserved area), OR (b) a hand-emitted `t-encode`-style marshaling
  helper. NOTE the resource path's `cabi_realloc` is a STUB `(ConstI32 0)` (serialize.rs ~1005) — works only
  because the resource ret-area is a FIXED offset; a real runtime arg buffer needs a genuine bump-allocator.
- The only existing value-heap→linear-mem bridge is the resource `t-encode` hand-emit (serialize.rs ~926) —
  reusable as a TEMPLATE but it's resource-envelope-internal, not general select-emitted code.
- NET: materially bigger than the earlier "~100-150 line abi-widen" estimate (which assumed the write path
  existed). Real scope: a new store-Lir path + a mem bump-allocator + the rope→mem copy loop at the host-arg
  site (read rope via bytes-get/bytes-len, store each byte, push (ptr,len)). ~200-350 lines, HIGHER byte-emit
  risk (mem aliasing between const-string data + runtime writes; canonical-ABI correctness for an imported-func
  string arg). Worth an OPERATOR priority check + a v-peer-linking design sync (they build the RESULT direction
  on the same envelope) BEFORE committing a multi-tick build to it. NOT a quick win; schedule deliberately.

## OPERATOR: PROCEED (pr-sync relay, 2026-07-21) — build the _mem runtime-string-arg path, land INCREMENTALLY.
Directive: it's the Bedrock-unblock linchpin (operator interop directive); 2x-estimate is a reason to scope
carefully, not drop. Land in SEPARABLE slices, each gating green on its own (capped `xtask check`, NOT a bare
2h full `cdz test`): SLICE 1 = the memory-store Lir op + bump-allocator (self-contained, unit-testable); SLICE 2
= the rope→mem copy loop at the host-call site, on top. Sync v-peer-linking (they build the RESULT direction on
the same envelope). Ask only on a genuine design fork (e.g. allocator strategy).

## SLICE 1 DESIGN (v-effects read-only prep, 2026-07-21 — turnkey; build the tick after my e2e MR c82893ad8 lands).
The Lir memory-store op (the missing write primitive):
- `wasm_abi` ALREADY has `I32_STORE`=0x36 + `I32_STORE8`=0x3a (no new opcode const needed).
- ADD Lir variant(s): `I32Store8 { align: u32, offset: u32 }` (+ maybe `I32Store` for the ptr/len words).
  Precedent: the closure-resource core module hand-emits ptr/len stores (serialize.rs, `const_i32(0)`+store)
  — but that's raw bytes in a hand-built module, NOT a select-emittable Lir. Expose it as Lir.
- ADD the `instr()` encoder arm (serialize.rs ~135, mirror `Lir::ConstI32`): push `op::I32_STORE8` then the
  memarg = `uleb128(align)` + `uleb128(offset)` (both 0 for a byte store at a computed addr). Stack: [addr, val].
- The generated-op-table cross-check test (`opcodes_match_wasm_encoder`, mod.rs ~7903) may need the new op
  added to its match (verify — it asserts every Lir opcode == wasm_encoder's).
BUMP-ALLOCATOR: the shared `mem` (envelope shared_mem_module, min 1 page = 64KiB) holds const host strings in
`[0, next_offset)` (data segment). Runtime writes need a region past that. SIMPLEST sound scheme for slice 1:
reserve a fixed scratch region at a known high offset (e.g. a page boundary) and a mutable bump pointer — OR,
since a single host call's string arg is consumed immediately by the call (not retained), a FIXED scratch
buffer at `[reserved_base, reserved_base+cap)` reused per call may suffice for the non-nested case (a nested
host call with two runtime string args needs distinct buffers — gate/decline that for slice 1, note it). Decide
fixed-scratch vs bump when building; fixed-scratch is the smaller slice-1 (a real bump ptr can be slice 1.5).
SLICE-1 TEST: a unit test that emits a tiny fn using the new store Lir + validates the module (wasm-tools) +
runs it writing a known byte to mem and reading it back. Self-contained, no host-call-site change yet.

## SLICE 1 PROTOTYPED + VALIDATED + STASHED (v-effects, 2026-07-21). The `I32Store8` Lir op is BUILT + proven
on scratch (can't commit — my e2e MR c82893ad8 still pending, no stacking): added the `Lir::I32Store8 { offset }`
variant (lir.rs, after GlobalSet) + the `instr()` encoder arm (serialize.rs, after LocalSet: push op::I32_STORE8
0x3a + uleb align=0 + uleb offset) + an op-table cross-check assertion in `opcodes_match_wasm_encoder` (mod.rs:
`I32_STORE8 == opcode(I::I32Store8(MemArg{offset:0,align:0,memory_index:0}))`). VALIDATED: compiles clean, the
op-table test PASSES (opcode byte matches wasm_encoder), clippy clean (no unused-variant warning — the encoder
arm counts as a use). STASHED as `stash@{0}` ("SITE _mem slice-1: I32Store8 Lir op…"). ⏭️ pop + commit + send as
slice-1 MR the tick after c82893ad8 lands (it's a tiny self-contained MR: 1 Lir variant + 1 encoder arm + 1 test
assertion). SLICE 2 (allocator + rope→mem copy loop at the host-call site, select.rs ~10518) builds on it next.

## SLICE 2 DESIGN (v-effects, 2026-07-21 — turnkey; build after slice-1 MR de72e45b9 lands + ideally v-peer-linking allocator input).
Slice 1 (Lir::I32Store8) sent (de72e45b9). Slice 2 = the runtime-string-arg marshaling at the host-call site.
BUILD SITES:
1. ROUTING (mod.rs ~854 `assemble_host_mem` vs `assemble_host`, + host.rs `set_needs_memory`): today the mem
   path triggers on `host_strings` (const args laid in data segment). A RUNTIME string arg lays NO const →
   `set_needs_memory` currently returns false → no `mem` → the runtime-arg emit would have no memory to write.
   FIX: `set_needs_memory` (host.rs:641) must ALSO return true when any host op has a `Ty::String` PARAM
   REGARDLESS of whether the arg is const (so `mem` is always present for a String-param op). Verify the
   routing in mod.rs picks `assemble_host_mem` on that condition.
2. ALLOCATOR (the SHARED piece — v-peer-linking reuses for host-String-RESULT; note sent, they're stopped so
   use a sound default + let them adapt on review): `mem` is 1 page (64KiB). Const host strings occupy
   [0, host_strings_end). Reserve the runtime scratch region at a FIXED base past that (e.g. round
   host_strings_end up to a 256-byte boundary = `scratch_base`). SLICE-2 SIMPLEST-SOUND: a FIXED scratch buffer
   at [scratch_base, scratch_base+cap) reused per host call — sound because a host string arg is consumed
   IMMEDIATELY by the call (not retained across calls). A NESTED host call with TWO runtime string args in one
   arg list needs distinct buffers → DECLINE that shape cleanly in slice 2 (note it; a real bump-ptr global is
   slice 2.5). Grow `mem` min pages if a string could exceed cap (or trap-guard len<cap for slice 2).
3. COPY LOOP (select.rs ~10513 Ty::String arm, replace the non-const decline): emit the arg → rope handle in a
   local; `bytes-len(rope)` → len local; then a Lir Loop: counter i=0; body: if i>=len br out; push
   `scratch_base + i` (addr), `bytes-get(rope, i)` (val i32 0..255), `I32Store8{offset:0}`; i+=1; br loop.
   After: push `(scratch_base, len)`. Drop the rope if it's an owned temp (heap_operand_ownership). Mirror the
   RuntimeBytes escape-walker's bytes-get copy loop (serialize.rs ~1100, `DESIGN-runtime-bytes-escape-walker`)
   for the exact loop/BrIf shape.
GATE: e2e test — `(effect H (op h (-> String Int64))) (def (main (: s String)) (host (H) (H.h s)))` COMPILES +
runs with a host-response fixture (verify the host receives the runtime string bytes). + the nested-two-runtime-
string decline pin. + wasm-tools validate. Capped `xtask check`. SIZE ~120-200 lines. Then PING v-peer-linking:
the allocator is ready for their host-String-RESULT read-lift to reuse.

## SLICE 2 DESIGN — SIMPLIFIED (v-effects, 2026-07-21 read-only prep, slice-1 de72e45b9 still pending):
Verified against the code — slice 2 is SMALLER than the 3-part design above:
- ROUTING IS ALREADY DONE (#1 dropped): `set_needs_memory` (host.rs:641) ALREADY returns true for ANY host op
  with a `HostParam::Str` param (const OR runtime) → `assemble_host_mem` (the shared-mem envelope) is ALREADY
  selected for a String-param op. So `mem` is present; NO routing/set_needs_memory change needed. The ONLY gap
  is the select.rs emit declining a NON-const string arg.
- So slice 2 = JUST the copy loop at select.rs ~10513 (the `Ty::String` arm's non-const branch, currently the
  decline). Template: the `bytes-len`/`bytes-get` copy loop in `closure_bytes_resource_core_module_borrow`
  (serialize.rs ~2137) + `encode_bytes_walk_body` — BUT those are HAND-EMITTED raw-byte core modules; my loop
  is SELECT-emitted Lir (Loop/Br/BrIf/I32Store8/CallImport(bytes-get/bytes-len) + scratch locals via high/
  scratch_ty). Same structure, Lir instead of raw bytes.
- ALLOCATOR: still need a scratch base in `mem` past the const-string data. `layout.host_strings` gives the
  const offsets; compute `scratch_base = max(offset+len) rounded up` (or a fixed high offset like 4096 if
  simpler + assert const data < that). Fixed scratch reused per call; nested-two-runtime-string decline.
- COPY LOOP Lir sketch at the host-string-arg site (rope handle already emitted to a local `r`):
    len_local = bytes-len(r); i_local = 0;
    Loop: (i >= len) BrIf→done; addr = scratch_base + i; val = bytes-get(r, i); [addr,val] I32Store8;
          i = i+1; Br→loop;  done: push scratch_base; push len_local;  (drop r if owned)
  Uses `high`/`scratch_ty` for the 2 i32 locals + the rope local. Mirror an existing select.rs Loop emit for
  the exact block/br-depth bookkeeping.
- GATE: e2e `(op h (-> String Int64))` + `(host (H) (H.h s))` with `(: s String)` param → compiles + runs
  (host-response fixture confirms the bytes arrived) + wasm-tools validate + nested-2-runtime-string decline
  pin. ~80-140 lines (down from 120-200, routing free). Build after slice-1 lands.

## SLICE 2 PROTOTYPED (v-effects, 2026-07-21 read-only, reverted — slice-1 de72e45b9 still pending so can't commit).
Built the copy-loop on scratch (on my slice-1 tip) + probed it. FINDINGS (de-risks the real build):
1. ✅ The copy loop COMPILES + is REACHED: replaced the select.rs ~10513 non-const-string decline with a Lir
   Block/Loop { pos<len: I32Store8(scratch_base+pos, bytes-get(rope,pos)); pos++ } then push (scratch_base,len),
   mirroring the String.scalar-len byte-scan loop (select.rs ~7086) + slice-1's I32Store8. Emit fires (no decline).
2. 🪤 REMAINING BUG (known class): the produced component is INVALID — `wasmparser::validate` → "unknown
   function 4294967295 (out of bounds)". CAUSE: the parallel `collect_used_ops` pass does NOT declare the
   `bytes-len`/`bytes-get` (+ I32Store8's mem) this new path uses → their CallImport resolves to u32::MAX. SAME
   class as the host-resource fusion collect_used_ops gap I fixed. FIX for the real slice 2: add bytes-len +
   bytes-get to `collect_used_ops_into`'s Core::HostCall arm WHEN an arg is a runtime (non-const) String.
3. 🪤🪤 REPRO CONSTRAINT (important for the e2e test): a runtime String into a host arg is HARD to construct —
   an export String PARAM declines earlier ("String has no component boundary representation"), so the test
   needs an IN-PROGRAM runtime string source: `(match (String.from-bytes (Bytes.of (list ((UInt 8).wrap n))))
   ((Some s) (host (H) (H.h s))) (None 0))` with `(main (: n Int64))` REACHES the emit (that's the repro that
   surfaced finding #2). Use that shape for the slice-2 e2e test.
4. ALLOCATOR: `scratch_base = round_up(max const-string end, 256) + 1024` in the 1-page mem worked for compile;
   validate mem bounds (len < 64KiB - scratch_base) or trap-guard. Fixed-scratch, single runtime-string-arg;
   decline nested-two.
NET: slice 2 = the copy loop (written, works) + the collect_used_ops declaration (the invalid-module fix) + the
e2e test using the from-bytes source. ~80-120 lines. Build after slice-1 lands; the collect_used_ops fix is the
one non-obvious piece (now known).

## ✅ SLICE 2 SENT (v-effects, 2026-07-21, MR 3e175fca6, base trunk 185f1664c). The host _mem RUNTIME-string-arg
path is COMPLETE (built for real after slice-1 I32Store8 landed): select.rs copy loop (bytes-len/bytes-get →
I32Store8 into mem[scratch_base+pos], push (ptr,len)) + collect_used_ops_into declares bytes-len/bytes-get +
Layout.host_needs_memory gates the core mem import for a String-param op with no const string. GATE all
substantive stages green (4348/12/0, gate--check OK, opt-sweep, 80 host tests); e2e test runs (String.from-bytes
→ host op → 42). v-peer-linking notified: the mem/scratch envelope is ready for their host-String-RESULT read-
lift (the mirror direction — they add i32.load8_u + str-from-bytes/rope-build on the same envelope). ⏭️ once this
lands, the WRITE side of the host/String boundary is done; the RESULT side (v-peer-linking) unblocks Bedrock
String→String. REMAINING v-effects follow-ups: nested-two-runtime-string-args (bump allocator, declined today);
list<u8>/compound host args (further). Both non-urgent.

## BACKLOG FEASIBILITY (v-effects, 2026-07-21 read-only probe, no consumer yet): host BYTES/LIST args.
Probed: a Bytes or (List Int64) host ARG declines cleanly today ("type ... has no component boundary form
this compiler emits yet"). KEY INSIGHT: a BYTES host arg is a NEAR-TRIVIAL extension of the landed `_mem`
runtime-string-arg copy loop — Bytes IS already a byte rope, so the SAME bytes-len/bytes-get→I32Store8→mem
marshaling applies verbatim (a String is just a Bytes with UTF-8 validation; the mem copy is identical). The
select.rs Ty::String host-arg arm's runtime branch would extend to `Ty::String | Ty::Bytes` with the same
scratch/copy code + `host_needs_memory` already fires for it (set_needs_memory checks HostParam::Str — would
need a HostParam::Bytes variant, OR widen the param-ABI to admit Bytes as (ptr,len) like Str). A LIST<Int64>
arg is bigger (needs a per-element value-encode walker into mem, like the peer compound path — not just a byte
copy). SO: Bytes-arg ≈ 30-50 lines reusing `_mem` (do FIRST if a consumer wants it); List/compound-arg is the
value-encode-walker increment (bigger). NO forcing consumer today — build on demand. Recorded so it's turnkey.
