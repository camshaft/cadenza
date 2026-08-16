(case "bcp1 Bytes.compact of the EFFECT-GROWN rope inside the arm — the compacted flat rep reads exactly at a computed index and re-threads"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op flat (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S (Bytes.of (list))
                ((add (v) s (resume 0 (Bytes.concat s (Bytes.of (list (UInt8.wrap (+ 60 v)))))))
                 (flat (i) s
                  (let ((c (Bytes.compact s)))
                    (resume (+ (* 100 (Bytes.len c))
                               (match (Bytes.at c i) ((Some v) v) ((None _u) -1)))
                            c))))
                (let ((_w (walk n)))
                  (S.flat (- n 1)))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 461 Int64))
  (call   main (: 1 Int64)) (output (: 161 Int64)))
