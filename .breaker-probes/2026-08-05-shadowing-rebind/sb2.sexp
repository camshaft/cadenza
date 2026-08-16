(case "sb2 a let-bound perform result shadowed by an INNER let (scope nesting under effects)"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (let ((x (St.a)))
                  (+ (let ((x (St.a))) (* 10 x))
                     x))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 65 Int64)))
