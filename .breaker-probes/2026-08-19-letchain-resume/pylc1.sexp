(case "pylc1 probe: arm binds a LET CHAIN of intermediates and the resume answer + next-state both reference earlier let-bindings (a used in b, both used in the resume)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s
        (let ((a (* s 2)))
          (let ((b (+ a 3)))
            (resume (+ a b) (+ b 1))))))
      (+ (E.tick) (* 100 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 2707 Int64))
  (call   main (: 0 Int64)) (output (: 1903 Int64)))
