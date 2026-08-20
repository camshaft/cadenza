(case "pynd5 probe: the handled body calls a HELPER FUNCTION that performs the effect — (def (twice) (+ (E.tick) (E.tick))) is called from inside the handle body, so two of the three tick dispatches originate in a SEPARATE function frame yet must route to the enclosing handler and thread its state in call order (twice's two ticks then the direct tick)"
  (input (do
  (effect E (op tick (-> Int64)))
  (def (twice) (+ (E.tick) (E.tick)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((tick () s (resume (* s 10) (+ s 1))))
      (+ (twice) (* 1000 (E.tick)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 30030 Int64))
  (call   main (: 0 Int64)) (output (: 20010 Int64)))
