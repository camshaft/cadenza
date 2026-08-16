(case "ac1 a pipeline chain alternating a 1-ary and a 2-ary op — each result feeds the next call's argument while the thread advances"
  (input  (do
            (effect E (op inc (-> Int64 Int64)) (op mix (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((inc (x) s (resume (+ x s) (+ s 1)))
                 (mix (x y) s (resume (+ (* x y) s) (+ s 2))))
                (E.inc (E.mix (E.inc 3) 10))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 94 Int64))
  (call   main (: 0 Int64)) (output (: 34 Int64))
  (call   main (: -3 Int64)) (output (: -2 Int64)))
