(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((r (record (a 3) (b 4))))
        (+ (* 10 (. r a)) (. r b)))))
  (export main))
