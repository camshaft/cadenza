(case "vm2 the narrowed view's CONTENT is right at each stage (byte-0 read per narrow, not just length)"
  (input  (do
            (effect St (op peek (-> Unit Int64)) (op narrow (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40 50)) 0 5) "in")
                ((peek (u) s (resume (match (Bytes.at s 0) ((Some v) (Int64.of v)) ((None _u) -1)) s))
                 (narrow (u) s
                  (resume 0
                          (match (Bytes.slice s 1 (- (Bytes.len s) 2))
                            ((Some w) w)
                            ((None _x) s)))))
                (+ (* 100 (St.peek)) (+ (* 0 (St.narrow)) (+ (* 10 (St.peek)) (+ (* 0 (St.narrow)) (St.peek)))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 1230 Int64)))
