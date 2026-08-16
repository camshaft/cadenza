(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (* s 2))))
      (let ((k (St.next)))
        (match k
          ((guard _a (> (St.next) 50)) 111)
          ((guard _b (> (St.next) 10)) (+ 200 (St.next)))
          (_o (- 0 (St.next)))))))
  (export main))
