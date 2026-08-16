(case "msx1 multi-shot resumes carry DIVERGENT states; two performs branch 2x2"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 1 (+ s 10)) (resume 2 (+ s 20)))))
                (+ (Amb.flip) (Amb.flip))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 12 Int64)))
