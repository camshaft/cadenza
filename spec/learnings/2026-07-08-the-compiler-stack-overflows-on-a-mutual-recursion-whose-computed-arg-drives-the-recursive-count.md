# The compiler stack-overflows on a mutual recursion whose computed arg drives the recursive count

*2026-07-08*

**What happened.** The behavior gate began aborting entirely — `cadenza-seed behavior-gate` dies
with "thread 'main' has overflowed its stack, fatal runtime error: stack overflow, aborting" and
prints no summary, so the FAIL count reads as empty/zero. Bisecting the corpus localized it to
`spec/semantics/10-bytes.sexp`, and within it to four CBOR-reader cases (skip-nested-item,
recursive-reader, variable-length-array, skip-tagged) — each overflows the COMPILER's stack at
compile time. These are the self-hosting reader's navigation cases, recently landed; they are
supposed to compile.

**Minimal reproducer.** The overflow needs a mutual recursion where a COMPUTED value flows into
the argument that drives the recursion:

    (def (byte-at b i)      (match (Bytes.at b i) ((Some x) x) ((None _) 0)))
    (def (skip-elems b i k) (if (< k 1) i (skip-elems b (cbor-skip b i) (- k 1))))
    (def (cbor-skip b i)    (if (= (byte-at b i) 4) (skip-elems b (+ i 1) (byte-at b i)) (+ i 1)))
    (def (main) (cbor-skip (Bytes.of (list 1)) 0))

This overflows after ~13s of 99%-CPU recursion. Two one-line changes make it compile: (a) pass a
CONSTANT for `skip-elems`'s count `k` (`(skip-elems b (+ i 1) 1)`) instead of the computed
`(byte-at b i)`; or (b) call the computed `byte-at`/`cbor-arg` only in `cbor-skip`'s
NON-recursive branch. So the trigger is specifically: `skip-elems` and `cbor-skip` are mutually
recursive, and `cbor-skip` passes a computed value as `skip-elems`'s recursion-driving count `k`.
Simpler mutual recursions (even/odd), self-recursion, and `cbor-skip`↔`skip-elems` with a constant
count all compile fine.

**Why it is a break.** self-hosting-and-bootstrap.md #An Unsupported Construct Is Declined, Not
Miscompiled and the corpus's "the compiler never crashes" require the compiler to be a total
function over its input — decline or compile, never panic/abort. A stack overflow is a crash. And
because the triggering case lives in the CORPUS, the overflow aborts the whole behavior gate: no
FAIL can be seen, so every other regression is masked (a green-looking `grep '^  FAIL'` that is
actually the process dying before it prints). This is the most disruptive failure mode — it hides
the gate's signal.

**Likely root cause — a non-terminating inference/monomorphization fixpoint.** The ~13s of
recursive CPU before the stack blows points to a compile-time fixpoint (return-kind /
argument-to-parameter inference, or per-call monomorphization) that does not converge for this
mutual-recursion shape. Memory records this hazard family: "arg→callee param inference fixpoint
OOM" (arg-to-callee-param-inference-fixpoint-oom) and "threaded-compound-accumulator inference
blowup" both note a fixpoint re-walk that can re-introduce work; here the computed value flowing
from `cbor-skip` into `skip-elems`'s count `k`, and `skip-elems` calling `cbor-skip` back, forms a
cycle the fixpoint chases without a fuel/visited guard. The fix is to bound the inference fixpoint
(a visited-set or iteration cap that forces a conservative Kind and stops, or memoizes per
(function, arg-kinds) so a cycle terminates), so a legal mutual recursion compiles or declines
rather than overflowing the host stack.

**The lesson.** A gate whose corpus can crash the compiler loses its own signal: the abort happens
before the summary, so "0 FAIL" from a truncated run is indistinguishable from a clean pass —
diff the FAIL SET and confirm the summary line printed, never trust an empty `grep` for FAIL. And
a compile-time fixpoint over a recursive call graph needs a termination guard independent of the
program's shape: this one converges for constant-count recursion but chases a cycle forever when a
computed value feeds the recursion-driving argument. The two-line "make it compile" edits localize
the trigger to that value-into-recursive-count flow, which is exactly where the fixpoint's
monotonicity/visited-tracking must hold.

**No corpus case added** — the triggering cases are ALREADY in the corpus (`10-bytes.sexp`, the
four CBOR-reader cases); they are what surfaced the abort. The fix is in the (gitignored) seed's
inference/monomorphization fixpoint; until it lands, the behavior gate aborts on `10-bytes.sexp`.
Native seed.
