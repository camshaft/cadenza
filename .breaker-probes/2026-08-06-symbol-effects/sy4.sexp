(case "sy4 a mode-REPLACING arm swaps the Symbol state; a conditional-value arm reads it"
  (input  (do
            (effect St (op emit (-> Int64 Int64)) (op flip (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "loud")
                ((emit (v) s (resume (if (= s (Symbol.of "loud")) (* v 100) v) s))
                 (flip (u) s (resume 0 (Symbol.of "quiet"))))
                (+ (St.emit n) (+ (St.flip) (St.emit 3)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 503 Int64)))
