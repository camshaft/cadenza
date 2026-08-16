(case "nv1e dissect: nv1b minus outer perform — inner cut takes a LITERAL Bytes arg, arm slices"
  (input  (do
            (effect B (op cut (-> Bytes Bytes)))
            (def (main (: a Int64))
              (handle B 0
                ((cut (b) t
                  (match (Bytes.slice b 1 2)
                    ((Some w) (resume w t))
                    ((None _x) (resume (Bytes.of (list)) t)))))
                (+ (match (Bytes.at (B.cut (Bytes.of (list 20 30 40))) 0) ((Some v) v) ((None _u) -1)) a)))
            (export main)))
  (call   main (: 12 Int64))
  (output (: 42 Int64)))
