(case "ky2 a two-map JOIN: enumeration of A feeds lookups into B, accumulating matched pairs"
  (input  (do
            (def (filla (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fill-done m i)))
            (def (fill-done (: m (Map Int64 Int64)) (: i Int64))
              (filla (- i 1) (Map.insert m i (* i 2))))
            (def (fillb (: i Int64) (: m (Map Int64 Int64)))
              (if (= i 0) m (fillb (- i 1) (if (= (% i 2) 0) (Map.insert m (* i 2) (* i 100)) m))))
            (def (join (: ps (List (Tuple Int64 Int64))) (: b (Map Int64 Int64)) (: acc Int64))
              (match ps
                ((list) acc)
                ((list h .. t) (match h ((tuple _k v)
                  (join t b (+ acc (match (Map.lookup b v) ((Some w) 1) ((None _u) 0)))))))))
            (def (main (: n Int64))
              (do
                (def a (filla n Map.empty))
                (def b (fillb n Map.empty))
                (join (Map.to-list a) b 0)))
            (export main)))
  (call   main (: 40 Int64)) (output (: 20 Int64)))
