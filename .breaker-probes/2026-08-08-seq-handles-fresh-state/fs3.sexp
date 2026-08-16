(case "fs3 THREE value-chained regions — each seed is the previous region's result, arms +1 / x2 / -5"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (main (: n Int64))
              (let ((r1 (handle St n
                          ((next () s (resume s (+ s 1))))
                          (+ (St.next) (St.next)))))
                (let ((r2 (handle St r1
                            ((next () s (resume s (* s 2))))
                            (+ (St.next) (St.next)))))
                  (handle St r2
                    ((next () s (resume s (- s 5))))
                    (+ (St.next) (St.next))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 61 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
