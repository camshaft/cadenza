(case "st1 a two-site arm over a SET state (dedup accumulator: insert on new, hold on dup)"
  (input  (do
            (effect St (op add (-> Int64 Int64)) (op card (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Set.of (list))
                ((add (v) s (if (Set.contains s v) (resume 0 s) (resume v (Set.insert s v))))
                 (card (u) s (resume (Set.len s) s)))
                (+ (St.add 7) (+ (St.add n) (+ (St.add 7) (* 100 (St.card)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 210 Int64)))
