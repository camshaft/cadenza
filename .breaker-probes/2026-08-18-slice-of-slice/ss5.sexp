(case "ss5 Bytes.compact of a deferred rope equals the rope as a trie key (compact is identity-preserving)"
  (input  (do
            (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
            (def (fill (: i Int64) (: m (Map Bytes Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Bytes.of (list (UInt8.wrap i) 50)) i))))
            (def (main (: s Int64))
              (do
                (def rope (Bytes.concat (pick s (Bytes.of (list 20)) (Bytes.of (list 99)))
                                        (pick s (Bytes.of (list 50)) (Bytes.of (list 99)))))
                (def m (Map.insert (fill 40 Map.empty) rope 777))
                (match (Map.lookup m (Bytes.compact rope)) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 777 Int64)))
