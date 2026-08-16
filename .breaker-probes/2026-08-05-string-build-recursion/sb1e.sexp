(case "sb1e dissect: pure-recursive call inside handle body but NOT as op-arg ((+ (count..) (Log.emit 5)))"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64) (: acc Int64))
              (if (= n 0) acc (count (- n 1) (+ acc 2))))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (+ (count n 0) (Log.emit 5))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 405 Int64)))
