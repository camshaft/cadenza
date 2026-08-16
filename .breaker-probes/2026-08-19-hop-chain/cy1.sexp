(case "cy1 a CYCLE in the hop chain: fuel-bounded pointer chase terminates cleanly"
  (input  (do
            (def (chase (: m (Map Int64 Int64)) (: k Int64) (: fuel Int64))
              (if (= fuel 0) -99
                (match (Map.lookup m k)
                  ((Some nxt) (if (= nxt k) k (chase m nxt (- fuel 1))))
                  ((None _u) k))))
            (def (main (: n Int64))
              (do
                (def m (Map.insert (Map.insert (Map.insert Map.empty 1 2) 2 3) 3 1))
                (def r (Map.insert (Map.insert Map.empty 10 11) 11 11))
                (+ (* 10 (chase r 10 20))
                   (if (= (chase m 1 20) -99) 1 0))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
