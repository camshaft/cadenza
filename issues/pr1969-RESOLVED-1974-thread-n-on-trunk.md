# PR #1969 review — spec/semantics/19-sets.sexp (breaker) — OPEN — corpus consistency [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1969 (2-pin trie-scale set identity — intersection). Copilot (id
3709863329) flags `drop-half` hard-codes 100 while `main` is parameterized by `n`. Corpus zone (`.sexp`).

## `drop-half` stops at `> 100` (hard-coded) but `main(n)` builds `full = (build n …)` — the removal range is decoupled from the constructed set size (Copilot, 19-sets.sexp:1005 & :1020) — corpus consistency [VERIFIED, LOW]
> `drop-half` is hard-coded to stop at 100, while `main` accepts a parameter `n` and uses it to build
> `full`. This makes the case input misleading and easy to accidentally break if the call ever changes
> (and it's inconsistent with the doc's "100-element" being driven by `n`). Consider threading `n`
> through `drop-half` so the removal range matches the constructed set size.

VERIFIED in the diff:
  `(def (drop-half (: i Int64) (: s (Set Int64))) (if (> i 100) s (drop-half (+ i 2) (Set.remove s i))))`
  `(def (main (: n Int64)) … (def full (build n (Set.of (list)))) (def odds (drop-half 2 full)) …)`
  `(call main (: 100 Int64))`
`main` builds `full` from `n`, but `drop-half`'s stop bound is the literal `100`, not `n`. It happens to be
correct because the sole call passes `n = 100` — but the removal range and the build size are decoupled:
change the call's `n` and `drop-half` would still stop at 100, either under-draining (leaving evens above
100·… no, 2-stepping to 100) or mismatching a larger `full`, silently changing what the case exercises.
LOW/corpus-consistency — the pin is correct as-called; the coupling is fragile. Fix per Copilot: thread
`n` into `drop-half` (`(if (> i n) s …)`, pass `n` down) so the removal range tracks the constructed size.
Breaker's call — `.sexp` corpus zone; batchable, no urgency (PR still open, so could fold pre-merge). If
breaker prefers the hard-coded literal for a fixed-100 pin, add a comment noting the intentional decoupling.
