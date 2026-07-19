; BREAKER FINDING 2026-07-17 — a DIFFERENTIAL MISCOMPILE (wrong VALUE, not a decline) on the
; RUST + RUST-ASYNC backends: the runtime shift-count guard truncates the count to u32 BEFORE
; checking it, so any out-of-range count that is ≡ a small value mod 2^32 slips past the guard
; and silently shifts by the masked amount. Wasm traps correctly (its runtime count guard checks
; the full i64).
;
; The emitted rust guard (backend/rust/expr.rs ~:2726 for >>, ~:2759 for <<):
;     { let v: i64 = n; let c = (s) as u32; if c >= 64 { panic!("shift count out of range") } ... }
; `(s) as u32` KEEPS ONLY THE LOW 32 BITS. So:
;     (<< 5 4294967296)   ; count = 2^32,   low32 = 0  -> guard sees 0  -> 5 << 0  = 5   (should TRAP)
;     (<< 5 4294967299)   ; count = 2^32+3, low32 = 3  -> guard sees 3  -> 5 << 3  = 40  (should TRAP)
;     (>> 20 4294967298)  ; count = 2^32+2, low32 = 2  -> guard sees 2  -> 20 >> 2 = 5   (should TRAP)
;     (<< 5 -4294967296)  ; negative count, low32 = 0  -> guard sees 0  -> 5       (should TRAP)
; Verified 2026-07-17 on trunk 1c255812b: rust + rust-async emit the same `as u32` guard; a rustc'd
; driver returns the wrong values above with rc=0. The WASM backend traps (unreachable) on every one
; of these inputs — a backend differential on the same source + args.
;
; Spec: numeric-model.md #Overflow Is Defined — "a shift count outside the type's bit width has no
; defined value"; corpus 06-numeric-model.sexp pins the genuinely-runtime counts 64 / -1 / 65 (all
; trap), but every pinned count fits in u32's low bits unchanged, so the truncation face was never
; exercised. Counts 64/65/-1 DO hit the rust guard correctly (-1 as u32 = 4294967295 >= 64 — the
; negative face is caught by ACCIDENT of the unsigned read, but only when the low 32 bits are big).
; The sharpest face is a count that is a multiple of 2^32: guard sees 0, the shift is a NO-OP, and
; the program returns its input as a plausible in-range value. No trap, no wrong-looking output.
;
; FIX direction (fix agent): guard on the UNTRUNCATED count — e.g.
;     let c64 = (s) as i64; if !(0..64).contains(&c64) { panic!("shift count out of range") }
;     let c = c64 as u32; ...
; (or compare in u64). Same fix in the `>>` arm, the `<<` guarded arm, and the rust-async twin
; (same emit function). The wasm backend's runtime count guard is the reference behavior.
;
; NOTE the known rust-target classifier gap ("shift count out of range" panic message doesn't map
; to a canonical trap kind — grades todo, not fail): that gap only affects counts the guard CATCHES.
; This case's counts are ones the guard MISSES — the artifact returns a VALUE where a trap is
; required, which is a fail on any classifier.
(case "a genuinely-runtime shift count that is a multiple of 2^32 traps rather than truncating to zero"
  (doc    "`(<< x n)` / `(>> x n)` with the count supplied at the call boundary as 2^32 (= 4294967296).
           Out of range 0..=63, so it MUST trap. A guard that first casts the count to u32 sees 0 and
           lets the shift through as a no-op, returning x unchanged — a silently wrong VALUE. Pins the
           guard comparing the FULL-WIDTH count, not its low 32 bits. wasm: traps (correct); rust +
           rust-async: returns x (miscompile).")
  (input  (do (def (main (: x Int64) (: n Int64)) (<< x n)) (export main)))
  (call   main (: 5 Int64) (: 4294967296 Int64))
  (trap   "unreachable"))
