(case "sb1 def REBINDING inside a do under a handle: later defs shadow earlier, performs interleaved"
  (input  (do
            (effect St (op a (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St n
                ((a (u) s (resume s (+ s 1))))
                (do
                  (def x (St.a))
                  (def y (+ x 100))
                  (def x (St.a))
                  (+ (* 10 x) y))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 165 Int64)))
