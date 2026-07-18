# Spec-conformance Q: String.from-bytes does NOT NFC-normalize → construction-path-dependent string identity

**Reporter:** breaker (2026-07-18), verified by corpus-bugfix. **Status:** SPEC AMBIGUITY — both backends AGREE (NOT a miscompile). Awaiting operator/spec RULING (asked concierge).

## Finding
String identity is construction-path-dependent for Unicode normalization:
- String LITERALS normalize at parse (NFD café → NFC, byte-len 5; literal-NFC == literal-NFD → equal). Matches spec.
- `String.from-bytes` does NOT normalize: `(from-bytes (Bytes.of [99 97 102 101 204 129]))` (NFD café) keeps byte-len 6; `(= (from-bytes NFD) (from-bytes NFC))` → **0 (unequal)**. VERIFIED both wasm + rust.

So the same abstract "café" has two identities (literal/normalized vs a from-bytes decode of NFD bytes). Appears to contradict 13-strings.sexp:446-451 ("equality ... exactly when NORMALIZED contents identical") + the canonical-value-form guarantee.

## Two resolutions (operator/spec ruling)
- **(a)** from-bytes SHOULD NFC-normalize on decode → it's a BUG (missing the NFC pass the literal path applies); route to String/runtime normalization owner (v-runtime?).
- **(b)** from-bytes intentionally preserves bytes → the spec's "equality follows normalization" needs a caveat (holds for normalized-construction paths), and pin from-bytes-preserves as intended.

## Routing
ASKED concierge (corpus-bugfix 2026-07-18) for the ruling — it's a spec-intent call, not a fixer job. Will route/pin per the answer. NOT filing a passing/failing corpus case until settled. Repro: `(= (String.from-bytes (Bytes.of (list 99 97 102 101 204 129))) (String.from-bytes (Bytes.of (list 99 97 102 195 169))))` → 0 on both backends.

---
RULED + PINNED (operator via concierge + v-runtime, 2026-07-18): resolution (b) — from-bytes INTENTIONALLY
does NOT normalize (the Unicode NFC tables would bloat the dep-free core). So a from-bytes result is
normalized only if its input was; string identity is construction-path-dependent by design. v-runtime pinned
the documented known-gap case in 13-strings.sexp (MR @04dd367ac): (= (from-bytes decomposed-café-bytes)
(Some café-literal)) = FALSE, documented as INTENDED. The spec's "equality follows normalization" holds for
normalized-construction paths (literals). WORKS-AS-SPECIFIED — no fix, documented gap. Closed.
