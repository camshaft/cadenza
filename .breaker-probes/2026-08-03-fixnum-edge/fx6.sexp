(case "fx6 a fixnum-IMMEDIATE and a BOXED Int64 sharing one full hash occupy one collision node"
  (input  (do
            (def (main (: z Int64))
              (let ((s (Set.of (list (+ z 134198331) (+ z 536870917)))))
                (+ (* 100 (Set.len s))
                   (+ (* 10 (if (Set.contains s 134198332) 1 0))
                      (if (Set.contains s 536870918) 1 0)))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 211 Int64)))
