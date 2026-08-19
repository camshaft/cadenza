(case "pymf1 probe: MULTI-SHOT arm resumes twice, each resume answer draws a stateful foreign counter F so the two shots see distinct foreign values"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op aux (-> Int64)))
  (def (main (: n Int64))
    (handle F (: 0 Int64)
      ((aux () fc (resume fc (+ fc 1))))
      (handle E (% n 3)
        ((tick () s
          (+ (resume (+ s (F.aux)) (+ s 1))
             (resume (+ s (F.aux)) (+ s 100)))))
        (let ((x (E.tick))) (+ x 7)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 17 Int64))
  (call   main (: 0 Int64)) (output (: 15 Int64)))
