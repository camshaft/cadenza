(case "sb3c confirm: identical accumulator op-arg, RESUMPTIVE op, minimal arm (resume v s)"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64) (: acc Int64))
              (if (= n 0) acc (count (- n 1) (+ acc 2))))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume v s)))
                (Log.emit (count n 0))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
