(case "me3 control: = over two loop-built Maps Int->Int (same shape, walkable both sides)"
  (input  (do
            (def (up (: i Int64) (: n Int64) (: m (Map Int64 Int64)))
              (if (> i n) m (up (+ i 1) n (Map.insert m i (* 2 i)))))
            (def (main (: n Int64))
              (if (= (up 1 n Map.empty) (up 1 n Map.empty)) 1 0))
            (export main)))
  (call   main (: 50 Int64)) (output (: 1 Int64)))
