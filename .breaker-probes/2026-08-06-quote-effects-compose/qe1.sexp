(case "qe1 eval of a quoted expression INSIDE a handle body (pure quote, effects around it)"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume s (+ s 1))))
                (+ (St.next) (+ (eval (quote (+ 1 2))) (St.next)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 14 Int64)))
