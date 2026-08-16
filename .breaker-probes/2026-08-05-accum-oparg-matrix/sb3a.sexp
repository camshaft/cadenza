(case "sb3a boundary: 3-param accumulator (two carried values) direct in op-arg"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64) (: acc Int64) (: mul Int64))
              (if (= n 0) (+ acc mul) (count (- n 1) (+ acc 2) mul)))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (Log.emit (count n 0 7))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 407 Int64)))
