(case "ta2 a CONSTANT-failure ? inside the arm's helper — the cut stays in the helper, dispatch unharmed"
  (input  (do
            (effect St (op next (-> Unit Int64)))
            (def (probe (: v Int64))
              (let ((x (try (None unit))))
                (Some (+ x v))))
            (def (main (: n Int64))
              (handle St n
                ((next (u) s (resume (match (probe s) ((Some v) v) ((None _u) -1)) (+ s 7))))
                (+ (* 10 (St.next)) (St.next))))
            (export main)))
  (call   main (: 5 Int64)) (output (: -11 Int64)))
