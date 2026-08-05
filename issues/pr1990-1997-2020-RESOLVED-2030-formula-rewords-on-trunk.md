# PR #1990 + #1997 review — spec/semantics corpus (breaker) — MERGED — doc-accuracy [VERIFIED, LOW] (batched)

Recurring corpus churn-count off-by-one (same class as #1986 #1978), plus a multi-limb overclaim. All LOW,
breaker's zone (`.sexp`). Batched — a family of doc-count nits worth one sweep.

## #1990 (18-units-of-measure.sexp:2566, id 3710923011): doc says "25 keys" but churn runs i=1..24 = 24 [VERIFIED]
> The docstring says this test churns "25 keys" … but with `n = 25` the `grow`/`shrink` loops run `i =
> 1..(n-1)`, so it actually churns 24 keys.
VERIFIED (diff): `grow`/`shrink` start `i=1`, stop `(if (= i n) …)`, `(call main (: 25))` → i=1..24 = 24
Qty keys. Doc says 25. Reword to "24 keys" (or start i=0 / call with 26).

## #1997 (06-numeric-model.sexp:7168, id 3711171614): churn doc says "30 keys/cycles" but runs i=1..29 = 29 [VERIFIED]
> With `grow`/`shrink` written as `if (= i n)` and called with `i=1`, passing `n=30` performs 29
> insert/remove steps (i=1..29), not the 30 cycles described … call `main` with 31 (so i runs 1..30).
VERIFIED (diff): churn case key `(i+10)·(2^63-1)`, `grow 1 n`, `(call main (: 30))` → i=1..29 = 29 keys.
Doc says 30. Reword to 29, or call with 31 for a true 30. (NOTE: this case's MULTI-LIMB claim is FINE —
`(i+10)·(2^63-1)` with i≥1 means i+10≥11, always multi-limb; only the count is off.)

## #1997 (06-numeric-model.sexp:7131, id 3711171643): "40 ALL-multi-limb keys" but i=1 key `1·(2^63-1)` is SINGLE-limb [VERIFIED]
> This case claims the trie is populated with "ALL-multi-limb" keys, but with the current `i·(2^63-1)`
> construction the first couple of keys (i=1,2) are still within 64 bits and therefore not multi-limb.
VERIFIED (diff): the "40 MULTI-LIMB keys enumerates in magnitude order" case uses `fill n` counting DOWN
i=40..1, key = `(BigInt.of 9223372036854775807) · (BigInt.of i)`. For **i=1**: `1·(2^63-1)` = 2^63-1 =
9223372036854775807, which FITS in a single 64-bit limb (u64 max is 2^64-1). So the i=1 key is NOT
multi-limb — the doc's "every one past the single-limb boundary / ALL-multi-limb" overclaims. (i=2:
2·(2^63-1)=2^64-2, also fits u64 → arguably still single-limb depending on the limb width; i≥2 crosses only
if limbs are <64-bit — breaker knows the BigInt limb width. Either way i=1 is definitely single-limb.)
The pin BEHAVIOR (magnitude-order enumeration) is fine; only the "all multi-limb" framing is wrong. Fix per
Copilot: adjust the key construction so all 40 are guaranteed multi-limb — e.g. `(i+2)·(2^63-1)` or a large
additive offset — and update the formula in the doc. (Count here is fine: i=40..1 = 40 keys.)

All three LOW/doc-accuracy — pins are valid, only prose/framing off. Breaker's call; batchable with the
#1986/#1978 rewords. Corpus/breaker zone.
