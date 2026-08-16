(case "cc1 a CLOSURE built inside a handle body captures a perform RESULT and runs after another perform"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (do
                  (def first (Cnt.bump))
                  (def f (fn ((: x Int64)) (+ x first)))
                  (+ (f (Cnt.bump)) (* 100 (f 0))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 511 Int64)))
