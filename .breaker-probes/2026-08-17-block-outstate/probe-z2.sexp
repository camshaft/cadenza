(case "z2 control: nested let wrapping a BARE perform plus arithmetic (no conditional)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((k 1)) (+ k (St.get)))))
                  (+ (* 10 v) (St.get)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 44 Int64)))
