(case "a byte-wise fold over a multi-chunk rope reads every byte across the seams"
  (doc    "A positional byte-wise fold (acc·2 + byte — order-sensitive, so a skipped or doubled byte
           at a seam shifts every later contribution) over a 3-chunk rope built by NESTED concat
           ((x,2)+(3))+(4,5): x=1 → 57, x=0 → 41 (recomputed). Bytes.at must address bytes 0..4
           continuously across BOTH seams (the [x,2]|[3] seam inside the left subtree and the
           [..3]|[4,5] top seam). The Fletcher-16 pin (:1461) covers a 2-chunk rope; the nested
           3-chunk shape adds a rope-tree DEPTH level to the per-index addressing walk.")
  (input  (do
            (def (sum-bytes (: b Bytes) (: i Int64) (: acc Int64))
              (if (= i (Bytes.len b))
                acc
                (sum-bytes b (+ i 1)
                  (+ (* acc 2) (Int64.of (Option.expect (Bytes.at b i) "in range"))))))
            (def (main (: x UInt8))
              (let ((rope (Bytes.concat (Bytes.concat (Bytes.of (list x 2)) (Bytes.of (list 3)))
                                        (Bytes.of (list 4 5)))))
                (sum-bytes rope 0 0)))
            (export main)))
  (call   main (: 1 UInt8)) (output (: 57 Int64))
  (call   main (: 0 UInt8)) (output (: 41 Int64)))
