(case "tr1 a TUPLE snapshot resume value — each dispatch returns (state, state*10), two snapshots differ by the stride"
  (input  (do
            (effect St (op snap (-> (Tuple Int64 Int64))))
            (def (main (: n Int64))
              (handle St n
                ((snap () s (resume (tuple s (* s 10)) (+ s 1))))
                (match (St.snap)
                  ((tuple a b) (match (St.snap)
                                 ((tuple c d) (+ (* 1000 a) (+ (* 100 b) (+ (* 10 c) d)))))))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 4060 Int64))
  (call   main (: 0 Int64)) (output (: 20 Int64)))
