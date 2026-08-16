(case "tc1 a tail-recursive CLOSURE (self via fixpoint param) at depth 100000"
  (input  (do
            (def (main (: n Int64))
              (do
                (def (go (: i Int64) (: acc Int64))
                  (if (= i 0) acc (go (- i 1) (+ acc 1))))
                (go n 0)))
            (export main)))
  (call   main (: 100000 Int64)) (output (: 100000 Int64)))
