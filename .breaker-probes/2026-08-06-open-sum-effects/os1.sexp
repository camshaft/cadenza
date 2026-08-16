(case "os1 a user SUM is constructed AND matched inside the arm (per-dispatch classification)"
  (input  (do
            (effect St (op classify (-> Int64 Int64)))
            (type Status (Active Int64) (Idle))
            (def (main (: n Int64))
              (handle St 0
                ((classify (v) s
                  (resume (match (if (> v 10) (Status.Active v) (Status.Idle))
                            ((Status.Active x) x)
                            ((Status.Idle) 0)) s)))
                (+ (St.classify 20) (St.classify n))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 20 Int64)))
