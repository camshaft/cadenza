(case "mb2 String.at walks a multibyte rope state ACROSS the seam boundaries — each one-scalar read is the one the growth order placed there"
  (input  (do
            (effect S (op add (-> Int64 Int64)) (op pick (-> Int64 Int64)))
            (def (walk (: k Int64))
              (if (< k 1) 0 (let ((_d (S.add k))) (walk (- k 1)))))
            (def (main (: n Int64))
              (handle S ""
                ((add (v) s (resume 0 (String.concat s (if (= (% v 2) 0) "é" "z"))))
                 (pick (i) s
                  (resume (match (String.at s i)
                            ((Some c) (String.byte-len c))
                            ((None _u) -1))
                          s)))
                (let ((_w (walk n)))
                  (+ (* 100 (S.pick 0)) (+ (* 10 (S.pick 1)) (S.pick (- n 1)))))))
            (export main)))
  (call   main (: 4 Int64)) (output (: 211 Int64))
  (call   main (: 5 Int64)) (output (: 121 Int64)))
