(case "mc1 #2233 cycle-guard probe: MUTUALLY-recursive arms sharing binder refs (the hang class the guard fixed)"
  (input  (do
            (effect St (op ping (-> Int64 Int64)) (op pong (-> Int64 Int64)))
            (def (even-w (: n Int64))
              (if (= n 0) 0 (+ (St.ping n) (odd-w (- n 1)))))
            (def (odd-w (: n Int64))
              (if (= n 0) 0 (+ (St.pong n) (even-w (- n 1)))))
            (def (main (: k Int64))
              (handle St 0
                ((ping (v) s (resume (* 10 v) s))
                 (pong (v) s (resume v s)))
                (even-w k)))
            (export main)))
  (call   main (: 4 Int64)) (output (: 64 Int64)))
