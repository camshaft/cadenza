# FOLLOW-UP (v-compiler-ml, self): recursive user functions DECLINE (feature-not-yet, not a miscompile)

Found 2026-07-20 (trunk af0a646f7) while conformance-probing user-function module shapes.

## The gap
A recursive `def` (its body calls itself) DECLINES in compiler-ml while the reference RUNS it:

| program (module-wrapped) | run-ml | reference |
|---|---|---|
| `(def (countdown n) (if (< n 1) 0 (countdown (- n 1))))` → `(countdown 3)` | declined | 0 |
| `(def (f n) (if (< n 1) 7 (f (- n 1))))` → `(f 2)` | declined | 7 |
| `(def (sum n) (if (< n 1) 0 (+ n (sum (- n 1)))))` → `(sum 3)` | declined | 6 |
| `(def (fac n) (if (< n 1) 1 (* n (fac (- n 1)))))` → `(fac 5)` | declined | 120 |

NON-recursive user functions all work (nullary helper, 1-param, 2-param, nested calls, chained g→f — verified
`(add 3 4)`→7, `(dbl (dbl 5))`→20, `(g 42)` via `(f x)`→42). It is specifically SELF/mutual reference that declines.

## This is a DECLINE (honest coverage-not-yet), NOT a miscompile
A module with an UNUSED recursive `f` still runs (`(do (def (f n) (f n)) (def (main) 5) (export main))` → 5),
so a self-referential def does not poison the module — only the recursive CALL SITE declines (resolves to nothing).

## Root cause (mechanism understood)
`sread.read-do-def` records the def in the def-table AFTER reading its body: `record-def(t1, nameId, bodyId)`
(sread.cdz ~396). So while the body is being read, the def's OWN name is NOT yet in the def-table → a recursive
call `(f …)` inside the body finds no `def-body-of` entry → declines. Recursion needs:
  1. FORWARD-DECLARE the def name before reading its body (record name→(placeholder/self) so the body's
     self-call resolves), and
  2. a CALL LOWERING that does not infinitely inline a recursive body (today a call inlines the callee body;
     a recursive call would inline forever). Needs a real call/return mechanism (a Core call node + a
     bounded/looping eval), OR eval-db handling a recursive CLet/CApp by NAME rather than by inlining.

## Scope (multi-slice, MY lane — resolve-db + lower-db + eval-db + emit-db)
Genuinely a feature, not a 1-liner: touches the reader (forward-declare), resolve (self-name in scope),
lower (a non-inlining call form for a recursive callee), eval (recursion via the env/def-table, not inlining),
and emit (a wasm `call` to a real function rather than inlined instrs — today emit is nullary-main-only, so a
recursive callee needs a second emitted function + a call opcode). Likely sequence: (a) reader forward-declare
+ resolve self-name → the recursive call RESOLVES (still declines in lower); (b) eval-db recursion by def-table
lookup (interpreter runs it — run-ml goes green); (c) emit a real wasm function + call (run-emitted goes green,
W4 differential covers it). Each gated + reference-checked.

## Why HELD (not started this tick)
A boundary-pins MR (9e963fae9) was pending → couldn't sync to a clean base, and this is too large to start on a
stacked base. Also func[27] fix (a9340242d, v-memory-safety) + the db-records structural-record conversion are
higher-priority unblocks landing imminently. Pick this up on a clean trunk once those clear. Corpus relevance:
recursion is pervasive in the real corpus (fac/sum/fib-style), so this materially widens run-ml conformance.

## UPDATE 2026-07-20: GENERALIZES to FORWARD/MUTUAL references (not just self-recursion)
Same root cause, broader symptom. A def whose body calls a LATER-defined def declines (definition order matters):
  (do (def (g x) (+ x 1)) (def (f x) (g x)) (def (main) (f 5)) (export main))  -> 6   (g BEFORE f: works)
  (do (def (f x) (g x)) (def (g x) (+ x 1)) (def (main) (f 5)) (export main))  -> declined  (g AFTER f: GAP, ref=6)
The reader records each def's name->bodyId AFTER reading its body (read-do-def), so f's body (g x) finds no
def-body-of("g") yet when g is defined later -> declines. The reference is definition-order-INDEPENDENT. This
is the SAME forward-declaration fix recursion needs (self-reference is the f==g case). A clean fix covers BOTH:
PRE-SCAN the do-block's (def (name ...) ...) headers to seed the def-table (name -> reserved body node-id)
BEFORE reading any body, so every call (forward, backward, self) resolves. Recursion then falls out for free.
Scope: sread read-do-form two-pass (collect headers, then read bodies) + the lower inline path needs a
non-inlining call form ONLY for true cycles (forward refs to non-recursive defs inline fine once bodyId is
reserved). Highest-value FEATURE now (definition-order independence + recursion together). Needs a clean
sread/resolve/lower base (blocked on my pending 3-param MRs).
