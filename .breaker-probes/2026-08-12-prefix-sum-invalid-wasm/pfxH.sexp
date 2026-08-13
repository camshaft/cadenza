(case "pfxH finding-23 sibling: LIST state, computed-index read via List.update (not at) + push, three dispatches"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (list 7)
                ((add (v) pre
                  (let ((i (- (List.len pre) 1)))
                    (let ((up (List.update pre i v)))
                      (resume (List.len up) (List.push up v))))))
                (let ((a (S.add n)))
                  (let ((b (S.add 4)))
                    (let ((c (S.add 9)))
                      (+ (* 100 a) (+ (* 10 b) c)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 123 Int64)))
