(case "cn2 performs in BOTH operands of an or — the LHS always fires, the RHS only when LHS is false"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (main (: k Int64))
              (handle Ctr k ((tick (u) s (resume s (+ s 1))))
                (+ (if (or (> (Ctr.tick) 10) (> (Ctr.tick) 3)) 100 200)
                   (Ctr.tick))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 121 Int64))
  (call   main (: 4 Int64)) (output (: 106 Int64))
  (call   main (: 0 Int64)) (output (: 202 Int64)))
