(case "pyw1 the TOLL COMPUTED BEFORE THE SUSPEND CONSUMED AFTER THE REPLAY — each arm let-binds a hundredfold toll from its pre-resume state then resumes and adds the SAVED binding to whatever the rest-of-body returned, the binding must ride the continuation across the suspend and the replay, and a lowering that recomputes the toll from post-resume state or drops the saved slot shifts every frame's contribution"
  (input  (do
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle E (% n 3)
                ((tick () s
                  (let ((t (* 100 (+ s 1))))
                    (+ (resume s (+ s 2)) t))))
                (let ((a (E.tick)))
                  (let ((b (E.tick)))
                    (+ a (* 10 b))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 631 Int64))
  (call   main (: 0 Int64)) (output (: 420 Int64)))
