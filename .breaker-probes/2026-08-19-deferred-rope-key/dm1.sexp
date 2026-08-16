(case "dm1 a DEFERRED byte-rope key (concat of runtime-selected chunks) probes a populated trie"
  (input  (do
            (def (pick (: s Int64) (: t Bytes) (: f Bytes)) (if (= s 0) t f))
            (def (fill (: i Int64) (: m (Map Bytes Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Bytes.of (list (UInt8.wrap i) 200)) i))))
            (def (main (: s Int64))
              (do
                (def m (fill 40 Map.empty))
                (def rope (Bytes.concat (pick s (Bytes.of (list 25)) (Bytes.of (list 99)))
                                        (pick s (Bytes.of (list 200)) (Bytes.of (list 99)))))
                (match (Map.lookup m rope) ((Some v) v) ((None _u) -1))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 25 Int64)))
