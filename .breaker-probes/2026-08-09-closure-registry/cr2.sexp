(case "cr2 a fold whose ACCUMULATOR is a closure (function composition by folding)"
  (input  (do
            (def (compose-all (: fs (List (-> Int64 Int64))) (: acc (-> Int64 Int64)))
              (match fs
                ((list) acc)
                ((list h .. t) (compose-all t (fn ((: x Int64)) (h (acc x)))))))
            (def (main (: k Int64))
              (do
                (def pipeline (compose-all (list (fn ((: x Int64)) (+ x 1))
                                                 (fn ((: x Int64)) (* x 2))
                                                 (fn ((: x Int64)) (- x 3)))
                                           (fn ((: x Int64)) x)))
                (pipeline k)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 9 Int64))
  (call   main (: 0 Int64)) (output (: -1 Int64)))
