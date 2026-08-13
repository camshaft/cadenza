(case "pfxG finding-23 sibling: the SAME shape over a BYTES state — computed-index Bytes.at read + concat-append, three dispatches"
  (input  (do
            (effect S (op add (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle S (Bytes.of (list (UInt8.wrap 5)))
                ((add (v) bs
                  (let ((last (match (Bytes.at bs (- (Bytes.len bs) 1)) ((Some x) x) ((None u) 0))))
                    (let ((t (% (+ last v) 256)))
                      (resume t (Bytes.concat bs (Bytes.of (list (UInt8.wrap t)))))))))
                (let ((a (S.add n)))
                  (let ((b (S.add 4)))
                    (let ((c (S.add 9)))
                      (+ (* 100000 a) (+ (* 100 b) c)))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 801221 Int64))
  (call   main (: 60 Int64)) (output (: 6506978 Int64)))
