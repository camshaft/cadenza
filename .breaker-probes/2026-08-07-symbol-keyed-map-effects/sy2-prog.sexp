(do
  (effect St (op label (-> Unit Symbol)))
  (def (main (: n Int64))
    (handle St 0
      ((label (u) s (resume (Symbol.of (if (> s 0) "warm" "cold")) (+ s 1))))
      (let ((xs (Set.of (list (St.label) (St.label) (St.label)))))
        (Set.len xs))))
  (export main))
