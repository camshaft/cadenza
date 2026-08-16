(case "pm1 a @param accessor read inside a RECURSIVE walk (one response per iteration)"
  (input  (do
            (pragma param (param (: widget slider)) (: gain Int64))
            (def (walk (: n Int64) (: acc Int64))
              (if (= n 0) acc (walk (- n 1) (+ (* 10 acc) (Param.gain)))))
            (def (main)
              (host (Param) (walk 3 0)))
            (export main)))
  (call   main)
  (host-responses (respond Param.gain (: 3 Int64))
                  (respond Param.gain (: 7 Int64))
                  (respond Param.gain (: 5 Int64)))
  (output (: 375 Int64)))
