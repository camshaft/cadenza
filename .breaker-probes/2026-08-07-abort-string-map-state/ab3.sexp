(case "ab3 the abort VALUE is itself a draw from the outer STRING-state handler — the pre-abort dispatch commits before the unwind"
  (input  (do
            (effect L (op emit (-> Int64)))
            (effect Bail (op bail (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle L "abc"
                ((emit () s (resume (String.byte-len s) (String.concat s s))))
                (handle Bail 0
                  ((bail (v) s v))
                  (let ((g (if (> n 3) (Bail.bail (L.emit)) 0)))
                    (+ g (* 10 (L.emit)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 3 Int64))
  (call   main (: 0 Int64)) (output (: 30 Int64)))
