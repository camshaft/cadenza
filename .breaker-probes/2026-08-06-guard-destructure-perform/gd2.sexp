(case "gd2 the guard-MISS path re-performs in the fallback arm — dispatch continues past a failed guard"
  (input  (do
            (effect St (op pair (-> Unit (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((pair (u) s (resume (tuple s (* s 2)) (+ s 1))))
                (match (St.pair)
                  ((guard (tuple a b) (> (+ a b) 100)) (+ (* 100 a) b))
                  ((tuple a b) (match (St.pair) ((tuple c d) (+ (* 10 (+ a b)) (+ c d))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 168 Int64)))
