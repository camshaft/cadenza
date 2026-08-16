(case "sg6 the rope's LENGTH PARITY routes its own growth — odd appends two, even appends one; four draws read 1,3,4,5"
  (input  (do
            (effect St (op step (-> Int64)))
            (def (main (: n Int64))
              (handle St "x"
                ((step () s (resume (String.byte-len s)
                                    (if (= (% (String.byte-len s) 2) 0)
                                        (String.concat s "a")
                                        (String.concat s "bb")))))
                (+ (St.step) (+ (* 10 (St.step)) (+ (* 100 (St.step)) (* 1000 (St.step)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 7531 Int64)))
