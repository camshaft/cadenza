(case "av1 the inner arm ABORTS with an OUTER draw as the abort value — the escaping value performs on the way out"
  (input  (do
            (effect O (op next (-> Int64)))
            (effect Bail (op out (-> Int64)))
            (def (main (: n Int64))
              (handle O n
                ((next () s (resume s (+ s 1))))
                (+ (* 100 (handle Bail 0
                            ((out () t (O.next)))
                            (+ (Bail.out) 999)))
                   (* 10 (O.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 560 Int64))
  (call   main (: 0 Int64)) (output (: 10 Int64))
  (call   main (: -2 Int64)) (output (: -210 Int64)))
