# three-variant-sum-answer — op resumes a 3-variant user Sum, body matches all arms

## py3v1 — Lo/Mid/Hi resume value selected by state, matched in body
```
(type Sig (Lo Int64) (Mid Int64) (Hi Int64))
(handle E (% n 3)
  ((tick () s (resume (if (< s 1) (Lo (* s 10)) (if (< s 2) (Mid (* s 100)) (Hi (* s 1000)))) (+ s 1))))
  (+ (* 100 (match (E.tick) ((Lo x) x) ((Mid x) (+ x 1)) ((Hi x) (+ x 2))))
     (match (E.tick) ((Lo x) x) ((Mid x) (+ x 1)) ((Hi x) (+ x 2)))))
```
State selects the variant; two dispatches thread state (s -> s+1) so they land in
DIFFERENT variants across the seed boundary. seed 0: Lo/Mid; seed 1: Mid/Hi; seed 2: Hi/Hi.
Model 12102/101 (n=10/0).

## Verdict: PASS-WITNESS (correctly compiled)
- Model 12102/101; compiler PASSES on wasm+rust+rust-async.
- All three variants exercised across seeds; a payload/tag-extraction bug or wrong-arm
  selection would miss the values. Wider tag space (3 variants) than the corpus's
  2-variant Ok/Err Result resume answers.

Confirms: a user-defined 3-variant Sum constructed in the resume answer and destructured
by a 3-arm match in the body compiles correctly across the effect seam (tag + payload
both preserved through the resume).

## Promotion
py3v1 promotable as a pass-witness (batch-347+ candidate).
