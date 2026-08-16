; breaker probe Q — the {state,step} adapter-record iterator protocol driven MANUALLY through a
; user-composed take-while-then-sum over an unfold counter, mixing TWO adapter records with
; DIFFERENT state types in one program (Int64 counter state + tuple state). The v-iterators
; surface is corpus-thin (module-level giter tests only); this pins the protocol shape at the
; corpus tier: an adapter record {state, step} where step : s -> Option (tuple elem s).
; Hand-derived: counter from 1: yields 1,2,3,... take-while <4 → 1+2+3 = 6 (stops at 4).
;   pair-state adapter: fib-ish (a,b) → yields a, state (b, a+b), from (1,1): 1,1,2,3,5;
;   take-while <=3 → 1+1+2+3 = 7. main = 6*10 + 7 = 67.

(case "two adapter records with different state types drive a take-while fold in one program"
  (input  (do
            (def (sum-while (: st Int64) stepf (: lim Int64) (: acc Int64) (: fuel Int64))
              (if (= fuel 0)
                acc
                (match (stepf st)
                  ((Some p) (match p
                              ((tuple e s2) (if (< e lim)
                                              (sum-while s2 stepf lim (+ acc e) (- fuel 1))
                                              acc))))
                  ((None u) acc))))
            (def (sum-while-t st stepf (: lim Int64) (: acc Int64) (: fuel Int64))
              (if (= fuel 0)
                acc
                (match (stepf st)
                  ((Some p) (match p
                              ((tuple e s2) (if (<= e lim)
                                              (sum-while-t s2 stepf lim (+ acc e) (- fuel 1))
                                              acc))))
                  ((None u) acc))))
            (def (main)
              (+ (* 10 (sum-while 1 (fn ((: s Int64)) (Some (tuple s (+ s 1)))) 4 0 20))
                 (sum-while-t (tuple 1 1)
                              (fn (p) (match p ((tuple a b) (Some (tuple a (tuple b (+ a b)))))))
                              3 0 20)))
            (export main)))
  (output (: 67 Int64)))
