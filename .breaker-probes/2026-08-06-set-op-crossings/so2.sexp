(case "so2 a SET as op ARGUMENT — the arm measures and probes the set it is handed"
  (input  (do
            (effect St (op tally (-> (Set Int64) Int64)))
            (def (main (: n Int64))
              (handle St 0
                ((tally (xs) s (resume (+ (Set.len xs) (if (Set.contains xs 5) 100 0)) s)))
                (St.tally (Set.of (list n 2 9)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 103 Int64)))
