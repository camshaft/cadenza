# nested-match-arm — two-level nested match in the arm drives the resume

## pynm1 — Option-of-Option arg, nested match picks the resume
```
(cmd (m) s
  (match m
    ((Some inner)
      (match inner
        ((Some x) (resume (+ s x) (+ s 1)))
        ((None)   (resume s (* s 2)))))
    ((None) (resume (* s 10) (+ s 3)))))
```
Two dispatches: (Some (Some 7)) -> inner-Some path (answer s+7); (None) -> outer-None path
(answer 10s). Model 820/710.

## Verdict: PASS-WITNESS (correctly compiled)
- Model 820/710; compiler PASSES on wasm+rust+rust-async.
- Two distinct nested-match arms exercised; a nested-tag mis-dispatch or payload-extraction
  bug at depth-2 would miss 820/710.

Confirms: a two-level nested match over a Sum-of-Sum op ARGUMENT, each leaf driving a
different resume (answer + next-state), compiles correctly across the effect seam — deeper
pattern nesting in the arm than the single-level pysm1.

## Promotion
pynm1 promotable as a pass-witness (batch-347+ candidate).
