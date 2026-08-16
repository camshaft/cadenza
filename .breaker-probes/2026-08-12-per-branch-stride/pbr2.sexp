(case "pbr2 THREE-way match-branch resumes — each residue class answers and strides differently, three dispatches walk the classes"
  (input  (do
            (effect S (op step (-> Int64)))
            (def (main (: n Int64))
              (handle S n
                ((step () s
                  (match (% s 3)
                    (0 (resume (* s 100) (+ s 1)))
                    (1 (resume (- 0 s) (+ s 2)))
                    (_ (resume s (+ s 3))))))
                (+ (S.step) (+ (* 100 (S.step)) (* 100000 (S.step))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 29999900 Int64))
  (call   main (: 1 Int64)) (output (: -370001 Int64))
  (call   main (: 2 Int64)) (output (: 800502 Int64)))
