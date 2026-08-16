(case "rn2 HEAP outer state: the nested-op resume's Map.insert is dropped across the recursion"
  (input  (do
            (effect A (op add (-> Unit Int64)) (op count (-> Unit Int64)))
            (effect B (op step (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 0 (+ (B.step) (loop (- n 1)))))
            (def (main)
              (handle A Map.empty
                ((add (u) s (resume (Map.len s) (Map.insert s (Map.len s) 1)))
                 (count (u) s (resume (Map.len s) s)))
                (handle B 0
                  ((step (u) t (resume (A.add) t)))
                  (+ (loop 2) (A.count)))))
            (export main)))
  (output (: 3 Int64)))
