(case "pyw2 TWO PRE-SUSPEND BINDINGS RIDE THE CONTINUATION TOGETHER — the arm let-binds a hundredfold and a tenfold toll from different state offsets BEFORE resuming then sums both saved slots after the replay, both bindings must survive the suspend side by side, and dropping or recomputing either slot shifts a distinct digit range"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (let ((t1 (* 100 (+ s 1))))
                    (let ((t2 (* 10 (+ s 2))))
                      (+ (resume s (+ s 3)) (+ t1 t2))))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 831 Int64))
  (call   main (: 0 Int64)) (output (: 600 Int64)))
