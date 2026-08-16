(do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (record (cnt n) (tot 0))
      ((tick () st (resume (. st cnt)
                           (record (cnt (+ (. st cnt) 1))
                                   (tot (+ (. st tot) 1))))))
      (E.tick)))
  (export main))
