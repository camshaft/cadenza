(case "py3w1 probe: a THREE-WAY branch on the SIGN of (op-arg minus captured state) — cmd(v) scales by 10 when v>s, by 100 when v<s, answers zero when equal, threading a different advance per branch; two dispatches with different args cross the state boundary so both the greater and the equal/less arms fire across seeds"
  (input (do
  (effect E (op cmd (-> Int64 Int64)))
  (def (main (: n Int64))
    (handle E (% n 3)
      ((cmd (v) s
        (if (> v s) (resume (* v 10) (+ s 1))
            (if (< v s) (resume (* v 100) (+ s 2))
                (resume (: 0 Int64) (+ s 3))))))
      (+ (* 1000 (E.cmd 5)) (E.cmd 1))))
  (export main)))
  (call   main (: 10 Int64)) (output (: 50100 Int64))
  (call   main (: 0 Int64)) (output (: 50000 Int64)))
