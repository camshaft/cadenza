(case "c1" (input (do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (* s 2))))
      (match (St.next)
        ((guard x (> x 100)) (* 100 x))
        (_o _o))))
  (export main)))
  (call main (: 3 Int64)) (output (: 3 Int64)))
