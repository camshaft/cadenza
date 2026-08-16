(case "sb1 a string-builder fold at 60 appends then measured + indexed at multibyte positions"
  (input  (do
            (def (build (: i Int64) (: acc String))
              (if (= i 0) acc
                  (build (- i 1) (String.concat acc (if (= (% i 3) 0) "é" "x")))))
            (def (main (: n Int64))
              (do
                (def s (build n ""))
                (+ (* 100 (String.scalar-len s))
                   (- (String.byte-len s) (String.scalar-len s)))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 6020 Int64)))
