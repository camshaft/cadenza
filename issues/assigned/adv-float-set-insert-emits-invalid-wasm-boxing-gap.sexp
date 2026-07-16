; ADVERSARIAL FINDING (breaker, 2026-07-16) — 🔴 INVALID-WASM MISCOMPILE: `Set.insert` of a FLOAT
; element (constant -0.0/0.0/1.5 or a runtime Float64 param) into an empty/any set emits an invalid
; component ("failed to compile: wasm[0]::function[5]") from a check-clean program. Float MAP keys
; work (both constant and computed — Map.insert/lookup/len all correct, canonical byte form holds:
; NaN-key unification and -0.0/0.0 key distinctness both verified). The gap is the SET-element
; boxing path for Ty::Float.
;
; REPRODUCER (wasm trunk@513e08556, fresh store):
;   (Set.len (Set.insert (Set.of (list)) a))   with a : Float64 = 1.5   → invalid component
;   (Set.len (Set.insert (Set.of (list)) 1.5)) constant                  → invalid component
;
; ISOLATION:
;   Set.of (list 1.5 2.5) — CONSTANT literal set, no insert       → 2 ✓ (the const fold path is fine)
;   Set.insert of an Int element                                  → ✓ (long-standing coverage)
;   Map.insert of a float KEY (const -0.0/0.0, computed (+ x 1.25)) → ✓ all correct, canonical form holds
;   Set.insert of a NaN / -0.0 / 0.0 / 1.5 / runtime param        → 🔴 invalid component, all shapes
;   (Historical echo: 13-strings pinned the same class for an empty-set STRING element — "the backend
;    defaulted the element box to box-int"; the float element repeats it: the element-boxing gate
;    likely has no Ty::Float arm and emits an f64 where the box op expects i64/i32.)
;
; NOTE: my earlier NaN-dedup probe "passed" (len 1) — misleadingly: single-insert also crashes, so
; that pass came through a different (const-fold?) path; treat set-float behavior as UNRELIABLE until
; the boxing gap is fixed. The map-key faces below are pinned separately as the working control.
;
; SEVERITY: 🔴 invalid component from a valid program (load-time crash, no diagnostic). Float sets
; are a natural shape (deduped measurement values). Graded case Fails (trap where a value is
; expected).

(case "a float element inserts into a set and is counted"
  (doc    "`(Set.len (Set.insert (Set.of (list)) a))` with `a : Float64` — one runtime float element
           inserted into the empty set → len 1. Emits an INVALID COMPONENT instead (the set-element
           boxing path has no Float arm — the same class as the historical empty-set String-element
           box-int bug, now on the float leaf). The constant `Set.of (list 1.5 2.5)` fold works (2),
           and float MAP keys work throughout (canonical byte form verified: NaN-key unification,
           -0.0/0.0 distinctness) — only the runtime set-element insert path is broken. Expected: 1.")
  (input  (do
            (def (main (: a Float64))
              (Set.len (Set.insert (Set.of (list)) a)))
            (export main)))
  (call   main (: 1.5 Float64))
  (output (: 1 Int64)))
