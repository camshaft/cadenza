(case "ac2 a closure-driver result feeds a perform's ARGUMENT"
  (input  (do
            (effect St (op log (-> Int64 Int64)))
            (def (apply-twice f (: a Int64)) (+ (f a) (f (+ a 1))))
            (def (main (: n Int64))
              (handle St 0
                ((log (v) s (resume (* v 10) (+ s 1))))
                (St.log (apply-twice (fn ((: x Int64)) (* x 2)) n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 220 Int64)))
