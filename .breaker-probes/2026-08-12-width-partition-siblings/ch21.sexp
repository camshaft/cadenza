(case "ch21 sibling: CHAINED computed keys — the second perform's key is computed FROM the first's answer, both dispatches walk the two-lookup-match arm"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S Map.empty
                ((put (k v) m
                  (let ((m2 (match (Map.lookup m k)
                              ((Some x) (Map.insert m k v))
                              ((None u) (Map.insert m k v)))))
                    (resume (match (Map.lookup m2 k) ((Some x) x) ((None u) 0)) m2))))
                (let ((a (S.put (+ n 1) n)))
                  (let ((b (S.put (* a 2) (+ a 1))))
                    (+ (* 10 a) b)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 34 Int64))
  (call   main (: 8 Int64)) (output (: 89 Int64)))
