(case "li3 draw PARITY picks prepend vs push while building — the final shape encodes the whole draw sequence"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (build (: k Int64) (: xs (List Int64)))
              (if (<= k 0)
                  xs
                  (let ((d (E.next)))
                    (build (- k 1) (if (= (% d 2) 0) (List.prepend xs d) (List.push xs d))))))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1))))
                (let ((xs (build 4 (list))))
                  (match (List.at xs 0)
                    ((Some h) (match (List.at xs 3)
                      ((Some t) (+ (* 100 h) (+ (* 10 t) (List.len xs))))
                      ((None) -1)))
                    ((None) -1)))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 234 Int64))
  (call   main (: 1 Int64)) (output (: 434 Int64))
  (call   main (: -4 Int64)) (output (: -206 Int64)))
