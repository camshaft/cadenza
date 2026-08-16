(case "ms3 multi-shot x RECURSION: a loop performs the multi-shot op per iteration (n=2, 2^n leaves)"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)))
            (def (loop (: n Int64))
              (if (= n 0) 1 (* (Amb.flip) (loop (- n 1)))))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 2 s) (resume 3 s))))
                (loop n)))
            (export main)))
  (call   main (: 2 Int64)) (output (: 25 Int64)))
