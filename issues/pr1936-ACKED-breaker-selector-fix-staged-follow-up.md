# PR #1936 review comment — spec/semantics/10-bytes.sexp (breaker/corpus) — test-precision [VERIFIED]

https://github.com/camshaft/cadenza/pull/1936 (2-pin slice-of-slice over a concat rope — seam-crossing).
Copilot (copilot-pull-request-reviewer, overview + inline id 3709269969) flags both new cases build the
rope from two LITERAL operands, which may const-fold and defeat the seam the pins claim to exercise.

## `rope` built from two literal `Bytes.of` operands can const-fold, collapsing the 2-segment rope to a flat leaf → seam never exists at runtime (Copilot, 10-bytes.sexp:236 & :260) — test-precision [VERIFIED]
> `rope` is built from two literal `Bytes.of` operands; the compiler can constant-fold `Bytes.concat`
> when both operands lower to `Core::BytesOf`, collapsing the intended 2-segment rope into a single flat
> `Bytes.of` and no longer exercising seam-crossing slice-of-slice behavior. This issue also appears on
> line 260 of the same file.

VERIFIED against the file's OWN trunk guidance. Both new cases use:
  `(def rope (Bytes.concat (Bytes.of (list 10 20 30)) (Bytes.of (list 40 50 60 70))))`
— a concat of two CONSTANT chunks. The pre-existing comment in this very file (origin/main
10-bytes.sexp:410-417) explicitly warns: "The seam case … slices across the seam of a concat of CONSTANT
chunks, which the fold may materialize before slicing. A GENUINELY-runtime byte rope — a `Bytes.concat`
of chunks selected at run time (an `if` the fold cannot decide) — assembles a multi-chunk rope that
survives to the emitted `bytes-slice`." The already-passing rope test ("a slice crosses the seam of a
runtime-assembled byte rope") therefore uses a runtime `pick s …` selector so the fold CANNOT decide the
chunks. The two new cases DROP that guard → if the fold materializes the concat, the slice-of-slice runs
over a flat leaf and passes on the correct value (380 / 17) WITHOUT ever composing offsets across a
segment boundary — i.e. it no longer tests what its doc claims (view-of-view offset composition across a
seam). MED/test-precision — the pin is green but may be vacuous.

Fix (owner's call): route each `rope` through a runtime-undecidable selector, mirroring the existing
"runtime-assembled byte rope" case — e.g. `(def rope (Bytes.concat (pick n (Bytes.of (list 10 20 30)) …)
(pick n …)))` keyed on the `main` param `n`/`s` so the fold cannot pre-join. That preserves the same
expected outputs while guaranteeing the concat survives to a real deferred rope. Corpus/breaker zone —
`.sexp` semantics pin.
