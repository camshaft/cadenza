(do
  (effect Amb (op pick (-> Unit (List Int64))))
  (def (main (: n Int64))
    (handle Amb 0
      ((pick (u) s (+ (resume (list 4 2 9) s) (resume (list 7) s))))
      (let ((xs (Amb.pick)))
        (+ (* 10 (List.len xs)) (List.len xs)))))
  (export main))
