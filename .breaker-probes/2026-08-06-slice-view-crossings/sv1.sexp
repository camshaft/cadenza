(case "sv1 a Bytes.slice VIEW crosses as op ARGUMENT — the arm reads through the window it was handed"
  (input  (do
            (effect St (op sum2 (-> Bytes Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((sum2 (w) s (resume (+ (* 100 (Bytes.len w))
                                        (+ (match (Bytes.at w 0) ((Some a) a) ((None _u) -1))
                                           (match (Bytes.at w 1) ((Some b) b) ((None _u) -1))))
                             s)))
                (match (Bytes.slice (Bytes.of (list 9 20 30 8)) 1 2)
                  ((Some w) (St.sum2 w))
                  ((None _u) -999))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 250 Int64)))
