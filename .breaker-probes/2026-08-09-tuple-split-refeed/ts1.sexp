(case "ts1 a tuple-returning op SPLIT-refed — each destructured half feeds a DIFFERENT later op against advancing state"
  (input  (do
            (effect E (op split (-> (Tuple Int64 Int64)))
                      (op mixa (-> Int64 Int64))
                      (op mixb (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle E n
                ((split () s (resume (tuple s (* 2 s)) (+ s 1)))
                 (mixa (a) s (resume (+ a s) (+ s 2)))
                 (mixb (b) s (resume (* b s) s)))
                (match (E.split)
                  ((tuple a b) (+ (E.mixa a) (E.mixb b))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 91 Int64))
  (call   main (: 0 Int64)) (output (: 1 Int64))
  (call   main (: -3 Int64)) (output (: -5 Int64)))
