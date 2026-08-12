# 2026-08-12 prefix-sum invalid wasm — FINDING #23 (tick 1359)

- `pfx1.sexp` — the full prefix-sum probe (3 adds + 3 ranges incl. OOB sentinel).
  HELD BACK: trips #23 on wasm (rust/rust-async PASS 110801/251501). Pin on fix.
- `pfxmin5.sexp` — minimal: THREE adds + one range → FAIL (filed as the queue repro).
- `pfxmin3.sexp` — three adds (one computed arg) + range → FAIL (computed-ness irrelevant).
- `pfxmin4.sexp` — TWO adds + range → PASS (dispatch-count boundary).
- `pfxmin2.sexp` — one add + range → PASS.
- `pfxmin.sexp` — range alone → PASS.

Validator: func 13 "expected i32, found i64" @0x4fd — local.tee 19 gets an i64
heap-read result into an i32-declared local. INVERSE of #21; both #21+#22 fixes in,
#21 fence still green. Trigger needs the THIRD add dispatch.

## Tick 1360 — SHARPENED MINIMAL SHAPE
The range op is IRRELEVANT (pfxmin9: three adds alone FAIL). Reduction chain:
| probe | arm shape | wasm |
|---|---|---|
| pfxA | plain len answer + push (no at-read) | PASS |
| pfxB | computed-index at-read, NO push | PASS |
| pfxC/D/E | computed-index at-read + push (let / inlined-twice / no-def) | FAIL |
| pfxF | FIXED-index at-read (0) + push | PASS |
| pfxmin4 | two dispatches of the failing arm | PASS |
MINIMAL: an arm that (a) reads the threaded list at a COMPUTED index (len-1),
(b) pushes to the same list, and (c) is dispatched THREE times. Computed-index
read xor push alone is fine; two dispatches fine. #18's computed-index class x
state-thread growth x dispatch count.
