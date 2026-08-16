(case "c" (input (do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((f (let ((a (St.next))) (fn ((: x Int64)) (* a x)))))
        (f 10))))
  (export main)))
  (call main (: 5 Int64)) (output (: 50 Int64)))
