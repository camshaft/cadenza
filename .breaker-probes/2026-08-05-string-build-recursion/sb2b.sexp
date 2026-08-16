(case "sb2b variant: two-param accumulator recursion with CONSTANT arg (count 200 0)"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64) (: acc Int64))
              (if (= n 0) acc (count (- n 1) (+ acc 2))))
            (def (main)
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (Log.emit (count 200 0))))
            (export main)))
  (output (: 400 Int64)))
