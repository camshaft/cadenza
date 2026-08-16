(case "c2" (input (do
  (effect St (op next (-> Int64)))
  (def (main (: n Int64))
    (handle St n
      ((next () s (resume s (* s 2))))
      (match (St.next)
        ((guard x (> x (St.next))) (* 100 x))
        (_o _o))))
  (export main)))
  (call main (: -4 Int64)) (output (: -400 Int64)))
