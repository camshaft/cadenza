(case "ti3b control: same arm-cross WITHOUT recursion (2 direct performs)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (main (: k Int64))
              (handle B 100
                ((b (u) t (resume t (+ t 10))))
                (handle A 0
                  ((a (u) s (resume (+ s (B.b)) (+ s 1))))
                  (+ (A.a) (A.a)))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 211 Int64)))
