(case "mk1 a MAP handler state keyed by the op ARGUMENT — per-key counters, lookup-or-default then insert per dispatch"
  (input  (do
            (effect Reg (op touch (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Reg (map (1 10))
                ((touch (k) s (resume (match (Map.lookup s k) ((Some v) v) ((None) 0))
                                      (Map.insert s k (+ (match (Map.lookup s k) ((Some v) v) ((None) 0)) 1)))))
                (+ (Reg.touch n) (+ (* 10 (Reg.touch n)) (* 100 (Reg.touch 1))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1010 Int64))
  (call   main (: 1 Int64)) (output (: 1320 Int64)))
