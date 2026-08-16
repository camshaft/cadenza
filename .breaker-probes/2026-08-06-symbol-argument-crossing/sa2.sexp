(case "sa2 a SYMBOL handler STATE threads dispatches — each resume reads the prior symbol's identity"
  (input  (do
            (effect St (op swap (-> Symbol Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "start")
                ((swap (next) prev (resume (if (= prev (Symbol.of "start")) 10 20) next)))
                (+ (* 100 (St.swap (Symbol.of "mid")))
                   (St.swap (Symbol.of "end")))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 1020 Int64)))
