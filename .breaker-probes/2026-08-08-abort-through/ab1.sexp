(case "ab1 an ABORT from a Map-stated handler discards the partial state cleanly (no leak, value out)"
  (input  (do
            (effect Store (op put (-> Int64 Unit)) (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle Store (Map.empty)
                ( (put (k) m (resume unit (Map.insert m k k)))
                  (bail (v) m v) )
                (do
                  (Store.put 1) (Store.put 2) (Store.put 3)
                  (if (> n 0) (Store.bail 99) 0))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 99 Int64))
  (call   main (: 0 Int64)) (output (: 0 Int64)))
