(do
  (effect St (op add (-> Int64 Int64)) (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((add (v) s (resume (+ v s) s))
       (next () s (resume s (+ s 1))))
      (let ((a (St.next)))
        (handle St 100
          ((add (v) s (resume (* v s) s))
           (next () s (resume s (+ s 10))))
          (St.add a)))))
  (export main))
