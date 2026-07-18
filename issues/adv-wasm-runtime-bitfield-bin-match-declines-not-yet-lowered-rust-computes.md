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

---
FACE A (dependent-size) RESOLVED-PENDING-MERGE (v-patterns, 2026-07-18, MR 9b9d39760): the crown-jewel
length-prefixed-frame parse now lowers on wasm. New Core::BinSizedRead = bytes-slice(scrutinee, off, n) where
n is the runtime BinIntRead of the earlier header segment; arm length-probe = bytes-len == prefix + n; both
backends emit. Verified n=2->2 payload bytes, n=1->fall-through; gate green all 3 backends; graded corpus pin
in 16-binary (--arg driven, runtime). Real protocol-frame parsing (read a size, then that many bytes) works
on wasm now. The priority-context relay (dependent-size first) was validated by v-patterns.
FACE B (BIT-FIELD, sub-byte (bits x k)) STILL OPEN — separate follow-up increment (BinIntRead the byte-run +
shift/mask); v-patterns building it next. Item stays open until Face B lands too.

---
FACE A LANDED + CONTENT-VERIFIED (corpus-bugfix 2026-07-18, trunk c74ec4d0e): Core::BinSizedRead in core.rs.
(bin (u8 2) (u8 k) (u8 99)) matched (bin (u8 n) (bytes payload n)) -> wasm computes Bytes.len 2 (n=2 first
byte, payload next 2 bytes). The crown-jewel dependent-size length-prefixed-frame parse works on wasm now.
The Face B decline message REFINED to "bit-field or NON-FINAL variable-length segment" — confirms FINAL
variable-length (Face A) landed; only BIT-FIELD + NON-FINAL-varlen (Face B) remain. Face B = v-patterns'
next increment (BinIntRead the byte-run + shift/mask). Item stays open until Face B lands.
