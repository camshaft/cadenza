(case "nv1c dissect: cross-handler op-arg where the INNER op takes a BYTES arg (scalar version passed xe1-3)"
  (input  (do
            (effect A (op src (-> Unit Bytes)))
            (effect B (op cut (-> Bytes Int64)))
            (def (main (: a Int64))
              (handle A 0
                ((src (u) s (resume (Bytes.of (list 20 30 40)) s)))
                (handle B 0
                  ((cut (b) t (resume (Bytes.len b) t)))
                  (+ (B.cut (A.src)) a))))
            (export main)))
  (call   main (: 12 Int64))
  (output (: 15 Int64)))
