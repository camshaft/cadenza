(case "c4 const-true if around put no shadow"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (let ((_go (if true (St.put 7) unit)))
                  (+ (* 10 (St.get)) x))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
