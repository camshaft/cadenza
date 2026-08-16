(do
  (effect Acc (op feed (-> Int64 Int64)) (op tally (-> Unit Int64)))
  (def (main (: a Int64))
    (handle Acc (list)
      ((feed (v) s (if (> v 10) (resume v (List.push s v)) (resume 0 s)))
       (tally (u) s (resume (List.len s) s)))
      (+ (Acc.feed 20) (+ (Acc.feed a) (+ (Acc.feed 30) (* 1000 (Acc.tally)))))))
  (export main))
