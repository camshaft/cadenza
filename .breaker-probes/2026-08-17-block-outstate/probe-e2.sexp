(case "e2 if cond is a PARAM bool performing branch"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64) (: b Bool))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (do
                  (if b (St.put 7) unit)
                  (+ (* 10 (St.get)) x))))
            (export main)))
  (call   main (: 3 Int64) (: true Bool)) (output (: 73 Int64)))
