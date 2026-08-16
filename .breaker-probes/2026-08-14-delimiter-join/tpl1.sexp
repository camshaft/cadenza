(case "tpl1 a DELIMITER-JOIN rope builder — the first fragment lands bare, every later one is prefixed with the separator, the empty-vs-seeded start decides whether the FIRST dispatch takes the bare branch, and multibyte fragments keep the byte-length answers honest"
  (input  (do
            (effect J (op sep (-> String Int64)))
            (def (main (: n Int64))
              (handle J (if (= n 0) "" "id")
                ((sep (frag) s
                  (if (= (String.byte-len s) 0)
                      (resume (String.byte-len frag) frag)
                      (let ((r (String.concat (String.concat s ";") frag)))
                        (resume (String.byte-len r) r)))))
                (let ((a (J.sep "ab")))
                  (let ((b (J.sep "é")))
                    (let ((c (J.sep "xyz")))
                      (let ((d (J.sep "é")))
                        (+ (* 100 (+ (* 100 (+ (* 100 a) b)) c)) d)))))))
            (export main)))
  (call   main (: 10 Int64)) (output (: 5081215 Int64))
  (call   main (: 0 Int64)) (output (: 2050912 Int64)))
