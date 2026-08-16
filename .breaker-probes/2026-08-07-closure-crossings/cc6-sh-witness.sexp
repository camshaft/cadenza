(do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (+ s 1))))
      (let ((a (St.next)))
        (let ((f (fn ((: x Int64)) (+ x (* a 100)))))
          (+ (handle St 50
               ((next () t (resume t (* t 2))))
               (f (St.next)))
             (f (St.next)))))))
  (export main))
