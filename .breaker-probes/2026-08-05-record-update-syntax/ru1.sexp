(case "ru1 record REBUILD with one changed field across a perform (field-copy correctness under effects)"
  (input  (do
            (effect St (op v (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((v (u) s (resume s (+ s 1))))
                (do
                  (def r1 (record (x (St.v)) (y (St.v)) (z 100)))
                  (def r2 (record (x (. r1 x)) (y (St.v)) (z (. r1 z))))
                  (+ (* 100 (. r2 x)) (+ (* 10 (. r2 y)) (. r1 y))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 576 Int64)))
