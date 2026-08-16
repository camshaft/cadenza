(case "sb1c control: handler + emit of a PRECOMPUTED len (build outside the handle)"
  (input  (do
            (effect Log (op emit (-> Int64 Int64)))
            (def (build (: n Int64) (: acc String))
              (if (= n 0) acc (build (- n 1) (String.concat acc "ab"))))
            (def (main (: n Int64))
              (do
                (def len (String.scalar-len (build n "")))
                (handle Log 0
                  ((emit (v) s (resume (+ s v) (+ s v))))
                  (Log.emit len))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
