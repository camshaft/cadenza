(case "dh1 a FIVE-deep handler stack, innermost body performs each layer once (state isolation × depth)"
  (input  (do
            (effect E1 (op o1 (-> Unit Int64)))
            (effect E2 (op o2 (-> Unit Int64)))
            (effect E3 (op o3 (-> Unit Int64)))
            (effect E4 (op o4 (-> Unit Int64)))
            (effect E5 (op o5 (-> Unit Int64)))
            (def (main (: n Int64))
              (handle E1 1
                ((o1 (u) s (resume s s)))
                (handle E2 20
                  ((o2 (u) s (resume s s)))
                  (handle E3 300
                    ((o3 (u) s (resume s s)))
                    (handle E4 4000
                      ((o4 (u) s (resume s s)))
                      (handle E5 50000
                        ((o5 (u) s (resume s s)))
                        (+ (E1.o1) (+ (E2.o2) (+ (E3.o3) (+ (E4.o4) (E5.o5)))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 54321 Int64)))
