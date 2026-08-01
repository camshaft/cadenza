# PR#971 review comment — memoized-fib case's memo-size encode truncates via /10 (corpus-bugfix)

Mirrored from GitHub PR#971 review comment (Copilot), id `3694491816`.
File: `spec/semantics/05-compound-types.sexp:17185` — corpus → corpus-bugfix. Blame `0aa12359d`
"corpus(compound): 3-pin drain AP — little-interpreter idioms: memoized recursion, …".

## Comment (verbatim)

- (id 3694491816, 05-compound-types.sexp:17185) "This case's doc says the output 'encodes memo SIZE
  beside the value' and cites '21 entries for fib(20)', but the current computation adds `(/ (Map.len
  memo) 10)`, which truncates integer division (e.g. `(/ 7 2) = 3` in 06-numeric-model) and only
  witnesses the tens digit of the memo size. That weakens the pin (20 and 21 both map to 2) and makes
  the doc misleading. Consider encoding the exact memo size in the output."

## Liaison verification (DERIVED by hand — Copilot correct)

Output: `(+ (* 10 v) (/ (Map.len memo) 10))` with `v = fib(20) = 6765`, pinned `67652`. So `67650 + (/
(Map.len memo) 10) = 67652` ⇒ `(/ (Map.len memo) 10) = 2`. The memoized fib inserts keys 0,1 (base
`n<2`) then n=2..20 → keys 0..20 = **21 entries**, and `21 / 10 = 2` (integer truncation). Copilot is
right: the `/10` means the output only witnesses the TENS digit — `20/10=2` AND `21/10=2` both give 2, so
a memo bug that produced 20 (or any of 20..29) entries would still pass `67652`. The doc claims the output
"encodes memo SIZE" / "21 entries", but it only pins "memo size is in [20,29]" — the exact-21 property is
NOT pinned. Fix (Copilot's, sound): encode the EXACT memo size, e.g. `(+ (* 100 v) (Map.len memo))` (or
`(+ (* 1000 v) (Map.len memo))` for headroom) so 21 is pinned exactly and a 20-entry regression flips the
output. Corpus coverage; the current pin 67652 would change with the encode.

(Note: I DERIVED the arithmetic by hand rather than mirroring Copilot's claim — 6765·10 + 21/10 = 67652,
21 entries = keys 0..20. Confirmed.)

Owner: **corpus-bugfix** (`spec/semantics/05-compound-types.sexp`; `0aa12359d`). Encode the exact memo
size (drop the `/10` truncation) so the "21 entries" property is actually pinned.
