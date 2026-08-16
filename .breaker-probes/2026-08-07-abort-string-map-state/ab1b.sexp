(do
  (effect L (op emit (-> Int64)))
  (effect Bail (op bail (-> Int64 Int64)))
  (def (main (: n Int64))
    (+ (handle Bail 0
         ((bail (v) s v))
         (handle L 10
           ((emit () s (resume s (+ s 1))))
           (do
             (L.emit)
             (let ((g (if (> n 3) (Bail.bail 99) 0)))
               (+ g (+ (L.emit) 500))))))
       (* 1000 n)))
  (export main))
