(case "sb2d variant: single-param ACCUMULATOR-STYLE via nested helper (tail-recursive, one visible param)"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count-acc (: n Int64) (: acc Int64))
              (if (= n 0) acc (count-acc (- n 1) (+ acc 2))))
            (def (count (: n Int64)) (count-acc n 0))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (Log.emit (count n))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
