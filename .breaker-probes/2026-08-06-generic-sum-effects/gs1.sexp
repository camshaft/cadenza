(case "gs1 a GENERIC user sum ((Box Int64)) as op result — nominal tag + instantiated payload cross resume"
  (input  (do
            (effect St (op wrap (-> Int64 (Box Int64))))
            (type (Box a) (Full a) (Empty))
            (def (main (: n Int64))
              (handle St 0
                ((wrap (v) s (resume (if (> v 10) (Box.Full (* v 3)) (Box.Empty)) (+ s 1))))
                (+ (match (St.wrap 20) ((Box.Full x) x) ((Box.Empty) -1))
                   (match (St.wrap n) ((Box.Full x) x) ((Box.Empty) -1)))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 59 Int64)))
