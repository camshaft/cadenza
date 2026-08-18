(case "pyt1 a POST-RESUME TOLL that PERFORMS A FOREIGN EFFECT — each inner tick resumes then adds an OUTER levy to what the rest-of-body returned, the two levies fire during the innermost-first unwind so the outer handler's counter advances in unwind order not dispatch order, and the seed sets the levy schedule the unwinding frames drain"
  (input  (do
            (effect T (op levy (-> Int64)))
            (effect E (op tick (-> Int64)))
            (def (main (: n Int64))
              (handle T (% n 3)
                ((levy () t (resume t (+ t 5))))
                (handle E (: 1 Int64)
                  ((tick () s
                    (+ (resume s (+ s 1)) (T.levy))))
                  (let ((a (E.tick)))
                    (let ((b (E.tick)))
                      (+ a (* 10 b)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 28 Int64))
  (call   main (: 0 Int64)) (output (: 26 Int64)))
