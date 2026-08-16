(do
  (effect Out (op base (-> Unit Int64)))
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle Out n
      ((base (u) s (resume s (+ s 100))))
      (let ((f (fn ((: k Int64))
                 (handle St k
                   ((next (u) s (resume (+ s (Out.base)) (+ s 1))))
                   (St.next)))))
        (+ (* 1000 (f 1)) (f 2)))))
  (export main))
