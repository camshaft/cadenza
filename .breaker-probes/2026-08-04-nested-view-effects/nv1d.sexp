(case "nv1d dissect: inner op Bytes->Bytes with cross-handler arg, result consumed by Bytes.len only"
  (input  (do
            (effect A (op src (-> Unit Bytes)))
            (effect B (op cut (-> Bytes Bytes)))
            (def (main (: a Int64))
              (handle A 0
                ((src (u) s (resume (Bytes.of (list 20 30 40)) s)))
                (handle B 0
                  ((cut (b) t (resume b t)))
                  (+ (Bytes.len (B.cut (A.src))) a))))
            (export main)))
  (call   main (: 12 Int64))
  (output (: 15 Int64)))
