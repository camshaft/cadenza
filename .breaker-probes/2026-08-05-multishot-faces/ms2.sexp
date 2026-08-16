(case "ms2 multi-shot where the SECOND shot's resume value derives from the FIRST shot's result"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s
                  (do
                    (def first (resume n s))
                    (+ first (resume (+ first 1) s)))))
                (* 2 (Amb.flip))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 32 Int64)))
