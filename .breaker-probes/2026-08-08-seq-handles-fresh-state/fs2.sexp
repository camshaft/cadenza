(case "fs2 two SEQUENTIAL handles where the second's SEED is computed from the first's RESULT — regions chained by value"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((r1 (handle St n
                          ((next () s (resume s (+ s 1))))
                          (+ (St.next) (St.next)))))
                (handle St (* r1 2)
                  ((next () s (resume s (- s 3))))
                  (+ (St.next) (* 10 (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 212 Int64))
  (call   main (: 0 Int64)) (output (: -8 Int64)))
