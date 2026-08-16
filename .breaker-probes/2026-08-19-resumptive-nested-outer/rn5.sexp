(case "rn5 accum-DISABLED variant: TREE recursion (two self-calls) — does the drop survive without accum?"
  (input  (do
            (effect A (op tick (-> Unit Int64)) (op get (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (tree (: n Int64))
              (if (= n 0) 0 (+ (B.step) (+ (tree (- n 1)) (tree (- n 1))))))
            (def (main)
              (handle A 10
                ((tick (u) s (resume s (+ s 1)))
                 (get (u) s (resume s s)))
                (handle B 0
                  ((step (u) t (resume (A.tick) t)))
                  (+ (tree 1) (A.get)))))
            (export main)))
  (output (: 21 Int64)))
