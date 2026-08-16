(case "en1 an @ensures postcondition on a PERFORMING def (checked against the effectful result)"
  (input  (do
            (effect St (op bump (-> Unit Int64)))
            (@ (ensures (>= ret 100)) (def (f (: x Int64)) (+ x (St.bump))))
            (def (main (: n Int64))
              (handle St 100
                ((bump (u) s (resume s (+ s 1))))
                (+ (f n) (f 2))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 208 Int64)))
