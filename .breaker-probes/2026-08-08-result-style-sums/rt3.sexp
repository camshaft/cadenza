(case "rt3 the Err VALUE seeds a RECOVERY region — a fallback same-effect handle runs inside the Err arm"
  (input  (do
            (type Res (Ok Int64) (Err Int64))
            (effect E (op go (-> Res)))
            (def (main (: n Int64))
              (handle E n
                ((go () s (resume (if (> s 3) (Ok (* s 10)) (Err (+ s 100))) s)))
                (match (E.go)
                  ((Ok v) v)
                  ((Err e) (handle E e
                             ((go () s (resume (Ok (+ s 1)) s)))
                             (match (E.go)
                               ((Ok v2) (* v2 2))
                               ((Err _e2) -1)))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 50 Int64))
  (call   main (: 2 Int64)) (output (: 206 Int64)))
