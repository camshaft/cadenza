(case "sb3b boundary: accumulator recursion in op-arg of an ABORT op (halt with computed arg)"
  (input  (do
            (effect St (op halt (-> Int64 Int64)))
            (def (count (: n Int64) (: acc Int64))
              (if (= n 0) acc (count (- n 1) (+ acc 2))))
            (def (main (: n Int64))
              (+ 5 (handle St 0
                ((halt (v) s (* 2 v)))
                (+ 100 (St.halt (count n 0))))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 805 Int64)))
