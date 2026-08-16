(do
  (effect Acc (op feed (-> Int64 Int64)))
  (def (main (: a Int64))
    (handle Acc (list)
      ((feed (v) s (if (> v 10) (resume v (List.push s v)) (resume 0 s))))
      (+ a (+ (Acc.feed 20) (+ (Acc.feed 3) (Acc.feed 30))))))
  (export main))
