(case "ck1 a TUPLE-keyed Map op result — the body looks up by a reconstructed compound key"
  (input  (do
            (effect St (op grid (-> Int64 (Map (Tuple Int64 Int64) Int64))))
            (def (main (: n Int64))
              (handle St 0
                ((grid (k) s (resume (Map.insert (Map.insert Map.empty (tuple 1 2) (* k 10)) (tuple 3 4) 7) s)))
                (let ((m (St.grid n)))
                  (+ (* 100 (Map.len m))
                     (+ (match (Map.lookup m (tuple 1 2)) ((Some a) a) ((None _u) -1))
                        (match (Map.lookup m (tuple 4 3)) ((Some b) b) ((None _u) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 249 Int64)))
