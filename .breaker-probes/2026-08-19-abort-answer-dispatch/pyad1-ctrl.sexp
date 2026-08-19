(case "pyad1-ctrl: F.aux replaced by literal 100"
  (input (do
  (effect E (op tick (-> Int64)) (op stop (-> Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (+ s 1) (+ s 10)))
       (stop () s (+ (* s 1000) (: 100 Int64))))
      (let ((a (E.tick)))
        (+ a (E.stop)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 11100 Int64))
  (call   main (: 0 Int64)) (output (: 10100 Int64)))
