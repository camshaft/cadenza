(case "sb1b control: same rope build WITHOUT the handler (pure)"
  (input  (do
            (def (build (: n Int64) (: acc String))
              (if (= n 0) acc (build (- n 1) (String.concat acc "ab"))))
            (def (main (: n Int64))
              (String.scalar-len (build n "")))
            (export main)))
  (call   main (: 200 Int64)) (output (: 400 Int64)))
