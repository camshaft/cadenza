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

## xhsD dropped-arg complement (tick 1625)
v-effects reported the dropped-frozen-arg case (outer arm IGNORES its op param)
folds CORRECTLY — measured on their safe-floor build 07e85af7c. VERIFIED
CONTRAST on pre-floor trunk e261604c6: xhsD miscompiles with the SAME
duplication signature (ran 40102003/30082003 vs expected 40101002/30081002 —
final acc 3 = three note executions across two steps, uniform wasm+rust).
So the floor's arm-local freeze doesn't just gate the escape case — it also
FIXES the dropped case outright. xhsD banked with hand-modeled (correct)
outputs: it is a STAGED PASS-WITNESS that flips green when 07e85af7c lands,
guarding the already-correct drop case against a future correct-fold regression
(exactly what v-effects requested).

## PLAN CHANGE (tick 1626): floor DEAD, correct-fold 8f32b07a0 queued
- Safe-floor 07e85af7c was REJECTED at pr-sync (over-declined the as7
  fold-strict unit) and is DEAD. v-effects replaced it with ONE squashed
  CORRECT-FOLD commit 8f32b07a0 (drain-level freeze + narrowed collapse
  trigger; v-inference sign-off, rcdzc 2660/0, 14c 620/2/0), queued now.
- Witness re-keying (all to 8f32b07a0, NOT the dead floor):
  xhs1 44104114/33081111 · xhsA 44060110/33058108 · xhsB 4014114/3011111 ·
  xhsC 45100110/35080110 · xhsD 40101002/30081002 — ALL become PASS-witnesses
  (the case files already carry these correct oracles; no edits needed).
- NEW xhsE (computed perform-arg `(O.note (+ c2 1))`): on CURRENT trunk it
  MISCOMPILES 45103133/expected 45106116 (same duplication family); on the
  correct-fold build it SAFELY DECLINES (freeze completeness boundary,
  list-branch init not drain-safe yet). DECLINE-witness at 8f32b07a0,
  flipping to PASS (45106116/34083113, hand-verified) on the post-merge
  follow-up. n=0 oracle 34083113 computed here — v-effects' note only gave
  n=10.

## tick 1628: land claim REFUTED by fresh-binary verify
v-effects reported 8f32b07a0 "integrated by content" — but at origin/main
6106503ee (fresh fetch + rebuild): xhs1 still miscompiles 44100130 (the
ORIGINAL duplication answer), all 6 witnesses fail, and the freeze markers
(reaches_perform_with_args / #fa) are absent from rcdzc source. No integration
commit in origin's last three. Their diff likely ran against a local ref.
Promotion HELD; re-verifying on flip-sweep each tick. This is exactly why
verify-on-origin-by-content with a FRESH BINARY is the standing rule.

## xhsF multi-perform (tick 1631)
v-effects' hardening-sweep candidate: the inner arm performs the outer note
TWICE with the same shared binder (two frozen args drain-bind in one arm).
Their n=10 verify: 52116128 (wasm+rust, variant==control). My independent
hand-model MATCHES (52116128) + derived the n=0 oracle they lacked: 39086122.
Banked + gated PASS ×3 wasm, ×1 rust, ×1 rust-async at 931c11dd3. Joins the
family as pass-witness #6 (batch-296 candidate; 295 carries the first 6).
Their hardening backlog (next shapes): two-distinct-outer-ops 3-handler nest,
conditional mid-arm perform.

## xhsTwo two-distinct-outer-handlers (tick 1642)
v-effects' probe #2 (last hardening axis): inner performs O.note(c2) then
P.tick(c2) — two DIFFERENT foreign handlers, shared binder, freezes at
different drain levels. Their n=10 verify 445044228; my independent model
MATCHES + fills n=0 = 333811222 (their template had a placeholder 0 output —
banked with real oracles). PASS ×3 wasm + ×1 rust + ×1 rust-async on origin
3c06de590. The family hardening matrix is now complete: single/multi/
conditional(4 shapes)/two-handlers performs + 2 no-touch controls.

## xhsMulti merged-multi-slot audit (tick 1655) — SAFE DECLINE, audit CLOSES
v-effects' last exclusion axis (slots.len()==1), authored from my side as the
xh1-shape: outer T = Map-state put (answers PRIOR or 99), inner I.step
let-binds c2, performs T.put(c2, c2*2) mid-arm, packs + threads c2.
- xhsMulti (shared binder): CLEAN DECLINE — CDZ0101 unbound c2 (the freeze's
  scope-escape signature; todo x3). NO wrong answer.
- xhsMultiCtrl (inlined): PASSES with the hand-modeled oracles
  (49109008/39089099) — model confirmed, distribute handles multi-slot.
Verdict: the multi-slot exclusion is SAFE (decline, not miscompile) — same
class as recursive-driver, unlike growing-state. All 3 collapse exclusions
audited: 1 miscompile (fixed), 2 safe declines. AUDIT FULLY CLOSED.
Both banked: xhsMultiCtrl promotable as a pass-pin; xhsMulti a todo-witness.

## xhsGGrow composition witness (tick 1665)
v-effects' intersection probe: the foreign perform BOTH inside an if-then
branch (7106ad497 selector-freeze territory) AND feeding a growing List.push
next-state (95f5ab8d2 correct-fold territory) — both fixes triggered in one
arm, never differential-tested before. Folds CORRECTLY: my independent model
matches their oracles (40077107/30066106); verified PASS x3 wasm + rust +
rust-async here (their opt-sweep O0..O3 also 0-divergence). Banked as the
intersection regression guard; rides batch-300.
