(case "bl1 Bytes VALUES at trie depth: per-entry ropes retrieved and measured"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Bytes)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i
                  (Bytes.concat (Bytes.of (list (UInt8.wrap i))) (Bytes.of (list 200 201)))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (match (Map.lookup m 25)
                  ((Some b) (+ (* 10 (Bytes.len b))
                               (match (Bytes.at b 0) ((Some v) v) ((None _u) -1))))
                  ((None _u) -1))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 55 Int64)))
