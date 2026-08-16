(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (+ (handle St 5
           ((next () s (resume s (* s 10))))
           (let ((a (St.next)))
             (let ((b (St.next)))
               (+ a b))))
         (St.next))))
  (export main))
