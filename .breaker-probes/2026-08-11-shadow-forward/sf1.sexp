(case "sf1 an inner SAME-effect handler's arm re-performs the effect it discharges — the re-perform routes to the OUTER handler, both states advance independently"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (handle St 100
                  ((get () t (resume (+ (St.get) t) (+ t 10))))
                  (+ (St.get) (* 1000 (St.get))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 114103 Int64))
  (call   main (: 0 Int64)) (output (: 111100 Int64)))
