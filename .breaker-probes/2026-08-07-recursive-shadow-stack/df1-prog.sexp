(do
  (effect St (op depth (-> Unit Int64)))
  (def (walk (: k Int64))
    (if (= k 0)
        (St.depth)
        (handle St k
          ((depth (u) s (resume s s)))
          (walk (- k 1)))))
  (def (main (: n Int64))
    (handle St 100
      ((depth (u) s (resume s s)))
      (walk 3)))
  (export main))
