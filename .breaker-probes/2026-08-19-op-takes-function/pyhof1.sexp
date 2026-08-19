(case "pyhof1 probe: the op takes a FUNCTION argument and the arm APPLIES it to the captured state before resuming — apply(f) resumes (f s); two dispatches pass different closures (multiply-by-ten then add-hundred) while the state threads, so the higher-order argument is invoked inside the handler arm at the current state"
  (input (do
  (effect E (op apply (-> (-> Int64 Int64) Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((apply (f) s (resume (f s) (+ s 1))))
      (+ (* 100 (E.apply (fn ((: x Int64)) (* x 10))))
         (E.apply (fn ((: y Int64)) (+ y 100))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 1102 Int64))
  (call   main (: 0 Int64)) (output (: 101 Int64)))
