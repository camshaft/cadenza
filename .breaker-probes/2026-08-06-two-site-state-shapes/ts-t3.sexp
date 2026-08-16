(do
  (effect St (op feed (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle St 0
      ((feed (v) s (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
      (+ (St.feed 20) (+ (St.feed n) (St.feed 30)))))
  (export main))
