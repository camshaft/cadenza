(case "pytf1 probe: TAIL-resumptive arm with a BARE foreign perform in the next-state hole (tail sibling of pyfn1's two-hole foreign next-state)"
  (input (do
  (effect E (op tick (-> Int64)))
  (effect F (op aux (-> Int64)))
  (def (main (: n Int64))
    (handle F (: 0 Int64)
      ((aux () fs (resume (: 40 Int64) fs)))
      (handle E (% n 3)
        ((tick () s (resume (+ s 1) (F.aux))))
        (+ (E.tick) (* 10 (E.tick))))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 412 Int64))
  (call   main (: 0 Int64)) (output (: 411 Int64)))
