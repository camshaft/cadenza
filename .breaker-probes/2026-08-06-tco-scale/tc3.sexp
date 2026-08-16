(case "tc3 tail recursion under a HANDLER at depth 20000 (handler frame + TCO compose)"
  (input  (do
            (effect Ctr (op tick (-> Unit Int64)))
            (def (go (: i Int64) (: acc Int64))
              (if (= i 0) acc (go (- i 1) (+ acc (Ctr.tick unit)))))
            (def (main (: n Int64))
              (handle Ctr 1 ((tick (u) s (resume 1 s)))
                (go n 0)))
            (export main)))
  (call   main (: 20000 Int64)) (output (: 20000 Int64)))
