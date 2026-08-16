(case "af1 TWO closures capture one let-bound perform result — the effect fires ONCE"
  (input  (do
            (effect St (op pull (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St 40
                ((pull (u) s (resume s (+ s 1))))
                (let ((v (St.pull)))
                  (let ((f (fn ((: x Int64)) (+ x v)))
                        (g (fn ((: x Int64)) (* x v))))
                    (+ (f 1) (g 2))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 121 Int64)))
