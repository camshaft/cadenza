(case "ss2 a slice window ENTIRELY inside the second rope segment reads through the offset chain"
  (input  (do
            (def (main (: n Int64))
              (do
                (def rope (Bytes.concat (Bytes.of (list 10 20 30)) (Bytes.of (list 40 50 60 70))))
                (match (Bytes.slice rope 4 3)
                  ((Some w)
                    (+ (* 100 (Bytes.len w))
                       (+ (match (Bytes.at w 0) ((Some v) v) ((None _u) -1))
                          (match (Bytes.at w 2) ((Some v) v) ((None _u) -1)))))
                  ((None _u) -3))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 420 Int64)))
