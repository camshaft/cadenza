# xhs1 — CROSS-HANDLER shared-let MISCOMPILE (2026-08-16, tick 1614)

v-effects' tpwJ close (8276ad1a6) deferred three classes as "safe floor,
clean-decline": recursive-driver / growing-state / cross-handler shared-let.
FIRST PROBE into the cross-handler class found a SILENT MISCOMPILE instead.

## Shape (xhs1.sexp)
Nested handlers: outer O.note accumulates, inner I.step let-binds
`c2 = col + x + bias`, performs `(O.note c2)` MID-ARM, resumes
`(c2*10 + nv%10)` threading c2 as next-state. The binder feeds: the outer
perform's argument, the resume value, and the next-state — tpwJ's shape plus
a cross-handler dispatch inside the binder's live range.

## Evidence
- Expected (hand model): 44104114 @ n=10, 33081111 @ n=0.
- Got: 44100130 / 33084124 — WRONG, stable ×3 runs, UNIFORM across
  wasm + rust + rust-async (shared-lowering bug, like tpwJ).
- CONTROL (xhs1-nolet-control.sexp): same program with c2 inlined at every
  use — CORRECT on all backends. The mid-arm cross-handler perform is fine;
  only binder + cross-handler-perform corrupts.
- First divergence: dispatch 2's note answer (nv%10 = 0 vs 4), cascading
  into the outer accumulator (final 130 vs 114).

ISSUE filed to v-effects (ref 9522fabd6). xhs1 HELD from corpus. When fixed:
promote xhs1 + control as the fix-pin pair.

## Boundary map (tick 1617, all UNIFORM wrong x3 backends)
- xhsA (binder → perform + answer, state threads OLD col): WRONG (44068118/33055115
  vs 44060110/33058108).
- xhsB (binder → perform + state, answer = nv only): WRONG (4030130/3024124 vs
  4014114/3011111).
- xhsC (perform takes a CONSTANT — binder feeds only answer + state): WRONG
  (45105115/35085115 vs 45100110/35080110). Decode: final acc = 15 = THREE
  note(5) executions across two steps — the mid-arm perform is DUPLICATED.
So the corruption does NOT require the binder to feed the perform, the answer,
or the state specifically: every combination of (let binder) + (let-bound
mid-arm cross-handler perform) is wrong, and xhsC shows the foreign perform
RE-EXECUTES. Common ingredient across all four: an arm with a binder chain
whose inner let binds a cross-handler perform result. Contrast rlyC (same-
handler binder-over-perform DECLINES) — cross-handler slips the guard and
miscompiles instead.
