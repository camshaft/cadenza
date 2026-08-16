(case "by1 bytes built from THREE draws then read at a draw-picked index — Bytes.at follows the thread into the buffer"
  (input  (do
            (effect E (op next (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 7))))
                (let ((b (Bytes.of (list (UInt8.wrap (+ (% (E.next) 200) 56)) (UInt8.wrap (+ (% (E.next) 200) 56)) (UInt8.wrap (+ (% (E.next) 200) 56))))))
                  (let ((i (% (E.next) 3)))
                    (match (Bytes.at b i)
                      ((Some v) (+ (* 100 v) (+ (* 10 (Bytes.len b)) i)))
                      (None -1))))))
            (export main)))
  (call   main (: 0 Int64)) (output (: 5630 Int64))
  (call   main (: 1 Int64)) (output (: 6431 Int64))
  (call   main (: 2 Int64)) (output (: 7232 Int64)))
