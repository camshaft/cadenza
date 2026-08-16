(case "tr1 an op returning a TUPLE destructured by the body's match (multi-value effect results)"
  (input  (do
            (effect St (op both (-> Unit (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((both (u) s (resume (tuple s (* s 10)) (+ s 1))))
                (match (St.both)
                  ((tuple a b) (+ (* 100 a) (+ b (match (St.both) ((tuple c _d) c))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 556 Int64)))
