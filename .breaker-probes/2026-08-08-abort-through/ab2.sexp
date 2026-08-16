(case "ab2 SEPARATE effects: abort from within a Map-stated handler's body (two handlers)"
  (input  (do
            (effect Store (op put (-> Int64 Unit)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Bail 0 ((bail (v) s v))
                (handle Store (Map.empty)
                  ((put (k) m (resume unit (Map.insert m k k))))
                  (do
                    (Store.put 1) (Store.put 2) (Store.put 3)
                    (if (> n 0) (Bail.bail 99) 0)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 99 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
