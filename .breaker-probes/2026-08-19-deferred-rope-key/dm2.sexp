(case "dm2 a deferred rope STORED as a trie key is found by its flat twin among 40 entries"
  (input  (do
            (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
            (def (fill (: i Int64) (: m (Map Bytes Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Bytes.of (list (UInt8.wrap i) 200)) i))))
            (def (main (: s Int64))
              (do
                (def rope (Bytes.concat (pick s (Bytes.of (list 77)) (Bytes.of (list 99)))
                                        (pick s (Bytes.of (list 78 79)) (Bytes.of (list 99)))))
                (def m (Map.insert (fill 40 Map.empty) rope 777))
                (match (Map.lookup m (Bytes.of (list 77 78 79))) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 777 Int64)))
