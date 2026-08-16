(case "su2 a THREE-variant cyclic machine — the op arg selects the transition, both Mid exits exercised"
  (input  (do
            (type Gear (Lo) (Mid Int64) (HiG Int64))
            (effect G (op shift (-> Int64 Int64)))
            (def (main (: n Int64))
              (handle G (Lo)
                ((shift (v) s (match s
                                ((Lo) (resume 1 (Mid v)))
                                ((Mid k) (if (> v k) (resume (* 10 k) (HiG (+ k v))) (resume (- 0 k) (Lo))))
                                ((HiG k) (resume (* 100 k) (Lo))))))
                (+ (G.shift n) (+ (G.shift 4) (+ (G.shift 2) (G.shift 9))))))
            (export main)))
  (call   main (: 5 Int64)) (output (: 17 Int64))
  (call   main (: 1 Int64)) (output (: 512 Int64)))
