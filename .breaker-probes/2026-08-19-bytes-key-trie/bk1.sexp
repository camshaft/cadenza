(case "bk1 a trie of 40 BYTES keys (shared prefixes) resolves content descent at depth"
  (input  (do
            (def (fill (: i Int64) (: m (Map Bytes Int64)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m (Bytes.of (list (UInt8.wrap (% i 4)) (UInt8.wrap (/ i 4)) 255)) i))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (+ (* 10 (Map.len m))
                   (match (Map.lookup m (Bytes.of (list 3 5 255))) ((Some v) v) ((None _u) -1)))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 423 Int64)))
