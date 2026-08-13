(case "het1 ONE effect with THREE result types — Int64, Bool, and String ops share a single scalar thread, each consumed by its own idiom in one run"
  (input  (do
            (effect S
              (op num (-> Int64))
              (op flag (-> Bool))
              (op name (-> String)))
            (def (main (: n Int64))
              (handle S n
                ((num () s (resume (* s 2) (+ s 1)))
                 (flag () s (resume (= (% s 2) 0) (+ s 1)))
                 (name () s (resume (if (= (% s 3) 0) "hi" "wxyz") (+ s 1))))
                (let ((a (S.num)))
                  (let ((b (S.flag)))
                    (let ((c (S.name)))
                      (let ((d (S.num)))
                        (+ (* 100000 a)
                           (+ (* 10000 (if b 1 0))
                              (+ (* 1000 (String.byte-len c)) d)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 614012 Int64))
  (call   main (: 4 Int64)) (output (: 802014 Int64)))
