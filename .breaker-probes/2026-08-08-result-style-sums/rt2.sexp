(case "rt2 an Err SHORT-CIRCUITS a recursive Result walk — Ok accumulates, the first Err multiplies out and stops"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op try (-> Int64 Res)))
            (def (chain (: i Int64) (: acc Int64))
              (if (> i 3)
                  acc
                  (match (E.try i)
                    ((Ok v) (chain (+ i 1) (+ acc v)))
                    ((Err e) (* acc e)))))
            (def (main (: n Int64))
              (handle E n
                ((try (k) s (resume (if (> s 0) (Ok (* k 10)) (Err k)) (- s 1))))
                (chain 1 0)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64))
  (call   main (: 2 Int64)) (output (: 90 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
