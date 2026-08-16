(case "gd5 guards over TUPLE pattern binders — the predicate compares the tuple's own components, all three orderings reachable"
  (input  (do
            (effect E (op pair (-> (Tuple Int64 Int64))) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((pair () s (resume (tuple s (* 2 s)) (+ s 3)))
                 (probe () s (resume s s)))
                (+ (* 10 (match (E.pair)
                           ((guard (tuple a b) (< a b)) (+ 100 b))
                           ((guard (tuple a b) (= a b)) 200)
                           ((tuple a b) (+ 300 b))))
                   (- (E.probe) n))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 1083 Int64))
  (call   main (: 0 Int64)) (output (: 2003 Int64))
  (call   main (: -5 Int64)) (output (: 2903 Int64)))
