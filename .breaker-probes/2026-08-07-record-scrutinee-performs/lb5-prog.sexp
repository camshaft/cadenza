(do
  (effect St (op next (-> Unit Int64)))
  (def (f (: r (Record (a Int64) (b Int64))))
    (match r ((record (a x) (b y)) (+ (* 10 x) y))))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (f (record (a 3) (b 4)))))
  (export main))
