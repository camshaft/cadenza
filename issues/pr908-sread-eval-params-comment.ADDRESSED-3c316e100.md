# PR#908 review comment — sread-eval-params comment says se-int-literal "above" but it's in sread-eval.cdz after the split (v-compiler-ml)

Mirrored from GitHub PR#908 review comment (Copilot), id `3679690450`.
File: `implementation/compiler-ml/src/sread-eval-params.cdz:209` — compiler-ml PORT source, comment →
v-compiler-ml. Blame `bb857f03d` "compiler-ml: split sread-eval.cdz — extract the param/call cohort to
sread-eval-params.cdz (throughput)".

## Comment (verbatim)

- (id 3679690450, sread-eval-params.cdz:209) "This note says the boundary-guard regression is covered by
  `se-int-literal` 'above', but `se-int-literal` lives in sread-eval.cdz after the split, not in this
  file. This makes the comment misleading for readers."

## Liaison verification (confirmed on trunk 9c77673b0)

The comment at sread-eval-params.cdz:208-209: "…the 'a CLEAN int literal still runs' boundary-guard
regression … is covered by `se-int-literal` ABOVE (same `run-src(\"42\")` → 42) — the former
`se-plain-int-still-runs` was a byte-identical duplicate … removed…". Grepping: in
`sread-eval-params.cdz`, `se-int-literal` appears ONLY in this comment (:209) — it is NOT defined here.
Its actual definition `def se-int-literal() = match run-src("42") …` is in `sread-eval.cdz:38`. The split
(`bb857f03d`) extracted the param/call cohort into this new file but the comment's "above" (same-file
back-reference) went stale — the referenced test now lives in the SIBLING file. Reword to point at
`sread-eval.cdz`'s `se-int-literal` (cross-file) instead of "above". Comment-only, behavior-neutral.

Owner: **v-compiler-ml** (compiler-ml port source, their `bb857f03d` split). Fix the stale "above"
cross-file reference (→ "covered by `se-int-literal` in sread-eval.cdz").
