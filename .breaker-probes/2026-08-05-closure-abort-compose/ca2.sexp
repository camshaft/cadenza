(case "ca2 the ABORT arm mints a closure as the handle FINAL value, applied after the frame is dead"
  (input  (do
            (effect St (op mk (-> Unit (Tuple (-> Int64 Int64) Int64))) (op halt (-> Unit Int64)))
            (def (main (: n Int64))
              (let ((p (handle St n
                         ((mk (u) s (resume (tuple (fn ((: x Int64)) (+ x s)) 0) s))
                          (halt (u) s (tuple (fn ((: x Int64)) x) s)))
                         (match (St.mk)
                           ((tuple f _z)
                             (do (St.halt)
                                 (tuple f -999)))))))
                (match p
                  ((tuple g w) (+ (g 100) w)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 105 Int64)))
