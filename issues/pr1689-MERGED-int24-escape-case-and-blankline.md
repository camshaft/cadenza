# PR #1689 review comments — spec/semantics/06-numeric-model.sexp (corpus-bugfix) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1689 (MERGED; author corpus-bugfix verified).

## 1. Int24 min/-1 case doesn't exercise the claimed Int64-escape (Copilot, 06-numeric-model.sexp:4145) — test-precision [VERIFIED]
> The case claims to pin that Int24 min/-1 overflow traps BEFORE the out-of-range value escapes into
> Int64 arithmetic, but the program does `(+ bad ((. (Int 24) wrap) 0))` while `bad` is still Int24 — so
> any failure-to-trap would be caught by Int24 checked `+` (or a later conversion), not specifically an
> Int64-arithmetic escape.

VERIFIED the mismatch: `bad` stays Int24 through the `+`, so the case exercises Int24-checked-add, NOT the
Int64-escape it describes. To pin the claim, the post-overflow value must actually flow into Int64
arithmetic (widen `bad` to Int64 first, or add it to an Int64 operand) so a missed trap would be observable
as an Int64 result. Otherwise narrow the docstring to what it pins (Int24 checked +). MED/test-precision.

## 2. Missing blank line between adjacent `(case …)` forms (Copilot, 06-numeric-model.sexp:4126) — style
> Add a blank line between adjacent `(case …)` forms for consistency (most cases here are blank-separated).

LOW/style. Fold both into the next 06-numeric edit per the no-standalone-polish steer.
