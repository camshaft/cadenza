(case "fx1 a Float64 state with a REGIME SWITCH in the arm — doubling below the threshold, halving above, three draws sum the trajectory"
  (input  (do
            (effect E (op draw (-> Float64)))
            (def (main (: n Int64))
              (handle E (+ 1.0 (Float64.of-int n))
                ((draw () s (resume s (if (< s 10.0) (* s 2.0) (* s 0.5)))))
                (+ (E.draw) (+ (E.draw) (E.draw)))))
            (export main)))
  (call   main (: 2 Int64)) (output (: 21.0 Float64))
  (call   main (: 0 Int64)) (output (: 7.0 Float64))
  (call   main (: 5 Int64)) (output (: 24.0 Float64)))
