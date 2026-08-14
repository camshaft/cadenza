# dst — three-way partition INVALID WASM x2 faces (2026-08-14, tick 1492) — FINDING

A scalar-only probe family (3-way partition counter: nested-if arm buckets a
value against two pivots, counts in tuple state) produces TWO DISTINCT
invalid-wasm errors, no Lists anywhere:

| probe | shape | dispatches | verdict |
|-------|-------|-----------|---------|
| dst1 | pivots recompute (% n 4) IN the arm, 3-tuple state, 5 cls + prod | 6 | **"Code for function is too large"** ×3 |
| dstA | same, 4 cls + prod | 5 | PASS |
| dstB | same as dst1, 6 cls no prod | 6 | **"Code for function is too large"** |
| dstC | pivots in STATE (5-tuple), no n in arm | 6 | **invalid: function[7]** |
| dstD | dstC at 5 dispatches | 5 | **invalid: function[7]** |
| dstE | dstC at 4 dispatches | 4 | PASS |

Face 1 (dst1/dstB): the seed-expression (% n 4) recomputed inside the arm ×
6 dispatches → code-size explosion (wasmtime 'function too large') — F24-like
per-dispatch duplication, but of the ARM body (the n-capture chain?).
Face 2 (dstC/dstD): 5-WIDE tuple state × 3-way nested-if arm × 5 dispatches →
function[7] fails validation (error kind unknown — need wasm-tools dump;
could be F24 locals-count or a width-alias face on the 5-tuple).

Wider-state threshold note: 2-tuple probes routinely run 7-9 dispatches green
(odf1, qrm1, rrb1); the 5-tuple breaks at 5. State WIDTH lowers the dispatch
threshold. All scalar — this is NOT the List-dependent lstM/medK territory.

## Face-2 kind extracted (tick 1494)
`cdz compile dstC-prog -o dstC.wasm` → **165,963,565 bytes from a 1KB source**;
`wasm-tools validate` → **'function body size count exceeds limit of 7654321'**
— BODY-SIZE count (F24 code-dup family), NOT type-mismatch. Size ladder:
dstE(4 disp)=551KB → dstC(6 disp)=166MB (~300x per 2 dispatches, matches the
~4x/call-site geometric growth of the original F24 evidence). Face 1's
'Code for function is too large' = the same cap reported by cranelift at
function-compile instead of validate. Both faces are F24; addendum sent.
