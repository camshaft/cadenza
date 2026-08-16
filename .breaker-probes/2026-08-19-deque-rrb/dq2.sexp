(case "dq2 the deque-built list equals a DIRECTLY-ordered build (assembly-order independence)"
  (input  (do
            (def (build (: i Int64) (: acc (List Int64)))
              (if (= i 0) acc
                (build (- i 1)
                  (if (= (% i 2) 0)
                      (List.concat acc (list i))
                      (List.concat (list i) acc)))))
            (def (aseg (: lo Int64) (: hi Int64) (: step Int64) (: acc (List Int64)))
              (if (> lo hi) acc (aseg (+ lo step) hi step (List.push acc lo))))
            (def (dseg (: hi Int64) (: lo Int64) (: step Int64) (: acc (List Int64)))
              (if (< hi lo) acc (dseg (- hi step) lo step (List.push acc hi))))
            (def (main (: n Int64))
              (do
                (def deq (build n (list)))
                (def direct (List.concat (aseg 1 59 2 (list)) (dseg 60 2 2 (list))))
                (+ (* 10 (if (= deq direct) 1 0))
                   (if (= (List.len direct) n) 1 0))))
            (export main)))
  (call   main (: 60 Int64)) (output (: 11 Int64)))
