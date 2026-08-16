(case "hc5 a helper RETURNS a tuple of two draws — the caller destructures it and re-performs"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (pair2) (tuple (St.next) (St.next)))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (* s 2))))
                (match (pair2)
                  ((tuple a b) (+ (* 100 a) (+ (* 10 b) (St.next)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 620 Int64))
  (call   main (: 1 Int64)) (output (: 124 Int64)))
