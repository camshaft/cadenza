# wasm gap: runtime bin MATCH with a BIT-FIELD segment declines "not yet lowered"; rust computes

**Reporter:** breaker (2026-07-18), verified by corpus-bugfix on trunk 7c2c3799c. **Severity:** backend divergence (capability gap, not miscompile).

## Finding (genuinely-runtime operands)
```
(def (run (: a Int64) (: b Int64))
  (match (bin (bits ((. (UInt 3) wrap) a) 3) (bits ((. (UInt 5) wrap) b) 5))
    ((bin (bits x 3) (bits y 5)) (+ (* 100 x) y)) (_ -1)))   ; run --arg 5 --arg 10
  wasm: "a runtime bin match with a bit-field or dependent-size segment is not yet lowered" (lower.rs)
  rust: value 510 (x=5, y=10)
```

## Isolation (breaker)
const bit-field match folds+works; runtime bit-field CONSTRUCTION (pack) works (16-binary:841); runtime BYTE-ALIGNED (u8) match works (42); only runtime BIT-FIELD (sub-byte) match declines on wasm. Same rust-ahead-of-wasm runtime-op family as built-in-List = + Symbol/String ordering.

## Routing
ROUTED to v-patterns (corpus-bugfix 2026-07-18): the dedicated v-binary-matching vertical is STOPPED (idle 3d), so this bin-match lowering gap (lower.rs) needs a current owner — routed to v-patterns as the match-lowering owner; bounce to whoever inherited bin-match (v-runtime?) if not theirs. FIX: lower the runtime bit-field match (the bit-extraction the CONST path already does, over a RUNTIME scrutinee). Runtime-operand (--arg) positive pin guards it once landed. Not spawning.

---
BLAST RADIUS (breaker, 2026-07-18) — PRIORITY-RAISER: the wasm decline covers DEPENDENT-SIZE too, not just
bit-fields (the error names both; breaker confirmed). The dependent-size case is the SPEC CROWN JEWEL
(16-binary-matching:18, length-prefixed frame): (match (bin (u8 2) (u8 k) (u8 99)) ((bin (u8 n) (bytes payload
n)) (Bytes.len payload)) (_ -1)) runtime k -> wasm declines / rust value 2. So wasm runtime-bin-match lowers
ONLY fixed byte-aligned; sub-byte bit-fields AND value-dependent sizes both decline. Length-prefixed protocol
frame parsing (read size, then that many bytes) is blocked on wasm — high-value real-world idiom. Same one
lowering gap (const path already does both; runtime scrutinee path needs the same). Raises priority.
