(case "mns1 a MIN-TRACKING stack state — (stack, min) pushes thread the heap and the scalar together, mid-run and final min reads"
  (input  (do
            (effect S (op push (-> Int64 Int64)) (op mn (-> Int64)))
            (def (main (: n Int64))
              (handle S (tuple (list) 9999)
                ((push (v) st
                  (match st
                    ((tuple stk mn)
                      (resume (List.len stk)
                              (tuple (List.push stk v) (if (< v mn) v mn))))))
                 (mn () st (match st ((tuple _stk m) (resume m st)))))
                (let ((_a (S.push 5)))
                  (let ((_b (S.push n)))
                    (let ((m1 (S.mn)))
                      (let ((_c (S.push 1)))
                        (+ (* 100 m1) (S.mn))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 301 Int64))
  (call   main (: 8 Int64)) (output (: 501 Int64)))
