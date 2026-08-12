(case "rsl1 a Result whose Ok payload is a HEAP LIST snapshot of the state — the first snap Errs on the empty list, the second Oks the grown snapshot, both cross resume"
  (input  (do
            (effect S
              (op snap (-> (Result (List Int64) Int64)))
              (op push (-> Int64 Int64)))
            (def (score (: r (Result (List Int64) Int64)))
              (match r
                ((Ok xs) (+ (* 10 (List.len xs)) (match (List.at xs 0) ((Some h) h) ((None u) 0))))
                ((Err e) (* e -1))))
            (def (main (: n Int64))
              (handle S (list)
                ((snap () xs
                  (resume (if (= (List.len xs) 0)
                              (: (Err 7) (Result (List Int64) Int64))
                              (Ok xs))
                          xs))
                 (push (v) xs (resume (List.len xs) (List.push xs v))))
                (let ((a (score (S.snap))))
                  (let ((_p (S.push n)))
                    (let ((_q (S.push (+ n 1))))
                      (let ((b (score (S.snap))))
                        (+ (* 100 a) b)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: -677 Int64))
  (call   main (: 50 Int64)) (output (: -630 Int64)))
