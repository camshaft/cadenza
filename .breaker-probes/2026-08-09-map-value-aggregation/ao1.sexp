(case "ao1 a recursive KEY WALK sums the values of a draw-built map — collection aggregation after construction"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (sumk (: m (Map Int64 Int64)) (: k Int64) (: acc Int64))
              (if (> k 3)
                  acc
                  (sumk m (+ k 1) (+ acc (match (Map.lookup m k)
                                           ((Some v) v)
                                           ((None) 0))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 4)))
                 (probe () s (resume s s)))
                (let ((m (Map.insert (Map.insert (Map.insert (map) 1 (E.next)) 2 (E.next)) 3 (E.next))))
                  (+ (* 10 (sumk m 1 0)) (- (E.probe) n)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 192 Int64))
  (call   main (: 0 Int64)) (output (: 132 Int64))
  (call   main (: -5 Int64)) (output (: -18 Int64)))
