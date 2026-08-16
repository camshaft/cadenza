(case "bs2 a String built from a trie-stored Bytes value keys ANOTHER trie (cross-type value flow)"
  (input  (do
            (def (fillb (: i Int64) (: m (Map Int64 Bytes)))
              (if (= i 0) m
                (fillb (- i 1) (Map.insert m i (Bytes.of (list 107 (UInt8.wrap (+ 97 (% i 26)))))))))
            (def (fills (: i Int64) (: m (Map String Int64)))
              (if (= i 0) m
                (fills (- i 1) (Map.insert m (String.concat "k" (match (String.from-bytes (Bytes.of (list (UInt8.wrap (+ 97 (% i 26)))))) ((Some c) c) ((None _u) "?"))) i))))
            (def (main (: n Int64))
              (do
                (def mb (fillb n Map.empty))
                (def ms (fills n Map.empty))
                (match (Map.lookup mb 3)
                  ((Some b) (match (String.from-bytes b)
                              ((Some s) (match (Map.lookup ms s) ((Some v) v) ((None _u) -3)))
                              ((None _u) -2)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 20 Int64)) (output (: 3 Int64)))
