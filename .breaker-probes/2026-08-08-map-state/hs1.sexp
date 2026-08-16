(case "hs1 a MAP-stated handler threads 50 inserts through recursion and survives a snapshot read"
  (input  (do
            (effect Store (op put (-> Int64 Unit)) (op size (-> Unit Int64)))
            (def (fill (: i Int64))
              (if (= i 0) unit (do (Store.put i) (fill (- i 1)))))
            (def (main (: n Int64))
              (handle Store (Map.empty)
                ( (put (k) m (resume unit (Map.insert m k (* k 2))))
                  (size (u) m (resume (Map.len m) m)) )
                (do
                  (fill n)
                  (Store.size))))
            (export main)))
  (call   main (: 50 Int64)) (output (: 50 Int64)))
