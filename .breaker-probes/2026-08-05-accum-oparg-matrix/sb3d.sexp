(case "sb3d cut: accumulator op-arg with arm that ADVANCES but resumes plain v (resume v (+ s v))"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64) (: acc Int64))
              (if (= n 0) acc (count (- n 1) (+ acc 2))))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume v (+ s v))))
                (Log.emit (count n 0))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
