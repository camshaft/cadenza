(case "se1 an inner let shadows an outer binder at a DIFFERENT type across a handler arm's resume"
  (input  (do
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (let ((_go (let ((x true))
                             (if x (St.put 7) unit))))
                  (+ (* 10 (St.get)) x))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
