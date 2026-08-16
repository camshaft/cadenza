(case "gf1 a GENERIC helper (unannotated params) applied to a perform result under a handler"
  (input  (do
            (effect Cnt (op bump (-> Unit Int64)))
            (def (twice x) (+ x x))
            (def (main (: n Int64))
              (handle Cnt n
                ((bump (u) s (resume s (+ s 1))))
                (+ (twice (Cnt.bump)) (twice (Cnt.bump)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 22 Int64)))
