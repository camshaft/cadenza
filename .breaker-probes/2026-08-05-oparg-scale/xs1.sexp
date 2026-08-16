(case "xs1 op-arg lift at SCALE: 100-iteration loop each performing (B.put (A.get)) — the xh1 shape recursively"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op put (-> Int64 Int64)))
            (def (loop (: n Int64) (: acc Int64))
              (if (= n 0) acc (loop (- n 1) (+ acc (B.put (A.get))))))
            (def (main (: k Int64))
              (handle A 0
                ((get (u) s (resume s (+ s 1))))
                (handle B 0
                  ((put (v) s (resume v s)))
                  (loop k 0))))
            (export main)))
  (call   main (: 100 Int64)) (output (: 4950 Int64)))
