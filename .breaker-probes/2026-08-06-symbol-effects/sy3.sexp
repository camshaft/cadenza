(case "sy3 a two-site arm branching on SYMBOL equality of the state"
  (input  (do
            (effect St (op emit (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle St (Symbol.of "loud")
                ((emit (v) s (if (= s (Symbol.of "loud")) (resume (* v 100) s) (resume v s))))
                (+ (St.emit n) (St.emit 3))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 800 Int64)))
