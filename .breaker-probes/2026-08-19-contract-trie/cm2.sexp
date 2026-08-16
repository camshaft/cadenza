(case "cm2 @ensures relates the RESULT map's len to the ARGUMENT map's len (growth postcondition at depth)"
  (input  (do
        (def (fill (: i Int64) (: m (Map Int64 Int64)))
          (if (= i 0) m (fill (- i 1) (Map.insert m i i))))
        (@ (ensures (> (Map.len ret) (Map.len m))) (def (add-one (: m (Map Int64 Int64)) (: k Int64))
          (Map.insert m k 999)))
        (def (main (: n Int64))
          (do
            (def m (fill n Map.empty))
            (def m2 (add-one m 5000))
            (+ (* 10 (Map.len m2)) (match (Map.lookup m2 5000) ((Some v) (if (= v 999) 1 0)) ((None _u) -1)))))
        (export main)))
  (call   main (: 40 Int64)) (output (: 411 Int64)))
