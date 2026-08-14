(case "led1 BULLY leader election — registrations grow the candidate set, elect scans for the highest id, and deregistering the ELECTED leader (its id threaded through the body) forces re-election from the survivors"
  (input  (do
            (effect S
              (op reg (-> Int64 Int64))
              (op elect (-> Int64))
              (op dereg (-> Int64 Int64)))
            (def (maxid (: xs (List Int64)) (: i Int64) (: best Int64))
              (match (List.at xs i)
                ((Some v) (maxid xs (+ i 1) (if (> v best) v best)))
                ((None u) best)))
            (def (main (: n Int64))
              (handle S (Set.of (list 0))
                ((reg (k) c
                  (let ((c2 (Set.insert c k)))
                    (resume (- (Set.len c2) 1) c2)))
                 (elect () c (resume (maxid (Set.to-list c) 0 -1) c))
                 (dereg (k) c
                  (let ((c2 (Set.remove c k)))
                    (resume (- (Set.len c2) 1) c2))))
                (let ((a (S.reg 5)))
                  (let ((b (S.reg n)))
                    (let ((c (S.reg 2)))
                      (let ((d (S.elect)))
                        (let ((e (S.dereg d)))
                          (let ((f (S.elect)))
                            (+ (* 100 (+ (* 10 (+ (* 100 (+ (* 10 (+ (* 10 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 9 Int64)) (output (: 12309205 Int64))
  (call   main (: 3 Int64)) (output (: 12305203 Int64)))
