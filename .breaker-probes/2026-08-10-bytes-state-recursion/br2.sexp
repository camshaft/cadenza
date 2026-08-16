(case "br2 the arm returns a SLICE of the growing Bytes state — a window over rope-accumulated bytes crosses dispatch"
  (input  (do
            (effect Acc
              (op push (-> Int64 Bytes)))
            (def (at-or (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) v) ((None u) -1)))
            (def (main (: n Int64))
              (handle Acc (Bytes.of (list (UInt8.wrap 5) (UInt8.wrap 6)))
                ((push (v) s
                  (let ((grown (Bytes.concat s (Bytes.of (list (UInt8.wrap v))))))
                    (match (Bytes.slice grown 1 (- (Bytes.len grown) 1))
                      ((Some w) (resume w grown))
                      ((None u) (resume grown grown))))))
                (let ((w1 (Acc.push 40)))
                  (let ((w2 (Acc.push 50)))
                    (+ (* 100000 (Bytes.len w1))
                       (+ (* 10000 (at-or w1 0))
                          (+ (* 100 (Bytes.len w2))
                             (+ (* 10 (at-or w2 0)) (at-or w2 (- (Bytes.len w2) 1))))))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 260410 Int64)))
