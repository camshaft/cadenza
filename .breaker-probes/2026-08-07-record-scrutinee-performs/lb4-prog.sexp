(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (do
        (St.next)
        (let ((r (record (a 3) (b 4))))
          (match r ((record (a x) (b y)) (+ (* 10 x) y)))))))
  (export main))
