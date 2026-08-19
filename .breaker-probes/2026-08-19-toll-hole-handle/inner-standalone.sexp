(case "inner-standalone: is the nested handle really 42"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (main (: n Int64))
    (handle E (: 40 Int64)
      ((tick () t (resume t (+ t 1))))
      (+ (E.tick) 2)))
  (export main)))
  (call main (: 10 Int64)) (output (: 42 Int64)))
