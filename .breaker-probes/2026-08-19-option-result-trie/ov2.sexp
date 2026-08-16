(case "ov2 a trie of RESULT values dispatches Ok/Err payloads retrieved from depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 (Result Int64 String))))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i (if (= (% i 4) 0) (Err "bad") (Ok (* i 3)))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 100 (match (Map.lookup m 10) ((Some r) (match r ((Ok v) v) ((Err _e) -5))) ((None _u) -1)))
                   (match (Map.lookup m 12) ((Some r) (match r ((Ok _v) 0) ((Err e) (String.byte-len e)))) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 3003 Int64)))
