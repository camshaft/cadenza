# 2026-08-12 multimap handler state — FINDING #21 (invalid wasm)

- `mml1.sexp` — the full multimap probe: (Map Int64 (List Int64)) buckets, put answers
  bucket length, total sums a bucket (present keys + absent-key 0). Hand-modeled
  11216060 / 11224140. **HELD BACK from corpus**: trips finding #21 on wasm
  (rust + rust-async both PASS). Pin on fix land.
- `minT-finding21-repro.sexp` — the minimal twin filed to corpus-bugfix
  (queue/adv-handler-arm-two-lookup-matches-computed-perform-key-slot-width-alias-invalid-wasm.sexp).

## Finding #21 boundary map (all gated on trunk 234c320ed)
| variant | shape | verdict |
|---|---|---|
| minT | computed key `(+ n 1)`, arm has state-match + value-match, Map producers | **FAIL invalid wasm** |
| minL/minQ | ONE put, computed key, helper-match both sites | **FAIL** |
| minH/minK | scalar `(Map Int64 Int64)` value | **FAIL** (not about List payloads) |
| minI | same-key `n` both puts | pass |
| minJ | literal key `9` | pass |
| minW | key hoisted `(let ((k2 (+ n 1))))` | todo (fold declines) |
| minU | only state-match | pass |
| minV | only value-match | pass |
| minR/minS | helper without match / one match total | pass |
| minX | if-built Option instead of Map.lookup | todo |

Root evidence: `wasm-tools validate` → func 10 `type mismatch: expected i64, found i32
(at offset 0x343)`; `local.tee 5` on an i32 Option-handle if-result, but local 5 is i64 —
it's also the checked-add scratch for the re-materialized computed perform key in the arm.
