; FINDING (breaker, 2026-07-21): a CONST-FOLDABLE conditional whose taken branch is a
; Float32-overflowing literal COMPUTES ±inf instead of rejecting — inconsistent with both
; the direct literal (rejects CDZ0302) and the INTEGER analogue (rejects).
;
;   (: 1.0e300 Float32)                      → CDZ0302 (direct literal, correct)
;   (: (if c 1.0e300 0.0) Float32) runtime c → CDZ0302 (ffa733b67's descent, correct)
;   (: (if true 1.0e300 0.5) Float32)        → RUNS, yields inf         ← THE BUG
;   (: (let ((c true)) (if c 1.0e300 0.5)) Float32) → RUNS, yields inf  ← same via let
;   (: (if true 10000 0) UInt8)              → CDZ0302 (int analogue REJECTS taken branch)
;   (: (if false 1.0e300 0.5) Float32)       → RUNS, yields 0.5 (dead-branch skip — matches
;                                              the int dead-branch (: (if false 10000 7) UInt8) → 7)
;
; The taken-branch asymmetry is the defect: the const fold resolves the conditional to the
; literal 1.0e300 FIRST, and the folded value is then converted to f32 WITHOUT re-running the
; width fit-check — materializing ±inf, which ffa733b67's own commit message calls "a malformed
; value with no written form". The integer path re-checks after the fold (or checks before);
; the float path does not. Same on wasm and rust; O0..O3 same outcome.
;
; The dead-branch skip (false → the overflowing literal vanishes before checking) matches the
; established integer behavior, so only the TAKEN-branch face is filed as the bug; if the
; ruling is that BOTH branches of even a const conditional must width-check (the runtime
; descent checks both), the dead-branch face below is the companion repro.

(case "REPRO a const-folded taken branch with a Float32-overflowing literal must reject"
  (input  (do (def (main) (: (if true 1.0e300 0.5) Float32)) (export main)))
  (error  CDZ0302))

(case "COMPANION dead-branch face (currently computes 0.5, matches int precedent — ruling needed)"
  (input  (do (def (main) (: (if false 1.0e300 0.5) Float32)) (export main)))
  (error  CDZ0302))
