(case "vm1 a Bytes SLICE-VIEW as handler state, re-sliced narrower per perform (view-of-view state chain)"
  (input  (do
            (effect St (op narrow (-> Unit Int64)))
            (def (main (: n Int64))
              (handle St (Option.expect (Bytes.slice (Bytes.of (list 10 20 30 40 50)) 0 5) "in")
                ((narrow (u) s
                  (resume (Bytes.len s)
                          (match (Bytes.slice s 1 (- (Bytes.len s) 2))
                            ((Some w) w)
                            ((None _x) s)))))
                (+ (* 100 (St.narrow)) (+ (* 10 (St.narrow)) (St.narrow)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 531 Int64)))
