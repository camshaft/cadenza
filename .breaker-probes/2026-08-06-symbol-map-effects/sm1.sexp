(case "sm1 a Map keyed by SYMBOLS as handler state (route-table accumulator)"
  (input  (do
            (effect St (op hit (-> Int64 Int64)) (op total (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Map.insert (Map.insert Map.empty (Symbol.of "a") 0) (Symbol.of "b") 0)
                ((hit (v) s
                  (if (> v 10)
                    (resume v (Map.insert s (Symbol.of "a") v))
                    (resume 0 (Map.insert s (Symbol.of "b") v))))
                 (total (u) s
                  (resume (+ (match (Map.lookup s (Symbol.of "a")) ((Some x) x) ((None _u) -1))
                            (match (Map.lookup s (Symbol.of "b")) ((Some y) y) ((None _u) -1))) s)))
                (+ (St.hit 20) (+ (St.hit n) (* 100 (St.total))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 2320 Int64)))
