(case "sb2a variant: single-param recursion as op-arg (no accumulator)"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64))
              (if (= n 0) 0 (+ 2 (count (- n 1)))))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (Log.emit (count n))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
