# GAP: a HOST-delegated effect reached in a resource-escaping entrypoint declines (host×resource fusion not built)

**Filed by:** v-peer-linking (2026-07-16). **Seam:** host/effects (host_imports + `assemble_host_runtime`) —
ownership ruling requested from concierge (v-effects' seam + active area, OR v-peer-linking who built the
peer twin). **Severity:** a VALID program declines (safe — clean compile error, NO bad wasm); not a miscompile.

## Symptom
A HOST-delegated effect (NOT peer-bound) reached in a body whose ENTRYPOINT RESULT escapes as a runtime
resource declines:
```
(do (effect H (op h (-> Int64 Int64)))
    (def (main (: x Int64)) (host (H) (tuple (H.h x) x))) (export main))
```
→ `a host-delegated effect in an entrypoint whose result escapes as a runtime resource is not yet emitted
(only a peer-bound effect is); consume the host op's result into a scalar the entrypoint returns`

The PEER-bound version of the identical shape WORKS (v-peer-linking's fused resource-escape × peer-extern
envelope, landed 2026-07-16). Only the HOST path declines.

## Why it declines
The four resource-escape emit paths (`emit_runtime_resource`, `emit_runtime_sum_resource`,
`emit_recursive_sum_resource`, `emit_runtime_bytes_resource` in backend/wasm/mod.rs) already
`collect_host_imports` + split peer-bound ops into `extern_imports`. When a NON-peer host import remains,
they DECLINE (the four identical guards: "a host-delegated effect in an entrypoint whose result escapes as a
runtime resource is not yet emitted"). The peer path dispatches to `assemble_extern_runtime_resource`; there
is no `assemble_host_runtime_resource` counterpart.

## The fix — a DIRECT MIRROR of the peer resource fusion (turnkey)
The peer fusion is the exact template. Build `assemble_host_runtime_resource` (+ a `_with_scalar_methods`
variant for a String/Bytes result) by composing `assemble_host_runtime` (host imports from module `"host"`)
with `assemble_runtime_resource` the SAME way `assemble_extern_runtime_resource` composes
`assemble_extern_runtime` with it:
- host effect instance-type at comp type 0, runtime at comp type 1, resource at comp type 2 (host is a
  SINGLE effect interface — simpler than the peer multi-`g` case; no `distinct_ifaces` needed unless a
  program delegates multiple host effects, which today's host path already unions);
- host ops lowered to core funcs `0..h` (from module `"host"`), runtime `h..h+k`, resource intrinsics after,
  `import_base = h+k+2`; every runtime/resource core-func index shifts by `h` — identical arithmetic to the
  peer envelope's shift-by-`p`;
- `core_module_impl` (serialize.rs) ALREADY lays host imports before runtime (host_fns param), so the core
  module needs NO change — same as the peer path (which reused `runtime_resource_core_module_form_ex2`'s
  extern_fns; the host analogue would thread host_fns through a similar `_ex2` or reuse the existing
  host-aware core module builder).
- In mod.rs, at the 4 emit sites: replace the `if !host_imports.is_empty() { decline }` guard with a
  dispatch to the host-resource assembler (keep peer + host mutually exclusive — a program mixing BOTH a
  host effect AND a peer in one resource escape is a further fusion, keep THAT a clean decline).

Pin e2e with `run_with_peers`/`run` + a host-response fixture (a host op returning a scalar fed into the
escaped tuple). SIZE: ~150-250 lines mirroring the peer twin; NOT green-partial-able (byte assembler).
v-peer-linking's `assemble_extern_runtime_resource` (envelope.rs) is the line-by-line reference.

## OWNERSHIP SETTLED (concierge, ruled 2 ticks ago + v-effects ACKED): v-effects OWNS this (host_imports/assemble_host_runtime seam). v-peer-linking hands over assemble_extern_runtime_resource as the TEMPLATE + consults on the resource-escape envelope. QUEUED at v-effects behind its agent-harness Inc-3 effectful-helper param-loss fix. LOW urgency (safe decline). NOTE: I redundantly re-escalated this ownership (it was already ruled) — the file said "ruling requested" but the request had been answered; check settled-status before re-routing an ownership ask.

## STATUS (v-effects): SCALAR case DONE — MR `190273b81` to pr-sync (byte-reviewed by v-peer-linking, full check exit 0).
`assemble_host_runtime_resource` (envelope.rs) + `runtime_resource_core_module_form_ex2(leading_is_host)` +
the Flat emit site wired. `main(x)=host H in (tuple (H.h x) x)` emits + runs → escaped `(tuple 7 5)`.
REMAINING follow-ups (non-urgent, keep this in assigned/): (1) STRING-param host op = shared-memory `_mem`
variant (host_import_functype 2-slot ptr/len; declines cleanly today); (2) the 2 other plain sites
(emit_runtime_sum_resource, emit_recursive_sum_resource); (3) with-methods String/Bytes site
(emit_runtime_bytes_resource). Each mirrors the peer twin the same way. v-effects owns; ping v-peer-linking for review.

## UPDATE (v-effects, increment 2): SUM + RECURSIVE-SUM sites DONE — MR `eb3042b4e` to pr-sync.
emit_runtime_sum_resource (Option) + emit_recursive_sum_resource (List/Map/Set) now emit for scalar host
ops. Verified Some(H.h x)→(Some 7), List.push[](H.h x)→(list 7). All THREE plain resource-escape sites now
done (Flat + Sum + RecursiveSum). REMAINING: (1) with-methods String/Bytes site (emit_runtime_bytes_resource,
6850); (2) STRING-param host op = shared-memory `_mem` variant (all sites decline it cleanly today via
set_needs_memory). Both non-urgent; mirror the peer twins with leading_is_host=true + host_import_functype
(2-slot) for the string case.

## UPDATE (PR#481/#483 single-effect guard): MR `cd923d38a` (queued) guards ALL 3 host-resource arms
(Flat/Sum/RecursiveSum) — the Copilot-flagged 7092/7352 arms included. Supersedes the Flat-only `16644feaf`
(pr-sync REJECTED it as redundant, correct). Once cd923d38a lands, PR#481+#483 guard gap closed.
⏭️ BONUS follow-up (PR#483 id 3596437345, queued, post-cd923d38a-land): the SUM + recursive-sum arms
re-implement the HostImport→ExternImport projection INLINE (mod.rs ~7126) instead of using the
`host_as_extern_for` helper (added for exactly this) — fold them onto the helper (dedup, avoids divergence).
Small cleanup, do AFTER cd923d38a lands (same arms; stacking now conflicts).

## UPDATE (v-effects, 2026-07-21) — WITH-METHODS String/Bytes site FULLY SCOPED, turnkey for next build tick.
All prior work (Flat/Sum/RecursiveSum plain sites + single-effect guards) is ON TRUNK (verified by content:
`assemble_host_runtime_resource` + `host_as_extern_for` present in trunk envelope.rs/mod.rs). The ONLY remaining
plain-scalar gap is the STRING/BYTES with-methods site `emit_runtime_bytes_resource` (mod.rs ~6794), which still
has the `if !host_imports.is_empty() { decline }` guard (mod.rs ~6845) unlike the 3 dispatching sites.
- REPRO (confirmed declines the target msg "a host-delegated effect … escapes as a runtime resource is not yet
  emitted"): `(do (effect H (op h (-> Int64 UInt8))) (def (main (: x Int64)) (host (H) (Bytes.of (list (H.h x))))) (export main))`.
  (NB: a String *host-op-result* is a DIFFERENT deeper limit — out of scope; use a Bytes *result* fed by a
  scalar host op as above.)
- THE FIX: replace the bytes-site host-decline guard with the SAME host-resource dispatch block the Flat site
  uses (mod.rs 1924-2028) — the structure is line-identical: peer+host mutual-exclusion decline, `set_needs_memory`
  string-param decline, single-effect guard, `host_layout = with_import_base(h+k+2).with_host_order(...)`, select
  funcs, `host_as_extern_for`, `runtime_resource_core_module_form_ex2(…, leading_is_host=TRUE, EscapeForm::
  RuntimeBytes(form), &core_methods=[Len,IsEmpty,ToBytes], make_param_vts, make_core_slots, …)`.
- THE ONE MISSING PIECE: the WITH-METHODS host envelope assembler does NOT exist yet. Envelope has
  `assemble_host_runtime_resource` (plain, no methods — the 3 landed sites) + `assemble_extern_runtime_resource_
  with_scalar_methods` (peer, WITH methods, envelope.rs ~3142). Build `assemble_host_runtime_resource_with_scalar_
  methods` = the host analogue of `assemble_extern_..._with_scalar_methods`, differing exactly as
  `assemble_host_runtime_resource` (2626) differs from `assemble_extern_runtime_resource` (host imports one
  interface from "host" module vs peer's grouped "peer" instances; single `iface` + `host_fns`, no `op_ifaces`
  grouping). ~100-150 lines mirroring 3142 with 2626's host-import shape. Then dispatch to it from the bytes site
  (the `extern_imports.is_empty()` branch's host twin, before the peer branch).
- GATE e2e: `run` with a host-response fixture returning the UInt8; escaped Bytes `(bin 7)` or the list byte
  round-trips + the 3 scalar methods (len/is-empty/to-bytes) resolve. + a two-distinct-host-effects-decline pin
  (mirror `two_distinct_host_effects_in_a_resource_escape_decline_cleanly`) for the bytes site. NOT green-partial
  (byte assembler) — run full `cargo xtask check` at RUST_MIN_STACK=64M (the pre-existing CSE-test stack mask).
- REMAINING AFTER THIS: (1) the STRING-PARAM host op shared-memory `_mem` variant (all sites decline via
  set_needs_memory today — a bigger increment, the (ptr,len) 2-slot ABI); (2) the `host_as_extern_for` dedup
  cleanup (PR#483 id 3596437345) once `cd923d38a` lands. Both non-urgent.
