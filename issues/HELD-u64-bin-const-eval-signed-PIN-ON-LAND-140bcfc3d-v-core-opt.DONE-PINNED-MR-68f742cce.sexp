;; WIDTH PERIMETER (breaker #30, informational): the const-eval signedness bug is U64-SPECIFIC —
;; const u32 [128,0,0,1] folds (> m 5)=1 and ((m%1000)%10)=9 unsigned, u16 [128,1] compares right
;; (combined witness 191, all 3 targets). Sub-64 widths have i64-carrier headroom and route correctly;
;; only u64 saturates the i64 carrier. So the fix must be u64-fold/re-materialization-specific, NOT a
;; change to the general const bin-segment path. breaker banked i64-sign-agreement + u32/u16 perimeter
;; pins to guard against overcorrection on either axis.

;; ROOT LOCATED (v-core-opt, confirmed mine — const twin of 7ff56255f): two seams in lower.rs —
;; bin_match_decode (~23788) decodes a u64 int segment as `val as i64` (top-bit-set u64 → NEGATIVE
;; i64; BinDecoded::Int is typed i64, lossy for a wide u64); decode_bin_field (~23903) re-wraps that
;; negative i64 via Core::ConstInt(IntValue::from_i64(*n)). F1/F2/F3 all flow from the negative fold.
;; v-core-opt builds this once ae24405b6 lands (serializing — same lower.rs fold region, one MR at a
;; time). Pin HELD until the const-eval fix lands.

;; HELD PIN (corpus-bugfix, 2026-07-28) — do NOT land until v-core-opt fixes the CONST-EVAL u64-bin
;; binding (const-fold twin of 7ff56255f). Origin: breaker FINDING (issue 000000017224). CONFIRMED
;; on trunk 7f19bc1ca: [main 1] const face expected 1, ran → 0. The runtime fix 7ff56255f is good
;; (809/905), but the CONST-FOLD path still binds the (bin (u64 n)) segment through signed Int64.
;; 3 faces (breaker): F1 const (> m 5u64) folds to 0 (runtime twin=1); F2 const (% m 1000u64) →
;; CDZ0304 'constant arithmetic overflows' (817 fine unsigned); F3 two const u64 matches → bogus
;; CDZ0302 'integer literal does not fit its width' (folded value re-materialized at signed width).
;; FIX SEAM: mirror the binding-type fix in the const evaluator + folded-UInt64 literal re-mat.
;; ON FIX: rebuild cdz; gate x3 → 1/1 (this case); pin into 06-numeric-model.sexp beside the runtime
;; u64-bin pin (once that lands too); baseline x3; roundtrip + silent-omission + --check; MR.

(case "a constant-folded u64 bin binding with the top bit set compares unsigned"
  (input  (do
        (def (main (: mode Int64))
          (if (= mode 1)
              (match (Bytes.of (list 128 0 0 0 0 0 0 9))
                ((bin (u64 m)) (if (> m (: 5 UInt64)) 1 0))
                (_ -2))
              (match (Bytes.of (list (UInt8.wrap (* mode 64)) 0 0 0 0 0 0 9))
                ((bin (u64 m)) (if (> m (: 5 UInt64)) 1 0))
                (_ -2))))
        (export main)))
  (call   main (: 1 Int64)) (output (: 1 Int64))
  (call   main (: 2 Int64)) (output (: 1 Int64)))
