(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((seed (St.next)))
        (handle St (* seed 2)
          ((next () s (resume s (+ s 100))))
          (St.next)))))
  (export main))
