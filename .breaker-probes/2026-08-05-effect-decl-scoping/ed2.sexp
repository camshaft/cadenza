(case "ed2 the same-named ops CROSS in arm context: A's arm performs B.get (both spelled get)"
  (input  (do
            (effect A (op get (-> Unit Int64)))
            (effect B (op get (-> Unit Int64)))
            (def (main (: n Int64))
              (handle B n
                ((get (u) t (resume (* t 2) t)))
                (handle A 0
                  ((get (u) s (resume (+ 1000 (B.get)) s)))
                  (A.get))))
            (export main)))
  (call   main (: 21 Int64)) (output (: 1042 Int64)))
