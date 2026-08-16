(case "sd3 a draw-bounded String.slice window — start and end both come from the thread, byte-len pins the window width"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 2))))
                (let ((st (% (E.next) 4)))
                  (let ((en (+ st (+ (% (E.next) 3) 1))))
                    (match (String.slice "abcdefgh" st en)
                      ((Some w) (+ (* 100 (String.byte-len w)) (+ (* 10 st) (- en st))))
                      ((None _u) -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 303 Int64))
  (call   main (: 1 Int64)) (output (: 111 Int64))
  (call   main (: 5 Int64)) (output (: 212 Int64)))
