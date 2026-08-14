(case "tplM the tpl1 arm with MIXED dispatches — a second blen op interleaves with sep draws, the String-valued dual-use let now sits in a mixed-op region"
  (input  (do
            (effect J
              (op sep (-> String Int64))
              (op blen (-> Int64)))
            (def (main (: n Int64))
              (handle J (if (= n 0) "" "id")
                ((sep (frag) s
                  (if (= (String.byte-len s) 0)
                      (resume (String.byte-len frag) frag)
                      (let ((r (String.concat (String.concat s ";") frag)))
                        (resume (String.byte-len r) r))))
                 (blen () s (resume (String.byte-len s) s)))
                (let ((a (J.sep "ab")))
                  (let ((b (J.blen)))
                    (let ((c (J.sep "xyz")))
                      (let ((d (J.blen)))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5050909 Int64))
  (call   main (: 0 Int64)) (output (: 2020606 Int64)))
