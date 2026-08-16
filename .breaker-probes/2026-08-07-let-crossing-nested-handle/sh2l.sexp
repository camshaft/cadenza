(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (let ((seed (+ n 1)))
      (handle St n
        ((next () s (resume s (+ s 1))))
        (handle St 5
          ((next () s (resume s (+ s 100))))
          (+ seed (St.next))))))
  (export main))
