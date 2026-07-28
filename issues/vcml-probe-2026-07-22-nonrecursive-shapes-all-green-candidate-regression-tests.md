# PROBE (v-compiler-ml, self): 2026-07-22 run-ml conformance sweep — 14 non-recursive shapes ALL GREEN

Read-only run-ml probe this tick (base-pinned behind pending MR dc204e163, so no committed test yet). Goal:
surface any UNKNOWN front-end gap beyond the known recursion gap (Slice B). Result: **no new gap found** —
the front-end is robust across every covered non-recursive shape. Recording the verified shapes as CANDIDATE
regression tests to add to `sread-eval-fns.cdz` (or a sibling) once the base is clean, prioritizing the ones
NOT already pinned.

Binary: `./target/release/cdz run-ml <file.sexp>` (release; ~32s/run). Module form:
`(do (def (main) ...) (export main))`. `let` form is `(let ((x V)) body)` (NOT `(let x V body)`).

| # | program (inside `(do ... (export main))`) | run-ml | notes |
|---|---|---|---|
| A | `(def (main) (let ((x 2)) (+ (* x 3) 1)))` | 7 | milestone (already pinned) |
| B | `(def (main) (let ((x 2) (y 3)) (+ x y)))` | 5 | **multi-binding let** — candidate if unpinned |
| C | `(def (main) (< 3 5))` | true | bool-returning main |
| D | `(def (main) (let ((x 1)) (let ((x 10)) (+ x 5))))` | 15 | **let shadowing** — candidate |
| E | `(def (a) 10) (def (b) 20) (def (main) (if (< 1 2) (a) (b)))` | 10 | nullary calls in if-branches |
| F | `(def (main) (+ (* 2 (+ 3 4)) (- 10 (* 2 3))))` | 18 | deep nested arith |
| G | `(def (add3 a b c) (+ (+ a b) c)) (def (main) (add3 1 2 3))` | 6 | 3-param call |
| H | `(def (dbl x) (* x 2)) (def (main) (dbl (dbl 5)))` | 20 | call-of-call |
| I | `(def (sq x) (* x x)) (def (main) (sq 7))` | 49 | **param used twice** — candidate |
| J | `(def (inc x) (+ x 1)) (def (main) (let ((y 41)) (inc y)))` | 42 | let-bound var as call arg |
| K | `(def (inc x) (+ x 1)) (def (main) (inc (if (< 1 2) 100 200)))` | 101 | **if-expr as call arg** — candidate |
| L | `(def (clamp x) (if (< x 0) 0 x)) (def (main) (clamp (- 0 5)))` | 0 | **param in if-condition** — candidate |
| M | `(def (pos x) (< 0 x)) (def (main) (if (pos 7) 111 222))` | 111 | **bool-returning helper in if-cond** — candidate |

## Highest-value candidates to pin (not currently in the suite, verified green)
- **M** — a helper that RETURNS a Bool, consumed by main's if-condition (`(pos 7)` → the call node types Bool,
  flows into `if`). Exercises the cross-def Bool-type flow through a param call feeding a branch. NOT pinned.
- **L** — a param compared in the callee's OWN if-condition (`(if (< x 0) 0 x)`), arg is a negative computed
  literal. Exercises param→if-cond typing + negative arg.
- **B** — multi-binding `let ((x 2) (y 3))`. If the suite only pins single-binding lets, this widens coverage.
- **D** — let shadowing across nested lets (inner x=10 shadows outer x=1).
- **I** — a param referenced twice (`(* x x)`) — pins that a param CVar can appear multiple times.
- **K** — an if-EXPRESSION passed as a call argument (the arg lowers to a CIf inside the CLet).

## Action (deferred — base-pinned)
When dc204e163 lands and the base is clean, fold these as cheap run-src @tests into sread-eval-fns.cdz — BUT
mind the suite-time cap (sread-eval-fns is the fattest file, ~8min, nearest the 12min per-file gate cap; see
the RUNAWAY-compile note). Prefer adding the 2-3 HIGHEST-value ones (M, L, B) and/or a sibling file rather than
loading all 6 into sread-eval-fns. These are ADDITIVE regression pins (all currently green) — low risk, done
alongside a Slice-B slice or a dedicated small hardening MR. No gap to report to corpus-bugfix (all pass).
