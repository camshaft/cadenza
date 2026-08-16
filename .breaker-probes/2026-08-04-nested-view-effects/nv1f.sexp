(case "nv1f dissect: cross-handler arg + arm MATCHES an Option (slice) — result read by Bytes.len"
  (input  (do
            (effect A (op src (-> Unit Bytes)))
            (effect B (op cut (-> Bytes Bytes)))
            (def (main (: a Int64))
              (handle A 0
                ((src (u) s (resume (Bytes.of (list 20 30 40)) s)))
                (handle B 0
                  ((cut (b) t
                    (match (Bytes.slice b 1 2)
                      ((Some w) (resume w t))
                      ((None _x) (resume (Bytes.of (list)) t)))))
                  (+ (Bytes.len (B.cut (A.src))) a))))
            (export main)))
  (call   main (: 12 Int64))
  (output (: 14 Int64)))
