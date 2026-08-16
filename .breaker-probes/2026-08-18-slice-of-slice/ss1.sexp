(case "ss1 a slice OF a slice over a concat rope composes offsets across the seam"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (Bytes.concat (Bytes.of (list 10 20 30)) (Bytes.of (list 40 50 60 70))))
                (match (Bytes.slice rope 1 5)
                  ((Some outer)
                    (match (Bytes.slice outer 1 3)
                      ((Some inner)
                        (+ (* 100 (Bytes.len inner))
                           (+ (match (Bytes.at inner 0) ((Some v) v) ((None _u) -1))
                              (match (Bytes.at inner 2) ((Some v) v) ((None _u) -1)))))
                      ((None _u) -2)))
                  ((None _u) -3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 380 Int64)))
