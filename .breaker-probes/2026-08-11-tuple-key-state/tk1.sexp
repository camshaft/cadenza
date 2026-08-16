(case "tk1 a TUPLE-keyed Map STATE grown across dispatches — compound-key inserts and lookups thread, the flipped key misses"
  (input  (do
            (effect S (op mark (-> Int64 Int64 Int64)) (op check (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((mark (x y) s (resume (Map.len s) (Map.insert s (tuple x y) (+ x y))))
                 (check (x y) s (resume (match (Map.lookup s (tuple x y)) ((Some v) v) ((None _u) -1)) s)))
                (let ((_a (S.mark 1 2)))
                  (let ((_b (S.mark n 4)))
                    (+ (* 10000 (S.check 1 2))
                       (+ (* 100 (S.check n 4)) (S.check 2 1)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 30699 Int64))
  (call   main (: 1 Int64)) (output (: 30499 Int64)))
