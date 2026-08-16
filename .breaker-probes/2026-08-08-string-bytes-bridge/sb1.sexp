(case "sb1 String.to-bytes of a draw-picked string read at a draw index — the text-to-bytes bridge follows the thread twice"
  (input  (do
            (effect E (op next (-> Int64)) (op probe (-> Int64)))
            (def (main (: n Int64))
              (handle E n
                ((next () s (resume s (+ s 1)))
                 (probe () s (resume s s)))
                (let ((b (String.to-bytes (if (= (% (E.next) 2) 0) "abc" "wxyz"))))
                  (let ((i (% (E.next) (Bytes.len b))))
                    (match (Bytes.at b i)
                      ((Some v) (+ (* 10 v) (- (E.probe) n)))
                      (None -1))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 992 Int64))
  (call   main (: 3 Int64)) (output (: 1192 Int64))
  (call   main (: 0 Int64)) (output (: 982 Int64)))
