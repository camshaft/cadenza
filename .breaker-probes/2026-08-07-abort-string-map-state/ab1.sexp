(do
  (effect L (op emit (-> Int64)))
  (effect Bail (op bail (-> Int64 Int64)))
  (def (main (: n Int64))
    (+ (handle Bail 0
         ((bail (v) s v))
         (handle L "x"
           ((emit () s (resume (String.byte-len s) (String.concat s "yz"))))
           (do
             (L.emit)
             (let ((g (if (> n 3) (Bail.bail 99) 0)))
               (+ g (+ (L.emit) 500))))))
       (* 1000 n)))
  (export main))
