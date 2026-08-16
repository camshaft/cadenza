(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next (u) s (resume s (+ s 1))))
      (let ((w (String.concat "x" (if (> (St.next) 4) "big" "sm"))))
        (let ((again (String.concat w w)))
          (+ (* 10 (String.byte-len again)) (String.byte-len w))))))
  (export main))
