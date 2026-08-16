(case "ta1 a try INSIDE a handler arm cuts the ARM's option, not the handled body"
  (input  (do
            (effect Ask (op get (-> Unit Int64)))
            (def (arm-helper (: s Int64))
              (: (let ((v (try (if (> s 0) (Some s) (None unit))))) (Some (* v 10))) (Option Int64)))
            (def (main (: k Int64))
              (handle Ask k ((get (_u) s (resume (match (arm-helper s) ((Some v) v) ((None _u) -99)) s)))
                (+ (Ask.get) 1)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 51 Int64))
  (call   main (: 0 Int64)) (output (: -98 Int64)))
