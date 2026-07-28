;; ROOT-CAUSE NARROWED (breaker 3rd pass, 2026-07-27): the seam is the (bin (u64 n)) BINDING
;; type/rep, NOT the arithmetic dispatch table. A GENUINE runtime UInt64 (entry param, x*x top-bit
;; set) computes %/÷ CORRECTLY unsigned (907) — so UInt64 ops are not globally signed-routed. Only
;; the bin-segment binding produces a signed-typed/signed-repped n. CORROBORATION: Int64.of over a
;; UInt64 ENTRY PARAM declines cleanly ('runtime checked integer conversion not yet emitted'), but
;; the BIN route silently wraps negative — the bin binding bypasses that decline (mistyped at bind).
;; FIX SEAM: rcdzc lower.rs bin-pattern u64 segment lowering (~:23534/:24054) + resolve.rs — bind n
;; as a genuine UInt64 (unsigned type+rep); downstream %/÷/Int64.of then route correctly for free.

;; HELD PIN (corpus-bugfix, 2026-07-27) — do NOT land until v-core-opt fixes the signed-dispatch on
;; a UInt64 (bin (u64 n)) binding. Origin: breaker FINDING #29 (issues 000000017121 + note 017130).
;; CONFIRMED on trunk d6f27a445 (fresh build): [main 128] expected 809, ran → -807 on BOTH wasm AND
;; rust (identical → shared lowering/typing miscompile, NOT backend emit). SILENT wrong value.
;; SCOPE (breaker): the (bin (u64 n)) binding is UInt64 but downstream sign-sensitive DIVISION ops
;; pick signed variants — % → rem_s (needs rem_u), / → div_s (needs div_u), Int64.of range-check
;; trusts the sign (silently negative instead of trapping). COMPARE is correctly unsigned; +/* are
;; two's-complement sign-agnostic (fine); u32 top-bit twin is CLEAN (fits i64). Working precedent in
;; the SAME dispatch table: rcdzc tests.rs:12751-12806 pin div_u/rem_u/shr_u off the UInt64 type for
;; CONST UInt64 — only the BINDING path loses unsigned-ness at div/rem/narrow. Correct oracle: 809
;; (unsigned rem of 2^63+1 mod 1000). rust is NOT an oracle (both backends wrong).
;; ON FIX (v-core-opt routes the u64-binding div/rem/narrow to unsigned): rebuild cdz; gate x3 →
;; 809 (main 128) + 905 (main 64 control); pin into 06-numeric-model.sexp beside the UInt64/narrow
;; pins; baseline x3; roundtrip + silent-omission + --check; MR; notify v-core-opt + breaker.
;; (Related const face breaker flagged, may share root — track separately if it doesn't fall out:
;;  (+ (: Int64.max UInt64) (: 2 UInt64)) wrongly rejects CDZ0304 'overflows Int64' — const UInt64
;;  add misrouted through Int64 checked-add.)

(case "a u64 bin binding with the top bit set does unsigned arithmetic"
  (input  (do
        (def (main (: x UInt8))
          (do
            (def b (Bytes.of (list x 0 0 0 0 0 0 1)))
            (match b
              ((bin (u64 n)) (Int64.of (% n 1000)))
              (_ -1))))
        (export main)))
  (call   main (: 128 UInt8)) (output (: 809 Int64))
  (call   main (: 64 UInt8)) (output (: 905 Int64)))
