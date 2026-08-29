(do (def (main (: n Int64)) (do
      (def m (Map.insert (Map.empty) 1 10))
      (def m2 (Map.insert m 1 (* n 9)))
      (+ (match (Map.lookup m 1) ((Some v) v) ((None _u) -1))
         (match (Map.lookup m2 1) ((Some v) v) ((None _u) -1)))))
    (export main))
