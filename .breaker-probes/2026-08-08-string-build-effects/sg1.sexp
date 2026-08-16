(case "sg1 a string BUILT by a recursive walk of draws — one H/L character per dispatch, exact content compared"
  (input  (do
            (effect St (op next (-> Int64)))
            (def (build (: d Int64) (: acc String))
              (if (<= d 0)
                  acc
                  (build (- d 1) (String.concat acc (if (> (St.next) 4) "H" "L")))))
            (def (main (: n Int64))
              (handle St n
                ((next () s (resume s (+ s 2))))
                (let ((w (build 3 "")))
                  (if (= w "LHH") 1 (if (= w "HHH") 2 (if (= w "LLH") 3 0))))))
            (export main)))
  (call   main (: 3 Int64)) (output (: 1 Int64))
  (call   main (: 5 Int64)) (output (: 2 Int64))
  (call   main (: 1 Int64)) (output (: 3 Int64)))
