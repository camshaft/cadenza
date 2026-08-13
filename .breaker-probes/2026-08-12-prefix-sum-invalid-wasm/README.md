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

## Tick 1362 — family boundary
- `pfxG.sexp` — BYTES twin (computed-index Bytes.at + Bytes.concat append ×3) → **PASS**.
  The trigger is LIST-specific (RRB heap path), not a general rope/growth shape.
- `pfxH.sexp` — List.update at computed index (instead of at-read) + push ×3 → **FAIL**
  (function[9], same class). So ANY computed-index LIST access (read OR write) + push
  ×3 dispatches trips it. Both are held-back pin candidates on fix.

## Tick 1363 — threshold matrix
| seed len | dispatches | final len | wasm |
|---|---|---|---|
| 1 | 2 | 3 | PASS (pfxmin4) |
| 3 | 1 | 4 | PASS (pfxL) |
| 1 | 3 | 4 | FAIL (pfxmin5/pfxC) |
| 2 | 2 | 4 | FAIL (pfxK) |
| 3 | 2 | 5 | FAIL (pfxI) |
| 1 | 4 | 5 | FAIL (pfxJ) |
Trigger = dispatches ≥ 2 AND (seed+dispatches) ≥ 4 — i.e. the RE-DISPATCHED arm
must cross the length-4 boundary (RRB tail→node transition?). A single dispatch
crossing it is fine; the fold only mis-slots when the arm both RE-enters (≥2) and
the list crosses the depth threshold. Strengthens the fold-depth-dependent
slot-allocation hypothesis.

## Tick 1368 — FINDING #23 FULLY CLOSED (both faces)
ListUpdate residual fixed (v-effects f15cfb605, width-partition Core::ListUpdate
index scratch); corpus-bugfix pinned pfxH (MR c2eebcecd). My fresh-binary verify:
pfxH + pfx1 + pfxmin5 + pfxG all PASS ×3. Both #23 faces (ListAt d52544411,
ListUpdate f15cfb605) fixed and pinned. pfx1 (the rich prefix-sum table) remains
MY pin candidate — fold into a future batch with a distinct title.
