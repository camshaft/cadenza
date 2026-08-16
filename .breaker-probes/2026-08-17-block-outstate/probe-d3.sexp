(case "d3 inner-let-wrapped if BOTH branches perform"
  (input  (do
            (effect St (op get (-> Unit Int64)) (op put (-> Int64 Unit)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s s))
                 (put (v) _s (resume unit v)))
                (let ((_go (let ((b true)) (if b (St.put 7) (St.put 9)))))
                  (+ (* 10 (St.get)) x))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 73 Int64)))
