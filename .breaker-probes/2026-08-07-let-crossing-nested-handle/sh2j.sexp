(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((seed (+ n 1)))
        (+ seed
           (handle St 5
             ((next () s (resume s (+ s 100))))
             (St.next))))))
  (export main))
