(case "bp1 sibling: Bytes.at as the Option producer — two at-matches in the arm + computed perform arg used as index"
  (input  (do
            (effect S (op put (-> Int64 Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Bytes.of (list (UInt8.wrap 5) (UInt8.wrap 6)))
                ((put (k v) bs
                  (let ((bs2 (match (Bytes.at bs k)
                               ((Some x) (Bytes.concat bs (Bytes.of (list (UInt8.wrap x)))))
                               ((None u) (Bytes.concat bs (Bytes.of (list (UInt8.wrap 40))))))))
                    (resume (match (Bytes.at bs2 2) ((Some y) y) ((None u) -1)) bs2))))
                (S.put (+ n 1) n)))
            (export main)))
  (call   main (: 0 Int64)) (output (: 6 Int64))
  (call   main (: 3 Int64)) (output (: 40 Int64)))
