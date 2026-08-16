(do
  (effect St (op next (-> Unit Int64)))
  (def (main (: n Int64))
    (let ((total (handle St n
                   ((next (u) s (resume s (+ s 1))))
                   (+ (St.next) (St.next)))))
      (handle St total
        ((next (u) s (resume s (* s 2))))
        (+ (St.next) (St.next)))))
  (export main))
