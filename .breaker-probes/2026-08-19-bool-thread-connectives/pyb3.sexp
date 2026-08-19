(case "pyb3 a BOOL STATE THREAD UNDER and-PLUS-not COMPOSITION — the flip answers the current flag and negates the thread, the body demands the FIRST draw true AND the SECOND false which the alternating thread delivers exactly when the seed starts true, and the short-circuit skips the second draw entirely on the false-starting seed"
  (input  (do
            (effect E (op flip (-> Bool)))
            (def (main (: n Int64))
              (if (handle E (= (% n 3) 0)
                    ((flip () b (resume b (not b))))
                    (and (E.flip) (not (E.flip))))
                  (: 1 Int64) (: 2 Int64)))
            (export main)))
  (call   main (: 10 Int64)) (output (: 2 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64)))
