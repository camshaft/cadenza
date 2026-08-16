(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (* s 2))))
      (let ((k (St.next)))
        (match k
          ((guard _a (> _a 50)) 111)
          ((guard _b (> _b 10)) 222)
          (_o (- 0 (St.next)))))))
  (export main))
