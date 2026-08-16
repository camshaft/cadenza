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

## xhsGmatch + fix status (tick 1635)
v-effects folded a match-scrutinee freeze into the complete conditional fix
(18d7cd137 → integrated as 1647a8782 with freeze_selector_refs) and reported
both witnesses PASS. Banked xhsGmatch (the boolean-match twin, same oracles
40100110/30088108). BUT origin/main at 4c75635d9 does NOT yet carry
1647a8782 (ancestor check negative; xhsG still miscompiles 40104114 here —
the SAME publish-lag as eead20a60). Both witnesses stay TODO until the next
pr-sync batch push reaches origin; promotion follows the standing
fresh-binary-on-origin verify.

## Plan-change #2 + xhsGdeep (tick 1637)
- 1647a8782 was REJECTED (over-declined quo1 — the selector freeze fired on
  pure-branch matches in shared-let collapses), NOT publish-lagged; rewound.
  NARROWED resend = 967b0966b (freeze only when a branch reaches an
  effect-op-with-args perform; quo1 untouched); witnesses flip on THAT.
- NEW xhsGdeep: the perform TWO selector levels deep (parity if, magnitude
  if, only the even-and-large leaf performs; 3 silent leaves with distinct
  tags). Hand oracles 41100110/33088108. On current origin (4c75635d9):
  miscompiles 41104114 — the else-spurious signature AGAIN, at depth 2. Sent
  to v-effects as a narrowing-robustness datapoint: their fix should catch
  it (a branch DOES reach the perform) but it's exactly the shape a
  first-level-only freeze would miss.

## xhsH perform-AS-selector (tick 1638)
The inverse shape: the foreign note's answer is let-bound then ROUTES the
branch (both branches silent). PASSES ×3 wasm + rust + rust-async on CURRENT
origin 4c75635d9 (44107114/33087111, hand-modeled) — correct today, unlike
the whole G family. Banked as a PASS-witness to pin BEFORE 967b0966b lands:
that fix rewrites selector handling (freeze_selector_refs fires exactly when
a branch reaches a perform), and xhsH is the adjacent shape it must NOT
disturb — a selector whose VALUE came from the perform, with no perform in
any branch. Guard against over-freeze regression.
