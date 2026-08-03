; adv-61b (v-core-opt self-found, 2026-08-03, sibling of adv-61) — MED, wrong-value, BOTH backends,
; const-vs-runtime divergence in a FLOAT-WIDTH CONVERSION const-fold (lower_float_of, lower.rs:17265).
;
; (Float64.of (: 0.1 Float32)) const-folds to 0.1 (WRONG) — it promotes the source literal's UN-DEMOTED
; f64 payload (0.1000000000000000055…) instead of the source's binary32 value. The runtime twin
; (Float64.of x) at x=0.1f32 gives 0.10000000149011612 (CORRECT — f32.promote of the real f32 slot).
;
; ROOT: lower_float_of reads `f64::from_bits(d.to_f64_bits())` as the source and rounds to the TARGET
; width, but never demotes to the SOURCE operand's own width first. For a NARROWING/same-width target
; the target round happens to mask it (Float32.of Float64 is fine); for a WIDENING/identity target
; (Float64.of Float32, or Float32.of Float32) the un-demoted f64 payload passes straight through.
; The compare folds had the exact same shape (adv-61) — fixed by demoting each operand to its own width.
;
; FIX (turnkey, v-core-opt): in lower_float_of, demote the source via the SOURCE operand's width before
; rounding to target: `let src_bits = const_float_bits_at_operand_width(db, args[0], d.to_f64_bits());
; let src = f64::from_bits(src_bits);` then round to target as today. Reuses the adv-61 helper.
;
; scope: BOTH backends (shared lower.rs const fold); opt-sweep O0..O3 identical (fold level-independent);
; the runtime face is correct. PIN the const face + runtime control on fix land.
(case "adv-61b Float64.of a Float32 literal promotes the binary32 value not the un-demoted f64 payload"
  (input (do (def (main) (Float64.of (: 0.1 Float32))) (export main)))
  (call main) (output (: 0.10000000149011612 Float64)))
(case "adv-61b runtime control Float64.of x at x=0.1f32 promotes the f32 value"
  (input (do (def (main (: x Float32)) (Float64.of x)) (export main)))
  (call main (: 0.1 Float32)) (output (: 0.10000000149011612 Float64)))
