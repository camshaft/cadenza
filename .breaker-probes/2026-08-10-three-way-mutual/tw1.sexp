(case "tw1 a THREE-function mutual cycle where each leg draws once — the SCC multi-value fold at width 3"
  (input  (do
            (effect St (op get (-> Int64)))
            (def (fa (: k Int64))
              (if (= k 0) (St.get) (+ (* 2 (St.get)) (fb (- k 1)))))
            (def (fb (: k Int64))
              (if (= k 0) (St.get) (+ (* 3 (St.get)) (fc (- k 1)))))
            (def (fc (: k Int64))
              (if (= k 0) (St.get) (+ (* 5 (St.get)) (fa (- k 1)))))
            (def (main (: n Int64))
              (handle St n
                ((get () s (resume s (+ s 1))))
                (+ (fa 4) (* 1000 (St.get)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 7049 Int64))
  (call   main (: 0 Int64)) (output (: 5023 Int64)))
