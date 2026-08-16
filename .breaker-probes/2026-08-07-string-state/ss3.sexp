(case "ss3 nested SAME-effect handlers with independent STRING states — inner self-doubles, outer appends"
  (input  (do
            (effect Log (op emit (-> Int64)))
            (def (main (: n Int64))
              (handle Log "aa"
                ((emit () s (resume (String.byte-len s) (String.concat s "b"))))
                (+ (Log.emit)
                   (+ (* 10 (handle Log "wxyz"
                              ((emit () t (resume (String.byte-len t) (String.concat t t))))
                              (+ (Log.emit) (Log.emit))))
                      (* 1000 (Log.emit))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 3122 Int64)))
