# multishot-foreign-answer — per-replay re-evaluation of a foreign draw in the resume answer

## pymf1 — multi-shot arm, each resume answer draws a STATEFUL foreign counter F
```
(handle F 0 ((aux () fc (resume fc (+ fc 1))))       ; F = counter: answers state, increments
  (handle E (% n 3)
    ((tick () s
      (+ (resume (+ s (F.aux)) (+ s 1))               ; shot 1: F.aux = 0
         (resume (+ s (F.aux)) (+ s 100)))))          ; shot 2: F.aux = 1
    (let ((x (E.tick))) (+ x 7))))
```
Deep handler, one E.tick in the body, arm resumes TWICE (multi-shot). Continuation
k(v) = v + 7. Each resume answer = s + F.aux; the two shots must see DISTINCT foreign
values (0 then 1) because F is a stateful counter drawn once per shot.
Model: shot1 = k(s+0) = s+7, shot2 = k(s+1) = s+8, arm = 2s+15. seed = n%3 → 17 / 15.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 17/15; compiler PASSES 17/15 on wasm+rust+rust-async.
- DISCRIMINATOR (built into the observable): if the compiler SHARED one F.aux draw
  across both shots (both see F=0), the sum would be 2(s+0)+14 = 16 / 14. The observed
  17/15 proves each replay RE-DRAWS the foreign effect — no cached/shared foreign value
  across multi-shot resumes.

Confirms: a foreign (distinct-effect) draw in the resume-answer position is correctly
re-evaluated PER REPLAY under multi-shot resumption; the foreign counter advances once
per shot in arm evaluation order. Complements pyce1 (single-shot foreign answer) and
pyad1 (abort-arm foreign answer).

## Promotion
pymf1 promotable as a pass-witness (batch-343 candidate alongside pyce1, pyad1, pyx2).
