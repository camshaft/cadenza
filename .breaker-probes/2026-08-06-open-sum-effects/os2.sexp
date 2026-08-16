(case "os2 a USER-SUM op result (Status) crosses resume; the body matches per variant"
  (input  (do
            (effect St (op poll (-> Int64 Status)))
            (type Status (Active Int64) (Idle))
            (def (main (: n Int64))
              (handle St 0
                ((poll (v) s (resume (if (> v 10) (Status.Active (* v 2)) (Status.Idle)) (+ s 1))))
                (+ (match (St.poll 20) ((Status.Active x) x) ((Status.Idle) -1))
                   (match (St.poll n) ((Status.Active x) x) ((Status.Idle) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 39 Int64)))
