; adv-57 (breaker, 2026-08-02): wasm emits runtime NARROW-width wrapping-add/sub/mul as the raw
; machine op with NO width re-normalization — the un-wrapped value is observable, a wrong-value
; miscompile. rust + rust-async are CORRECT (differential); const folds are correct; Int64
; full-width is correct. All opt levels O0..O3 (always-on emit path, select.rs ~:10934 — the
; comment claims "the result is masked to the width by the ordinary operand/consumer
; normalization", but neither a widening read (Int64.of) nor an =-compare at the narrow width
; re-masks it).
;
; observed (wasm):  UInt8.wrapping-add 250 10 → 260 (spec: 4, modulo 256)
;                   Int8.wrapping-add -128 -1 → observable -129 (= against 127 is FALSE;
;                     widen reads -129); Int8.wrapping-mul -128 -1 → 128 (spec: -128)
;                   Int16.wrapping-add 32767 1 → 32768 (spec: -32768)
; expected:         numeric-model.md #A Wrapping Operation Has A Defined Modular Outcome; the
;                   rust backend returns the modular value on the SAME programs.
; brackets:         const forms fold correctly (compile-time); Int64.wrapping-add MAX 1 → MIN
;                   correct at runtime (the raw i64 op IS the modular op at full width); the bug
;                   is exactly runtime narrow (Int8/UInt8/Int16/...) where the slot is wider than
;                   the type and the emit skips the mask/sign-extend.
(case "adv-57 runtime UInt8 wrapping-add overflow wraps modulo 256 (wasm returns the unmasked 260)"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (UInt8.wrapping-add x (UInt8.wrap 10))))
            (export main)))
  (call   main (: 250 UInt8)) (output (: 4 Int64))
  (call   main (: 5 UInt8)) (output (: 15 Int64)))

; --- ADDENDUM (breaker, 2026-08-02): chained narrow wrapping ops COMPOUND the unmasked error ---
; Pin this face too when the fix lands — it pins the intermediate-feeds-next-op path (each op's
; result must be re-normalized, not just a final consumer read):
;   (UInt8.wrapping-mul (UInt8.wrapping-add x (UInt8.wrap 10)) (UInt8.wrap 2)) at x=250
;     wasm (BUG): 520  (unmasked 260 feeds the mul, product also unmasked)
;     spec:       260 mod 256 = 4; 4 * 2 = 8
;   control x=100 → 220 (passes today; (100+10)=110, *2=220, in-width).
; Sibling narrow emit paths swept CLEAN (breaker): narrow << is checked (traps, both backends),
; Int8 //% at MIN corner correct, xor-all-ones in width, negate MIN traps — adv-57 is ISOLATED
; to the wrapping-* family. Probe: .breaker-probes/2026-08-02-narrow-norm/n5-wrapping-chain.sexp

(case "adv-57 chained UInt8 wrapping ops each re-normalize (wasm compounds the unmasked error to 520)"
  (input  (do
            (def (main (: x UInt8))
              (Int64.of (UInt8.wrapping-mul (UInt8.wrapping-add x (UInt8.wrap 10)) (UInt8.wrap 2))))
            (export main)))
  (call   main (: 250 UInt8)) (output (: 8 Int64))
  (call   main (: 100 UInt8)) (output (: 220 Int64)))

; --- FOLLOW-UP SEAM FACES (breaker, 2026-08-02, adv57-postfix pf1/pf2) — add after 730fb20a5 lands ---
; Prove the MASKED result (not the raw wide value) is what downstream ops consume. Both PASS x3 on
; trunk 19c4b9358; trap reason-match accommodates wasm 'integer overflow' + rust 'integer overflow in
; addition'. VALIDATED by corpus-bugfix (2 pass x3-backend). Land as a follow-up pin (my 5-face pin
; 730fb20a5 is already queued — keep MRs one clean commit).
(case "a UInt8 wrapping-add result feeds a CHECKED add — the MASKED value is what the checked op sees"
  (input  (do
            (def (main (: x UInt8)) (Int64.of (+ (UInt8.wrapping-add x (UInt8.wrap 10)) (: 6 UInt8))))
            (export main)))
  (call   main (: 250 UInt8)) (output (: 10 Int64))
  (call   main (: 245 UInt8)) (trap "overflow"))
(case "an Int8 wrapping-add result drives a comparison — the SIGN-EXTENDED masked value is compared"
  (input  (do
            (def (main (: x Int8)) (if (< (Int8.wrapping-add x (Int8.wrap 1)) (: 0 Int8)) 1 0))
            (export main)))
  (call   main (: 127 Int8)) (output (: 1 Int64))
  (call   main (: 10 Int8)) (output (: 0 Int64)))
