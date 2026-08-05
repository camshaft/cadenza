# PR #1986 review — spec/semantics/03-equality-and-observation.sexp (breaker) — MERGED — doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1986 (2-pin Rational keys at trie depth). Copilot (id 3710680757)
flags the docstring's key count is off by one vs the loop bounds. Corpus zone (`.sexp`).

## docstring says "30 keys INSERTED/REMOVED" and "Three spellings of every value", but with `main(: 30)` the loops run i=1..29 (29 keys) and only the survivor is probed as `2/4` (Copilot, 03-equality-and-observation.sexp:352) — doc-accuracy [VERIFIED, LOW]
> The docstring claims "30 keys are INSERTED … and REMOVED …" and "Three spellings of every value in
> play", but with `main (: 30 Int64)` the loops stop at `i = n`, so only 29 churn keys (i=1..29) are
> inserted/removed, and only the survivor `1/2` is additionally probed as `2/4`. This makes the test
> explanation misleading even though the code looks fine.

VERIFIED in the diff: `grow`/`shrink` start at `i = 1` (`shrink 1 n (grow 1 n direct)`) and stop at
`(if (= i n) m …)`, so for `n = 30` they iterate i = 1..29 → **29** inserts/removes, not 30. And the "three
spellings of EVERY value" is loose: the churn keys get two spellings (insert `2i/6`, remove `i/3`); only
the seeded survivor `1/2` gets the third (`2/4` probe). The pin BEHAVIOR is correct (normalization-identity
churn holds either way); only the docstring's count + "every value" phrasing overclaim. LOW/doc-accuracy.
Fix (breaker's call): reword to "29 keys are churned (i=1..n-1)" — or, if a round 30 is wanted, start the
loops at `i = 0` (i=0..29 = 30 keys, with `Rational.of 0 6 = 0/6` a valid extra key) so the count matches
the prose. And soften "three spellings of every value" to "three spellings of the survivor; two for each
churn key". Batchable with any other 03-equality touch. Corpus/breaker zone.
