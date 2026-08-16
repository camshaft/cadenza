(case "as4 decisive: read B's state after the state-slot perform (did the VALUE land?) + A after (did the ADVANCE land?)"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle A n
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((step (u) t (resume t (+ t (A.get))))
                   (peek (u) t (resume t t)))
                  (+ (* 100 (B.step)) (+ (* 10 (B.peek)) (A.get))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 56 Int64)))
