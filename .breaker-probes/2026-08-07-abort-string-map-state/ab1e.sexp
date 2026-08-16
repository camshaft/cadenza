(case "ab1e a branch-conditional abort under a STRING-state outer handler — the taken abort skips the post-abort emit, the untaken row grows the rope"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (+ (handle L "x"
                   ((emit () s (resume (String.byte-len s) (String.concat s "yz"))))
                   (handle Bail 0
                     ((bail (v) s v))
                     (do
                       (L.emit)
                       (let ((g (if (> n 3) (Bail.bail 99) 0)))
                         (+ g (+ (L.emit) 500))))))
                 (* 1000 n)))
            (export main)))
  (call   main (: 5 Int64)) (output (: 5099 Int64))
  (call   main (: 0 Int64)) (output (: 503 Int64)))
