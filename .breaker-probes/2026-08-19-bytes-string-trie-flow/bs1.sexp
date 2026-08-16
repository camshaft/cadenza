(case "bs1 a Bytes value stored at trie depth round-trips through String.from-bytes on retrieval"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Bytes)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i (Bytes.of (list 104 105 (UInt8.wrap (+ 48 (% i 10)))))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (match (Map.lookup m 25)
                  ((Some b) (match (String.from-bytes b)
                              ((Some s) (String.byte-len s))
                              ((None _u) -2)))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 3 Int64)))
