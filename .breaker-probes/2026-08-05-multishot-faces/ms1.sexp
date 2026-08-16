(case "ms1 MULTI-SHOT arm (two k-calls) x heap STATE: each shot sees the same pre-shot state"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb (list n)
                ((flip (u) s (+ (resume (List.len s) s) (resume 10 s))))
                (+ 100 (Amb.flip))))
            (export main)))
  (call   main (: 7 Int64)) (output (: 211 Int64)))
