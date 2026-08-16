(case "hr2 a handle whose result is a CLOSURE capturing perform results, applied after the handle exits"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (do
                (def f (handle St n
                         ((a (u) s (resume s (+ s 1))))
                         (do
                           (def x (St.a))
                           (def y (St.a))
                           (fn ((: k Int64)) (+ (* k x) y)))))
                (+ (f 10) (f 100))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 562 Int64)))
