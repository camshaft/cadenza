; breaker probe M — a NESTED list pattern in REST position: `(list a .. (list b .. r))` — the rest
; binder is a binder position (Patterns Compose, core-semantics.md:149), so it may be a pattern;
; nested there it is REFUTABLE (fails on the 1-element list whose rest is empty), so the arm set
; needs the shorter-length arms for exhaustiveness.
; Hand-derived: [1,2,3]: a=1, rest=[2,3] ~ (list b .. r): b=2, r=[3] → 100*1+10*2+1 = 121.
;   [1]: rest=[] fails the inner pattern → falls to (list a) arm → a*1000 = 1000.
;   []: → -1. [5,6]: a=5,b=6,r=[] → 500+60+0 = 560.

(case "a nested list pattern in rest position destructures the tail and refutes on empty rest"
  (input  (do
            (def (main (: xs (List Int64)))
              (match xs
                ((list a .. (list b .. r)) (+ (* 100 a) (+ (* 10 b) (List.len r))))
                ((list a) (* a 1000))
                ((list) -1)))
            (export main)))
  (call   main (list 1 2 3)) (output (: 121 Int64))
  (call   main (list 5 6)) (output (: 560 Int64))
  (call   main (list 1)) (output (: 1000 Int64))
  (call   main (list)) (output (: -1 Int64)))
