(case "g4 nested-let if init NO second perform (value-only control)"
  (input  (do
            (effect St (op get (-> Unit Int64)))
            (def (main (: x Int64))
              (handle St x
                ((get (u) s (resume s (+ s 1))))
                (let ((v (let ((b true)) (if b (St.get) 99))))
                  (* 10 v))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 30 Int64)))
