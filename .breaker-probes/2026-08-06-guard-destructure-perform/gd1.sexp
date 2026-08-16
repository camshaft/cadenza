(case "gd1 a guard destructures a perform-result TUPLE and its condition reads both binders"
  (input  (do
            (effect St (op pair (-> Unit (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((pair (u) s (resume (tuple s (* s 2)) (+ s 1))))
                (match (St.pair)
                  ((guard (tuple a b) (> (+ a b) 10)) (+ (* 100 a) b))
                  ((tuple a b) (+ a b)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 510 Int64)))
