(do
  (effect Acc (op feed (-> Int64 Int64)) (op size (-> Unit Int64)))
  (def (main (: a Int64))
    (handle Acc Map.empty
      ((feed (v) s (if (> v 10) (resume v (Map.insert s v v)) (resume 0 s)))
       (size (u) s (resume (Map.len s) s)))
      (+ a (+ (Acc.feed 20) (+ (Acc.feed 3) (+ (Acc.feed 30) (* 1000 (Acc.size))))))))
  (export main))
