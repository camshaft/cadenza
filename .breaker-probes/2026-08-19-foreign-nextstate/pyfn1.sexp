(case "pyfn1 probe: BARE foreign perform in a TWO-HOLE arm's NEXT-STATE hole (not a nested handle) — should FOLD unlike a nested handle in next-state"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op aux (-> Int64)))
  (def (main (: n Int64))
    (handle F (: 0 Int64)
      ((aux () fs (resume (: 40 Int64) fs)))
      (handle E (% n 3)
        ((tick () s (+ (resume (+ s 1) (F.aux)) (* 1000 s))))
        (+ (E.tick) (* 10 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 41412 Int64))
  (call   main (: 0 Int64)) (output (: 40411 Int64)))
