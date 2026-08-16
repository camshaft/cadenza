(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((d (Qty.of (St.next) (Unit.base #"meter"))))
        (let ((t (Qty.of 2 (Unit.base #"second"))))
          (Qty.value (* d t))))))
  (export main))
