(case "od2 odd-width remainder takes the dividend sign and min % -1 is zero not a trap"
  (input  (do
            (def (main (: k Int64))
              (+ (* 100 (Int64.of (% ((. (Int 24) wrap) -7) ((. (Int 24) wrap) 2))))
                 (Int64.of (% ((. (Int 24) wrap) -8388608) ((. (Int 24) wrap) k)))))
            (export main)))
  (call   main (: -1 Int64)) (output (: -100 Int64)))
