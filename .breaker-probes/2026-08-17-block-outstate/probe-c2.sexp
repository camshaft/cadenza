(case "c2 control USED let effectful init with shadow"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (let ((go (let ((x true)) (if x (St.put 7) unit))))
                  (match go (_ (+ (* 10 (St.get)) x))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
