(case "gc2 a map churned up to 200 and back to its ORIGINAL 2 entries equals the direct 2-entry build"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m i (* i 10)))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m i))))
            (def (main (: n Int64))
              (do
                (def m0 (Map.insert (Map.insert Map.empty 5000 50) 6000 60))
                (def big (grow 1 n m0))
                (def back (shrink 1 n big))
                (+ (* 10 (if (= back (Map.insert (Map.insert Map.empty 5000 50) 6000 60)) 1 0))
                   (if (= (Map.len back) 2) 1 0))))
            (export main)))
  (call   main (: 200 Int64)) (output (: 11 Int64)))
