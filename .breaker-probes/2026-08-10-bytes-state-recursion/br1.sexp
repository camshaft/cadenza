(case "br1 a BYTES handler state grows one byte per recursive dispatch — the final frame's length and bytes pin the accumulation"
  (input  (do
            (effect Acc
              (op push (-> Int64 Int64))
              (op dump (-> Bytes)))
            (def (walk (: k Int64))
              (if (= k 0)
                  0
                  (match (Acc.push k) (_ (walk (- k 1))))))
            (def (at-or (: b Bytes) (: i Int64))
              (match (Bytes.at b i) ((Some v) v) ((None u) -1)))
            (def (main (: n Int64))
              (handle Acc (Bytes.of (list))
                ((push (v) s (resume (Bytes.len s) (Bytes.concat s (Bytes.of (list (UInt8.wrap (+ 60 v)))))))
                 (dump () s (resume s s)))
                (match (walk n)
                  (_ (let ((b (Acc.dump)))
                       (+ (* 1000 (Bytes.len b))
                          (+ (* 10 (at-or b 0))
                             (at-or b (- (Bytes.len b) 1)))))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 3691 Int64)))
