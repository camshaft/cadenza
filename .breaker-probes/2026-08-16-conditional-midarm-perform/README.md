# xhsG — conditional mid-arm foreign perform (2026-08-16, tick 1632)

v-effects' hardening probe #4, escalated to me for oracle + regression triage.

## Oracles (hand-modeled, independent of any build)
- n=10: p=40 (else, no perform), q=100 (then, note(10): acc 0→10, nv%10=0),
  r=110 (10+100) → 40100110
- n=0: p=30 (else, c2=3), q=88 (then, note(8): acc 0→8), r=108 → 30088108

## Verdict: MISCOMPILE CONFIRMED — but NOT their regression; BOTH sides wrong
- CURRENT trunk 931c11dd3 (with correct-fold eead20a60): 40104114 / 30081111,
  uniform wasm+rust+rust-async. Decode: the ELSE step SPURIOUSLY PERFORMS
  note(c2) (acc arrives at the then-step already loaded with the else's c2).
  v-effects' unconditional-perform read is right.
- PARENT 6106503ee (pre-fold, fresh temp-worktree build + own runtime store):
  40100120 / 30086116 — ALSO WRONG, different signature: the THEN branch
  DOUBLE-PERFORMS (acc gets 2x note(c2); n=0 decode 8→16 confirms).
So the conditional mid-arm perform was ALREADY miscompiled (then-double) on
the distribute path pre-fold; eead20a60 changed the wrong answer's shape
(else-spurious) rather than introducing the first wrongness. NOT a clean
regression — a second face of the same duplication family, present on both
paths. v-effects' safe-floor-era belief that this "declined pre-fix" is
wrong for THIS shape (their inline control declines; the let-bound form
compiled + miscompiled all along).

xhsGmin (else-only) correct per v-effects. Banked xhsG with correct oracles:
todo-witness → flips PASS on their narrowing fix.
