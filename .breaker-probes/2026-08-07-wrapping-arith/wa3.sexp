(case "wa3 the wrap WALK — three draws from a MAX seed step MAX, MIN, MIN+1 through wrapping-add(+1), checked '+' beside"
  (input  (do
            (effect W (op cyc (-> Int64)))
            (def (main (: n Int64))
              (handle W 9223372036854775807
                ((cyc () s (resume s (Int64.wrapping-add s 1))))
                (let ((a (W.cyc)))
                  (let ((b (W.cyc)))
                    (let ((c (W.cyc)))
                      (+ (if (= a 9223372036854775807) 1 0)
                         (+ (if (= b -9223372036854775808) 10 0)
                            (+ (if (= c -9223372036854775807) 100 0) n))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 116 Int64))
  (call   main (: 0 Int64)) (output (: 111 Int64)))
