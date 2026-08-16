(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((base (St.next)))
        (+ (if (> base 3) (* base 10) 0)
           (if (> base 3) (* base 10) 0)))))
  (export main))
