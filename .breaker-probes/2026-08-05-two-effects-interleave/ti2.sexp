(case "ti2 interleave INSIDE recursion: each iteration performs BOTH effects (10 iterations)"
  (input  (do
            (effect A (op a (-> Unit Int64)))
            (effect B (op b (-> Unit Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc (loop (- n 1) (+ acc (+ (A.a) (B.b))))))
            (def (main (: k Int64))
              (handle A 0
                ((a (u) s (resume s (+ s 1))))
                (handle B 100
                  ((b (u) t (resume t (+ t 10))))
                  (loop k 0))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 1495 Int64)))
