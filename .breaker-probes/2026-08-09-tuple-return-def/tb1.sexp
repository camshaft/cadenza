(case "tb1 a def RETURNS a tuple of two draws — the caller destructures the multi-value result of a performing helper"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (pair2)
              (tuple (E.next) (E.next)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 3)))
                 (probe () s (resume s s)))
                (match (pair2)
                  ((tuple a b) (+ (* 100 a) (+ (* 10 b) (- (E.probe) n)))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 256 Int64))
  (call   main (: 0 Int64)) (output (: 36 Int64))
  (call   main (: -4 Int64)) (output (: -404 Int64)))
