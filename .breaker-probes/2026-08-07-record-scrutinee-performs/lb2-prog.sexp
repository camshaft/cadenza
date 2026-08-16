(do
  (def (main (: n Int64))
    (let ((r (record (a 3) (b 4))))
      (match r ((record (a x) (b y)) (+ (* 10 x) y)))))
  (export main))
