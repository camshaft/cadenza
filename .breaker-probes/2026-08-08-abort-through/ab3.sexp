(case "ab3 control: same 2-handler abort with SCALAR inner state"
  (input  (do
            (effect Store (op put (-> Int64 Unit)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail 0 ((bail (v) s v))
                (handle Store 0
                  ((put (k) m (resume unit (+ m k))))
                  (do
                    (Store.put 1) (Store.put 2)
                    (if (> n 0) (Bail.bail 99) 0)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 99 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
