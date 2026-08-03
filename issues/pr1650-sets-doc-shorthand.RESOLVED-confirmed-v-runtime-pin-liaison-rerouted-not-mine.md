# PR #1650 review comment — spec/semantics/19-sets.sexp (corpus-bugfix) — MERGED, fix-forward

https://github.com/camshaft/cadenza/pull/1650 (MERGED — CHAMP hash-collision-node Set/Map cases).

## "wrong-entry" example "21 not 12" reads inconsistently with the full result formula (Copilot, 19-sets.sexp) — doc/clarity
> The "wrong-entry" numeric example is inconsistent with the actual result formula. If lookups for `a`
> and `b` were swapped, the expression would evaluate to 221 (100*2 + 10*2 + 1), not "21 not 12".

The Map collision-node case doc says: "…a scan matching the wrong entry would return 21 not 12). Result:
100·len + 10·(m[a]) + m[b] = 100·2 + 10·1 + 2 = 212." The "21 not 12" is a shorthand for JUST the low
digits (10·m[a]+m[b]: 12 correct, 21 if the lookups swap) — which is arithmetically fine in isolation.
But juxtaposed with the full 212 formula it misleads: a reader expects the FULL swapped result, which is
221 (100·2 + 10·2 + 1), not 212. Copilot's arithmetic checks out. Reword to "would return 221 not 212"
(full result) OR clarify it's the 10·a+b sub-term ("the low two digits would be 21 not 12"). LOW/doc-
clarity, fix-forward. (corpus .sexp rationale — the fix-forward can ride the next 19-sets edit per the
no-standalone-polish steer.)
