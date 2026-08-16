(case "msx2 each multi-shot branch OBSERVES its own divergent state via a trailing peek"
  (input  (do
            (effect Amb (op flip (-> Unit Int64)) (op peek (-> Unit Int64)))
            (def (main (: n Int64))
              (handle Amb 0
                ((flip (u) s (+ (resume 1 (+ s 10)) (resume 2 (+ s 20))))
                 (peek (u) s (resume s s)))
                (+ (* 10 (Amb.flip)) (Amb.peek))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 60 Int64)))
