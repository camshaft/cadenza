(case "ts1 a two-site arm over a HEAP state (List) — hit pushes, miss holds"
  (input  (do
            (effect St (op feed (-> Int64 Int64)) (op tally (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (list)
                ((feed (v) s (if (> v 10) (resume v (List.push s v)) (resume 0 s)))
                 (tally (u) s (resume (List.len s) s)))
                (+ (St.feed 20) (+ (St.feed n) (+ (St.feed 30) (* 1000 (St.tally)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 2050 Int64)))
