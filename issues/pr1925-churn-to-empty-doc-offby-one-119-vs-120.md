# PR #1925 review comment — spec/semantics/05-compound-types.sexp (breaker) — OPEN

https://github.com/camshaft/cadenza/pull/1925 (2-pin CHAMP churn-to-EMPTY).

## Doc says "120-entry trie" but grow/shrink stop at `i = n` → only 119 entries (Copilot, 05-compound-types.sexp:2143, also :2153) — doc/accuracy [VERIFIED]
> The churn-to-empty case claims a 120-entry map drain, but grow/shrink stop when `i = n`, so with
> `(call main (: 120 ...))` the test inserts/removes keys 1..119 (119 entries). Weakens what the case
> pins at the empty boundary + makes the doc inaccurate. Iterate 1..=n by stopping at `i = n + 1`.
VERIFIED against the diff: `grow`/`shrink` are `if (= i n) m (recurse (+ i 1) n ...)` starting at i=1 —
so n=120 touches keys 1..119 (stops when i reaches 120, exclusive) = 119 entries, not 120. The doc says
"a 120-entry trie". Off by one. NOTE: the churn-to-empty EQUALITY/regrow property (drains to Map.empty,
= holds) is unaffected by the exact count — it drains fully either way — so this is a DOC-accuracy nit,
not a test-correctness bug. Fix: either say "119 entries", pass n=121, or stop the loops at i = n+1 to
genuinely cover 1..=120. LOW/doc — fold into your next 05-compound edit. (Verified author breaker via gh.)
