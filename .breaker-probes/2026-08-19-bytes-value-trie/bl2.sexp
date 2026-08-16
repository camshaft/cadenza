(case "bl2 a retrieved rope value CONCATS with another retrieved rope (values compose post-retrieval)"
  (input  (do
            (def (fill (: i Int64) (: m (Map Int64 Bytes)))
              (if (= i 0) m
                (fill (- i 1) (Map.insert m i (Bytes.of (list (UInt8.wrap i) (UInt8.wrap (* i 2))))))))
            (def (main (: n Int64))
              (do
                (def m (fill n Map.empty))
                (def a (match (Map.lookup m 10) ((Some b) b) ((None _u) (Bytes.of (list)))))
                (def b (match (Map.lookup m 20) ((Some b) b) ((None _u) (Bytes.of (list)))))
                (def joined (Bytes.concat a b))
                (+ (* 100 (Bytes.len joined))
                   (+ (match (Bytes.at joined 1) ((Some v) v) ((None _u) -1))
                      (match (Bytes.at joined 3) ((Some v) v) ((None _u) -1))))))
            (export main)))
  (call   main (: 40 Int64)) (output (: 460 Int64)))
