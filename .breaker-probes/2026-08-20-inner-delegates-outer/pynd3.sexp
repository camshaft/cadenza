(case "pynd3 probe: nested handlers of TWO effects where the INNER handler's arm performs the OUTER effect — Out.oo answers (* s 10) threading (+ s 1), In.ii answers (+ t (Out.oo)) threading (+ t 1); each In.ii dispatch performs a fresh Out.oo that must route past the inner In handler to the outer Out handler and thread Out's state INDEPENDENTLY, so the two Out.oo calls see s0 then s0+1 (a cross-effect delegation the tail fold must not shadow or double-count)"
  (input (do
  (effect Out (op oo (-> Int64)))
  (effect In (op ii (-> Int64)))
  (def (main (: n Int64))
    (handle Out (% n 3)
      ((oo () s (resume (* s 10) (+ s 1))))
      (handle In (: 100 Int64)
        ((ii () t (resume (+ t (Out.oo)) (+ t 1))))
        (+ (In.ii) (In.ii)))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 231 Int64))
  (call   main (: 0 Int64)) (output (: 211 Int64)))
