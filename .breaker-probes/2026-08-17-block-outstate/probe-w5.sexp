(case "w5 same nested-let-if shape but in TAIL position (no continuation)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v 5))
                  (let ((b true)) (if b (St.get) 99)))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3 Int64)))
