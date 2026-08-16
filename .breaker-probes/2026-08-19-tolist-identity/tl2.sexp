(case "tl2 Map.to-list of the churned map equals to-list of the direct build (enumeration is history-blind)"
  (input  (do
            (def (grow (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (grow (+ i 1) n (Map.insert m (+ 1000 i) i))))
            (def (shrink (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (= i n) m (shrink (+ i 1) n (Map.remove m (+ 1000 i)))))
            (def (main (: n Int64))
              (do
                (def direct (Map.insert (Map.insert (Map.insert Map.empty 3 30) 1 10) 2 20))
                (def churned (shrink 1 n (grow 1 n direct)))
                (if (= (Map.to-list churned) (Map.to-list direct)) 1 0)))
            (export main)))
  (call   main (: 80 Int64)) (output (: 1 Int64)))
