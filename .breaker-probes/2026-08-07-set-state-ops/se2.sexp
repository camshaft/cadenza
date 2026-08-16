(case "se2 a membership-GATED arm — first visit admits and records, a revisit answers negated without growing"
  (input  (do
            (effect Sx (op visit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Sx (Set.of (list 3))
                ((visit (v) s (if (Set.contains s v)
                                  (resume (- 0 v) s)
                                  (resume v (Set.insert s v)))))
                (+ (Sx.visit n) (+ (* 10 (Sx.visit 3)) (* 100 (Sx.visit n))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -525 Int64))
  (call   main (: 3 Int64)) (output (: -333 Int64)))
