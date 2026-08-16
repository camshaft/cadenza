(case "hp1 a handler whose state is the ESCAPED result of a previous handle (handler-to-handler handoff)"
  (input  (do
            (effect Bld (op grow (-> Int64 Int64)) (op take (-> Unit (Map Int64 Int64))))
            (effect Rd (op find (-> Int64 Int64)))
            (def (feed (: i Int64) (: n Int64))
              (if (= i n) 0 (+ (Bld.grow i) (feed (+ i 1) n))))
            (def (main (: n Int64))
              (do
                (def built (handle Bld Map.empty
                             ((grow (v) s (resume 0 (Map.insert s v (* v 4))))
                              (take (u) s (resume s s)))
                             (do (feed 1 (+ n 1)) (Bld.take))))
                (handle Rd built
                  ((find (k) s (resume (match (Map.lookup s k) ((Some v) v) ((None _u) -1)) s)))
                  (+ (Rd.find 10) (Rd.find 25)))))
            (export main)))
  (call   main (: 30 Int64)) (output (: 140 Int64)))
