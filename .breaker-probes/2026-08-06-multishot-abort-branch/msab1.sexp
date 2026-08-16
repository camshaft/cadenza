(case "msab1 an arm SUMS one resumption with a constant (the 1.5-shot shape)"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 1 s) 100)))
                (+ (Amb.flip) 5)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 106 Int64)))
