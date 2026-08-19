(case "pyco1 probe: the handled body RETURNS A CLOSURE that captured a drawn value, and the closure is applied AFTER the handle has returned — the captured draw must survive the handler teardown and be usable outside the effect region"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (let ((f (handle E (% n 3)
               ((tick () s (resume s (+ s 1))))
               (let ((d (E.tick)))
                 (fn ((: x Int64)) (+ x d))))))
      (f (: 100 Int64))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 101 Int64))
  (call   main (: 0 Int64)) (output (: 100 Int64)))
