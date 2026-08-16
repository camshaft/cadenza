(do
  (effect St (op rank (-> Int64 Int64)) (op peek (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St 0
      ((rank (v) s
        (if (> v 20) (resume (* v 10) (+ s 100))
          (if (> v 10) (resume v (+ s 1)) (resume 0 s))))
       (peek (u) s (resume s s)))
      (+ (St.rank 25) (+ (St.peek) (St.rank 15)))))
  (export main))
