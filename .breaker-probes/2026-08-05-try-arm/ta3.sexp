(case "ta3 the try-helper alone (no handler) at both branches"
  (input  (do
            (def (arm-helper (: s Int64))
              (: (let ((v (try (if (> s 0) (Some s) (None unit))))) (Some (* v 10))) (Option Int64)))
            (def (main (: k Int64))
              (match (arm-helper k) ((Some v) v) ((None _u) -99)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 0 Int64)) (output (: -99 Int64)))
