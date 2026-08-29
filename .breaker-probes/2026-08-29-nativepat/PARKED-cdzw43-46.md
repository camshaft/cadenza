# PARKED: cdzw43-46 native-pattern fences — blocked on ML-surface pattern-position support
All 4 cases verified green on wasm+rust gates but FAIL the cadenza-syntax ML corpus roundtrip:
- #map pattern → "round-trip via ml errored" (printer/reader can't express it)
- #tuple/#list patterns → "not idempotent" (ML prints `tuple(a, b)` which re-reads as the CLASSIC spelling)
Re-pin verbatim (cases in git history of this bank commit) when v-ast-compound lands ML pattern-position native forms.
