(case "ab4 TWO sequential abort regions under one STRING-state outer — an aborted region leaves the rope where it was"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L "q"
                ((emit () s (resume (String.byte-len s) (String.concat s "z"))))
                (+ (handle Bail 0
                     ((bail (v) s v))
                     (let ((g (if (> n 3) (Bail.bail 50) 0)))
                       (+ g (L.emit))))
                   (* 100 (handle Bail 0
                            ((bail (v) s v))
                            (let ((h (if (> n 100) (Bail.bail 7) 0)))
                              (+ h (L.emit))))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 150 Int64))
  (call   main (: 0 Int64)) (output (: 201 Int64))
  (call   main (: 200 Int64)) (output (: 750 Int64)))
