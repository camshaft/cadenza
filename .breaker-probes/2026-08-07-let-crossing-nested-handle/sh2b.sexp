(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (let ((seed (+ n 1)))
      (handle St (* seed 2)
        ((next () s (resume s (+ s 100))))
        (St.next))))
  (export main))
