(case "ta4 try over a runtime-conditional option, minimal"
  (input  (do
            (def (f (: s Int64))
              (: (let ((v (try (if (> s 0) (Some s) (None unit))))) (Some v)) (Option Int64)))
            (def (main (: k Int64))
              (match (f k) ((Some v) v) ((None _u) -99)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5 Int64)))
