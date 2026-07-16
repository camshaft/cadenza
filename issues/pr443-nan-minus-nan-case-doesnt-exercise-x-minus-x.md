# PR review comment — mirrored from GitHub PR #443 (Copilot inline)

- **PR:** #443 "fleet: sixty-third batch (…, float-never-traps + mixed-scale pins, …)" (MERGED)
- **File:** `spec/semantics/06-numeric-model.sexp:1009`
- **Reviewer:** Copilot (automated)
- **Comment id:** 3592743206
- **Link:** https://github.com/camshaft/cadenza/pull/443#discussion_r3592743206

## Comment (verbatim)
> This case is titled/described as "nan minus nan" and as guarding against a float-specific `x - x = 0` rewrite when `x` is NaN, but the current program computes `(- x Float64.nan)` (a finite minus NaN). That still propagates NaN, but it does not exercise the intended `y - y` identity hazard, so it won't catch the regression the doc explains.

## Liaison triage — CONFIRMED against trunk — vacuous pin (like pr381)
Confirmed: the case doc says it pins that "a runtime float subtraction with NaN operands propagates NaN
rather than applying the `x - x = 0` integer identity", but the program is
`(if (= (- x Float64.nan) Float64.nan) 1 0)` called with `x = 5.0` — i.e. `(- x nan)` = finite MINUS
nan, NOT `x - x`. So it never feeds the `x - x` (same-operand) form that would trigger the `x-x=0`
identity rewrite → a wasm-opt that wrongly applied `x-x=0` to floats would NOT be caught by this case.
Same "vacuous pin" class as pr381's untaken-arm trap test. FIX: make the subtraction a genuine
same-operand `(- x x)` with `x = Float64.nan` (or bind `let y = nan in (- y y)`) so the `x-x` identity
hazard is actually exercised. The `x-x=0` identity rewrite is v-wasm-opt's arith surface, but this is a
corpus test-coverage fix → route to `corpus-bugfix` PM. Fix on `trunk`. Quote + link in queue file.
