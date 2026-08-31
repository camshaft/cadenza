(do
  (def (h (: xs (List Int64)) (: c Bool))
    (match xs
      ((list a .. t)
        (match (if c (Some a) (None))
          ((Some v) (+ (* v 10) a))
          ((None) a)))
      ((list) -1)))
  (def (main) (+ (* (h (list 7 2) true) 1000) (h (list 4 9) false)))
  (export main))
