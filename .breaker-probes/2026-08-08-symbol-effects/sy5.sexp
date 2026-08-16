(case "sy5 SYMBOL op args as COMMANDS — inc/dbl/nop route both the answer and the state transition"
  (input  (do
            (effect C (op cmd (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle C n
                ((cmd (w) s (resume (if (= w #"inc") (+ s 1) (if (= w #"dbl") (* s 2) 0))
                                    (if (= w #"inc") (+ s 1) (if (= w #"dbl") (* s 2) s)))))
                (+ (C.cmd #"inc") (+ (* 10 (C.cmd #"dbl")) (* 100 (C.cmd #"nop"))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 126 Int64))
  (call   main (: 0 Int64)) (output (: 21 Int64)))
