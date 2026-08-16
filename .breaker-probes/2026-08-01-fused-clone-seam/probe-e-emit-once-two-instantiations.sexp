; breaker probe E — emit-once memoization vs polymorphic reuse: one generic helper used at TWO
; instantiations (Int64 and a user sum) from multiple call sites; if the memoized per-callee
; emit-once decision leaks the first instantiation into the second, the sum path miscomputes.
; Hand-derived: pick (Some 4) (None) → first Some wins → 4; pick (None) (Some 9) → 9;
;   pickT (W 3) (V) → W wins → 3*2=6... using match to extract.
; main 1 → (+ (pick-int) (pick-sum)) = expected below.
;   a = pick (Some 4) (None unit) → 4
;   b = match (pickt (Wr 5) (Nw unit)) ((Wr w) w) ((Nw u) 0) → 5
;   main → 4 + 5 = 9. Second call flips choices: a2 = pick (None unit) (Some 7) → 7; b2 = Nw → 0 → 7.

(case "one generic chooser reused at scalar and user-sum instantiations"
  (input  (do
            (type Tk (Wr Int64) (Nw))
            (def (pick a b) (match a ((Some x) x) ((None u) (match b ((Some y) y) ((None v) 0)))))
            (def (pickt p q) (match p ((Wr w) (Wr w)) ((Nw u) q)))
            (def (main (: n Int64))
              (if (> n 0)
                (+ (pick (Some 4) (None unit))
                   (match (pickt (Wr 5) (Nw)) ((Wr w) w) ((Nw u) 0)))
                (+ (pick (None unit) (Some 7))
                   (match (pickt (Nw) (Wr 8)) ((Wr w) w) ((Nw u) 0)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: 15 Int64)))
