(case "cel1 a RULE-NINETY cellular automaton in one Int64 byte — step XORs the left-shifted and right-shifted worlds masked to eight bits answering the new generation, density popcounts between steps, and the seeds' initial worlds diverge into different orbits with different densities"
  (input  (do
            (effect C
              (op step (-> Int64))
              (op density (-> Int64)))
            (def (bits (: b Int64) (: acc Int64))
              (if (= b 0) acc (bits (>> b 1) (+ acc (& b 1)))))
            (def (main (: n Int64))
              (handle C (+ (% n 16) 4)
                ((step () w
                  (resume (^ (& (>> w 1) 255) (& (<< w 1) 255))
                          (^ (& (>> w 1) 255) (& (<< w 1) 255))))
                 (density () w (resume (bits w 0) w)))
                (let ((a (C.step)))
                  (let ((b (C.density)))
                    (let ((c (C.step)))
                      (let ((d (C.step)))
                        (let ((e (C.density)))
                          (let ((f (C.step)))
                            (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 (+ (* 1000 a) b)) c)) d)) e)) f)))))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 27004059107005227 Int64))
  (call   main (: 0 Int64)) (output (: 10002017042003065 Int64)))
