(case "sr3 the Err reason is BUILT in the arm — String.concat of a prefix and a sign-picked tag, matched against literals in the body"
  (input  (do
            (type Res (Ok Int64) (Err String))
            (effect E (op step (-> Res)))
            (def (main (: n Int64))
              (handle E n
                ((step () s
                  (resume (if (= (% s 2) 0)
                              (Res.Ok s)
                              (Res.Err (String.concat "e-" (if (< s 0) "lo" "hi"))))
                          (+ s 3))))
                (let ((score (fn ((: r Res))
                               (match r
                                 ((Res.Ok v) v)
                                 ((Res.Err why) (if (= why "e-lo") 3 9))))))
                  (+ (* 10 (score (E.step))) (score (E.step))))))
            (export main)))
  (call   main (: 1 Int64)) (output (: 94 Int64))
  (call   main (: -3 Int64)) (output (: 30 Int64))
  (call   main (: -4 Int64)) (output (: -37 Int64)))
