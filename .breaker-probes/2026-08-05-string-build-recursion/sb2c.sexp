(case "sb2c variant: single-param recursion with RUNTIME arg — already-passing sb2a IS this; now two-param NON-accum (second param unused)"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (count (: n Int64) (: junk Int64))
              (if (= n 0) 0 (+ 2 (count (- n 1) junk))))
            (def (main (: n Int64))
              (handle Log 0
                ((emit (v) s (resume (+ s v) (+ s v))))
                (Log.emit (count n 9))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
