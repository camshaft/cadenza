(case "hs2 the handler's FINAL Map state escapes as part of the handle's value and keys correctly"
  (input  (do
            (effect Store (op put (-> Int64 Unit)))
            (def (main (: n Int64))
              (do
                (def result
                  (handle Store (Map.empty)
                    ((put (k) m (resume unit (Map.insert m k (* k 10)))))
                    (do (Store.put 1) (Store.put n) (Store.put 3)
                        (tuple 99 unit))))
                77))
            (export main)))
  (call   main (: 2 Int64)) (output (: 77 Int64)))
