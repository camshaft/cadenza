(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (let ((f (fn ((: k Int64))
               (handle St k
                 ((next (u) s (resume s (+ s 1))))
                 (+ (St.next) (St.next))))))
      (+ (* 100 (f n)) (f 20))))
  (export main))
