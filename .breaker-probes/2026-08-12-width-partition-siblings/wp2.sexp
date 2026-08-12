(case "wp2 sibling: computed SHIFT perform arg (checked shl) + two lookup-matches"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k v))
                              ((None u) (Map.insert m k v)))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (S.put (<< n 2) n)))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64)))
