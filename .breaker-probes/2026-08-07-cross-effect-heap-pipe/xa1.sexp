(case "xa1 a heap result of effect A pipes directly into effect B's argument — cross-effect heap flow"
  (input  (do
            (effect A (op mk (-> Int64 (List Int64))))
            (effect B (op use (-> (List Int64) Int64)))
            (def (main (: n Int64))
              (handle A 0
                ((mk (k) s (resume (list k (* k 2)) s)))
                (handle B 0
                  ((use (xs) t (resume (+ (* 10 (List.len xs)) (match (List.at xs 1) ((Some v) v) ((None _u) -1))) t)))
                  (B.use (A.mk n)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 30 Int64)))
