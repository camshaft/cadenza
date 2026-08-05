# PR #1978 review — spec/semantics/05-compound-types.sexp (breaker) — MERGED — doc-accuracy [VERIFIED, LOW]

https://github.com/camshaft/cadenza/pull/1978 (2-pin signed-key trie order). Copilot (id 3710305068) flags
the doc claims "byte-identical" but the case checks `Map.to-list` equality, not `=`-canonical identity.
Corpus zone (`.sexp`).

## docstring says the churned survivor is "byte-identical" to the never-grown twin, but the check is `Map.to-list` list-equality + a lookup — not `=`-based canonical identity (Copilot, 05-compound-types.sexp:15680 & :15690) — doc-accuracy [VERIFIED, LOW]
> The docstring claims the churned survivor is "byte-identical" to the never-grown twin, but the current
> check only compares `Map.to-list` results + a lookup. If you want this case to pin canonical/structural
> identity (as the wording suggests), the doc should describe `=`-based identity rather than byte-level
> internals or `Map.to-list` list equality.

VERIFIED in the diff: the doc says "…leave the canonical structure — and the signed enumeration order —
byte-identical to the never-grown twin", but the case body checks `(inc (Map.to-list m) …)` and
`Map.to-list` LIST equality + a `List.at 0` head lookup — it compares the ENUMERATION (to-list order +
contents), not byte-level representation and not `=`-canonical map identity. "byte-identical" overclaims
what's pinned: two maps can enumerate identically via `Map.to-list` without a byte-identical internal
trie, and the case never invokes `=` on the maps. LOW/doc-accuracy — the pin is valid (it correctly pins
enumeration-order stability across churn); only the "byte-identical" wording is imprecise. Fix (breaker's
call): reword to "enumerates identically (same `Map.to-list` order + contents)" — or, if canonical
identity is the intended claim, add a `(= churned direct)` check and keep the wording. Batchable with any
other 05-compound touch. Corpus/breaker zone.
