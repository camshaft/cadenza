(case "sk1 a Map keyed by a recursive SUM as handler STATE: insert per perform, lookup by rebuilt key"
  (input  (do
            (type T (TI Int64) (TP T T))
            (effect Acc (op put (-> Int64 Int64)))
            (def (mk (: i Int64)) (T.TP (T.TI i) (T.TI (* 2 i))))
            (def (main (: a Int64) (: b Int64))
              (handle Acc Map.empty
                ((put (v) s (resume (Map.len s) (Map.insert s (mk v) v))))
                (do
                  (def l1 (Acc.put a))
                  (def l2 (Acc.put b))
                  (+ (* 100 l1) (+ (* 10 l2) (Acc.put 3))))))
            (export main)))
  (call   main (: 1 Int64) (: 2 Int64))
  (output (: 12 Int64)))
