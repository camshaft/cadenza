(case "c5 let-bound const bool if around put fresh binder"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (let ((_go (let ((b true)) (if b (St.put 7) unit))))
                  (+ (* 10 (St.get)) x))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
