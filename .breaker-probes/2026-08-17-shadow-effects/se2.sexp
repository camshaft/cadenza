(case "se2 a handler-arm binder shadowing an outer def-name of a different type resolves arm-locally"
  (input  (do
            (def s "outer-string")
            (def (main (: k Int64))
              (handle Ct k
                ((tick () s (resume s (+ s 1))))
                (+ (* 100 (Ct.tick)) (+ (* 10 (Ct.tick)) (String.len s)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 352 Int64)))
