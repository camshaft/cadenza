(case "nv1 a slice-of-slice VIEW resumed through TWO handler layers composes offsets"
  (input  (do
            (effect A (op src (-> Unit Bytes)))
            (effect B (op cut (-> Bytes Bytes)))
            (def (main (: a Int64))
              (handle A 0
                ((src (u) s
                  (match (Bytes.slice (Bytes.of (list 9 20 30 40 8)) 1 3)
                    ((Some w) (resume w s))
                    ((None _x) (resume (Bytes.of (list)) s)))))
                (handle B 0
                  ((cut (b) t
                    (match (Bytes.slice b 1 2)
                      ((Some w) (resume w t))
                      ((None _x) (resume (Bytes.of (list)) t)))))
                  (+ (match (Bytes.at (B.cut (A.src)) 0) ((Some v) v) ((None _u) -1)) a))))
            (export main)))
  (call   main (: 12 Int64))
  (output (: 42 Int64)))
