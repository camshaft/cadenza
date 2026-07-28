; FINDING (breaker, 2026-07-28): Set.to-list over FLOAT-LEAF TUPLE elements returns an EMPTY
; LIST on wasm — SILENT DATA LOSS, not a decline. rust computes the full enumeration (33).
;
;   (Set.of (list (tuple 1.5 1) (tuple 2.5 2) (tuple -1.0 3))):
;     Set.len            -> 3 on BOTH backends (the set itself is fine)
;     Set.to-list        -> wasm: [] (List.len 0)   rust: 3 elements ✔
;   2-element float-tuple set: to-list ALSO [] on wasm. NaN irrelevant (found chasing a NaN
;   probe; plain positive floats reproduce). INT-leaf tuples enumerate fine (the :1466 pin).
;   BARE float sets enumerate fine (the :1494 float-arm pin).
;
; This is the CROSS of the two fixed to-list families: compound elements (tuple arm, fixed) x
; float leaves (compare_scalar_leaf Float arm, fixed) — the COMPOSITION float-INSIDE-tuple takes
; a third path that silently yields [] instead of either sorting or declining. Worse than the
; historical false-DECLINES this family had: it returns a VALUE (empty) — any fold over the
; enumeration silently processes nothing.
;
; GRADED REPRO (= fix pin; rust passes 33 today, wasm answers 30):
(case "Set.to-list enumerates float-leaf tuple elements (compound x float-leaf composition)"
  (input  (do
        (def (main (: x Float64))
          (+ (* 10 (Set.len (Set.of (list (tuple 1.5 1) (tuple 2.5 2) (tuple -1.0 3)))))
             (List.len (Set.to-list (Set.of (list (tuple 1.5 1) (tuple 2.5 2) (tuple -1.0 3)))))))
        (export main)))
  (call   main (: 0.0 Float64)) (output (: 33 Int64)))
