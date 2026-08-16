(case "mi1 enumeration DELTA across dispatches — dumps before and after keyed inserts, collision rows shrink the delta"
  (input  (do
            (effect Db (op put (-> Int64 Int64)) (op dump (-> (List (Tuple Int64 Int64)))))
            (def (main (: n Int64))
              (handle Db (map (1 10))
                ((put (k) m (resume (Map.len m) (Map.insert m k (* k 2))))
                 (dump () m (resume (Map.to-list m) m)))
                (let ((before (List.len (Db.dump))))
                  (do
                    (Db.put n)
                    (Db.put 7)
                    (+ (* 100 (List.len (Db.dump))) (* 10 before))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 310 Int64))
  (call   main (: 1 Int64)) (output (: 210 Int64))
  (call   main (: 7 Int64)) (output (: 210 Int64)))
