(case "fo4 a float-sum as a MAP key with lookup by an equal reconstructed key"
  (input  (do
            (type Reading (Temp Float64) (Missing))
            (def (main (: x Float64))
              (let ((m (Map.insert (Map.insert Map.empty (Temp x) 10) (Missing) 20)))
                (+ (match (Map.lookup m (Temp (* x 1.0))) ((Some v) v) ((None _u) -1))
                   (* 10 (match (Map.lookup m (Missing)) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 2.5 Float64)) (output (: 210 Int64)))
