(do
  (effect St (op next (-> Int64)))
  (effect Ct (op get (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((seed (+ n 1)))
        (+ (handle Ct (* seed 2)
             ((get () s (resume s s)))
             (Ct.get))
           (St.next)))))
  (export main))
